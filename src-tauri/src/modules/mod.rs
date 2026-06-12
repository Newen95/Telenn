//! Audio modules: instruments and effects, one file per module.
//!
//! Each module implements the `AudioNode` trait and registers itself in the
//! module registry. Adding or removing a module never touches the engine core.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
