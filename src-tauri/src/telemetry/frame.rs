//! The telemetry frame written by the audio thread and the control-side
//! aggregation that feeds one UI event.
//!
//! M5 phase 5a is the minimal vertical slice: a playhead and a master peak. The
//! frame is `Copy` and fixed-size so the audio thread writes it into an `rtrb`
//! ring with no allocation; the pump drains the ring and aggregates a batch into
//! a single value emitted to the UI at ~60 Hz.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryFrame {
    /// Playhead position in samples from the loop start.
    pub playhead_samples: u64,
    /// Whether the sequencer is running (the UI interpolates only when true).
    pub playing: bool,
    /// Peak |sample| of the master output over the block.
    pub master_peak: f32,
}

/// Collapse a batch of frames into the one value the UI needs: the **latest**
/// playhead/playing and the **max** peak (so a transient between two UI frames
/// is never missed). `None` if the batch is empty.
pub fn aggregate(frames: impl IntoIterator<Item = TelemetryFrame>) -> Option<TelemetryFrame> {
    let mut out: Option<TelemetryFrame> = None;
    for f in frames {
        out = Some(match out {
            None => f,
            Some(prev) => TelemetryFrame {
                playhead_samples: f.playhead_samples,
                playing: f.playing,
                master_peak: prev.master_peak.max(f.master_peak),
            },
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
    fn aggregate_of_empty_is_none() {
        assert!(aggregate(std::iter::empty()).is_none());
    }
}
