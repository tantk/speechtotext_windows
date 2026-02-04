# Speech-to-Text Windows

Windows-native speech-to-text app with a pluggable backend system (whisper.cpp and CTranslate2), tray UI, and setup wizard for model downloads.

## Quick Start

```batch
:: CPU build
cargo build -p app --release

:: Or use the helper script
tools\scripts\build_win.bat
```

## Structure

- `apps/app` - main Windows GUI application
- `crates/app-core` - shared FFI types
- `crates/backends` - backend DLLs
- `tools/scripts` - build and packaging scripts
- `docs` - project documentation

## Build Outputs

- `target/release/app.exe`
- `target/release/whisper_cpp.dll`
- `target/release/whisper_ct2.dll`

## Packaging

```batch
tools\scripts\package_release.bat [cuda|cpu]
```

See `AGENTS.md` and `docs/CLAUDE.md` for contributor guidance.
