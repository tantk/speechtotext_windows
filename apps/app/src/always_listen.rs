//! Always-listen mode for hands-free speech recognition
//!
//! Uses Voice Activity Detection (VAD) to automatically detect speech,
//! record utterances, and trigger transcription without hotkey presses.

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, trace};

/// State machine for always-listen mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysListenState {
    /// Listening for speech (VAD active, audio mostly discarded)
    Listening,
    /// Speech detected, accumulating pre-roll buffer (reserved for future use)
    #[allow(dead_code)]
    Detecting { since: Instant },
    /// Actively recording speech
    Recording { since: Instant },
    /// Speech ended, transcribing
    Processing,
    /// Temporarily paused by user
    Paused,
}

impl AlwaysListenState {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            AlwaysListenState::Listening => "Listening",
            AlwaysListenState::Detecting { .. } => "Detecting",
            AlwaysListenState::Recording { .. } => "Recording",
            AlwaysListenState::Processing => "Processing",
            AlwaysListenState::Paused => "Paused",
        }
    }
}

/// Configuration for always-listen mode
#[derive(Clone, Debug)]
pub struct AlwaysListenConfig {
    /// How long to buffer before speech is confirmed (ms)
    pub pre_roll_duration_ms: u64,
    /// Minimum speech duration to trigger recording (ms)
    pub min_speech_duration_ms: u64,
    /// Silence duration to end recording (ms)
    pub post_silence_duration_ms: u64,
    /// VAD energy threshold (0.0 - 1.0)
    pub vad_threshold: f32,
    /// Maximum utterance length (seconds)
    pub max_utterance_seconds: f64,
    /// Cooldown between transcriptions (ms) - reserved for future use
    #[allow(dead_code)]
    pub cooldown_ms: u64,
    /// Frames to analyze per VAD check (must be power of 2, 10-30ms worth)
    pub frame_samples: usize,
}

impl Default for AlwaysListenConfig {
    fn default() -> Self {
        Self {
            pre_roll_duration_ms: 500,     // 500ms pre-roll
            min_speech_duration_ms: 300,   // 300ms min speech
            post_silence_duration_ms: 2000, // 2s silence = end
            vad_threshold: 0.015,          // Energy threshold (tuned for typical mics)
            max_utterance_seconds: 30.0,   // Max 30s utterance
            cooldown_ms: 200,              // 200ms between utterances
            frame_samples: 480,            // 30ms at 16kHz
        }
    }
}

/// Commands to control the always-listen controller
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysListenCommand {
    Start,
    Stop,
    #[allow(dead_code)]
    Pause,
    #[allow(dead_code)]
    Resume,
}

/// Audio buffer manager with circular pre-roll buffer
pub struct AudioBufferManager {
    /// Fixed-size circular buffer for pre-roll
    pre_roll: Vec<f32>,
    pre_roll_pos: usize,
    pre_roll_full: bool,
    /// Growing buffer for active recording
    recording: Vec<f32>,
    /// Sample rate
    sample_rate: u32,
    /// Pre-roll capacity in samples
    pre_roll_capacity: usize,
}

impl AudioBufferManager {
    pub fn new(sample_rate: u32, pre_roll_duration_ms: u64) -> Self {
        let pre_roll_capacity = (sample_rate as usize * pre_roll_duration_ms as usize) / 1000;
        Self {
            pre_roll: vec![0.0; pre_roll_capacity],
            pre_roll_pos: 0,
            pre_roll_full: false,
            recording: Vec::new(),
            sample_rate,
            pre_roll_capacity,
        }
    }

    /// Push samples to pre-roll buffer
    pub fn push_to_pre_roll(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.pre_roll[self.pre_roll_pos] = sample;
            self.pre_roll_pos += 1;
            if self.pre_roll_pos >= self.pre_roll_capacity {
                self.pre_roll_pos = 0;
                self.pre_roll_full = true;
            }
        }
    }

    /// Start recording - returns pre-roll buffer content
    pub fn start_recording(&mut self) -> Vec<f32> {
        let mut result = if self.pre_roll_full {
            // Return buffer from current position (oldest to newest)
            let mut ordered = Vec::with_capacity(self.pre_roll_capacity);
            ordered.extend_from_slice(&self.pre_roll[self.pre_roll_pos..]);
            ordered.extend_from_slice(&self.pre_roll[..self.pre_roll_pos]);
            ordered
        } else {
            // Buffer not full yet, return what we have
            self.pre_roll[..self.pre_roll_pos].to_vec()
        };

        // Move to recording buffer
        self.recording.clear();
        self.recording.append(&mut result);
        self.recording.clone()
    }

    /// Push samples to active recording
    pub fn push_to_recording(&mut self, samples: &[f32]) {
        self.recording.extend_from_slice(samples);
    }

    /// Finalize recording and return complete audio
    pub fn finalize(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.recording)
    }

    /// Reset for next utterance
    pub fn reset(&mut self) {
        self.pre_roll_pos = 0;
        self.pre_roll_full = false;
        self.recording.clear();
        // Clear pre-roll buffer
        for sample in &mut self.pre_roll {
            *sample = 0.0;
        }
    }

    /// Get current recording duration in seconds
    pub fn recording_duration(&self) -> f64 {
        self.recording.len() as f64 / self.sample_rate as f64
    }
}

/// Energy-based Voice Activity Detection
pub struct VadEngine {
    threshold: f32,
    frame_size: usize,
    /// Consecutive voice frames counter
    voice_frames: usize,
    /// Consecutive silence frames counter
    silence_frames: usize,
    /// Smoothing factor for energy (exponential moving average)
    smoothed_energy: f32,
    /// Alpha for EMA (0.0 = no smoothing, 1.0 = max smoothing)
    smoothing_alpha: f32,
}

impl VadEngine {
    pub fn new(threshold: f32, frame_size: usize) -> Self {
        Self {
            threshold,
            frame_size,
            voice_frames: 0,
            silence_frames: 0,
            smoothed_energy: 0.0,
            smoothing_alpha: 0.3, // Moderate smoothing
        }
    }

    /// Process a frame and return voice activity
    /// Returns: (is_voice, voice_probability)
    pub fn process(&mut self, frame: &[f32]) -> (bool, f32) {
        if frame.len() < self.frame_size {
            return (false, 0.0);
        }

        // Calculate RMS energy
        let energy: f32 = frame[..self.frame_size].iter().map(|s| s * s).sum::<f32>()
            / self.frame_size as f32;
        let rms = energy.sqrt();

        // Update smoothed energy with EMA
        self.smoothed_energy = self.smoothing_alpha * rms
            + (1.0 - self.smoothing_alpha) * self.smoothed_energy;

        // Normalize probability (0.0 to 1.0)
        let probability = (self.smoothed_energy / self.threshold).min(1.0);
        let is_voice = self.smoothed_energy > self.threshold;

        if is_voice {
            self.voice_frames += 1;
            self.silence_frames = 0;
        } else {
            self.silence_frames += 1;
            if self.silence_frames > 10 {
                // Reset voice counter after sustained silence
                self.voice_frames = 0;
            }
        }

        (is_voice, probability)
    }

    /// Check if we have sustained voice activity
    pub fn has_sustained_voice(&self, min_frames: usize) -> bool {
        self.voice_frames >= min_frames
    }

    /// Check if we have sustained silence
    pub fn has_sustained_silence(&self, min_frames: usize) -> bool {
        self.silence_frames >= min_frames
    }

    /// Get current voice frame count
    #[allow(dead_code)]
    pub fn voice_frames(&self) -> usize {
        self.voice_frames
    }

    /// Get current silence frame count
    #[allow(dead_code)]
    pub fn silence_frames(&self) -> usize {
        self.silence_frames
    }

    /// Reset state
    pub fn reset(&mut self) {
        self.voice_frames = 0;
        self.silence_frames = 0;
        self.smoothed_energy = 0.0;
    }
}

/// Controller for always-listen mode
pub struct AlwaysListenController {
    state: Arc<Mutex<AlwaysListenState>>,
    #[allow(dead_code)]
    config: AlwaysListenConfig,
    /// Control command sender
    command_tx: Sender<AlwaysListenCommand>,
    /// Audio result receiver (raw audio data for transcription)
    result_rx: Receiver<Vec<f32>>,
    /// Running flag
    running: Arc<AtomicBool>,
    /// Handle to processing thread
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl AlwaysListenController {
    /// Create and start the always-listen controller
    pub fn new(
        config: AlwaysListenConfig,
        audio_rx: Receiver<Vec<f32>>,
        _result_tx: Sender<Vec<f32>>, // Reserved for future use
    ) -> Self {
        let (command_tx, command_rx) = crossbeam_channel::bounded(10);
        let (internal_result_tx, result_rx) = crossbeam_channel::bounded::<Vec<f32>>(10);

        let state = Arc::new(Mutex::new(AlwaysListenState::Listening));
        let running = Arc::new(AtomicBool::new(true));

        // Clone values for the controller struct
        let state_for_controller = Arc::clone(&state);
        let running_for_controller = Arc::clone(&running);
        let config_for_controller = config.clone();
        let command_tx_for_controller = command_tx;

        // Spawn processing thread
        let thread_handle = std::thread::spawn(move || {
            processing_loop(
                state,
                running,
                config,
                audio_rx,
                command_rx,
                internal_result_tx,
            );
        });

        Self {
            state: state_for_controller,
            config: config_for_controller,
            command_tx: command_tx_for_controller,
            result_rx,
            running: running_for_controller,
            thread_handle: Some(thread_handle),
        }
    }

    /// Start listening
    pub fn start(&self) -> Result<()> {
        self.command_tx
            .send(AlwaysListenCommand::Start)
            .context("Failed to send start command")?;
        info!("Always-listen mode started");
        Ok(())
    }

    /// Stop listening
    pub fn stop(&self) -> Result<()> {
        self.command_tx
            .send(AlwaysListenCommand::Stop)
            .context("Failed to send stop command")?;
        info!("Always-listen mode stopped");
        Ok(())
    }

    /// Pause listening temporarily
    #[allow(dead_code)]
    pub fn pause(&self) -> Result<()> {
        self.command_tx
            .send(AlwaysListenCommand::Pause)
            .context("Failed to send pause command")?;
        info!("Always-listen mode paused");
        Ok(())
    }

    /// Resume listening
    #[allow(dead_code)]
    pub fn resume(&self) -> Result<()> {
        self.command_tx
            .send(AlwaysListenCommand::Resume)
            .context("Failed to send resume command")?;
        info!("Always-listen mode resumed");
        Ok(())
    }

    /// Get current state
    pub fn state(&self) -> AlwaysListenState {
        *self.state.lock()
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Try to receive an audio result (non-blocking)
    pub fn try_recv_result(&self) -> Option<Vec<f32>> {
        self.result_rx.try_recv().ok()
    }

    /// Receive audio result (blocking with timeout)
    #[allow(dead_code)]
    pub fn recv_result_timeout(&self, timeout: Duration) -> Option<Vec<f32>> {
        self.result_rx.recv_timeout(timeout).ok()
    }
}

impl Drop for AlwaysListenController {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.command_tx.send(AlwaysListenCommand::Stop);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main processing loop running in dedicated thread
fn processing_loop(
    state: Arc<Mutex<AlwaysListenState>>,
    running: Arc<AtomicBool>,
    config: AlwaysListenConfig,
    audio_rx: Receiver<Vec<f32>>,
    command_rx: Receiver<AlwaysListenCommand>,
    result_tx: Sender<Vec<f32>>,
) {
    let sample_rate = 16000u32;
    let frame_samples = config.frame_samples;
    let min_voice_frames =
        ((config.min_speech_duration_ms as f32 / 1000.0) * sample_rate as f32) as usize
            / frame_samples;
    let silence_frames_threshold =
        ((config.post_silence_duration_ms as f32 / 1000.0) * sample_rate as f32) as usize
            / frame_samples;

    let mut buffer_manager = AudioBufferManager::new(sample_rate, config.pre_roll_duration_ms);
    let mut vad = VadEngine::new(config.vad_threshold, frame_samples);

    // Accumulate samples for frame processing
    let mut sample_buffer: Vec<f32> = Vec::with_capacity(frame_samples * 2);

    info!(
        "VAD initialized: threshold={}, frame_samples={}, min_voice_frames={}",
        config.vad_threshold, frame_samples, min_voice_frames
    );

    while running.load(Ordering::SeqCst) {
        // Process commands
        if let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                AlwaysListenCommand::Stop => {
                    *state.lock() = AlwaysListenState::Paused;
                }
                AlwaysListenCommand::Pause => {
                    *state.lock() = AlwaysListenState::Paused;
                }
                AlwaysListenCommand::Resume | AlwaysListenCommand::Start => {
                    let mut s = state.lock();
                    if *s == AlwaysListenState::Paused {
                        *s = AlwaysListenState::Listening;
                        buffer_manager.reset();
                        vad.reset();
                    }
                }
            }
        }

        let current_state = *state.lock();

        // Skip processing if paused
        if current_state == AlwaysListenState::Paused {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Process audio
        match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => {
                sample_buffer.extend_from_slice(&samples);

                // Process complete frames
                while sample_buffer.len() >= frame_samples {
                    let frame: Vec<f32> = sample_buffer.drain(..frame_samples).collect();

                    let (is_voice, prob) = vad.process(&frame);
                    trace!("VAD: voice={}, prob={:.3}", is_voice, prob);

                    match current_state {
                        AlwaysListenState::Listening => {
                            buffer_manager.push_to_pre_roll(&frame);

                            if vad.has_sustained_voice(min_voice_frames) {
                                info!("Speech detected, starting recording");
                                *state.lock() = AlwaysListenState::Recording {
                                    since: Instant::now(),
                                };
                                buffer_manager.start_recording();
                                // Add current frame
                                buffer_manager.push_to_recording(&frame);
                            }
                        }
                        AlwaysListenState::Recording { since } => {
                            buffer_manager.push_to_recording(&frame);

                            // Check for max duration
                            if buffer_manager.recording_duration() > config.max_utterance_seconds {
                                info!("Max utterance duration reached, finalizing");
                                finalize_recording(
                                    &mut buffer_manager,
                                    &mut vad,
                                    &state,
                                    &result_tx,
                                );
                                continue;
                            }

                            // Check for sustained silence
                            if vad.has_sustained_silence(silence_frames_threshold) {
                                info!(
                                    "Silence detected after {:.2}s, finalizing",
                                    since.elapsed().as_secs_f64()
                                );
                                finalize_recording(
                                    &mut buffer_manager,
                                    &mut vad,
                                    &state,
                                    &result_tx,
                                );
                            }
                        }
                        AlwaysListenState::Processing => {
                            // Drop audio while processing
                        }
                        AlwaysListenState::Paused => {
                            // Should not reach here due to earlier check
                        }
                        AlwaysListenState::Detecting { .. } => {
                            // Not used in this implementation
                        }
                    }
                }
            }
            Err(_) => {
                // Timeout, continue to check commands
            }
        }
    }

    info!("Always-listen processing loop ended");
}

/// Finalize recording and send audio data for transcription
fn finalize_recording(
    buffer_manager: &mut AudioBufferManager,
    vad: &mut VadEngine,
    state: &Arc<Mutex<AlwaysListenState>>,
    result_tx: &Sender<Vec<f32>>,
) {
    let audio = buffer_manager.finalize();

    if audio.len() < 1600 {
        // Less than 100ms, probably noise
        debug!("Recording too short ({} samples), discarding", audio.len());
        *state.lock() = AlwaysListenState::Listening;
        buffer_manager.reset();
        vad.reset();
        return;
    }

    info!("Finalized recording: {} samples ({:.2}s)", audio.len(), audio.len() as f32 / 16000.0);

    // Send the actual audio data for transcription
    if result_tx.send(audio).is_err() {
        error!("Failed to send audio data for transcription");
    }

    // Return to listening state immediately - transcription happens async
    // This allows detecting the next utterance while previous one is being transcribed
    *state.lock() = AlwaysListenState::Listening;

    // Reset for next utterance
    buffer_manager.reset();
    vad.reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_buffer_manager() {
        let mut manager = AudioBufferManager::new(16000, 500); // 500ms pre-roll

        // Add some samples
        let samples: Vec<f32> = (0..8000).map(|i| i as f32 / 8000.0).collect();
        manager.push_to_pre_roll(&samples);

        // Start recording
        let pre_roll = manager.start_recording();
        assert!(!pre_roll.is_empty());

        // Add more samples
        manager.push_to_recording(&samples);

        // Finalize
        let final_audio = manager.finalize();
        assert!(final_audio.len() > samples.len());
    }

    #[test]
    fn test_vad_engine() {
        let mut vad = VadEngine::new(0.1, 160); // 10ms frames at 16kHz

        // Silence
        let silence = vec![0.0f32; 160];
        let (is_voice, _) = vad.process(&silence);
        assert!(!is_voice);

        // Loud signal
        let loud: Vec<f32> = (0..160).map(|_| 0.5f32).collect();
        let (_is_voice, _prob) = vad.process(&loud);
        // May need a few frames to detect
        for _ in 0..5 {
            let (v, p) = vad.process(&loud);
            if v {
                assert!(p > 0.0);
                break;
            }
        }
    }

    #[test]
    fn test_state_transitions() {
        let state = Arc::new(Mutex::new(AlwaysListenState::Listening));
        
        *state.lock() = AlwaysListenState::Recording { since: Instant::now() };
        assert_eq!(state.lock().name(), "Recording");

        *state.lock() = AlwaysListenState::Processing;
        assert_eq!(state.lock().name(), "Processing");
    }
}
