//! Control layer (regular threads): sequencer, project I/O and undo, MIDI input,
//! offline WAV export, graph construction and the project file watcher.
//!
//! Communicates with the audio thread only through lock-free ring buffers.

pub mod commands;

use crate::engine::graph::{compile, CompiledGraph, Connection, GraphDesc, GraphError, NodeDesc};
use crate::engine::node::{NodeIdx, ParamId};

/// Compact, allocation-free messages sent from the control side to the audio
/// thread through an `rtrb` ring buffer. The callback drains all pending
/// messages at the start of each block before rendering.
///
/// This enum grows over later milestones (per ARCHITECTURE.md: `Seek` at M3,
/// `SwapGraph` at M4).
pub enum ControlMsg {
    Transport(TransportCmd),
    NoteOn {
        node: NodeIdx,
        pitch: u8,
        vel: u8,
    },
    NoteOff {
        node: NodeIdx,
        pitch: u8,
    },
    SetParam {
        node: NodeIdx,
        param: ParamId,
        value: f32,
    },
}

#[derive(Clone, Copy)]
pub enum TransportCmd {
    Play,
    Stop,
}

/// Build the M2 test graph: subtractive synth (node 0) → gain (node 1) → output.
/// M3 replaces this with a graph compiled from `song.json`.
pub fn build_test_graph(sample_rate: f32, max_block: usize) -> Result<CompiledGraph, GraphError> {
    let desc = GraphDesc {
        nodes: vec![
            NodeDesc {
                module_id: "subtractive".into(),
                params: vec![],
            },
            NodeDesc {
                module_id: "gain".into(),
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
    compile(
        &desc,
        crate::modules::registry::create,
        sample_rate,
        max_block,
    )
}
