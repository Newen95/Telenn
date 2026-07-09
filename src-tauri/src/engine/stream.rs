//! cpal output stream, the real-time callback, and the control-side handle.
//!
//! `cpal::Stream` is `!Send`, so it cannot live in Tauri's managed state. A
//! dedicated audio thread builds the stream, starts it, then parks until the
//! engine is dropped — at which point the stream is dropped on that thread,
//! never inside the callback. The handle kept by the control side holds only
//! `Send + Sync` parts: the ring producer, the garbage consumer, the xrun
//! counter, the note-routing index and the real sample rate.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::control::project::{GraphIndex, ProjectError};
use crate::control::{ControlMsg, EngineInit};
use crate::engine::graph::GraphError;
use crate::engine::rt::RtState;
use crate::engine::swap::Garbage;
use crate::engine::MAX_BLOCK;
use crate::telemetry::frame::TelemetryFrame;

/// Capacity of the control→audio ring (per ARCHITECTURE.md).
const CONTROL_RING_CAPACITY: usize = 1024;
/// Capacity of the audio→control garbage ring for displaced schedules and graphs.
const GARBAGE_RING_CAPACITY: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no default output device available")]
    NoDevice,
    #[error("could not query the default output configuration")]
    Config,
    #[error("unsupported sample format (f32 expected)")]
    UnsupportedFormat,
    #[error("could not build the output stream: {0}")]
    BuildStream(String),
    #[error("could not start the output stream")]
    StreamPlay,
    #[error("could not spawn the audio thread")]
    ThreadSpawn,
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// Control-side handle to the running audio engine.
pub struct AudioEngine {
    commands: Mutex<Producer<ControlMsg>>,
    /// Displaced schedules and graphs returned by the audio thread, dropped here off-RT.
    garbage: Mutex<Consumer<Garbage>>,
    /// Track → node routing of the running graph, used to resolve a reloaded
    /// project's notes against the topology already on the audio thread.
    /// Updated with every graph swap (a track edit shifts instrument indices).
    index: Mutex<GraphIndex>,
    xruns: Arc<AtomicU32>,
    sample_rate: u32,
    /// Kept alive to hold the audio thread open; dropping it releases the
    /// thread, which then drops the stream off the real-time callback.
    #[allow(dead_code)]
    shutdown: SyncSender<()>,
}

impl AudioEngine {
    /// Start the engine. `init` compiles the graph and builds the schedule once
    /// the real sample rate is known, on the audio thread; `index` is the
    /// note-routing of that graph, kept for later `load_project` calls.
    pub fn start(
        init: EngineInit,
        index: GraphIndex,
        telemetry: Producer<TelemetryFrame>,
    ) -> Result<Self, EngineError> {
        let (producer, consumer) = RingBuffer::<ControlMsg>::new(CONTROL_RING_CAPACITY);
        let (garbage_prod, garbage_cons) = RingBuffer::<Garbage>::new(GARBAGE_RING_CAPACITY);
        let xruns = Arc::new(AtomicU32::new(0));
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel::<()>(0);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, EngineError>>();

        let xruns_audio = Arc::clone(&xruns);
        std::thread::Builder::new()
            .name("telenn-audio".into())
            .spawn(move || {
                match build_stream(consumer, garbage_prod, telemetry, xruns_audio, init) {
                    Ok((stream, sample_rate)) => {
                        if stream.play().is_err() {
                            let _ = ready_tx.send(Err(EngineError::StreamPlay));
                            return;
                        }
                        let _ = ready_tx.send(Ok(sample_rate));
                        // Park until the engine is dropped; `stream` is then dropped
                        // here, on this thread, never inside the callback.
                        let _ = shutdown_rx.recv();
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|_| EngineError::ThreadSpawn)?;

        let sample_rate = ready_rx.recv().map_err(|_| EngineError::ThreadSpawn)??;

        Ok(Self {
            commands: Mutex::new(producer),
            garbage: Mutex::new(garbage_cons),
            index: Mutex::new(index),
            xruns,
            sample_rate,
            shutdown: shutdown_tx,
        })
    }

    /// Push a control message to the audio thread. Returns `false` if the ring
    /// is full (message dropped) or the lock is unavailable. Also drains any
    /// schedules the audio thread returned, dropping them here (off-RT).
    pub fn send(&self, msg: ControlMsg) -> bool {
        self.drain_garbage();
        match self.commands.lock() {
            Ok(mut producer) => producer.push(msg).is_ok(),
            Err(_) => false,
        }
    }

    /// Drop every schedule the audio thread handed back. Off the real-time path.
    fn drain_garbage(&self) {
        if let Ok(mut cons) = self.garbage.lock() {
            while let Ok(old) = cons.pop() {
                drop(old);
            }
        }
    }

    /// Note-routing index of the running graph (a small map; cloned so no lock
    /// is held across schedule building).
    pub fn index(&self) -> GraphIndex {
        match self.index.lock() {
            Ok(index) => index.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Publish the routing index of a newly swapped graph, so later schedule
    /// builds resolve against the topology actually on the audio thread.
    pub fn set_index(&self, index: GraphIndex) {
        match self.index.lock() {
            Ok(mut current) => *current = index,
            Err(poisoned) => *poisoned.into_inner() = index,
        }
    }

    pub fn xruns(&self) -> u32 {
        self.xruns.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Open the default output device and build the f32 output stream. Returns the
/// stream and the real sample rate obtained (never assume it).
fn build_stream(
    mut consumer: Consumer<ControlMsg>,
    garbage: Producer<Garbage>,
    telemetry: Producer<TelemetryFrame>,
    xruns: Arc<AtomicU32>,
    init: EngineInit,
) -> Result<(cpal::Stream, u32), EngineError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(EngineError::NoDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|_| EngineError::Config)?;

    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(EngineError::UnsupportedFormat);
    }

    let mut config: cpal::StreamConfig = supported.into();
    // Use the device's native buffer size. A `Fixed` request is rejected by
    // WASAPI shared mode ("not supported in shared mode") even when the device
    // advertises a range, so we don't force one; the engine processes any block
    // size in `MAX_BLOCK` chunks and the default latency (~10 ms) is fine.
    config.buffer_size = cpal::BufferSize::Default;

    let sample_rate = config.sample_rate;
    let channels = config.channels as usize;

    let (graph, schedule) = init(sample_rate as f32, MAX_BLOCK)?;
    let mut rt = RtState::new(
        sample_rate as f32,
        graph,
        schedule,
        garbage,
        telemetry,
        Arc::clone(&xruns),
    );
    let err_xruns = Arc::clone(&xruns);

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // RT-SAFE: drain every pending control message, then render.
                while let Ok(msg) = consumer.pop() {
                    rt.handle(msg);
                }
                rt.process(data, channels);
            },
            move |_err| {
                // Stream errors (under/overruns) are counted, never logged
                // (logging from the audio path is a syscall).
                err_xruns.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| EngineError::BuildStream(e.to_string()))?;

    Ok((stream, sample_rate))
}
