//! Parameter smoothers, shared by the master gain and by modules.
//!
//! `LinearRamp` slews at a constant rate (gains, anti-click). `OnePole` is an
//! exponential glide for everything else (cutoff, resonance…). Both are
//! RT-SAFE: no allocation, pure arithmetic.

/// Constant-rate linear ramp toward a target. `Copy` so an effect can snapshot
/// it and replay the same trajectory on every channel of a block. // RT-SAFE
#[derive(Clone, Copy)]
pub struct LinearRamp {
    current: f32,
    target: f32,
    /// Maximum change per sample.
    step: f32,
}

impl LinearRamp {
    pub fn new(initial: f32, ramp_ms: f32, sample_rate: f32) -> Self {
        let samples = (ramp_ms / 1000.0 * sample_rate).max(1.0);
        Self {
            current: initial,
            target: initial,
            step: 1.0 / samples,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Current value (without advancing). // RT-SAFE
    pub fn value(&self) -> f32 {
        self.current
    }

    /// Advance one sample and return the new value. // RT-SAFE
    pub fn tick(&mut self) -> f32 {
        if self.current < self.target {
            self.current = (self.current + self.step).min(self.target);
        } else if self.current > self.target {
            self.current = (self.current - self.step).max(self.target);
        }
        self.current
    }
}

/// One-pole exponential smoother. `coef` is the per-sample retention factor
/// (close to 1.0 = slow). Advanced a whole block at a time. `Copy` for the
/// same per-channel replay idiom as [`LinearRamp`]. // RT-SAFE
#[derive(Clone, Copy)]
pub struct OnePole {
    current: f32,
    target: f32,
    coef: f32,
}

impl OnePole {
    pub fn new(initial: f32, coef: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            coef,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    /// Advance the smoother by `frames` samples and return the new value.
    /// Uses the closed form `coef^frames` so it stays cheap per block. // RT-SAFE
    pub fn advance(&mut self, frames: usize) -> f32 {
        let decay = self.coef.powi(frames as i32);
        self.current = self.target + (self.current - self.target) * decay;
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_ramp_reaches_target_monotonically() {
        let mut r = LinearRamp::new(0.0, 10.0, 48_000.0);
        r.set_target(1.0);
        let mut prev = 0.0;
        for _ in 0..480 {
            let v = r.tick();
            assert!(v >= prev);
            prev = v;
        }
        assert!((r.tick() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn one_pole_converges() {
        let mut p = OnePole::new(0.0, 0.995);
        p.set_target(100.0);
        for _ in 0..64 {
            p.advance(256);
        }
        assert!((p.current() - 100.0).abs() < 0.5);
    }
}
