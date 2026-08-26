//! The audio graph: a serializable description (`GraphDesc`) and the compiled,
//! RT-ready form (`CompiledGraph`).
//!
//! Compilation instantiates nodes, prepares them, validates the connections
//! (bounds and port signal types), and computes a flat topological order.
//! Output buffers are indexed by topological position so a node always reads
//! buffers that come before its own — `split_at_mut` then hands out the sources
//! (read) and the node's output (write) without `unsafe`.

use serde::{Deserialize, Serialize};

use crate::engine::mix::TrackMix;
use crate::engine::node::{AudioNode, NodeIdx, NoteEvent, ParamId, ProcessCtx, SignalKind};
use crate::telemetry::frame::Meter;

/// Pre-allocated capacity of each node's per-block note queue. Notes beyond
/// this in a single block are dropped rather than reallocating. // RT-SAFE
const NOTE_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDesc {
    /// Stable instance id (from `song.json`); the key that matches a node across
    /// a hot-swap so its live state is carried over. An id always maps to the
    /// same `module_id`.
    #[serde(default)]
    pub id: String,
    pub module_id: String,
    /// Initial parameter values, normalized 0..1.
    #[serde(default)]
    pub params: Vec<(ParamId, f32)>,
    /// Non-parameter setup (sample kit directories, file paths…), passed to
    /// [`AudioNode::configure`] at build time. Empty for most modules.
    #[serde(default)]
    pub config: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub src_node: NodeIdx,
    pub src_port: usize,
    pub dst_node: NodeIdx,
    pub dst_port: usize,
}

/// Serializable graph description (the future shape of `song.json` chains).
///
/// The graph has no master/output node: the mixer stage (`engine::mix`) reads
/// each track's chain-tail buffers (one per channel) to produce the interleaved
/// device signal, applying the per-track and master settings carried here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDesc {
    pub nodes: Vec<NodeDesc>,
    pub connections: Vec<Connection>,
    /// The last node of each track's chain, in track order — the signals the
    /// mixer stage consumes.
    #[serde(default)]
    pub tails: Vec<NodeIdx>,
    /// Per-track mixer settings, aligned with `tails`.
    #[serde(default)]
    pub track_mix: Vec<TrackMix>,
    /// Master gain (dB) the mixer applies after the track sum.
    #[serde(default)]
    pub master_gain_db: f32,
    /// Master bus chain: node indices run IN THIS ORDER on the mixed bus by
    /// [`CompiledGraph::process_master`], after the track sum and before the
    /// master gain. These nodes carry no connections and are excluded from the
    /// topological walk of `process_block`.
    #[serde(default)]
    pub master: Vec<NodeIdx>,
    /// Directory node `config` paths resolve against (the project directory).
    /// Empty when the graph has no file-backed nodes (tests, lint).
    #[serde(default)]
    pub base_dir: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("unknown module '{0}'")]
    UnknownModule(String),
    #[error("connection references a node out of range")]
    NodeOutOfRange,
    #[error("connection references a port out of range")]
    PortOutOfRange,
    #[error("incompatible signal types on a connection")]
    TypeMismatch,
    #[error("the graph contains a cycle")]
    Cycle,
    #[error("track tail node out of range")]
    TailOutOfRange,
    #[error("master chain node is not a 1-in/1-out audio effect")]
    MasterNotEffect,
}

/// Compiled graph, owned by the audio thread.
pub struct CompiledGraph {
    nodes: Vec<Box<dyn AudioNode>>,
    /// Stable instance id per node, indexed like `nodes`. Used to match nodes
    /// across a hot-swap so their live state is carried over.
    node_ids: Vec<String>,
    /// Topological order: `order[pos]` is the node processed at step `pos`.
    order: Vec<NodeIdx>,
    /// Inverse of `order`: node index → topological position.
    node_to_pos: Vec<usize>,
    in_counts: Vec<usize>,
    out_counts: Vec<usize>,
    conns: Vec<Connection>,
    /// One flat output buffer per node, indexed by topological position,
    /// sized `out_count * max_block`.
    outputs: Vec<Vec<f32>>,
    /// Reused gather scratch, sized `max_in_ports * max_block`.
    in_scratch: Vec<f32>,
    /// Per-node note queue for the current block, indexed by node index.
    node_notes: Vec<Vec<NoteEvent>>,
    /// Per-node post-process meter (peak + RMS of port 0), indexed by node
    /// index. Pre-allocated; filled at the end of each block. // RT-SAFE
    meters: Vec<Meter>,
    tails: Vec<NodeIdx>,
    track_mix: Vec<TrackMix>,
    master_gain_db: f32,
    /// Master chain node indices, in bus order (see `process_master`).
    master: Vec<NodeIdx>,
    /// Per-node flag: true = master-chain node, skipped by the topological
    /// walk and metered by `process_master` instead.
    master_set: Vec<bool>,
    /// Channels per port (the engine compiles at [`crate::engine::CHANNELS`];
    /// tests and probes may compile mono).
    channels: usize,
    last_frames: usize,
}

impl CompiledGraph {
    /// Route a note to a node's input for the current block. // RT-SAFE
    pub fn push_note(&mut self, node: NodeIdx, event: NoteEvent) {
        if let Some(buf) = self.node_notes.get_mut(node) {
            if buf.len() < buf.capacity() {
                buf.push(event);
            }
        }
    }

    /// Move live node instances from `old` into `self` for every node whose id
    /// (and port shape) matches, so their real-time state — voices, filter
    /// memory, effect tails — is carried across a hot-swap with no recompute and
    /// no buffer copy. `old` receives `self`'s fresh instances and is discarded
    /// off the audio thread afterwards. // RT-SAFE
    pub fn adopt_nodes_from(&mut self, old: &mut CompiledGraph) {
        for i in 0..self.nodes.len() {
            if self.node_ids[i].is_empty() {
                continue;
            }
            let Some(j) = old.node_ids.iter().position(|oid| *oid == self.node_ids[i]) else {
                continue;
            };
            // Same id ⟹ same module ⟹ same port shape; guard against a stale id
            // reused for a different module.
            if self.in_counts[i] == old.in_counts[j] && self.out_counts[i] == old.out_counts[j] {
                std::mem::swap(&mut self.nodes[i], &mut old.nodes[j]);
            }
        }
    }

    /// Release every sounding voice across all nodes (transport stop). // RT-SAFE
    pub fn all_notes_off(&mut self) {
        for n in &mut self.nodes {
            n.all_notes_off();
        }
    }

    /// Notes queued for a node in the current block, before they are consumed by
    /// `process_block`. Lets a test assert exactly which NoteOn/NoteOff a reload
    /// pushed (the reconciliation diff). // test-only
    #[cfg(test)]
    pub fn pending_notes(&self, node: NodeIdx) -> &[NoteEvent] {
        &self.node_notes[node]
    }

    /// Drop all queued notes (called once the block has consumed them). // RT-SAFE
    pub fn clear_notes(&mut self) {
        for buf in &mut self.node_notes {
            buf.clear();
        }
    }

    /// Forward a normalized parameter change to a node. // RT-SAFE
    pub fn set_param(&mut self, node: NodeIdx, param: ParamId, value: f32) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.set_param(param, value);
        }
    }

    /// Stable instance id of each node, indexed by node index.
    pub fn node_ids(&self) -> &[String] {
        &self.node_ids
    }

    /// The last node of each track's chain, in track order — the signals the
    /// mixer stage reads through [`Self::node_output_ch`].
    pub fn track_tails(&self) -> &[NodeIdx] {
        &self.tails
    }

    /// Per-track mixer settings, aligned with [`Self::track_tails`].
    pub fn track_mix(&self) -> &[TrackMix] {
        &self.track_mix
    }

    /// Master gain (dB) the mixer applies after the track sum.
    pub fn master_gain_db(&self) -> f32 {
        self.master_gain_db
    }

    /// A node's channel-0 output buffer for the last processed block. // RT-SAFE
    pub fn node_output(&self, node: NodeIdx) -> &[f32] {
        self.node_output_ch(node, 0)
    }

    /// One channel of a node's port-0 output for the last processed block.
    /// `ch` is clamped to the compiled channel count, so a mono-compiled graph
    /// transparently serves channel 0 to every reader (the mixer can always ask
    /// for L and R, including from a fading pre-stereo graph). // RT-SAFE
    pub fn node_output_ch(&self, node: NodeIdx, ch: usize) -> &[f32] {
        let pos = self.node_to_pos[node];
        let ch = ch.min(self.channels - 1);
        let start = ch * self.last_frames;
        &self.outputs[pos][start..start + self.last_frames]
    }

    /// Per-node post-process meters (peak + RMS of port 0) for the last block,
    /// indexed by node index — so `meters()[i]` is the node whose stable id is
    /// `node_ids()[i]`. A node with no audio output reads zero. // RT-SAFE
    pub fn meters(&self) -> &[Meter] {
        &self.meters
    }

    /// Process one block of `frames` samples (≤ the compiled `max_block`). The
    /// caller routes notes and params beforehand; `samples_per_beat` comes from
    /// the active schedule (0.0 = no tempo known). // RT-SAFE
    pub fn process_block(
        &mut self,
        sample_rate: f32,
        frames: usize,
        samples_per_beat: f32,
        beat_phase: f32,
    ) {
        self.last_frames = frames;
        for pos in 0..self.order.len() {
            let node = self.order[pos];
            // Master-chain nodes are fed the mixed bus by `process_master`,
            // never by the walk — running them here too would double-process
            // their state (delay lines, envelope followers).
            if self.master_set[node] {
                continue;
            }

            // Gather this node's inputs by summing the connected source buffers
            // into the reused scratch (disjoint fields, no allocation). A port's
            // channels are contiguous under the channel-major layout, so one
            // copy per connection moves the whole port block.
            let port_len = self.channels * frames;
            let in_len = self.in_counts[node] * port_len;
            for v in &mut self.in_scratch[..in_len] {
                *v = 0.0;
            }
            for c in &self.conns {
                if c.dst_node != node {
                    continue;
                }
                let src_pos = self.node_to_pos[c.src_node];
                let src_off = c.src_port * port_len;
                let src = &self.outputs[src_pos][src_off..src_off + port_len];
                let dst_off = c.dst_port * port_len;
                let dst = &mut self.in_scratch[dst_off..dst_off + port_len];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d += *s;
                }
            }

            // Process: sources are in the left split, this node's output is the
            // first element of the right split.
            let out_len = self.out_counts[node] * port_len;
            let (_left, right) = self.outputs.split_at_mut(pos);
            let out_buf = &mut right[0][..out_len];
            let ctx = ProcessCtx {
                samples_per_beat,
                beat_phase,
                sample_rate,
                frames,
                channels: self.channels,
            };
            let mut io = crate::engine::node::NodeIo::new(
                &self.node_notes[node],
                &self.in_scratch[..in_len],
                out_buf,
                frames,
                self.channels,
            );
            self.nodes[node].process(&ctx, &mut io);
        }

        // Meter every node's port-0 output (peak + RMS across all channels)
        // post-process. A node with no audio output keeps a zero meter;
        // master-chain nodes are metered by `process_master`. // RT-SAFE
        for node in 0..self.nodes.len() {
            if self.master_set[node] {
                continue;
            }
            if self.out_counts[node] == 0 {
                self.meters[node] = Meter::default();
                continue;
            }
            let len = self.channels * frames;
            let buf = &self.outputs[self.node_to_pos[node]][..len];
            let mut peak = 0.0f32;
            let mut sum_sq = 0.0f32;
            for &s in buf {
                peak = peak.max(s.abs());
                sum_sq += s * s;
            }
            let rms = if len > 0 {
                (sum_sq / len as f32).sqrt()
            } else {
                0.0
            };
            self.meters[node] = Meter { peak, rms };
        }
    }

    /// Run the master chain over the mixed bus, in place. `bus` is planar
    /// channel-major (`[L(frames), R(frames)]`) — the same layout as a node
    /// port, so each node processes it like any chain block. Node k reads the
    /// previous node's output (the bus for k = 0) staged through `in_scratch`;
    /// the final output is copied back into `bus`. Master nodes are metered
    /// here (the `process_block` loops skip them). Requires a stereo compile:
    /// on a mono-compiled graph (isolated module probes) the chain is skipped.
    /// // RT-SAFE
    pub fn process_master(
        &mut self,
        bus: &mut [f32],
        sample_rate: f32,
        frames: usize,
        samples_per_beat: f32,
        beat_phase: f32,
    ) {
        if self.master.is_empty() || self.channels < 2 {
            return;
        }
        let port_len = self.channels * frames;
        for k in 0..self.master.len() {
            let node = self.master[k];
            // Stage the input: the bus for the first node, the previous
            // node's output after. Copying breaks any aliasing with the
            // node's own output buffer.
            if k == 0 {
                self.in_scratch[..port_len].copy_from_slice(&bus[..port_len]);
            } else {
                let prev_pos = self.node_to_pos[self.master[k - 1]];
                let src = &self.outputs[prev_pos][..port_len];
                self.in_scratch[..port_len].copy_from_slice(src);
            }
            let pos = self.node_to_pos[node];
            let out_buf = &mut self.outputs[pos][..port_len];
            let ctx = ProcessCtx {
                samples_per_beat,
                beat_phase,
                sample_rate,
                frames,
                channels: self.channels,
            };
            let mut io = crate::engine::node::NodeIo::new(
                &self.node_notes[node],
                &self.in_scratch[..port_len],
                out_buf,
                frames,
                self.channels,
            );
            self.nodes[node].process(&ctx, &mut io);

            // Meter the node here (skipped by process_block's loop).
            let buf = &self.outputs[pos][..port_len];
            let mut peak = 0.0f32;
            let mut sum_sq = 0.0f32;
            for &s in buf {
                peak = peak.max(s.abs());
                sum_sq += s * s;
            }
            self.meters[node] = Meter {
                peak,
                rms: (sum_sq / port_len as f32).sqrt(),
            };
        }
        let last_pos = self.node_to_pos[self.master[self.master.len() - 1]];
        bus[..port_len].copy_from_slice(&self.outputs[last_pos][..port_len]);
    }
}

/// Compile a description into an RT-ready graph. `create` maps a module id to a
/// fresh node (the registry). `channels` is the per-port channel count: the
/// engine passes [`crate::engine::CHANNELS`]; unit tests and probes may compile
/// mono. Runs entirely off the audio thread.
pub fn compile(
    desc: &GraphDesc,
    create: impl Fn(&str) -> Option<Box<dyn AudioNode>>,
    sample_rate: f32,
    max_block: usize,
    channels: usize,
) -> Result<CompiledGraph, GraphError> {
    let n = desc.nodes.len();
    let channels = channels.max(1);
    if desc.tails.iter().any(|&t| t >= n) {
        return Err(GraphError::TailOutOfRange);
    }
    if desc.master.iter().any(|&m| m >= n) {
        return Err(GraphError::MasterNotEffect);
    }

    // Instantiate, prepare and inspect every node.
    let mut nodes: Vec<Box<dyn AudioNode>> = Vec::with_capacity(n);
    let mut in_counts = Vec::with_capacity(n);
    let mut out_counts = Vec::with_capacity(n);
    let mut in_kinds: Vec<Vec<SignalKind>> = Vec::with_capacity(n);
    let mut out_kinds: Vec<Vec<SignalKind>> = Vec::with_capacity(n);

    for node_desc in &desc.nodes {
        let mut node = create(&node_desc.module_id)
            .ok_or_else(|| GraphError::UnknownModule(node_desc.module_id.clone()))?;
        if !node_desc.config.is_empty() {
            node.configure(&node_desc.config, &desc.base_dir);
        }
        node.prepare(sample_rate, max_block);
        let ports = node.ports();
        in_counts.push(ports.inputs.len());
        out_counts.push(ports.outputs.len());
        in_kinds.push(ports.inputs.iter().map(|p| p.kind).collect());
        out_kinds.push(ports.outputs.iter().map(|p| p.kind).collect());
        for (id, value) in &node_desc.params {
            node.set_param(*id, *value);
        }
        nodes.push(node);
    }

    // Master-chain nodes must be plain audio effects (one audio in, one audio
    // out): they are fed the mixed bus, not graph edges.
    let mut master_set = vec![false; n];
    for &m in &desc.master {
        let effect_shape = in_counts[m] == 1
            && out_counts[m] == 1
            && in_kinds[m][0] == SignalKind::Audio
            && out_kinds[m][0] == SignalKind::Audio;
        if !effect_shape || master_set[m] {
            return Err(GraphError::MasterNotEffect);
        }
        master_set[m] = true;
    }

    // Validate connections: bounds, then port signal-type compatibility.
    for c in &desc.connections {
        if c.src_node >= n || c.dst_node >= n {
            return Err(GraphError::NodeOutOfRange);
        }
        if c.src_port >= out_counts[c.src_node] || c.dst_port >= in_counts[c.dst_node] {
            return Err(GraphError::PortOutOfRange);
        }
        if out_kinds[c.src_node][c.src_port] != in_kinds[c.dst_node][c.dst_port] {
            return Err(GraphError::TypeMismatch);
        }
    }

    // Topological order (Kahn). Edge src_node → dst_node, one per connection.
    let mut indegree = vec![0usize; n];
    for c in &desc.connections {
        indegree[c.dst_node] += 1;
    }
    let mut queue: Vec<NodeIdx> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    let mut head = 0;
    while head < queue.len() {
        let node = queue[head];
        head += 1;
        order.push(node);
        for c in &desc.connections {
            if c.src_node == node {
                indegree[c.dst_node] -= 1;
                if indegree[c.dst_node] == 0 {
                    queue.push(c.dst_node);
                }
            }
        }
    }
    if order.len() != n {
        return Err(GraphError::Cycle);
    }

    let mut node_to_pos = vec![0usize; n];
    for (pos, &node) in order.iter().enumerate() {
        node_to_pos[node] = pos;
    }

    // Allocate buffers (off the audio thread). Sized `× channels` per the
    // channel-major layout documented on `ProcessCtx::channels` — identical to
    // the historical mono layout while `channels == 1`.
    let outputs: Vec<Vec<f32>> = order
        .iter()
        .map(|&node| vec![0.0f32; out_counts[node].max(1) * max_block * channels])
        .collect();
    let max_in = in_counts.iter().copied().max().unwrap_or(0);
    let in_scratch = vec![0.0f32; max_in * max_block * channels];
    let node_notes = (0..n).map(|_| Vec::with_capacity(NOTE_CAPACITY)).collect();
    let node_ids = desc.nodes.iter().map(|d| d.id.clone()).collect();
    let meters = vec![Meter::default(); n];

    Ok(CompiledGraph {
        nodes,
        node_ids,
        order,
        node_to_pos,
        in_counts,
        out_counts,
        conns: desc.connections.clone(),
        outputs,
        in_scratch,
        node_notes,
        meters,
        tails: desc.tails.clone(),
        track_mix: desc.track_mix.clone(),
        master_gain_db: desc.master_gain_db,
        master: desc.master.clone(),
        master_set,
        channels,
        last_frames: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::{NodeIo, ParamSpec, Port, PortSpec};

    // A source with one audio output that writes a constant.
    struct ConstNode(f32);
    impl AudioNode for ConstNode {
        fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
            for s in io.output(0).iter_mut() {
                *s = self.0;
            }
            io.spread_output(0);
        }
        fn ports(&self) -> PortSpec {
            PortSpec {
                inputs: vec![],
                outputs: vec![Port {
                    name: "out",
                    kind: SignalKind::Audio,
                }],
            }
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn set_param(&mut self, _id: ParamId, _value: f32) {}
        fn prepare(&mut self, _sr: f32, _mb: usize) {}
    }

    // An effect: one audio input → one audio output, halving the signal.
    struct HalfNode;
    impl AudioNode for HalfNode {
        fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
            let frames = io.frames();
            for ch in 0..io.channels() {
                for i in 0..frames {
                    let x = io.input_ch(0, ch)[i];
                    io.output_ch(0, ch)[i] = x * 0.5;
                }
            }
        }
        fn ports(&self) -> PortSpec {
            PortSpec {
                inputs: vec![Port {
                    name: "in",
                    kind: SignalKind::Audio,
                }],
                outputs: vec![Port {
                    name: "out",
                    kind: SignalKind::Audio,
                }],
            }
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn set_param(&mut self, _id: ParamId, _value: f32) {}
        fn prepare(&mut self, _sr: f32, _mb: usize) {}
    }

    // A sink with a single Notes input (used to force a type mismatch).
    struct NoteSink;
    impl AudioNode for NoteSink {
        fn process(&mut self, _ctx: &ProcessCtx, _io: &mut NodeIo) {}
        fn ports(&self) -> PortSpec {
            PortSpec {
                inputs: vec![Port {
                    name: "notes",
                    kind: SignalKind::Notes,
                }],
                outputs: vec![],
            }
        }
        fn params(&self) -> &[ParamSpec] {
            &[]
        }
        fn set_param(&mut self, _id: ParamId, _value: f32) {}
        fn prepare(&mut self, _sr: f32, _mb: usize) {}
    }

    fn create(id: &str) -> Option<Box<dyn AudioNode>> {
        match id {
            "const" => Some(Box::new(ConstNode(1.0))),
            "half" => Some(Box::new(HalfNode)),
            "notesink" => Some(Box::new(NoteSink)),
            _ => None,
        }
    }

    #[test]
    fn chain_processes_in_topological_order() {
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "src".into(),
                    module_id: "const".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "fx".into(),
                    module_id: "half".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            tails: vec![1],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        let mut g = compile(&desc, create, 48_000.0, 64, 1).expect("compiles");
        g.process_block(48_000.0, 32, 0.0, 0.0);
        // const(1.0) → half(×0.5) = 0.5 on the tail buffer.
        assert!(g.node_output(1).iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn meters_are_aligned_by_node_index() {
        // node 0 = const(1.0) source, node 1 = half(×0.5) effect.
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "src".into(),
                    module_id: "const".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "fx".into(),
                    module_id: "half".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            tails: vec![1],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        let mut g = compile(&desc, create, 48_000.0, 64, 1).expect("compiles");
        g.process_block(48_000.0, 32, 0.0, 0.0);
        let m = g.meters();
        assert_eq!(m.len(), 2, "one meter per node");
        // A constant signal: peak == rms == the level.
        assert!((m[0].peak - 1.0).abs() < 1e-6, "const node peak");
        assert!((m[0].rms - 1.0).abs() < 1e-6, "const node rms");
        assert!((m[1].peak - 0.5).abs() < 1e-6, "halved node peak");
        assert!((m[1].rms - 0.5).abs() < 1e-6, "halved node rms");
    }

    #[test]
    fn source_is_ordered_before_its_consumer() {
        // Declare the consumer first to prove the order is computed, not given.
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "fx".into(),
                    module_id: "half".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "src".into(),
                    module_id: "const".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 1,
                src_port: 0,
                dst_node: 0,
                dst_port: 0,
            }],
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        let g = compile(&desc, create, 48_000.0, 64, 1).expect("compiles");
        // node 1 (const) must come before node 0 (half) in the order.
        let pos_const = g.node_to_pos[1];
        let pos_half = g.node_to_pos[0];
        assert!(pos_const < pos_half);
    }

    #[test]
    fn adopt_moves_live_node_state() {
        use crate::engine::swap::test_support;
        let desc = GraphDesc {
            nodes: vec![NodeDesc {
                id: "t".into(),
                module_id: "tail".into(),
                params: vec![],
                config: vec![],
            }],
            connections: vec![],
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        // Ring the tail (struck to 1.0 by prepare) well down.
        let mut a = compile(&desc, test_support::create, 48_000.0, 64, 1).expect("compiles");
        for _ in 0..16 {
            a.process_block(48_000.0, 64, 0.0, 0.0);
        }
        let decayed = a.node_output(0)[0];
        assert!(decayed < 0.7, "tail should have decayed: {decayed}");

        // A fresh graph's tail is struck back to 1.0 by prepare; adopting the
        // live node must carry the decayed state instead of restarting at 1.0.
        let mut b = compile(&desc, test_support::create, 48_000.0, 64, 1).expect("compiles");
        b.adopt_nodes_from(&mut a);
        b.process_block(48_000.0, 64, 0.0, 0.0);
        assert!(
            b.node_output(0)[0] <= decayed,
            "live state was not moved into the new graph: {}",
            b.node_output(0)[0]
        );
    }

    #[test]
    fn processing_is_block_size_independent() {
        // Channel offsets inside a port are packed with the CURRENT block's
        // frame count, so a stride mistake only shows once two runs process
        // the same signal through different (non-power-of-two) block sizes.
        use crate::engine::swap::test_support;
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "t".into(),
                    module_id: "tail".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "fx".into(),
                    module_id: "filter".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            tails: vec![1],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        let total = 480 * 8;
        let run = |block: usize| {
            let mut g = compile(&desc, test_support::create, 48_000.0, 4096, 2).expect("compiles");
            let mut left = Vec::new();
            let mut right = Vec::new();
            let mut done = 0usize;
            while done < total {
                let frames = block.min(total - done);
                g.process_block(48_000.0, frames, 0.0, 0.0);
                left.extend_from_slice(g.node_output_ch(1, 0));
                right.extend_from_slice(g.node_output_ch(1, 1));
                done += frames;
            }
            (left, right)
        };
        let (l64, r64) = run(64);
        let (l480, r480) = run(480);
        assert_eq!(l64, l480, "left channel diverged across block sizes");
        assert_eq!(r64, r480, "right channel diverged across block sizes");
        assert_eq!(l64, r64, "dual-mono invariant broken");
        assert!(l64.iter().any(|&s| s != 0.0), "silent run proves nothing");
    }

    #[test]
    fn every_module_fills_all_channels() {
        // Registry sweep: compiled stereo, every instrument (fed a note) and
        // every effect (fed DC) must fill channel 1 — a freshly allocated
        // buffer means a stale channel would stay exactly zero.
        use crate::engine::node::{NoteEvent, NoteKind};
        use crate::engine::swap::test_support;
        use crate::modules::registry::{self, ModuleCategory};

        // The sampler is content-driven: hand it a one-zone kit so the sweep
        // exercises the `config` path through `compile` too.
        let zone_dir = std::env::temp_dir().join(format!("telenn_sweep_{}", std::process::id()));
        std::fs::create_dir_all(&zone_dir).expect("temp zone dir");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(zone_dir.join("036.wav"), spec).expect("zone wav");
        for i in 0..9600 {
            let x = (std::f32::consts::TAU * 110.0 * i as f32 / 48_000.0).sin();
            w.write_sample((x * 0.8 * i16::MAX as f32) as i16)
                .expect("zone sample");
        }
        w.finalize().expect("zone finalize");

        for desc in registry::list() {
            let is_instrument = desc.category == ModuleCategory::Instrument;
            let config = if desc.id == "sampler" {
                vec![("zones".to_string(), zone_dir.to_string_lossy().into_owned())]
            } else {
                vec![]
            };
            let (nodes, connections, target) = if is_instrument {
                (
                    vec![NodeDesc {
                        id: "m".into(),
                        module_id: desc.id.into(),
                        params: vec![],
                        config,
                    }],
                    vec![],
                    0,
                )
            } else {
                (
                    vec![
                        NodeDesc {
                            id: "src".into(),
                            module_id: "dc".into(),
                            params: vec![],
                            config: vec![],
                        },
                        NodeDesc {
                            id: "m".into(),
                            module_id: desc.id.into(),
                            params: vec![],
                            config: vec![],
                        },
                    ],
                    vec![Connection {
                        src_node: 0,
                        src_port: 0,
                        dst_node: 1,
                        dst_port: 0,
                    }],
                    1,
                )
            };
            let graph_desc = GraphDesc {
                nodes,
                connections,
                tails: vec![target],
                track_mix: vec![],
                master_gain_db: 0.0,
                master: vec![],
                base_dir: Default::default(),
            };
            let mut g =
                compile(&graph_desc, test_support::create, 48_000.0, 256, 2).expect("compiles");
            if is_instrument {
                g.push_note(
                    target,
                    NoteEvent {
                        frame: 0,
                        kind: NoteKind::On {
                            pitch: 36,
                            vel: 100,
                        },
                    },
                );
            }
            // Chorus and reverb decorrelate on purpose; everything else must
            // stay sample-identical across channels on identical inputs.
            let decorrelates = matches!(desc.id, "chorus" | "reverb");
            let mut energy = [0.0f32; 2];
            for _ in 0..16 {
                g.process_block(48_000.0, 256, 0.0, 0.0);
                g.clear_notes();
                let (c0, c1) = (g.node_output_ch(target, 0), g.node_output_ch(target, 1));
                if !decorrelates {
                    assert_eq!(c0, c1, "'{}' channels differ", desc.id);
                }
                energy[0] += c0.iter().map(|s| s.abs()).sum::<f32>();
                energy[1] += c1.iter().map(|s| s.abs()).sum::<f32>();
            }
            assert!(energy[0] > 0.0, "'{}' left channel 0 silent", desc.id);
            assert!(energy[1] > 0.0, "'{}' left channel 1 silent", desc.id);
        }
        std::fs::remove_dir_all(&zone_dir).ok();
    }

    #[test]
    fn master_chain_rejects_non_effect_nodes() {
        // A source (no audio input) cannot sit on the master bus.
        let desc = GraphDesc {
            nodes: vec![NodeDesc {
                id: "src".into(),
                module_id: "const".into(),
                params: vec![],
                config: vec![],
            }],
            connections: vec![],
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![0],
            base_dir: Default::default(),
        };
        assert!(matches!(
            compile(&desc, create, 48_000.0, 64, 2),
            Err(GraphError::MasterNotEffect)
        ));
    }

    #[test]
    fn master_nodes_run_on_the_bus_not_in_the_walk() {
        // dc track + a half effect on the master list: the walk must leave the
        // master node untouched (its buffer stays zero) and process_master
        // must run it over the bus, halving it and metering it.
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "src".into(),
                    module_id: "const".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "mfx".into(),
                    module_id: "half".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![],
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![1],
            base_dir: Default::default(),
        };
        let frames = 32;
        let mut g = compile(&desc, create, 48_000.0, 64, 2).expect("compiles");
        g.process_block(48_000.0, frames, 0.0, 0.0);
        assert!(
            g.node_output_ch(1, 0).iter().all(|&s| s == 0.0),
            "the walk processed a master node"
        );

        let mut bus = vec![1.0f32; frames * 2];
        g.process_master(&mut bus, 48_000.0, frames, 0.0, 0.0);
        assert!(
            bus.iter().all(|&s| (s - 0.5).abs() < 1e-6),
            "master chain did not process the bus"
        );
        let meter = &g.meters()[1];
        assert!(
            (meter.peak - 0.5).abs() < 1e-6,
            "master node not metered: {}",
            meter.peak
        );
    }

    #[test]
    fn incompatible_port_types_are_rejected() {
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    id: "src".into(),
                    module_id: "const".into(),
                    params: vec![],
                    config: vec![],
                },
                NodeDesc {
                    id: "sink".into(),
                    module_id: "notesink".into(),
                    params: vec![],
                    config: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
            master: vec![],
            base_dir: Default::default(),
        };
        assert!(matches!(
            compile(&desc, create, 48_000.0, 64, 1),
            Err(GraphError::TypeMismatch)
        ));
    }
}
