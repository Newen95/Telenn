//! Reference module — copy this to start a new effect.
//!
//! It is a real, registered gain effect (`"gain"`): one audio input, one audio
//! output, a single smoothed `gain` parameter. Use it as the canonical example
//! of how to implement [`AudioNode`]: declare ports and params, smooth every
//! parameter, and keep `process` allocation-free.

use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::LinearRamp;

/// Gain smoothing time. Gains use a linear ramp (per ARCHITECTURE.md).
const GAIN_RAMP_MS: f32 = 10.0;

const P_GAIN: ParamId = 0;

const PARAMS: [ParamSpec; 1] = [ParamSpec {
    id: P_GAIN,
    name: "gain",
    unit: "dB",
    min: -60.0,
    max: 12.0,
    default: 0.0,
    curve: ParamCurve::Db,
}];

pub struct Gain {
    /// Smoothed linear gain (the dB→linear mapping happens in `set_param`).
    gain: LinearRamp,
}

impl Default for Gain {
    fn default() -> Self {
        // The real smoother is rebuilt in `prepare` once the sample rate is known.
        Self {
            gain: LinearRamp::new(1.0, GAIN_RAMP_MS, 48_000.0),
        }
    }
}

impl AudioNode for Gain {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let (input, output) = io.in_out(0, 0);
        for (out, &x) in output.iter_mut().zip(input) {
            *out = x * self.gain.tick();
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
        &PARAMS
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        if id == P_GAIN {
            self.gain.set_target(PARAMS[0].denormalize(value));
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        let initial = PARAMS[0].denormalize(PARAMS[0].normalize(PARAMS[0].default));
        self.gain = LinearRamp::new(initial, GAIN_RAMP_MS, sample_rate);
    }
}
