use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, StreamConfig};
use log::{error, info};
use tokio::sync::broadcast;

const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;
const FRAME_SIZE: usize = 1600;

pub struct AudioStream {
    is_streaming: Arc<AtomicBool>,
    audio_tx: broadcast::Sender<Vec<i16>>,
}

impl Clone for AudioStream {
    fn clone(&self) -> Self {
        Self {
            is_streaming: self.is_streaming.clone(),
            audio_tx: self.audio_tx.clone(),
        }
    }
}

impl AudioStream {
    pub fn new() -> Self {
        let (audio_tx, _) = broadcast::channel(256);
        Self {
            is_streaming: Arc::new(AtomicBool::new(false)),
            audio_tx,
        }
    }

    pub async fn start_capture(&self) -> Result<(), String> {
        if self.is_streaming.load(Ordering::Relaxed) {
            return Err("Already streaming".to_string());
        }

        // Set flag BEFORE spawning so the callback never sees a stale false.
        self.is_streaming.store(true, Ordering::Relaxed);

        let is_streaming = self.is_streaming.clone();
        let audio_tx = self.audio_tx.clone();

        // cpal::Stream is !Send, so build and run it on a dedicated OS thread
        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    error!("No input device available");
                    return;
                }
            };

            let config = StreamConfig {
                channels: CHANNELS,
                sample_rate: SampleRate(SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Fixed(FRAME_SIZE as u32),
            };

            let stream = match device.build_input_stream(
                &config,
                {
                    let is_streaming = is_streaming.clone();
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if !is_streaming.load(Ordering::Relaxed) {
                            return;
                        }
                        let i16_samples: Vec<i16> = data.iter()
                            .map(|&s| (s * 32767.0) as i16)
                            .collect();
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

            info!("Audio capture started ({}Hz, {}ch, PCM16)", SAMPLE_RATE, CHANNELS);

            // Keep stream alive until stopped
            while is_streaming.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }

            drop(stream);
            info!("Audio capture stopped");
        });

        Ok(())
    }

    pub fn stop_capture(&self) {
        self.is_streaming.store(false, Ordering::Relaxed);
    }

    pub async fn play_audio(&self, pcm_data: &[i16]) -> Result<(), String> {
        let float_data: Vec<f32> = pcm_data.iter()
            .map(|&s| s as f32 / 32767.0)
            .collect();
        let data_len = float_data.len();
        let duration = Duration::from_millis((data_len as u64 * 1000) / SAMPLE_RATE as u64);

        // cpal::Stream is !Send, so build and run it on a dedicated OS thread
        // We use a oneshot channel to signal completion
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
                sample_rate: SampleRate(SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Fixed(FRAME_SIZE as u32),
            };

            let stream = match device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for (i, sample) in data.iter_mut().enumerate() {
                        *sample = if i < float_data.len() { float_data[i] } else { 0.0 };
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

        // Wait for playback to finish (with timeout)
        let _ = tokio::time::timeout(duration + Duration::from_secs(2), done_rx).await;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<i16>> {
        self.audio_tx.subscribe()
    }
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
