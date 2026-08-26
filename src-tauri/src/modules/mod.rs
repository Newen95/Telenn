//! Audio modules: instruments and effects, one file per module.
//!
//! Each module implements the `AudioNode` trait and registers itself in the
//! module registry. Adding or removing a module never touches the engine core.

pub mod _template;
pub mod ampsim;
pub mod chorus;
pub mod compressor;
pub mod delay;
pub mod drum_sampler;
pub mod ducker;
pub mod eq;
pub mod filter;
pub mod fm;
pub mod modal;
pub mod plucked;
pub mod registry;
pub mod reverb;
pub mod sampler;
pub mod saturator;
pub mod subtractive;
pub mod wav;
pub mod wavetable;
