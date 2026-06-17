//! The telemetry frame written by the audio thread and the control-side
//! aggregation that feeds one UI event.
//!
//! The frame is `Copy` and fixed-size so the audio thread writes it into an
//! `rtrb` ring with no allocation; the pump drains the ring and aggregates a
//! batch into a single value emitted to the UI at ~60 Hz. M7 fills it out from
//! the M5 minimal slice (playhead + master peak) to the full capacity: per-node
//! peak/RMS, the audio compute load and the cumulative xrun count.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Maximum number of per-node meters carried in a frame (cf. ARCHITECTURE.md).
/// Nodes beyond this are not metered; `meter_count` reports how many are valid.
pub const MAX_METERS: usize = 64;

/// One node's post-process level over a block: peak |sample| and RMS.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TelemetryFrame {
    /// Playhead position in samples from the loop start.
    pub playhead_samples: u64,
    /// Whether the sequencer is running (the UI interpolates only when true).
    pub playing: bool,
    /// Peak |sample| of the master output over the block.
    pub master_peak: f32,
    /// Graph compute time over the block budget (`compute / (frames / sample_rate)`).
    /// Above 1.0 means the block did not fit its real-time budget.
    pub audio_load: f32,
    /// Cumulative output-stream under/overrun count.
    pub xruns: u32,
    /// Number of valid entries in `meters` (the graph's node count, capped).
    pub meter_count: u8,
    /// Per-node meters, indexed by node index (the `to_graph` order, so
    /// `meters[i]` is the node whose stable id is `node_ids()[i]`).
    pub meters: [Meter; MAX_METERS],
}

// Hand-written so the fixed `[Meter; MAX_METERS]` array (serde derives Serialize
// only up to 32 elements) is sent as just its `meter_count` valid entries — a
// slice serializes fine and the wire payload carries no padding nodes.
impl Serialize for TelemetryFrame {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let count = (self.meter_count as usize).min(MAX_METERS);
        let mut s = serializer.serialize_struct("TelemetryFrame", 7)?;
        s.serialize_field("playheadSamples", &self.playhead_samples)?;
        s.serialize_field("playing", &self.playing)?;
        s.serialize_field("masterPeak", &self.master_peak)?;
        s.serialize_field("audioLoad", &self.audio_load)?;
        s.serialize_field("xruns", &self.xruns)?;
        s.serialize_field("meterCount", &self.meter_count)?;
        s.serialize_field("meters", &self.meters[..count])?;
        s.end()
    }
}

impl Default for TelemetryFrame {
    fn default() -> Self {
        Self {
            playhead_samples: 0,
            playing: false,
            master_peak: 0.0,
            audio_load: 0.0,
            xruns: 0,
            meter_count: 0,
            meters: [Meter::default(); MAX_METERS],
        }
    }
}

/// Collapse a batch of frames into the one value the UI needs: the **latest**
/// playhead/playing/xruns/meter_count, and the **max** of every level
/// (`master_peak`, `audio_load`, and each node's peak/RMS) so a transient or a
/// load spike between two UI frames is never missed. `None` if the batch empty.
pub fn aggregate(frames: impl IntoIterator<Item = TelemetryFrame>) -> Option<TelemetryFrame> {
    let mut out: Option<TelemetryFrame> = None;
    for f in frames {
        out = Some(match out {
            None => f,
            Some(mut prev) => {
                prev.playhead_samples = f.playhead_samples;
                prev.playing = f.playing;
                prev.xruns = f.xruns;
                prev.meter_count = f.meter_count;
                prev.master_peak = prev.master_peak.max(f.master_peak);
                prev.audio_load = prev.audio_load.max(f.audio_load);
                for (m, n) in prev.meters.iter_mut().zip(f.meters.iter()) {
                    m.peak = m.peak.max(n.peak);
                    m.rms = m.rms.max(n.rms);
                }
                prev
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(playhead: u64, peak: f32) -> TelemetryFrame {
        TelemetryFrame {
            playhead_samples: playhead,
            playing: true,
            master_peak: peak,
            ..Default::default()
        }
    }

    #[test]
    fn aggregate_keeps_max_peak_and_latest_playhead() {
        let batch = [frame(100, 0.2), frame(200, 0.8), frame(300, 0.5)];
        let agg = aggregate(batch).expect("non-empty");
        assert_eq!(agg.playhead_samples, 300, "playhead must be the latest");
        assert!((agg.master_peak - 0.8).abs() < 1e-6, "peak must be the max");
    }

    #[test]
    fn aggregate_keeps_max_load_and_latest_xruns() {
        let mut a = frame(10, 0.0);
        a.audio_load = 0.3;
        a.xruns = 1;
        let mut b = frame(20, 0.0);
        b.audio_load = 0.9; // a spike between two UI frames
        b.xruns = 2;
        let mut c = frame(30, 0.0);
        c.audio_load = 0.4;
        c.xruns = 2;
        let agg = aggregate([a, b, c]).expect("non-empty");
        assert!((agg.audio_load - 0.9).abs() < 1e-6, "load must be the max");
        assert_eq!(agg.xruns, 2, "xruns must be the latest cumulative value");
    }

    #[test]
    fn aggregate_keeps_max_per_node_meters() {
        let mut a = frame(10, 0.0);
        a.meter_count = 2;
        a.meters[0] = Meter {
            peak: 0.2,
            rms: 0.1,
        };
        a.meters[1] = Meter {
            peak: 0.9,
            rms: 0.5,
        };
        let mut b = frame(20, 0.0);
        b.meter_count = 2;
        b.meters[0] = Meter {
            peak: 0.7,
            rms: 0.4,
        };
        b.meters[1] = Meter {
            peak: 0.3,
            rms: 0.2,
        };
        let agg = aggregate([a, b]).expect("non-empty");
        assert!((agg.meters[0].peak - 0.7).abs() < 1e-6);
        assert!((agg.meters[1].peak - 0.9).abs() < 1e-6);
        assert!((agg.meters[0].rms - 0.4).abs() < 1e-6);
        assert!((agg.meters[1].rms - 0.5).abs() < 1e-6);
        assert_eq!(agg.meter_count, 2);
    }

    #[test]
    fn aggregate_of_empty_is_none() {
        assert!(aggregate(std::iter::empty()).is_none());
    }
}
