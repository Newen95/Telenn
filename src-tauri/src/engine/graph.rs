//! The audio graph: a serializable description (`GraphDesc`) and the compiled,
//! RT-ready form (`CompiledGraph`).
//!
//! Compilation instantiates nodes, prepares them, validates the connections
//! (bounds and port signal types), and computes a flat topological order.
//! Output buffers are indexed by topological position so a node always reads
//! buffers that come before its own — `split_at_mut` then hands out the sources
//! (read) and the node's output (write) without `unsafe`.

use serde::{Deserialize, Serialize};

use crate::engine::node::{AudioNode, NodeIdx, NoteEvent, ParamId, ProcessCtx, SignalKind};

/// Pre-allocated capacity of each node's per-block note queue. Notes beyond
/// this in a single block are dropped rather than reallocating. // RT-SAFE
const NOTE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDesc {
    pub module_id: String,
    /// Initial parameter values, normalized 0..1.
    #[serde(default)]
    pub params: Vec<(ParamId, f32)>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Connection {
    pub src_node: NodeIdx,
    pub src_port: usize,
    pub dst_node: NodeIdx,
    pub dst_port: usize,
}

/// Serializable graph description (the future shape of `song.json` chains).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDesc {
    pub nodes: Vec<NodeDesc>,
    pub connections: Vec<Connection>,
    /// Node whose first output port feeds the device.
    pub output: NodeIdx,
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
    #[error("output node out of range")]
    OutputOutOfRange,
}

/// Compiled graph, owned by the audio thread.
pub struct CompiledGraph {
    nodes: Vec<Box<dyn AudioNode>>,
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
    output_node: NodeIdx,
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

    /// The output node's mono buffer for the last processed block. // RT-SAFE
    pub fn output(&self) -> &[f32] {
        let pos = self.node_to_pos[self.output_node];
        &self.outputs[pos][..self.last_frames]
    }

    /// Process one block of `frames` samples (≤ the compiled `max_block`). The
    /// caller routes notes and params beforehand. // RT-SAFE
    pub fn process_block(&mut self, sample_rate: f32, frames: usize) {
        self.last_frames = frames;
        for pos in 0..self.order.len() {
            let node = self.order[pos];

            // Gather this node's inputs by summing the connected source buffers
            // into the reused scratch (disjoint fields, no allocation).
            let in_len = self.in_counts[node] * frames;
            for v in &mut self.in_scratch[..in_len] {
                *v = 0.0;
            }
            for c in &self.conns {
                if c.dst_node != node {
                    continue;
                }
                let src_pos = self.node_to_pos[c.src_node];
                let src_off = c.src_port * frames;
                let src = &self.outputs[src_pos][src_off..src_off + frames];
                let dst_off = c.dst_port * frames;
                let dst = &mut self.in_scratch[dst_off..dst_off + frames];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d += *s;
                }
            }

            // Process: sources are in the left split, this node's output is the
            // first element of the right split.
            let out_len = self.out_counts[node] * frames;
            let (_left, right) = self.outputs.split_at_mut(pos);
            let out_buf = &mut right[0][..out_len];
            let ctx = ProcessCtx {
                sample_rate,
                frames,
            };
            let mut io = crate::engine::node::NodeIo::new(
                &self.node_notes[node],
                &self.in_scratch[..in_len],
                out_buf,
                frames,
            );
            self.nodes[node].process(&ctx, &mut io);
        }
    }
}

/// Compile a description into an RT-ready graph. `create` maps a module id to a
/// fresh node (the registry). Runs entirely off the audio thread.
pub fn compile(
    desc: &GraphDesc,
    create: impl Fn(&str) -> Option<Box<dyn AudioNode>>,
    sample_rate: f32,
    max_block: usize,
) -> Result<CompiledGraph, GraphError> {
    let n = desc.nodes.len();
    if desc.output >= n {
        return Err(GraphError::OutputOutOfRange);
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

    // Allocate buffers (off the audio thread).
    let outputs: Vec<Vec<f32>> = order
        .iter()
        .map(|&node| vec![0.0f32; out_counts[node].max(1) * max_block])
        .collect();
    let max_in = in_counts.iter().copied().max().unwrap_or(0);
    let in_scratch = vec![0.0f32; max_in * max_block];
    let node_notes = (0..n).map(|_| Vec::with_capacity(NOTE_CAPACITY)).collect();

    Ok(CompiledGraph {
        nodes,
        order,
        node_to_pos,
        in_counts,
        out_counts,
        conns: desc.connections.clone(),
        outputs,
        in_scratch,
        node_notes,
        output_node: desc.output,
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
            for i in 0..frames {
                let x = io.input(0)[i];
                io.output(0)[i] = x * 0.5;
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
                    module_id: "const".into(),
                    params: vec![],
                },
                NodeDesc {
                    module_id: "half".into(),
                    params: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            output: 1,
        };
        let mut g = compile(&desc, create, 48_000.0, 64).expect("compiles");
        g.process_block(48_000.0, 32);
        // const(1.0) → half(×0.5) = 0.5 on the output buffer.
        assert!(g.output().iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn source_is_ordered_before_its_consumer() {
        // Declare the consumer first to prove the order is computed, not given.
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    module_id: "half".into(),
                    params: vec![],
                },
                NodeDesc {
                    module_id: "const".into(),
                    params: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 1,
                src_port: 0,
                dst_node: 0,
                dst_port: 0,
            }],
            output: 0,
        };
        let g = compile(&desc, create, 48_000.0, 64).expect("compiles");
        // node 1 (const) must come before node 0 (half) in the order.
        let pos_const = g.node_to_pos[1];
        let pos_half = g.node_to_pos[0];
        assert!(pos_const < pos_half);
    }

    #[test]
    fn incompatible_port_types_are_rejected() {
        let desc = GraphDesc {
            nodes: vec![
                NodeDesc {
                    module_id: "const".into(),
                    params: vec![],
                },
                NodeDesc {
                    module_id: "notesink".into(),
                    params: vec![],
                },
            ],
            connections: vec![Connection {
                src_node: 0,
                src_port: 0,
                dst_node: 1,
                dst_port: 0,
            }],
            output: 0,
        };
        assert!(matches!(
            compile(&desc, create, 48_000.0, 64),
            Err(GraphError::TypeMismatch)
        ));
    }
}
