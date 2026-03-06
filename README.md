# Speech-to-Text Windows

A Windows-native speech-to-text application with real-time transcription, pluggable Whisper backends, and a system tray interface. Supports push-to-talk, continuous listening with voice activity detection, multilingual transcription, and audio file transcription to SRT subtitles.

![Setup Wizard](speechtotext_windows.png)

## Features

### Speech Recognition
- **Push-to-talk** - Hold a hotkey (default: `` ` ``) to record, release to transcribe
- **Always-listen mode** - Toggle continuous listening (default: `` Ctrl+` ``) with automatic voice activity detection
- **Multilingual support** - 99 Whisper-supported languages with auto-detection
- **Translation** - Translate any language to English using Whisper's built-in translation
- **Audio file transcription** - Transcribe audio files (MP3, WAV, FLAC, OGG, AAC) to SRT subtitles via `--transcribe`

### Output
- **Type to active window** - Transcribed text is automatically typed into the focused application
- **Clipboard paste mode** - Uses clipboard for reliable text insertion
- **Overlay** - Floating overlay shows recording/processing status with color-coded indicators
- **Subtitle bar** - Draggable, resizable subtitle display with auto font detection for CJK scripts
- **SRT subtitle export** - File transcription outputs standard SRT subtitle files with timestamps

### Backends
- **Faster Whisper (CTranslate2)** - High-performance inference with INT8/FP16 quantization
- **Whisper.cpp** - Whisper via whisper.cpp with GGML models
- **Pluggable backend system** - Backends are DLLs loaded at runtime, easy to add new ones
- **GPU acceleration** - CUDA support with automatic CUDA/cuDNN detection

### Models
- Multiple model sizes from Tiny (75 MB) to Large v3 (3000 MB)
- English-only and multilingual variants
- One-click download from Hugging Face in the setup wizard

### User Interface
- **Setup wizard** - Tabbed egui interface for model selection, audio settings, hotkeys, and GPU configuration
- **System tray** - Full-featured tray menu with language selection, audio source toggle, and settings access
- **Configurable hotkeys** - Customizable push-to-talk and toggle-listen key bindings
- **Tunable always-listen** - Adjustable silence timeout (0.1-5s) and streaming interval (0.2-3s)

### Audio
- **Microphone input** - Select from available input devices
- **System audio capture** - Loopback recording of desktop audio
- **16kHz mono resampling** - Automatic conversion from any sample rate

## Quick Start

```batch
:: Build
cargo build -p app --release

:: Run (opens setup wizard on first launch)
target\release\app.exe

:: Transcribe an audio file to SRT
target\release\app.exe --transcribe recording.mp3
target\release\app.exe --transcribe recording.wav --output custom.srt
```

## Usage

1. Run `app.exe` - the setup wizard opens on first launch
2. Select a model and click **Download**
3. Configure audio source, language, and hotkeys in the tabs
4. Click **Start** to launch the app
5. Use the system tray icon to access settings and toggle features

### Status Indicators
| Color | Meaning |
|-------|---------|
| Gray | Idle |
| Green | Always-listen mode active, waiting for speech |
| Red | Recording / speech detected |
| Yellow | Processing transcription |

## Config & Logs

- Config: `config-<exe>.json` next to the executable
- Logs: `app-<exe>.log` next to the executable
- Multiple instances supported by renaming the exe

## Project Structure

```
apps/app/              Main Windows GUI application
crates/app-core/       Shared FFI types for backend plugins
crates/backends/
  whisper-cpp/         Whisper.cpp backend (GGML)
  whisper-ct2/         Faster Whisper backend (CTranslate2)
tools/scripts/         Build and packaging scripts
docs/                  Project documentation
```

## Packaging

```batch
tools\scripts\package_release.bat [cuda|cpu]
```

## Changelog

See `CHANGELOG.md` for release notes.
