//! The `AudioNode` trait and its supporting types.
//!
//! A node is the unit of the audio graph: it declares typed ports and
//! parameters, and renders a block of audio. Everything on the `process` path
//! is RT-SAFE; allocation happens only in `prepare`.

/// Stable per-module parameter identifier (an index into the module's params).
pub type ParamId = u16;
/// Flat index of a node in a compiled graph.
pub type NodeIdx = usize;

/// The three signal types that can flow between ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Notes,
    Audio,
    // Control-rate modulation. Declared now for type-checking completeness;
    // modulation routing lands in a later milestone.
    #[allow(dead_code)]
    Modulation,
}

#[derive(Debug, Clone)]
pub struct Port {
    pub name: &'static str,
    pub kind: SignalKind,
}

#[derive(Debug, Clone, Default)]
pub struct PortSpec {
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

/// How a normalized 0..1 control value maps to the parameter's real range.
/// Serialized (lowercase) so the UI can display real values next to sliders.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamCurve {
    /// Linear between `min` and `max`.
    Linear,
    /// Geometric between `min` and `max` (both must be > 0), e.g. frequencies.
    Log,
    /// `min`/`max` are decibels; the real value is the corresponding gain.
    Db,
}

/// Declarative description of a parameter exposed by a module. Real values
/// (Hz, dB, ms…) live in `min`/`max`/`default`; the wire carries normalized
/// 0..1 values converted through `curve`.
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub id: ParamId,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub curve: ParamCurve,
}

impl ParamSpec {
    /// Map a normalized 0..1 value to the parameter's real value.
    pub fn denormalize(&self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self.curve {
            ParamCurve::Linear => self.min + (self.max - self.min) * v,
            ParamCurve::Log => self.min * (self.max / self.min).powf(v),
            ParamCurve::Db => {
                let db = self.min + (self.max - self.min) * v;
                10.0f32.powf(db / 20.0)
            }
        }
    }

    /// Normalized 0..1 position of a real value. The inverse of
    /// [`denormalize`](Self::denormalize) for `Linear`/`Log`. For `Db`, `min`,
    /// `max` and the `real` argument are all in decibels (denormalize then
    /// converts the chosen dB to a linear gain), so this stays a plain ratio.
    pub fn normalize(&self, real: f32) -> f32 {
        let v = match self.curve {
            ParamCurve::Linear | ParamCurve::Db => (real - self.min) / (self.max - self.min),
            ParamCurve::Log => (real / self.min).ln() / (self.max / self.min).ln(),
        };
        v.clamp(0.0, 1.0)
    }
}

/// A timed note event delivered to a node's `Notes` input for the current block.
#[derive(Debug, Clone, Copy)]
pub struct NoteEvent {
    /// Sample offset within the block. M2 always uses 0 (real-time input);
    /// sample-accurate offsets arrive with the sequencer (M3).
    pub frame: usize,
    pub kind: NoteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    On { pitch: u8, vel: u8 },
    Off { pitch: u8 },
}

/// Per-block context passed to every node.
pub struct ProcessCtx {
    pub sample_rate: f32,
    pub frames: usize,
}

/// The buffers and note events a node reads/writes for one block. Audio buffers
/// are flat, port-major (`port * frames`), so the node never sees a slice of
/// slices. // RT-SAFE
pub struct NodeIo<'a> {
    notes: &'a [NoteEvent],
    inputs: &'a [f32],
    output: &'a mut [f32],
    frames: usize,
}

impl<'a> NodeIo<'a> {
    pub fn new(
        notes: &'a [NoteEvent],
        inputs: &'a [f32],
        output: &'a mut [f32],
        frames: usize,
    ) -> Self {
        Self {
            notes,
            inputs,
            output,
            frames,
        }
    }

    pub fn notes(&self) -> &[NoteEvent] {
        self.notes
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Read-only audio input buffer for `port`.
    pub fn input(&self, port: usize) -> &[f32] {
        let start = port * self.frames;
        &self.inputs[start..start + self.frames]
    }

    /// Writable audio output buffer for `port`.
    pub fn output(&mut self, port: usize) -> &mut [f32] {
        let start = port * self.frames;
        &mut self.output[start..start + self.frames]
    }

    /// Borrow an input (read) and an output (write) at once. They are distinct
    /// buffers, so an effect can map one to the other in a single pass. // RT-SAFE
    pub fn in_out(&mut self, in_port: usize, out_port: usize) -> (&[f32], &mut [f32]) {
        let is = in_port * self.frames;
        let os = out_port * self.frames;
        (
            &self.inputs[is..is + self.frames],
            &mut self.output[os..os + self.frames],
        )
    }
}

/// A processing node: instrument or effect. `Send` so a compiled graph can be
/// moved to the audio thread.
pub trait AudioNode: Send {
    /// Render one block. // RT-SAFE
    fn process(&mut self, ctx: &ProcessCtx, io: &mut NodeIo);

    /// Declared ports (types and names). Called off the audio thread.
    fn ports(&self) -> PortSpec;

    /// Declared parameters. Called off the audio thread.
    fn params(&self) -> &[ParamSpec];

    /// Set a parameter from a normalized 0..1 value. Only posts a target;
    /// smoothing happens inside the node. // RT-SAFE
    fn set_param(&mut self, id: ParamId, value: f32);

    /// Release every sounding voice (transport stop / panic). Voices fade out
    /// through their normal release, so this stays click-free. Default no-op for
    /// nodes that hold no voices (effects). // RT-SAFE
    fn all_notes_off(&mut self) {}

    /// Off-thread preparation. All allocation belongs here, never in `process`.
    fn prepare(&mut self, sample_rate: f32, max_block: usize);
}
