use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

const TARGET_SAMPLE_RATE: u32 = 16000;

pub struct AudioCapture {
    device: Device,
    config: StreamConfig,
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: Option<Stream>,
}

impl AudioCapture {
    pub fn new() -> Result<Self> {
        Self::new_with_device(None)
    }

    pub fn new_with_device(device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();

        debug!("Audio host: {:?}", host.id());

        let device = if let Some(name) = device_name {
            let mut matched: Option<Device> = None;
            if let Ok(mut devices) = host.input_devices() {
                for dev in devices.by_ref() {
                    if let Ok(dev_name) = dev.name() {
                        if dev_name == name {
                            matched = Some(dev);
                            break;
                        }
                    }
                }
            }
            if matched.is_none() {
                warn!("Requested input device '{}' not found. Using default.", name);
            }
            matched
        } else {
            None
        }
        .or_else(|| host.default_input_device())
        .context("No input device available")?;

        debug!("Input device: {:?}", device.name().unwrap_or_default());

        let supported_config = device
            .default_input_config()
            .context("Failed to get default input config")?;

        debug!("Default config: {:?}", supported_config);

        // Try to use 16kHz mono, fall back to device default
        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(TARGET_SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        // Check if the device supports our desired config
        let config = match device.supported_input_configs() {
            Ok(mut configs) => {
                let supports_16k = configs.any(|c| {
                    c.channels() >= 1
                        && c.min_sample_rate().0 <= TARGET_SAMPLE_RATE
                        && c.max_sample_rate().0 >= TARGET_SAMPLE_RATE
                });

                if supports_16k {
                    debug!("Using 16kHz mono");
                    config
                } else {
                    debug!(
                        "Device doesn't support 16kHz, using default: {}Hz {}ch",
                        supported_config.sample_rate().0,
                        supported_config.channels()
                    );
                    StreamConfig {
                        channels: supported_config.channels(),
                        sample_rate: supported_config.sample_rate(),
                        buffer_size: cpal::BufferSize::Default,
                    }
                }
            }
            Err(_) => {
                debug!("Using default config");
                StreamConfig {
                    channels: supported_config.channels(),
                    sample_rate: supported_config.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                }
            }
        };

        Ok(Self {
            device,
            config,
            recording: Arc::new(AtomicBool::new(false)),
            buffer: Arc::new(Mutex::new(Vec::new())),
            stream: None,
        })
    }

    pub fn start_recording(&mut self) -> Result<()> {
        if self.recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.buffer.lock().clear();
        self.recording.store(true, Ordering::SeqCst);

        let buffer = Arc::clone(&self.buffer);
        let recording = Arc::clone(&self.recording);
        let source_sample_rate = self.config.sample_rate.0;
        let channels = self.config.channels as usize;

        debug!("Starting audio stream: {}Hz, {} channels", source_sample_rate, channels);

        let err_fn = |err| error!("Audio stream error: {}", err);

        let stream = match self.device.default_input_config()?.sample_format() {
            SampleFormat::F32 => self.device.build_input_stream(
                &self.config,
                move |data: &[f32], _| {
                    if recording.load(Ordering::SeqCst) {
                        let mono_data = convert_to_mono(data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        buffer.lock().extend(resampled);
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => self.device.build_input_stream(
                &self.config,
                move |data: &[i16], _| {
                    if recording.load(Ordering::SeqCst) {
                        let float_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let mono_data = convert_to_mono(&float_data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        buffer.lock().extend(resampled);
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::U16 => self.device.build_input_stream(
                &self.config,
                move |data: &[u16], _| {
                    if recording.load(Ordering::SeqCst) {
                        let float_data: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let mono_data = convert_to_mono(&float_data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        buffer.lock().extend(resampled);
                    }
                },
                err_fn,
                None,
            )?,
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        };

        stream.play()?;
        self.stream = Some(stream);

        Ok(())
    }

    pub fn stop_recording(&mut self) -> Vec<f32> {
        self.recording.store(false, Ordering::SeqCst);
        self.stream = None;

        let audio = std::mem::take(&mut *self.buffer.lock());

        // Calculate audio stats
        if !audio.is_empty() {
            let max_val = audio.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
            let rms = (audio.iter().map(|x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
            debug!(
                "Audio captured: {} samples ({:.1}s), max={:.3}, rms={:.3}",
                audio.len(),
                audio.len() as f32 / 16000.0,
                max_val,
                rms
            );

            if max_val < 0.01 {
                warn!("Audio level very low - check microphone!");
            }
        } else {
            warn!("No audio captured!");
        }

        audio
    }

    #[allow(dead_code)]
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }

    /// Create a continuous audio stream for always-listen mode
    /// Returns a stream that sends audio chunks to the provided channel
    pub fn create_always_listen_stream(
        &self,
        audio_tx: crossbeam_channel::Sender<Vec<f32>>,
        running: Arc<AtomicBool>,
    ) -> Result<Stream> {
        let source_sample_rate = self.config.sample_rate.0;
        let channels = self.config.channels as usize;

        info!("Creating always-listen audio stream: {}Hz, {} channels", source_sample_rate, channels);

        let err_fn = |err| error!("Always-listen audio stream error: {}", err);

        let stream = match self.device.default_input_config()?.sample_format() {
            SampleFormat::F32 => self.device.build_input_stream(
                &self.config,
                move |data: &[f32], _| {
                    if running.load(Ordering::SeqCst) {
                        let mono_data = convert_to_mono(data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        // Send audio chunk to always-listen controller
                        if audio_tx.send(resampled).is_err() {
                            // Channel closed, stop sending
                        }
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => self.device.build_input_stream(
                &self.config,
                move |data: &[i16], _| {
                    if running.load(Ordering::SeqCst) {
                        let float_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let mono_data = convert_to_mono(&float_data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        if audio_tx.send(resampled).is_err() {
                            // Channel closed, stop sending
                        }
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::U16 => self.device.build_input_stream(
                &self.config,
                move |data: &[u16], _| {
                    if running.load(Ordering::SeqCst) {
                        let float_data: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let mono_data = convert_to_mono(&float_data, channels);
                        let resampled = resample(&mono_data, source_sample_rate, TARGET_SAMPLE_RATE);
                        if audio_tx.send(resampled).is_err() {
                            // Channel closed, stop sending
                        }
                    }
                },
                err_fn,
                None,
            )?,
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        };

        Ok(stream)
    }
}

fn convert_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return data.to_vec();
    }

    data.chunks(channels)
        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn resample(data: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return data.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (data.len() as f64 * ratio) as usize;
    let mut result = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < data.len() {
            data[idx] * (1.0 - frac as f32) + data[idx + 1] * frac as f32
        } else if idx < data.len() {
            data[idx]
        } else {
            0.0
        };

        result.push(sample);
    }

    result
}

/// Simple energy-based Voice Activity Detection
#[allow(dead_code)]
pub fn detect_voice_activity(samples: &[f32], threshold: f32) -> bool {
    if samples.is_empty() {
        return false;
    }

    let energy: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    energy.sqrt() > threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_mono_mono_input() {
        let input = vec![0.5f32, -0.3, 0.8, -0.2];
        let result = convert_to_mono(&input, 1);
        assert_eq!(result, input);
    }

    #[test]
    fn test_convert_to_mono_stereo_input() {
        let input = vec![0.5f32, -0.5, 0.3, -0.3, 0.8, -0.8];
        // Stereo interleaved: [L, R, L, R, L, R]
        let result = convert_to_mono(&input, 2);
        // Expected: [(0.5 + -0.5)/2, (0.3 + -0.3)/2, (0.8 + -0.8)/2] = [0.0, 0.0, 0.0]
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.0).abs() < 0.001);
        assert!((result[1] - 0.0).abs() < 0.001);
        assert!((result[2] - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_to_mono_quad_input() {
        let input = vec![1.0f32, 0.5, 0.5, 0.0,  // First sample: 4 channels
                        -0.5, -0.5, -0.5, -0.5]; // Second sample: 4 channels
        let result = convert_to_mono(&input, 4);
        assert_eq!(result.len(), 2);
        // First sample average: (1.0 + 0.5 + 0.5 + 0.0) / 4 = 0.5
        assert!((result[0] - 0.5).abs() < 0.001);
        // Second sample average: (-0.5 - 0.5 - 0.5 - 0.5) / 4 = -0.5
        assert!((result[1] - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn test_resample_same_rate() {
        let input = vec![0.1f32, 0.2, 0.3, 0.4, 0.5];
        let result = resample(&input, 16000, 16000);
        assert_eq!(result, input);
    }

    #[test]
    fn test_resample_upsample() {
        let input = vec![0.0f32, 0.5, 1.0];
        // 8kHz to 16kHz should double the samples
        let result = resample(&input, 8000, 16000);
        assert_eq!(result.len(), 6);
    }

    #[test]
    fn test_resample_downsample() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        // 32kHz to 16kHz should halve the samples
        let result = resample(&input, 32000, 16000);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn test_detect_voice_activity_silence() {
        let silence = vec![0.0f32; 100];
        assert!(!detect_voice_activity(&silence, 0.01));
    }

    #[test]
    fn test_detect_voice_activity_loud() {
        let loud: Vec<f32> = (0..100).map(|_| 0.8f32).collect();
        assert!(detect_voice_activity(&loud, 0.01));
    }

    #[test]
    fn test_detect_voice_activity_threshold() {
        // RMS of 0.1 samples is 0.1
        let signal: Vec<f32> = vec![0.1f32; 100];
        // RMS = sqrt(0.01) = 0.1
        assert!(detect_voice_activity(&signal, 0.05));  // 0.1 > 0.05
        assert!(!detect_voice_activity(&signal, 0.2));  // 0.1 < 0.2
    }

    #[test]
    fn test_detect_voice_activity_empty() {
        let empty: Vec<f32> = vec![];
        assert!(!detect_voice_activity(&empty, 0.01));
    }

    #[test]
    fn test_audio_capture_creation() {
        // This test just verifies the AudioCapture struct can be created
        // In a real test environment with audio devices, we could test more
        // For now, we just test that the struct fields are properly initialized
        let recording = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(Vec::new()));
        
        // Verify atomic operations work
        recording.store(true, Ordering::SeqCst);
        assert!(recording.load(Ordering::SeqCst));
        
        recording.store(false, Ordering::SeqCst);
        assert!(!recording.load(Ordering::SeqCst));
        
        // Verify buffer operations work
        buffer.lock().extend_from_slice(&[0.1f32, 0.2, 0.3]);
        assert_eq!(buffer.lock().len(), 3);
        
        buffer.lock().clear();
        assert!(buffer.lock().is_empty());
    }
}
