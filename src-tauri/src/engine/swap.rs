//! Hot-swap support: the master-output declick ramp and the audio→control
//! garbage payload.
//!
//! A graph swap moves the persistent node instances into the new graph (so their
//! real-time state is sample-continuous) and keeps the old graph as a *fading
//! graph*: with its persistent instances moved out and fed nothing, it renders
//! only the removed effects' tails, which are faded out instead of cut. The only
//! residual artifact — the routing/level step at the swap boundary — is removed
//! by [`Declick`] on the master output.

use crate::control::sequencer::Schedule;
use crate::engine::graph::CompiledGraph;

/// Heavy objects the audio thread hands back to the control side to be dropped
/// off the real-time thread (a `drop` in the callback would free memory).
pub enum Garbage {
    Schedule(Box<Schedule>),
    Graph(Box<CompiledGraph>),
}

/// Removes the output step a graph swap leaves at the block boundary by carrying
/// the discontinuity forward as a decaying offset (a difference ramp): the first
/// post-swap sample is forced to equal the last pre-swap sample, then the offset
/// ramps to zero over ~10 ms so the output converges to the true new signal.
/// Inert when there is no step (offset ≈ 0). // RT-SAFE
pub struct Declick {
    total: u32,
    remaining: u32,
    offset: f32,
    pending: bool,
    last: f32,
}

impl Declick {
    pub fn new(sample_rate: f32) -> Self {
        let total = (sample_rate * 0.010) as u32; // ~10 ms
        Self {
            total: total.max(1),
            remaining: 0,
            offset: 0.0,
            pending: false,
            last: 0.0,
        }
    }

    /// Arm a declick; the boundary offset is measured at the next sample. // RT-SAFE
    pub fn arm(&mut self) {
        self.pending = true;
    }

    /// Feed the true master sample, return the declicked one. // RT-SAFE
    pub fn apply(&mut self, s: f32) -> f32 {
        if self.pending {
            self.offset = self.last - s;
            self.remaining = self.total;
            self.pending = false;
        }
        let out = if self.remaining > 0 {
            let g = self.remaining as f32 / self.total as f32;
            self.remaining -= 1;
            s + self.offset * g
        } else {
            s
        };
        self.last = out;
        out
    }
}

/// Test-only nodes used to validate the swap mechanism (state preservation and
/// tail fade-out) before the real reverb module exists (M8).
#[cfg(test)]
pub mod test_support {
    use crate::engine::node::{
        AudioNode, NodeIo, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
    };

    /// A decaying source: rings its internal state down each sample and ignores
    /// any input. Stands in for a tail effect (reverb) — its state is exactly the
    /// thing a hot-swap must carry on a move and ring out on a remove.
    pub struct TailSource {
        z: f32,
        decay: f32,
    }

    impl Default for TailSource {
        fn default() -> Self {
            Self {
                z: 0.0,
                decay: 0.999,
            }
        }
    }

    impl AudioNode for TailSource {
        fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
            for s in io.output(0).iter_mut() {
                *s = self.z;
                self.z *= self.decay;
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
        fn prepare(&mut self, _sr: f32, _mb: usize) {
            self.z = 1.0; // struck on prepare
        }
    }

    /// A source that outputs silence (a valid tail node with nothing to say).
    #[derive(Default)]
    pub struct Silence;

    impl AudioNode for Silence {
        fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
            io.output(0).fill(0.0);
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

    /// A unit DC source (constant 1.0) — a known level for mixer/pan tests.
    #[derive(Default)]
    pub struct Dc;

    impl AudioNode for Dc {
        fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
            io.output(0).fill(1.0);
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

    /// Registry for the test nodes (plus the real modules), for `compile`.
    pub fn create(module_id: &str) -> Option<Box<dyn AudioNode>> {
        match module_id {
            "tail" => Some(Box::<TailSource>::default()),
            "silence" => Some(Box::<Silence>::default()),
            "dc" => Some(Box::<Dc>::default()),
            other => crate::modules::registry::create(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declick_makes_a_step_continuous_then_converges() {
        let mut d = Declick::new(48_000.0);
        // Settle `last` at 1.0 (no declick armed yet).
        for _ in 0..10 {
            assert_eq!(d.apply(1.0), 1.0);
        }
        // A hard step to 0.0 with a declick armed: the first sample stays at the
        // old value, with no jump, then the offset ramps out.
        d.arm();
        let first = d.apply(0.0);
        assert!(
            (first - 1.0).abs() < 1e-6,
            "boundary not continuous: {first}"
        );
        // Far past the window it tracks the true signal exactly.
        for _ in 0..1000 {
            d.apply(0.0);
        }
        assert!(d.apply(0.0).abs() < 1e-6);
    }
}
