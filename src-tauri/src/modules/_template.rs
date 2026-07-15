//! Reference module — copy this to start a new effect.
//!
//! It is a real, registered gain effect (`"gain"`): one audio input, one audio
//! output, a single smoothed `gain` parameter. Use it as the canonical example
//! of how to implement [`AudioNode`]: declare ports and params, smooth every
//! parameter, and keep `process` allocation-free.
//!
//! Channel contract: a node processes `ctx.channels` channels per port
//! (channel-major buffers; `io.input_ch`/`io.output_ch`/`io.in_out_ch`).
//! Instruments synthesize channel 0 and call `io.spread_output(0)`. Effects
//! keep per-channel signal state (one biquad/delay line per channel) and
//! replay snapshotted param ramps per channel so every channel sees the same
//! per-frame trajectory — never tick a shared ramp once per channel-sample.
//! A detector that should react to the whole image (compressor) is the one
//! deliberate exception: one follower fed by the channel maximum.

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
        // RT-SAFE: replay the same ramp trajectory on every channel.
        let gain_start = self.gain;
        for ch in 0..io.channels() {
            if ch > 0 {
                self.gain = gain_start;
            }
            let (input, output) = io.in_out_ch(0, 0, ch);
            for (out, &x) in output.iter_mut().zip(input) {
                *out = x * self.gain.tick();
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
