//! The mixer stage: the single place where the mono track signals become stereo.
//!
//! Upstream of this — instruments, effects, chains — everything is strictly
//! mono; downstream, the device fan-out is already N-channel. The `Mixer` reads
//! each track's chain-tail buffer from the compiled graph, applies per-track
//! gain (dB) and constant-power pan (-3 dB at centre), sums into an interleaved
//! stereo accumulator, then applies the master gain. Every level is smoothed by
//! a per-sample `LinearRamp` (anti-zipper); everything is pre-allocated, so the
//! whole stage is RT-SAFE after construction.

use serde::{Deserialize, Serialize};

use crate::engine::graph::CompiledGraph;
use crate::engine::params::LinearRamp;

/// Tracks the mixer pre-allocates smoothers for. Tracks beyond this are not
/// mixed (silent) — far beyond anything the MVP produces, but a hard bound so
/// the mixer never allocates on the audio thread.
pub const MAX_TRACKS: usize = 64;

/// Centre pan gain (-3 dB); also the fallback panning for a fading graph whose
/// track layout no longer matches the live one.
const CENTER: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Gain/pan/master smoothing ramp.
const RAMP_MS: f32 = 10.0;

/// A track's mixer settings, as stored in `song.json` and carried by the graph
/// description: gain in dB and constant-power pan position (-1 left .. +1 right).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackMix {
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub pan: f32,
}

/// dB → linear gain.
pub fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Constant-power stereo gains for a pan position: `θ = (pan+1)·π/4`,
/// left = cos θ, right = sin θ. Centre is -3 dB on both channels; the extremes
/// route everything to one side at unity.
fn pan_gains(pan: f32) -> (f32, f32) {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (theta.cos(), theta.sin())
}

/// The settled left/right gains a strip ramps toward (gain × pan law).
fn strip_targets(mix: TrackMix) -> (f32, f32) {
    let gain = db_to_gain(mix.gain_db);
    let (l, r) = pan_gains(mix.pan);
    (gain * l, gain * r)
}

/// One track's smoothed stereo gains.
struct Strip {
    left: LinearRamp,
    right: LinearRamp,
}

pub struct Mixer {
    strips: Vec<Strip>,
    master: LinearRamp,
    /// Track count of the last `mix` call, so a fading graph's tails can tell
    /// whether they still align with the live layout.
    last_tracks: usize,
}

impl Mixer {
    /// Build with every smoother already settled on its target (no initial
    /// glide), so the offline render and the real-time path start from the same
    /// state. Allocates: construct off the audio thread.
    pub fn new(sample_rate: f32, tracks: &[TrackMix], master_db: f32) -> Self {
        let strips = (0..MAX_TRACKS)
            .map(|t| {
                let (l, r) = strip_targets(tracks.get(t).copied().unwrap_or_default());
                Strip {
                    left: LinearRamp::new(l, RAMP_MS, sample_rate),
                    right: LinearRamp::new(r, RAMP_MS, sample_rate),
                }
            })
            .collect();
        Self {
            strips,
            master: LinearRamp::new(db_to_gain(master_db), RAMP_MS, sample_rate),
            last_tracks: 0,
        }
    }

    /// Retarget every strip and the master (a project reload / graph swap); the
    /// ramps glide to the new settings. // RT-SAFE
    pub fn retarget(&mut self, tracks: &[TrackMix], master_db: f32) {
        for (t, strip) in self.strips.iter_mut().enumerate() {
            let (l, r) = strip_targets(tracks.get(t).copied().unwrap_or_default());
            strip.left.set_target(l);
            strip.right.set_target(r);
        }
        self.master.set_target(db_to_gain(master_db));
    }

    /// Live gain/pan change for one track (fader drag). // RT-SAFE
    pub fn set_track(&mut self, track: usize, mix: TrackMix) {
        if let Some(strip) = self.strips.get_mut(track) {
            let (l, r) = strip_targets(mix);
            strip.left.set_target(l);
            strip.right.set_target(r);
        }
    }

    // RT-SAFE
    pub fn set_master_db(&mut self, db: f32) {
        self.master.set_target(db_to_gain(db));
    }

    /// Mix the graph's track tails into `out` (interleaved stereo, `frames * 2`
    /// samples), overwriting it: per-track gain+pan ramps tick per sample, then
    /// the master ramp scales the sum. // RT-SAFE
    pub fn mix(&mut self, graph: &CompiledGraph, frames: usize, out: &mut [f32]) {
        let out = &mut out[..frames * 2];
        out.fill(0.0);
        let tails = graph.track_tails();
        self.last_tracks = tails.len().min(MAX_TRACKS);
        for (strip, &node) in self.strips.iter_mut().zip(tails) {
            let buf = &graph.node_output(node)[..frames];
            for (frame, &x) in out.chunks_exact_mut(2).zip(buf) {
                frame[0] += x * strip.left.tick();
                frame[1] += x * strip.right.tick();
            }
        }
        for frame in out.chunks_exact_mut(2) {
            let m = self.master.tick();
            frame[0] *= m;
            frame[1] *= m;
        }
    }

    /// Add a fading graph's tails (removed effects ringing out after a swap) on
    /// top of an already-mixed block. The live `mix` pass owns the ramp ticking,
    /// so this uses the *current* (frozen) strip and master values; if the
    /// fading layout no longer matches the live track count, the tails fall back
    /// to centre. `fade` ticks once per frame. Returns the energy added, so the
    /// caller can collect the graph once it falls silent. // RT-SAFE
    pub fn add_faded(
        &self,
        graph: &CompiledGraph,
        frames: usize,
        fade: &mut LinearRamp,
        out: &mut [f32],
    ) -> f32 {
        let tails = graph.track_tails();
        let count = tails.len().min(MAX_TRACKS);
        let aligned = count == self.last_tracks;
        let master = self.master.value();
        let mut energy = 0.0f32;
        for (i, frame) in out[..frames * 2].chunks_exact_mut(2).enumerate() {
            let g = fade.tick() * master;
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for (t, &node) in tails.iter().enumerate().take(count) {
                let x = graph.node_output(node)[i];
                let (gl, gr) = if aligned {
                    (self.strips[t].left.value(), self.strips[t].right.value())
                } else {
                    (CENTER, CENTER)
                };
                l += x * gl;
                r += x * gr;
            }
            let (l, r) = (l * g, r * g);
            frame[0] += l;
            frame[1] += r;
            energy += l * l + r * r;
        }
        energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{compile, GraphDesc, NodeDesc};
    use crate::engine::swap::test_support;

    /// One track: a `dc` unit source whose tail is the node itself.
    fn dc_graph(track_mix: TrackMix, master_db: f32) -> crate::engine::graph::CompiledGraph {
        let desc = GraphDesc {
            nodes: vec![NodeDesc {
                id: "d".into(),
                module_id: "dc".into(),
                params: vec![],
            }],
            connections: vec![],
            tails: vec![0],
            track_mix: vec![track_mix],
            master_gain_db: master_db,
        };
        compile(&desc, test_support::create, 48_000.0, 64, 1).expect("compiles")
    }

    fn mix_one_block(track: TrackMix, master_db: f32) -> Vec<f32> {
        let mut graph = dc_graph(track, master_db);
        let mut mixer = Mixer::new(48_000.0, graph.track_mix(), graph.master_gain_db());
        graph.process_block(48_000.0, 64);
        let mut out = vec![0.0f32; 64 * 2];
        mixer.mix(&graph, 64, &mut out);
        out
    }

    #[test]
    fn centre_pan_is_minus_3db_on_both_channels() {
        let out = mix_one_block(TrackMix::default(), 0.0);
        assert!((out[0] - CENTER).abs() < 1e-6, "left {}", out[0]);
        assert!((out[1] - CENTER).abs() < 1e-6, "right {}", out[1]);
    }

    #[test]
    fn extreme_pans_route_to_one_side_at_unity() {
        let left = mix_one_block(
            TrackMix {
                gain_db: 0.0,
                pan: -1.0,
            },
            0.0,
        );
        assert!((left[0] - 1.0).abs() < 1e-6, "hard left L {}", left[0]);
        assert!(left[1].abs() < 1e-6, "hard left R {}", left[1]);

        let right = mix_one_block(
            TrackMix {
                gain_db: 0.0,
                pan: 1.0,
            },
            0.0,
        );
        assert!(right[0].abs() < 1e-6, "hard right L {}", right[0]);
        assert!((right[1] - 1.0).abs() < 1e-6, "hard right R {}", right[1]);
    }

    #[test]
    fn gain_and_master_scale_in_db() {
        // -6.02 dB track + -6.02 dB master ≈ ×0.25 on the centre gains.
        let out = mix_one_block(
            TrackMix {
                gain_db: -6.0206,
                pan: 0.0,
            },
            -6.0206,
        );
        let expected = CENTER * 0.25;
        assert!(
            (out[0] - expected).abs() < 1e-4,
            "left {} vs {expected}",
            out[0]
        );
        assert!(
            (out[1] - expected).abs() < 1e-4,
            "right {} vs {expected}",
            out[1]
        );
    }

    #[test]
    fn retarget_glides_without_a_step() {
        let mut graph = dc_graph(TrackMix::default(), 0.0);
        let mut mixer = Mixer::new(48_000.0, graph.track_mix(), graph.master_gain_db());
        graph.process_block(48_000.0, 64);
        let mut out = vec![0.0f32; 64 * 2];
        mixer.mix(&graph, 64, &mut out);

        // Retarget hard left: the left channel must move smoothly, not jump.
        mixer.retarget(
            &[TrackMix {
                gain_db: 0.0,
                pan: -1.0,
            }],
            0.0,
        );
        let mut whole = Vec::new();
        for _ in 0..20 {
            graph.process_block(48_000.0, 64);
            mixer.mix(&graph, 64, &mut out);
            whole.extend(out.iter().step_by(2).copied()); // left channel
        }
        let max_delta = whole
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_delta < 0.01, "retarget stepped: {max_delta}");
        let settled = whole[whole.len() - 1];
        assert!(
            (settled - 1.0).abs() < 1e-3,
            "did not reach hard left: {settled}"
        );
    }
}
