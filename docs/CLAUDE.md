# App - AI Agent Guide

## Project Overview

App is a Windows-native speech-to-text application with system tray integration. It allows users to perform voice dictation using global hotkeys (push-to-talk and toggle listening) with the transcription automatically typed into the active window.

### Key Features
- **System tray integration** with status overlay
- **Multiple backend support**: whisper.cpp and CTranslate2 (Faster Whisper)
- **CUDA GPU acceleration** support (optional)
- **Global hotkeys** for push-to-talk and toggle listening
- **Setup wizard** for initial configuration and model download
- **Custom GUI** using softbuffer (no external GUI framework dependencies)

## Technology Stack

- **Language**: Rust (Edition 2021)
- **Build System**: Cargo with custom batch scripts
- **Platform**: Windows 10/11 (MSVC toolchain)
- **Audio**: cpal (Cross-platform Audio Library)
- **GUI**: tao + softbuffer (pixel buffer rendering)
- **System Integration**: tray-icon, global-hotkey, enigo (keyboard simulation)
- **ML Inference**: whisper-rs (whisper.cpp bindings), ct2rs (CTranslate2 bindings)

## Project Structure

```
.
├── Cargo.toml                   # Workspace root
├── .cargo/
│   └── config.toml             # Critical build configuration
├── apps/
│   └── app/                    # Main application
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs         # Entry point, event loop
│           ├── setup.rs        # Setup wizard UI (softbuffer-based)
│           ├── config.rs       # Configuration and CUDA detection
│           ├── backend_loader.rs # Dynamic DLL loading
│           ├── tray.rs         # System tray icon and menu
│           ├── overlay.rs      # Status overlay window
│           ├── audio.rs        # Audio capture (cpal)
│           ├── hotkeys.rs      # Global hotkey handling
│           ├── typer.rs        # Keyboard text injection
│           ├── downloader.rs   # Model download handling
│           └── always_listen.rs # VAD-based continuous listening mode
├── crates/
│   ├── app-core/               # Shared FFI types
│   │   └── src/lib.rs          # C-compatible interface definitions
│   └── backends/
│       ├── whisper-cpp/        # whisper.cpp backend
│       │   ├── src/lib.rs      # FFI exports
│       │   ├── manifest.json   # Model definitions
│       │   └── Cargo.toml
│       └── whisper-ct2/        # CTranslate2 backend
│           ├── src/lib.rs      # FFI exports
│           ├── manifest.json   # Model definitions
│           └── Cargo.toml
├── tools/
│   ├── cmake/                  # Pinned CMake distribution
│   └── scripts/                # Build + packaging scripts
└── dist/                        # Packaged outputs
```

## Architecture

### Backend Plugin System

The application uses a dynamic plugin architecture for speech recognition backends:

1. **FFI Interface** (`app-core`): Defines C-compatible structs and function pointers
2. **Backend DLLs**: Compiled as `cdylib`, expose standardized exports
3. **Manifest System**: Each backend includes a `manifest.json` defining models and capabilities
4. **Runtime Loading**: Main app uses `libloading` to dynamically load backend DLLs

### Backend DLL Exports

All backends must export these C functions:

```rust
pub extern "C" fn get_backend_info() -> BackendInfo;
pub extern "C" fn create_model(config: *const ModelConfig) -> *mut ModelHandle;
pub extern "C" fn destroy_model(handle: *mut ModelHandle);
pub extern "C" fn transcribe(...) -> TranscribeResult;
pub extern "C" fn free_result(result: *mut TranscribeResult);
pub extern "C" fn get_last_error() -> *const c_char;
```

### Data Flow

1. User presses hotkey → `HotkeyManager` detects via global-hotkey
2. Event sent to main event loop → Start audio capture (`cpal`)
3. User releases hotkey → Stop capture, get audio buffer
4. Audio sent to backend → `transcribe()` FFI call
5. Result received → `Typer` injects text via `enigo`

## Build Configuration

### Critical File: `.cargo/config.toml`

This file contains essential environment variables that must be configured correctly:

```toml
[env]
# Fix for system date being 2026 - CMake thinks VS 18 2026 should exist
CMAKE_GENERATOR = "Visual Studio 17 2022"

# Force correct Visual Studio 2022 toolchain
VSINSTALLDIR = "C:/Program Files/Microsoft Visual Studio/2022/Community/"
VCToolsInstallDir = "..."
VCToolsVersion = "14.44.35207"

# LLVM/Clang for bindgen (whisper-rs-sys)
LIBCLANG_PATH = "C:/Program Files/LLVM/bin"
BINDGEN_EXTRA_CLANG_ARGS = "..."

# CUDA paths (v13.0)
CUDA_PATH = "C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.0"
CUDA_TOOLKIT_ROOT_DIR = "..."
CMAKE_CUDA_COMPILER = ".../bin/nvcc.exe"
```

**Important**: If build fails with "Visual Studio 18 2026 not found", check this file first. The hardcoded paths must match your actual installation locations.

### Build Commands

```batch
:: CPU-only build
tools/scripts/build_win.bat

:: CUDA build (requires CUDA Toolkit 13.0 + cuDNN 9.x)
tools/scripts/build_cuda.bat

:: Package for release (builds + creates zip)
tools/scripts/package_release.bat [cuda|cpu]

:: Package only (requires binaries already built)
tools/scripts/package_only.bat
```

### Build Output

- `target/release/app.exe` (~5 MB) - Main application
- `target/release/whisper_cpp.dll` (~22 MB with CUDA) - whisper.cpp backend
- `target/release/whisper_ct2.dll` (~13 MB with CUDA) - CTranslate2 backend

### Build Notes

- whisper-ct2 backend uses `RUSTFLAGS=-C target-feature=+crt-static` for static CRT linking
- Do NOT use `crt-static` for `cdylib` builds (causes linker errors)
- whisper-cpp backend uses CMake and bindgen (requires LLVM/Clang)

## Dependencies

### System Requirements

- Windows 10/11
- Visual Studio 2022 (Build Tools or Community)
- LLVM/Clang (for bindgen)
- Rust toolchain (MSVC target)
- CUDA Toolkit 13.0 + cuDNN 9.x (for GPU builds only)

### Key Crates

| Crate | Purpose |
|-------|---------|
| `tao` | Window management |
| `softbuffer` | Pixel buffer rendering for UI |
| `tray-icon` | System tray integration |
| `global-hotkey` | Global hotkey capture |
| `enigo` | Keyboard text injection |
| `cpal` | Audio capture |
| `libloading` | Dynamic DLL loading |
| `rfd` | Native file/folder dialogs |
| `whisper-rs` | whisper.cpp Rust bindings |
| `ct2rs` | CTranslate2 Rust bindings |

## Development Guidelines

### Code Style

- Use `anyhow` for error handling
- Prefer `parking_lot` mutexes over std
- Use `Arc<Mutex<T>>` for shared state across threads
- Keep FFI boundary code in `app-core`

### Adding a New Backend

1. Create new directory under `crates/backends/`
2. Implement required FFI exports in `src/lib.rs`
3. Create `manifest.json` with model definitions
4. Add to workspace `Cargo.toml`
5. Update build scripts to compile the new backend

### Configuration

Config is stored next to the executable at `config.json` (portable):

```json
{
  "backend_id": "whisper-ct2",
  "model_name": "faster-whisper-small",
  "model_path": "C:\\...\\models\\faster-whisper-small",
  "use_gpu": true,
  "cuda_path": "C:\\Program Files\\...\\CUDA\\v13.0",
  "cudnn_path": "C:\\Program Files\\...\\CUDNN\\v9.18",
  "hotkey_push_to_talk": "Backquote",
  "hotkey_always_listen": "Control+Backquote"
}
```

### Models

Models are downloaded to `models/` directory next to the executable:

- **whisper-cpp**: Single `.bin` files (GGML format)
- **whisper-ct2**: Directory with `model.bin`, `config.json`, `tokenizer.json`, `vocabulary.txt`

## Testing

### Manual Testing Checklist

1. **First-run Setup**: Delete `config.json` next to exe, run exe
2. **Model Download**: Select model, click Download, verify progress
3. **Hotkey Recording**: Press push-to-talk hotkey, speak, release
4. **GPU Acceleration**: Enable "Use GPU", verify CUDA is used
5. **System Tray**: Right-click tray icon, test menu items
6. **Always-Listen Mode**: Press always-listen hotkey, verify continuous transcription
7. **Settings**: Click Settings in tray menu, verify wizard opens

### Debug Build

Comment out `#![windows_subsystem = "windows"]` in `main.rs` to see console output.

### Known Issues

1. **Dead Code Warnings**: Several `#[allow(dead_code)]` markers needed
2. **DPI Scaling**: UI tested at 96 DPI; may need adjustment for high-DPI displays
3. **cuDNN Path Structure**: cuDNN 9.x has nested directories (`bin/13.1/`) that require special handling

## Release Process

1. Update version in workspace `Cargo.toml`
2. Update version in all build scripts (`build_*.bat`, `package_*.bat`)
3. Run `tools/scripts/package_release.bat cuda` for full CUDA build
4. Verify `dist/app-v{VERSION}.zip` contents
5. Test on clean Windows installation

## Security Considerations

- FFI boundaries use `#[repr(C)]` structs with explicit padding
- DLL loading uses absolute paths relative to exe location
- No network connections except for model downloads (HuggingFace)
- Configuration stored in user's AppData (not system-wide)
- GPU libraries loaded dynamically (no hard dependency on CUDA)

## Troubleshooting

| Issue | Solution |
|-------|----------|
| "Visual Studio 18 2026 not found" | Check `.cargo/config.toml` paths |
| CUDA not detected | Verify `CUDA_PATH` environment variable |
| Backend DLL fails to load | Check backend DLL is in `backends/<name>/` relative to exe |
| Model download fails | Check internet connection, verify HuggingFace accessibility |
| Audio not capturing | Check Windows privacy settings for microphone access |
| Hotkeys not working | Check if another app has registered the same hotkeys |

## Contact / Resources

- Project uses models from: https://huggingface.co/ggerganov/whisper.cpp, https://huggingface.co/Systran
- whisper.cpp: https://github.com/ggerganov/whisper.cpp
- CTranslate2: https://github.com/OpenNMT/CTranslate2
