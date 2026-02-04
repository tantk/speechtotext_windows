# Always-Listen Mode Architecture Design

## Overview

Always-listen mode enables hands-free continuous speech recognition without holding a hotkey. The system listens continuously, detects speech using Voice Activity Detection (VAD), transcribes utterances automatically, and returns to listening state.

## Architecture Components

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Always-Listen Architecture                         │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐  │
│  │   Audio     │───▶│    VAD      │───▶│   Buffer    │───▶│ Transcribe  │  │
│  │   Capture   │    │   Engine    │    │   Manager   │    │   Engine    │  │
│  └─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘  │
│         │                  │                  │                  │          │
│         ▼                  ▼                  ▼                  ▼          │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                      State Machine Controller                        │   │
│  │  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐      │   │
│  │  │ LISTENING│───▶│ DETECTING│───▶│ RECORDING│───▶│PROCESSING│      │   │
│  │  └──────────┘    └──────────┘    └──────────┘    └──────────┘      │   │
│  │       ▲                                              │              │   │
│  │       └──────────────────────────────────────────────┘              │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## State Machine

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysListenState {
    /// Listening for speech (VAD active, audio discarded)
    Listening,
    /// Speech detected, accumulating pre-roll buffer
    Detecting { since: Instant },
    /// Actively recording speech
    Recording { since: Instant },
    /// Speech ended, transcribing
    Processing,
    /// Temporary pause (user triggered)
    Paused,
}
```

### State Transitions

| From | To | Trigger | Action |
|------|-----|---------|--------|
| `Listening` | `Detecting` | VAD: voice probability > threshold | Start pre-roll buffer |
| `Detecting` | `Recording` | VAD: sustained voice > min_duration | Start actual recording, keep pre-roll |
| `Detecting` | `Listening` | VAD: voice dropped < timeout | Discard pre-roll, return to listening |
| `Recording` | `Processing` | VAD: silence > post_silence_duration | Stop recording, submit for transcription |
| `Processing` | `Listening` | Transcription complete | Output text, return to listening |
| *Any* | `Paused` | User hotkey | Stop processing, mute |
| `Paused` | `Listening` | User hotkey | Resume listening |

## Component Details

### 1. VAD Engine

Two implementation options:

#### Option A: WebRTC VAD (Recommended)
- **Pros**: Battle-tested, lightweight, low latency
- **Cons**: Requires additional dependency (webrtc-vad or similar)
- **Mode**: Frame-based (10ms, 20ms, or 30ms frames)
- **Aggressiveness**: 0-3 (3 = most aggressive filtering)

```rust
pub struct VadEngine {
    inner: webrtc_vad::Vad,
    sample_rate: u32,
    frame_size: usize,
    /// Probability threshold (0.0 - 1.0)
    threshold: f32,
}

impl VadEngine {
    /// Returns voice probability for the frame
    pub fn process(&mut self, frame: &[i16]) -> f32;
    
    /// Quick check if frame contains voice
    pub fn is_voice(&mut self, frame: &[i16]) -> bool {
        self.process(frame) > self.threshold
    }
}
```

#### Option B: Energy-Based VAD (Simple fallback)
```rust
pub struct EnergyVad {
    noise_floor: f32,
    threshold_db: f32,
}
```

### 2. Circular Buffer Manager

Maintains audio buffers for pre-roll and recording:

```rust
pub struct AudioBufferManager {
    /// Fixed-size circular buffer for pre-roll (1-2 seconds)
    pre_roll: CircularBuffer<f32>,
    /// Growing buffer for active recording
    recording: Vec<f32>,
    /// Sample rate
    sample_rate: u32,
}

impl AudioBufferManager {
    /// Add samples to current state (pre-roll or recording)
    pub fn push(&mut self, samples: &[f32]);
    
    /// Transition from pre-roll to recording
    pub fn start_recording(&mut self) -> Vec<f32> {
        // Return pre-roll buffer + start accumulating recording
    }
    
    /// Finalize recording
    pub fn finalize(&mut self) -> Vec<f32> {
        // Return complete audio (pre-roll + recording)
    }
    
    /// Reset for next utterance
    pub fn reset(&mut self);
}
```

### 3. Always-Listen Controller

Main orchestrator running in dedicated thread:

```rust
pub struct AlwaysListenController {
    state: Arc<Mutex<AlwaysListenState>>,
    vad: VadEngine,
    buffer: AudioBufferManager,
    config: AlwaysListenConfig,
    /// Audio input source
    audio_rx: Receiver<Vec<f32>>,
    /// Transcription output
    result_tx: Sender<String>,
    /// Control commands (pause/resume/stop)
    command_rx: Receiver<AlwaysListenCommand>,
}

#[derive(Clone, Debug)]
pub struct AlwaysListenConfig {
    /// How long to buffer before speech is confirmed (ms)
    pub pre_roll_duration_ms: u64,
    /// Minimum speech duration to trigger recording (ms)
    pub min_speech_duration_ms: u64,
    /// Silence duration to end recording (ms)
    pub post_silence_duration_ms: u64,
    /// VAD sensitivity (0.0 - 1.0)
    pub vad_threshold: f32,
    /// Maximum utterance length (seconds)
    pub max_utterance_seconds: f64,
    /// Cooldown between transcriptions (ms)
    pub cooldown_ms: u64,
}

impl Default for AlwaysListenConfig {
    fn default() -> Self {
        Self {
            pre_roll_duration_ms: 500,      // 500ms pre-roll
            min_speech_duration_ms: 300,    // 300ms min speech
            post_silence_duration_ms: 800,  // 800ms silence = end
            vad_threshold: 0.7,             // 70% confidence
            max_utterance_seconds: 30.0,    // Max 30s utterance
            cooldown_ms: 200,               // 200ms between utterances
        }
    }
}
```

### 4. Integration with Existing Architecture

```rust
// In main.rs - extend AppMode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Idle,
    Recording,          // Push-to-talk recording
    Processing,         // Transcribing
    AlwaysListening,    // NEW: Always-listen mode active
}

// Hotkey handling
impl AppMode {
    fn handle_always_listen_toggle(&mut self, controller: &AlwaysListenController) {
        match self {
            AppMode::Idle => {
                controller.start();
                *self = AppMode::AlwaysListening;
            }
            AppMode::AlwaysListening => {
                controller.stop();
                *self = AppMode::Idle;
            }
            _ => {} // Ignore if recording/processing
        }
    }
}
```

## Implementation Plan

### Phase 1: Foundation (2-3 hours)
1. Add `webrtc-vad` crate dependency
2. Create `always_listen.rs` module with state machine
3. Implement `AudioBufferManager`
4. Basic unit tests

### Phase 2: VAD Integration (2-3 hours)
1. Implement `VadEngine` wrapper
2. Create `AlwaysListenController` thread
3. Audio capture integration (reuse existing `audio::AudioCapture`)
4. State machine transitions

### Phase 3: Transcription Integration (1-2 hours)
1. Connect to existing transcription pipeline
2. Pre-roll buffer concatenation
3. Result output via typer

### Phase 4: UI/UX (2 hours)
1. Visual indicator for always-listen state (green icon)
2. Settings UI for VAD parameters
3. Pause/resume functionality

## Configuration Schema

```json
{
  "always_listen": {
    "enabled": false,
    "vad_threshold": 0.7,
    "pre_roll_ms": 500,
    "min_speech_ms": 300,
    "post_silence_ms": 800,
    "max_utterance_sec": 30.0,
    "cooldown_ms": 200
  }
}
```

## Performance Considerations

| Metric | Target | Notes |
|--------|--------|-------|
| Latency (voice→text) | < 2s | Includes VAD + transcription |
| CPU usage (idle) | < 5% | VAD processing only |
| CPU usage (active) | < 30% | VAD + buffering + transcription |
| Memory | < 100MB | Circular buffer + model |
| Wake word latency | < 200ms | From speech start to recording |

## Alternative: Wake Word Detection

For even more control, consider adding optional wake word detection:

```rust
pub enum TriggerMode {
    /// Voice activity detection only
    VadOnly,
    /// Wake word + VAD (e.g., "Hey Computer")
    WakeWord { model: PathBuf },
    /// Hybrid: Wake word OR push-to-talk
    Hybrid { wake_word: PathBuf },
}
```

## Open Questions

1. **Should we support multiple utterance modes?**
   - Single utterance (current design)
   - Continuous dictation (no silence timeout)
   - Command mode (wake word + single command)

2. **Noise profile handling?**
   - Static noise floor
   - Adaptive noise learning
   - Profile per audio device

3. **Interruption handling?**
   - Can user interrupt long transcription?
   - How to handle barge-in during TTS output?
