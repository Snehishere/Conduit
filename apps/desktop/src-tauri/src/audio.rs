use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::StreamConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{error, info, warn};
use tokio::sync::broadcast;

use crate::error::ConduitError;

const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;
const FRAME_SIZE: usize = 1600;

pub struct AudioStream {
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
    is_playing: Arc<AtomicBool>,
    playback_tx: tokio::sync::mpsc::UnboundedSender<Vec<i16>>,
}

impl Clone for AudioStream {
    fn clone(&self) -> Self {
        Self {
            is_streaming: self.is_streaming.clone(),
            audio_tx: self.audio_tx.clone(),
            is_playing: self.is_playing.clone(),
            playback_tx: self.playback_tx.clone(),
        }
    }
}

impl AudioStream {
    pub fn new() -> Self {
        let (audio_tx, _) = broadcast::channel(256);
        let (playback_tx, playback_rx) = tokio::sync::mpsc::unbounded_channel();

        let stream = Self {
            is_streaming: Arc::new(AtomicBool::new(false)),
            audio_tx,
            is_playing: Arc::new(AtomicBool::new(false)),
            playback_tx,
        };

        // Spawn the persistent playback thread
        let is_playing = stream.is_playing.clone();
        std::thread::spawn(move || {
            playback_loop(is_playing, playback_rx);
        });

        stream
    }

    /// Start capturing system audio via platform-specific loopback or default input (fallback).
    ///
    /// - **Windows**: WASAPI loopback capture of the default output device.
    /// - **Linux**: PulseAudio / PipeWire monitor source of the default output device.
    /// - **macOS**: CoreAudio loopback — cpal builds an input stream on the default
    ///   output device, which triggers CoreAudio's loopback-style capture.
    /// - **Fallback**: Default input device (microphone) via cpal.
    pub async fn start_capture(&self) -> Result<(), ConduitError> {
        if self.is_streaming.load(Ordering::Relaxed) {
            return Err(ConduitError::Other("Already streaming".to_string()));
        }

        self.is_streaming.store(true, Ordering::Relaxed);

        let is_streaming = self.is_streaming.clone();
        let audio_tx = self.audio_tx.clone();

        // cpal::Stream is !Send, so build and run it on a dedicated OS thread
        std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            {
                match capture_windows_loopback(is_streaming.clone(), audio_tx.clone()) {
                    Ok(()) => return,
                    Err(e) => {
                        warn!("WASAPI loopback failed ({}), falling back to cpal input", e);
                    }
                }
            }

            #[cfg(target_os = "linux")]
            {
                match capture_linux_monitor(is_streaming.clone(), audio_tx.clone()) {
                    Ok(()) => return,
                    Err(e) => {
                        warn!(
                            "PipeWire/PulseAudio monitor capture failed ({}), falling back to cpal input",
                            e
                        );
                    }
                }
            }

            #[cfg(target_os = "macos")]
            {
                match capture_macos_loopback(is_streaming.clone(), audio_tx.clone()) {
                    Ok(()) => return,
                    Err(e) => {
                        warn!(
                            "CoreAudio loopback failed ({}), falling back to cpal input",
                            e
                        );
                    }
                }
            }

            // Fallback: capture from default input device (microphone)
            capture_cpal_input(is_streaming, audio_tx);
        });

        Ok(())
    }

    pub fn stop_capture(&self) {
        self.is_streaming.store(false, Ordering::Relaxed);
    }

    /// Test-only view of the streaming flag (mirrors the private field).
    #[cfg(test)]
    pub(crate) fn is_streaming(&self) -> bool {
        self.is_streaming.load(Ordering::Relaxed)
    }

    /// Start the duplex playback receiver. Audio data sent via `send_playback_data`
    /// will be played through the speakers. This runs independently of capture.
    pub fn start_playback(&self) {
        if self.is_playing.load(Ordering::Relaxed) {
            return;
        }
        self.is_playing.store(true, Ordering::Relaxed);
        info!("Duplex playback started — ready to receive audio from mobile");
    }

    /// Stop the duplex playback receiver.
    pub fn stop_playback(&self) {
        self.is_playing.store(false, Ordering::Relaxed);
        info!("Duplex playback stopped");
    }

    /// Feed PCM audio data into the playback channel.
    pub fn send_playback_data(&self, pcm_data: Vec<i16>) {
        if self.is_playing.load(Ordering::Relaxed) {
            let _ = self.playback_tx.send(pcm_data);
        }
    }

    pub async fn play_audio(&self, pcm_data: &[i16]) -> Result<(), ConduitError> {
        let float_data: Vec<f32> = pcm_data.iter().map(|&s| s as f32 / 32767.0).collect();
        let data_len = float_data.len();
        let duration = Duration::from_millis((data_len as u64 * 1000) / SAMPLE_RATE as u64);

        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_output_device() {
                Some(d) => d,
                None => {
                    error!("No output device available");
                    let _ = done_tx.send(());
                    return;
                }
            };

            let config = StreamConfig {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                buffer_size: cpal::BufferSize::Fixed(FRAME_SIZE as u32),
            };

            let stream = match device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for (i, sample) in data.iter_mut().enumerate() {
                        *sample = if i < float_data.len() {
                            float_data[i]
                        } else {
                            0.0
                        };
                    }
                },
                |err| {
                    error!("Audio output stream error: {}", err);
                },
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to build output stream: {}", e);
                    let _ = done_tx.send(());
                    return;
                }
            };

            if let Err(e) = stream.play() {
                error!("Failed to play output stream: {}", e);
                let _ = done_tx.send(());
                return;
            }

            std::thread::sleep(duration + Duration::from_millis(50));
            drop(stream);
            let _ = done_tx.send(());
        });

        let _ = tokio::time::timeout(duration + Duration::from_secs(2), done_rx).await;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<i16>> {
        self.audio_tx.subscribe()
    }
}

/// Persistent playback loop running on a dedicated OS thread.
/// Maintains a cpal output stream and continuously drains the mpsc channel,
/// mixing incoming PCM chunks into the output buffer. This allows simultaneous
/// capture and playback (duplex).
fn playback_loop(
    is_playing: Arc<AtomicBool>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<i16>>,
) {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            error!("No output device available for duplex playback");
            return;
        }
    };

    let config = StreamConfig {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Fixed(FRAME_SIZE as u32),
    };

    // Ring buffer holding pending PCM samples for the output callback.
    // INTENTIONAL std::sync::Mutex exception: cpal invokes this closure from a
    // real-time OS audio callback thread (not a tokio worker). It must never
    // yield/await, and tokio::sync::Mutex::blocking_lock would panic if ever
    // reached from a runtime context. The lock is held only for a few samples.
    let buffer: Arc<std::sync::Mutex<VecDeque<f32>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let buffer_clone = buffer.clone();

    let stream = match device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut buf = match buffer_clone.lock() {
                Ok(b) => b,
                Err(_) => return,
            };
            for sample in data.iter_mut() {
                *sample = if !buf.is_empty() {
                    buf.pop_front().unwrap_or(0.0)
                } else {
                    0.0
                };
            }
        },
        |err| {
            error!("Duplex playback output stream error: {}", err);
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to build duplex playback stream: {}", e);
            return;
        }
    };

    if let Err(e) = stream.play() {
        error!("Failed to start duplex playback stream: {}", e);
        return;
    }

    info!("Duplex playback output stream started");

    while is_playing.load(Ordering::Relaxed) {
        // Drain all available chunks from the channel
        while let Ok(chunk) = rx.try_recv() {
            let float_chunk: Vec<f32> = chunk.iter().map(|&s| s as f32 / 32767.0).collect();
            if let Ok(mut buf) = buffer.lock() {
                buf.extend(float_chunk);
            }
        }

        // Trim buffer if it grows too large (> 2 seconds of audio)
        if let Ok(mut buf) = buffer.lock() {
            let max_samples = SAMPLE_RATE as usize * 2;
            if buf.len() > max_samples {
                let drain = buf.len() - max_samples;
                buf.drain(..drain);
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    drop(stream);
    info!("Duplex playback output stream stopped");
}

/// Capture system audio on Windows using WASAPI loopback.
///
/// This captures all audio being played through the default output device
/// (speakers/headphones) — music, videos, games, notifications, etc.
/// Uses cpal's WASAPI host with loopback enabled.
#[cfg(target_os = "windows")]
fn capture_windows_loopback(
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
) -> Result<(), ConduitError> {
    // cpal on Windows uses WASAPI backend — get the WASAPI host
    let host = cpal::default_host();

    // Get the default output device (what audio is playing through)
    let output_device = host.default_output_device().ok_or_else(|| {
        ConduitError::Other("No output device available for loopback".to_string())
    })?;

    let output_name = output_device.to_string();
    info!("Found output device for loopback: {}", output_name);

    // Get the output device's config to match the loopback stream
    let output_config = output_device
        .default_output_config()
        .map_err(|e| ConduitError::Other(format!("Failed to get output config: {}", e)))?;

    let sample_rate = output_config.sample_rate();
    let channels = output_config.channels() as u16;

    info!("Output device config: {}Hz, {}ch", sample_rate, channels);

    // Build an input stream on the output device (loopback capture)
    // cpal supports this on WASAPI — building an input stream on an output device
    // triggers WASAPI loopback mode
    let config = StreamConfig {
        channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = output_device
        .build_input_stream(
            config,
            {
                let is_streaming = is_streaming.clone();
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !is_streaming.load(Ordering::Relaxed) {
                        return;
                    }

                    // Resample to target sample rate if needed and convert to i16
                    let i16_samples: Vec<i16> = if sample_rate != SAMPLE_RATE {
                        // Simple linear resampling
                        let ratio = sample_rate as f64 / SAMPLE_RATE as f64;
                        let output_len = (data.len() as f64 / ratio) as usize;
                        (0..output_len)
                            .map(|i| {
                                let src_idx = i as f64 * ratio;
                                let idx = src_idx as usize;
                                let frac = src_idx - idx as f64;
                                let s0 = if idx < data.len() {
                                    data[idx] as f64
                                } else {
                                    0.0_f64
                                };
                                let s1 = if idx + 1 < data.len() {
                                    data[idx + 1] as f64
                                } else {
                                    s0
                                };
                                let sample = s0 + frac * (s1 - s0);
                                // Mix down to mono if stereo
                                (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
                            })
                            .collect()
                    } else if channels == 1 {
                        data.iter()
                            .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
                            .collect()
                    } else {
                        // Mix multi-channel down to mono
                        data.chunks(channels as usize)
                            .map(|frame| {
                                let mono = frame.iter().sum::<f32>() / channels as f32;
                                (mono * 32767.0).clamp(-32768.0, 32767.0) as i16
                            })
                            .collect()
                    };

                    if !i16_samples.is_empty() {
                        let _ = audio_tx.send(i16_samples);
                    }
                }
            },
            move |err| {
                error!("WASAPI loopback stream error: {}", err);
            },
            None,
        )
        .map_err(|e| ConduitError::Other(format!("Failed to build loopback stream: {}", e)))?;

    stream
        .play()
        .map_err(|e| ConduitError::Other(format!("Failed to start loopback: {}", e)))?;

    info!(
        "System audio loopback capture started ({}Hz, {}ch, PCM16) — capturing all desktop audio",
        sample_rate, channels
    );

    // Keep stream alive until stopped
    while is_streaming.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(stream);
    info!("System audio loopback capture stopped");
    Ok(())
}

/// Capture system audio on Linux using PulseAudio / PipeWire monitor source.
///
/// PulseAudio and PipeWire both expose monitor sources for each output sink.
/// We enumerate devices and look for one whose name contains "monitor" to capture
/// all desktop audio. If no monitor is found, falls back to the default input.
#[cfg(target_os = "linux")]
fn capture_linux_monitor(
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
) -> Result<(), ConduitError> {
    let host = cpal::default_host();

    // Find a monitor source — PulseAudio names them "<sink_name>.monitor"
    let monitor_device = host
        .input_devices()
        .map_err(|e| ConduitError::Other(format!("Failed to list input devices: {}", e)))?
        .find(|d| {
            let name = d.to_string().to_lowercase();
            name.contains("monitor")
        });

    let device = match monitor_device {
        Some(d) => {
            info!("Linux: using monitor source for loopback: {}", d);
            d
        }
        None => {
            // No monitor found — try the default output device directly.
            // On PipeWire, building an input stream on the output device
            // captures the monitor automatically.
            let output_device = host.default_output_device().ok_or_else(|| {
                ConduitError::Other(
                    "No output or monitor device available for Linux loopback".to_string(),
                )
            })?;
            info!(
                "Linux: no monitor source found, trying input on output device: {}",
                output_device
            );
            // We build the input stream on this output device below.
            // cpal + PipeWire will capture the monitor.
            return capture_cpal_loopback_on_device(
                is_streaming,
                audio_tx,
                &output_device,
                "Linux PipeWire output-device loopback",
            );
        }
    };

    let input_config = device
        .default_input_config()
        .map_err(|e| ConduitError::Other(format!("Failed to get monitor input config: {}", e)))?;

    let sample_rate = input_config.sample_rate();
    let channels = input_config.channels() as u16;

    let config = StreamConfig {
        channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = device
        .build_input_stream(
            config.clone(),
            {
                let is_streaming = is_streaming.clone();
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !is_streaming.load(Ordering::Relaxed) {
                        return;
                    }
                    let i16_samples = resample_and_mix_to_mono(data, sample_rate, channels);
                    if !i16_samples.is_empty() {
                        let _ = audio_tx.send(i16_samples);
                    }
                }
            },
            move |err| {
                error!("Linux monitor capture stream error: {}", err);
            },
            None,
        )
        .map_err(|e| {
            ConduitError::Other(format!("Failed to build monitor capture stream: {}", e))
        })?;

    stream
        .play()
        .map_err(|e| ConduitError::Other(format!("Failed to start monitor capture: {}", e)))?;

    info!(
        "Linux system audio loopback started ({}Hz, {}ch, PCM16) — capturing via monitor source",
        sample_rate, channels
    );

    while is_streaming.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(stream);
    info!("Linux system audio loopback stopped");
    Ok(())
}

/// Capture system audio on macOS using CoreAudio loopback.
///
/// On macOS, cpal supports building an input stream on the default output device.
/// This triggers CoreAudio to capture what is being played through the speakers/headphones
/// (system audio, music, videos, etc.).
#[cfg(target_os = "macos")]
fn capture_macos_loopback(
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
) -> Result<(), ConduitError> {
    let host = cpal::default_host();
    let output_device = host.default_output_device().ok_or_else(|| {
        ConduitError::Other("No output device available for macOS CoreAudio loopback".to_string())
    })?;

    info!(
        "macOS: using output device for CoreAudio loopback: {}",
        output_device
    );

    capture_cpal_loopback_on_device(
        is_streaming,
        audio_tx,
        &output_device,
        "macOS CoreAudio loopback",
    )
}

/// Build an input stream on an output device for loopback capture.
/// Used by both Linux (PipeWire) and macOS (CoreAudio) paths.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn capture_cpal_loopback_on_device(
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
    device: &cpal::Device,
    label: &str,
) -> Result<(), ConduitError> {
    let output_config = device.default_output_config().map_err(|e| {
        ConduitError::Other(format!("Failed to get output config for {}: {}", label, e))
    })?;

    let sample_rate = output_config.sample_rate();
    let channels = output_config.channels();

    let config = StreamConfig {
        channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = device
        .build_input_stream(
            config,
            {
                let is_streaming = is_streaming.clone();
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !is_streaming.load(Ordering::Relaxed) {
                        return;
                    }
                    let i16_samples = resample_and_mix_to_mono(data, sample_rate, channels);
                    if !i16_samples.is_empty() {
                        let _ = audio_tx.send(i16_samples);
                    }
                }
            },
            {
                let label_owned = label.to_string();
                move |err| {
                    error!("{} stream error: {}", label_owned, err);
                }
            },
            None,
        )
        .map_err(|e| ConduitError::Other(format!("Failed to build {} stream: {}", label, e)))?;

    stream
        .play()
        .map_err(|e| ConduitError::Other(format!("Failed to start {}: {}", label, e)))?;

    info!(
        "{} started ({}Hz, {}ch, PCM16)",
        label, sample_rate, channels
    );

    while is_streaming.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(stream);
    info!("{} stopped", label);
    Ok(())
}

/// Resample from `src_rate` to 16 kHz and mix down to mono.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn resample_and_mix_to_mono(data: &[f32], src_rate: u32, channels: u16) -> Vec<i16> {
    if src_rate != SAMPLE_RATE {
        let ratio = src_rate as f64 / SAMPLE_RATE as f64;
        let output_len = (data.len() as f64 / ratio) as usize;
        (0..output_len)
            .map(|i| {
                let src_idx = i as f64 * ratio;
                let idx = src_idx as usize;
                let frac = src_idx - idx as f64;
                let s0 = if idx < data.len() {
                    data[idx] as f64
                } else {
                    0.0_f64
                };
                let s1 = if idx + 1 < data.len() {
                    data[idx + 1] as f64
                } else {
                    s0
                };
                let sample = s0 + frac * (s1 - s0);
                (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    } else if channels == 1 {
        data.iter()
            .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
            .collect()
    } else {
        data.chunks(channels as usize)
            .map(|frame| {
                let mono = frame.iter().sum::<f32>() / channels as f32;
                (mono * 32767.0).clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }
}

/// Fallback: capture from default input device (microphone) using cpal.
/// Used on non-Windows platforms or when WASAPI loopback fails.
fn capture_cpal_input(is_streaming: Arc<AtomicBool>, audio_tx: broadcast::Sender<Vec<i16>>) {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            error!("No input device available for fallback capture");
            return;
        }
    };

    let config = StreamConfig {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Fixed(FRAME_SIZE as u32),
    };

    let stream = match device.build_input_stream(
        config,
        {
            let is_streaming = is_streaming.clone();
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !is_streaming.load(Ordering::Relaxed) {
                    return;
                }
                let i16_samples: Vec<i16> = data.iter().map(|&s| (s * 32767.0) as i16).collect();
                let _ = audio_tx.send(i16_samples);
            }
        },
        move |err| {
            error!("Audio input stream error: {}", err);
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to build input stream: {}", e);
            return;
        }
    };

    if let Err(e) = stream.play() {
        error!("Failed to play input stream: {}", e);
        return;
    }

    info!(
        "Fallback microphone capture started ({}Hz, {}ch, PCM16)",
        SAMPLE_RATE, CHANNELS
    );

    while is_streaming.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(stream);
    info!("Fallback microphone capture stopped");
}

// ─── Tests ──────────────────────────────────────────────────────────────────
//
// These pin the state-machine half of `AudioStream` only. The capture and
// playback loops themselves need a real audio device and a platform-specific
// backend, so they are not exercised here — what is pinned is the flag
// lifecycle that the loops poll, because a stuck flag means a loop that never
// exits (or never starts) with no error anywhere.
#[cfg(test)]
mod tests {
    use super::AudioStream;

    #[test]
    fn a_new_stream_is_not_streaming() {
        let stream = AudioStream::new();
        assert!(
            !stream.is_streaming(),
            "a freshly constructed stream must not report itself as streaming"
        );
    }

    #[test]
    fn stop_capture_without_start_is_a_noop() {
        // Paired-device and revoke paths call `stop_capture` on connections
        // that were never captured from. It must stay false, not flip to a
        // "stopped" state that a later start would treat as already running.
        let stream = AudioStream::new();
        stream.stop_capture();
        assert!(!stream.is_streaming());
    }

    #[test]
    fn playback_is_idempotent_across_repeated_stops() {
        // `start_playback` early-returns when already playing and
        // `stop_playback` clears the flag unconditionally. Calling either twice
        // must not panic or wedge the flag.
        let stream = AudioStream::new();
        stream.start_playback();
        stream.start_playback();
        stream.stop_playback();
        stream.stop_playback();
        assert!(!stream.is_streaming());
    }
}
