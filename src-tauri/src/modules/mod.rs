//! Audio modules: instruments and effects, one file per module.
//!
//! Each module implements the `AudioNode` trait and registers itself in the
//! module registry. Adding or removing a module never touches the engine core.

pub mod _template;
pub mod compressor;
pub mod delay;
pub mod drum_sampler;
pub mod fm;
pub mod plucked;
pub mod registry;
pub mod reverb;
pub mod subtractive;
