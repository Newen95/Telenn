//! cpal output stream, the real-time callback, and the control-side handle.
//!
//! `cpal::Stream` is `!Send`, so it cannot live in Tauri's managed state. A
//! dedicated audio thread builds the stream, starts it, then parks until the
//! engine is dropped — at which point the stream is dropped on that thread,
//! never inside the callback. The handle kept by the control side holds only
//! `Send + Sync` parts: the ring producer, the xrun counter and the real
//! sample rate.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::control::ControlMsg;
use crate::engine::rt::RtState;

/// Capacity of the control→audio ring (per ARCHITECTURE.md).
const CONTROL_RING_CAPACITY: usize = 1024;
/// Block size requested from the device where it advertises a usable range.
const DESIRED_BUFFER: u32 = 256;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no default output device available")]
    NoDevice,
    #[error("could not query the default output configuration")]
    Config,
    #[error("unsupported sample format (f32 expected)")]
    UnsupportedFormat,
    #[error("could not build the output stream")]
    BuildStream,
    #[error("could not start the output stream")]
    StreamPlay,
    #[error("could not spawn the audio thread")]
    ThreadSpawn,
}

/// Control-side handle to the running audio engine.
pub struct AudioEngine {
    commands: Mutex<Producer<ControlMsg>>,
    xruns: Arc<AtomicU32>,
    sample_rate: u32,
    /// Kept alive to hold the audio thread open; dropping it releases the
    /// thread, which then drops the stream off the real-time callback.
    #[allow(dead_code)]
    shutdown: SyncSender<()>,
}

impl AudioEngine {
    pub fn start() -> Result<Self, EngineError> {
        let (producer, consumer) = RingBuffer::<ControlMsg>::new(CONTROL_RING_CAPACITY);
        let xruns = Arc::new(AtomicU32::new(0));
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel::<()>(0);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, EngineError>>();

        let xruns_audio = Arc::clone(&xruns);
        std::thread::Builder::new()
            .name("telenn-audio".into())
            .spawn(move || match build_stream(consumer, xruns_audio) {
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
            })
            .map_err(|_| EngineError::ThreadSpawn)?;

        let sample_rate = ready_rx.recv().map_err(|_| EngineError::ThreadSpawn)??;

        Ok(Self {
            commands: Mutex::new(producer),
            xruns,
            sample_rate,
            shutdown: shutdown_tx,
        })
    }

    /// Push a control message to the audio thread. Returns `false` if the ring
    /// is full (message dropped) or the lock is unavailable.
    pub fn send(&self, msg: ControlMsg) -> bool {
        match self.commands.lock() {
            Ok(mut producer) => producer.push(msg).is_ok(),
            Err(_) => false,
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
    xruns: Arc<AtomicU32>,
) -> Result<(cpal::Stream, u32), EngineError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(EngineError::NoDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|_| EngineError::Config)?;

    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(EngineError::UnsupportedFormat);
    }

    // Request 256-frame blocks where the device advertises a usable range; fall
    // back to the host default when the range is unknown (e.g. WASAPI), so the
    // stream never fails to build over buffer sizing alone.
    let buffer_size = match supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed(DESIRED_BUFFER.clamp(*min, *max))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    };

    let mut config: cpal::StreamConfig = supported.into();
    config.buffer_size = buffer_size;

    let sample_rate = config.sample_rate;
    let channels = config.channels as usize;

    let mut rt = RtState::new(sample_rate as f32);
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
        .map_err(|_| EngineError::BuildStream)?;

    Ok((stream, sample_rate))
}
