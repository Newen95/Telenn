//! Module registry: maps a stable module id to a fresh node instance.
//!
//! `song.json` (M3) and the graph compiler reference modules by id. Adding a
//! module means adding one arm here and one `mod` line — the engine core is
//! untouched.

use crate::engine::node::AudioNode;
use crate::modules::_template::Gain;
use crate::modules::plucked::Plucked;
use crate::modules::subtractive::Subtractive;

fn boxed<N: AudioNode + Default + 'static>() -> Box<dyn AudioNode> {
    Box::new(N::default())
}

/// Instantiate a module by id, or `None` if unknown.
pub fn create(module_id: &str) -> Option<Box<dyn AudioNode>> {
    match module_id {
        "subtractive" => Some(boxed::<Subtractive>()),
        "plucked" => Some(boxed::<Plucked>()),
        "gain" => Some(boxed::<Gain>()),
        _ => None,
    }
}
