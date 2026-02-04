# App Project Handover

## Project Overview

App is a Windows speech-to-text application with:
- System tray integration
- Multiple backend support (whisper-cpp, whisper-ct2/Faster Whisper)
- CUDA GPU acceleration
- Global hotkeys for push-to-talk and toggle listening
- **Always-Listen mode** (VAD-based continuous listening)
- Setup wizard for configuration

## Repository Structure

```
.
├── apps/
│   └── app/                 # Main application
│   └── src/
│       ├── main.rs          # Entry point
│       ├── always_listen.rs # VAD-based continuous listening
│       ├── setup.rs         # Setup wizard UI (softbuffer-based)
│       ├── config.rs        # Configuration and CUDA detection
│       ├── backend_loader.rs # Dynamic DLL loading
│       ├── tray.rs          # System tray
│       ├── overlay.rs       # Status overlay
│       ├── downloader.rs    # Model download handling
│       ├── hotkeys.rs       # Global hotkey handling
│       └── typer.rs         # Keyboard simulation
├── crates/
│   ├── app-core/            # Shared FFI types (#[repr(C)])
│   └── crates/backends/
│       ├── whisper-cpp/     # whisper.cpp backend (GGML models)
│       │   ├── src/lib.rs
│       │   ├── Cargo.toml
│       │   └── manifest.json    # Model definitions
│       └── whisper-ct2/     # CTranslate2/Faster Whisper backend
│           ├── src/lib.rs
│           ├── Cargo.toml
│           └── manifest.json
├── docs/
│   └── ALWAYS_LISTEN_DESIGN.md  # Architecture design doc
├── .cargo/config.toml       # Critical build configuration
├── tools/
│   ├── cmake/               # Pinned CMake distribution
│   └── scripts/             # Build + packaging scripts
└── dist/                    # Packaged outputs
```

## Build Configuration

### Critical File: `.cargo/config.toml`

Contains essential environment variables for building:
- `CMAKE_GENERATOR` - Forces VS 2022 (system date 2026 causes CMake to look for VS 18)
- `VSINSTALLDIR`, `VCToolsInstallDir`, `VCToolsVersion` - VS 2022 toolchain paths
- `LIBCLANG_PATH`, `BINDGEN_EXTRA_CLANG_ARGS` - For whisper-rs-sys bindgen
- `CUDA_PATH`, `CUDA_TOOLKIT_ROOT_DIR`, `CMAKE_CUDA_COMPILER` - CUDA 13.0 paths

### Build Commands

```batch
# CPU-only build
tools/scripts/build_win.bat

# CUDA build (RTX 4070 Ti) - optimized for compute capability 8.9
tools/scripts/build_cuda.bat

# Fast CUDA build (specific architecture only)
set CUDA_ARCH_LIST=8.9
cargo build -p whisper-cpp --release --features cuda

# Package for release
tools/scripts/package_only.bat   # or tools/scripts/package.sh in Git Bash
```

### Build Output

- `target/release/app.exe` (~5 MB)
- `target/release/whisper_cpp.dll` (~20 MB with CUDA, RTX 4070 Ti arch only)
- `target/release/whisper_ct2.dll` (~12 MB with CUDA)

## Current State

### Completed

1. **Backend System**
   - Dynamic DLL loading via `libloading`
   - FFI interface defined in `app-core`
   - Both backends build and work with CUDA
   - SHA256 checksum verification for downloaded models (disabled in current builds)

2. **Setup Wizard UI** (softbuffer-based, no external GUI framework)
   - Window size: 500x500 pixels
   - Model selection from unified list (all backends combined)
   - Backend auto-selected based on model choice (read-only field)
   - **Hotkey configuration with capture (FIXED)**
   - GPU toggle with CUDA configuration page
   - Model download with progress bar
   - Path traversal protection in downloader

3. **CUDA Configuration**
   - Auto-detection of CUDA Toolkit and cuDNN paths
   - Browse buttons for manual path selection (uses `rfd` crate)
   - Validation checks for cudart and cudnn DLLs
   - Supports cuDNN 9.x nested directory structure

4. **Release Packaging**
   - Scripts create `dist/app-v0.1.0-cuda/`
   - Contains exe, both backend DLLs, manifests, README

5. **Logging**
   - Replaced `println!` with `tracing` framework
   - Debug builds: console output visible
   - Release builds: clean GUI (no console window)

6. **Error Handling**
   - Replaced `.expect()` with proper error handling
   - User-friendly error dialogs for critical failures
   - Graceful degradation where possible

7. **Always-Listen Mode** (Partial Implementation)
   - VAD (Voice Activity Detection) engine implemented
   - State machine: Listening → Detecting → Recording → Processing
   - Pre-roll buffer for capturing speech onset
   - Configuration struct with sensible defaults
   - Architecture documented in `docs/ALWAYS_LISTEN_DESIGN.md`
   - **TODO: Full integration with transcription pipeline**

8. **Testing**
   - Unit tests for hotkey parsing in `hotkeys.rs`
   - Unit tests for config serialization in `config.rs`
   - Unit tests for setup UI in `setup.rs`
   - Integration test scaffold in `tests/integration_tests.rs`

### Recent Bug Fixes (Latest Session)

#### 1. Button Hit Detection Fix (Critical)
**Problem:** Clicking buttons on the home page didn't work because button hit rectangles were misaligned with rendered positions.

**Root Cause:** `get_home_buttons()` had a 65px Y-offset compared to `render_home_page()`. The render function started at y=65 for the first label, but the hit detection calculated button positions incorrectly.

**Fix:** Updated `get_home_buttons()` in `setup.rs` to match the exact positioning logic from `render_home_page()`:
```rust
// Layout constants - MUST match render_home_page exactly!
const FIELD_HEIGHT: u32 = 28;
const ROW_SPACING: u32 = 50;
const LABEL_FIELD_GAP: u32 = 15;

// Match render_home_page positioning exactly
let mut y: u32 = 65;

// Backend section (no button - display only)
y += LABEL_FIELD_GAP;  // y = 80 - backend field row
y += ROW_SPACING;      // y = 130 - move to next row

// Select Model button (at y=145 in render)
y += LABEL_FIELD_GAP;  // y = 145 - button row
buttons.push(ButtonRect { x: 380, y, width: 90, height: FIELD_HEIGHT, button: Button::SelectModel });
```

#### 2. Path Traversal Fix (Security)
**Problem:** Model download failed with path traversal error because `canonicalize()` was called on a file that doesn't exist yet (during download).

**Root Cause:** In `downloader.rs`, the validation code tried to canonicalize the destination file path before it was created.

**Fix:** Changed to validate the parent directory instead:
```rust
// Before (broken):
let canonical_dest = dest_path.canonicalize().unwrap_or_else(|_| dest_path.clone());
let canonical_base = dest_dir.canonicalize().unwrap_or_else(|_| dest_dir.to_path_buf());
if !canonical_dest.starts_with(&canonical_base) {

// After (fixed):
let canonical_base = dest_dir.canonicalize().unwrap_or_else(|_| dest_dir.to_path_buf());
let dest_parent = dest_path.parent().unwrap_or(dest_dir);
let canonical_parent = dest_parent.canonicalize().unwrap_or_else(|_| dest_parent.to_path_buf());
if !canonical_parent.starts_with(&canonical_base) {
```

#### 3. Hotkey Capture Fix (Critical)
**Problem:** After clicking "Set Hotkey" in the setup wizard, pressing keys showed "Press any key..." but never captured the actual key.

**Root Cause:** The keyboard event handler used incorrect API for tao 0.30.8. The code tried to match `Key::Named(named_key)` but this variant doesn't exist in tao 0.30.8. The `Key` enum has direct variants like `Key::Enter`, `Key::Character(c)`, etc.

**Fix:** Updated keyboard event handler in `setup.rs` to use correct tao 0.30.8 API:
```rust
use tao::keyboard::Key;

let key_name = match &key_event.logical_key {
    Key::Character(c) => c.to_uppercase().to_string(),
    Key::Enter => "Enter".to_string(),
    Key::Tab => "Tab".to_string(),
    Key::Space => "Space".to_string(),
    Key::Backspace => "Backspace".to_string(),
    Key::Escape => "Escape".to_string(),
    Key::ArrowUp => "ArrowUp".to_string(),
    Key::F1 => "F1".to_string(),
    // ... etc
    Key::Control | Key::Shift | Key::Alt | Key::Super => return, // Ignore modifiers
    _ => return, // Ignore unhandled keys
};
```

### Build Optimization

Created `build_whisper_ct2_cuda_fast.bat` for faster development builds:
- Sets `CUDA_ARCH_LIST=8.9` for RTX 4070 Ti only
- Reduces build time from 15+ minutes to 3-5 minutes
- Only generates code for specific GPU architecture

## Known Issues / TODO

1. **Always-Listen Mode** (In Progress)
   - Core implementation complete but not fully integrated
   - Need to connect to transcription pipeline
   - Need UI controls for VAD parameters
   - Need visual indicator for always-listen state

2. **DPI Scaling**
   - UI may need testing on high-DPI displays
   - softbuffer handles physical pixels, may need DPI awareness

3. **cuDNN Path Structure**
   - cuDNN 9.x has complex structure: `CUDNN/v9.18/bin/13.1/`
   - Detection updated but may need more testing

4. **Model Download**
   - Downloads models to exe-relative `models/` directory
   - Backend DLLs expected in `backends/<name>/` relative to exe
   - Checksum verification added but manifests need SHA256 values populated

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tao` 0.30.8 | Window management |
| `softbuffer` | Pixel buffer rendering |
| `tray-icon` | System tray |
| `global-hotkey` | Global hotkey capture |
| `enigo` | Keyboard simulation |
| `cpal` | Audio capture |
| `libloading` | Dynamic DLL loading |
| `rfd` | Folder picker dialogs |
| `tracing` | Logging framework |
| `tracing-subscriber` | Log output formatting |
| `sha2` | SHA256 checksums |
| `hex` | Hex encoding for hashes |
| `whisper-rs` | whisper.cpp bindings (backend) |
| `ct2rs` | CTranslate2 bindings (backend) |

**Important:** tao 0.30.8 uses different `Key` enum structure than newer versions. The `Key` enum has direct variants (not wrapped in `Key::Named`). See the hotkey capture fix above.

## Environment Requirements

- Windows 10/11
- Visual Studio 2022 (Build Tools)
- LLVM/Clang (for bindgen)
- Rust toolchain (MSVC)
- CUDA Toolkit 13.0 (for GPU builds)
- cuDNN 9.x (for GPU builds)

## Testing

```bash
# Run unit tests
cargo test -p app

# Run all tests
cargo test --workspace
```

### Manual Testing
1. Delete `config.json` next to the executable to trigger setup wizard
2. Run `app.exe`
3. Select a model, download it
4. Configure hotkeys (click "Configure Push-to-Talk", "Set Hotkey", press key, "Confirm")
5. Enable GPU if CUDA installed
6. Click Start
7. Test push-to-talk with backtick key (or configured hotkey)

## Debugging Tips

### Hotkey Capture Not Working
- Check console output for "DEBUG: Captured hotkey:" messages
- Verify tao version matches expected API (0.30.8)
- Check that `window.set_focus()` is called when entering capture mode

### Button Clicks Not Registering
- Compare `get_*_buttons()` Y positions with `render_*_page()` Y positions
- Add debug print of mouse coordinates and button rectangles
- Check that button rectangles are being generated for current page

### Download Fails with Path Error
- Verify path traversal validation in `downloader.rs`
- Check that parent directory exists before canonicalizing
- Ensure destination folder is within allowed base directory

## Files Added/Modified This Session

### Modified Files
- `apps/app/src/setup.rs` - Fixed button hit detection, fixed hotkey capture API
- `apps/app/src/downloader.rs` - Fixed path traversal validation for non-existent files

### New Files
- `build_whisper_ct2_cuda_fast.bat` - Fast build script for RTX 4070 Ti

## Contact / Resources

- Build issues: Check `.cargo/config.toml` paths first
- CUDA issues: Verify CUDA_PATH and cuDNN installation
- UI issues: Check render functions and button hit detection alignment in `setup.rs`
- Always-listen mode: See `docs/ALWAYS_LISTEN_DESIGN.md`
- tao API issues: Verify version 0.30.8 API (not newer versions)
