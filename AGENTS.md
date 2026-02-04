# Repository Guidelines

## Project Structure & Module Organization

- `apps/app/` holds the Windows GUI application (tray UI, setup wizard, audio capture).
- `crates/app-core/` defines the shared FFI types used by the app and backend DLLs.
- `crates/backends/` contains backend plugins (e.g., `whisper-cpp`, `whisper-ct2`) and their `manifest.json` files.
- `tools/scripts/` contains build and packaging scripts; `tools/cmake/` is the pinned CMake distribution.
- `dist/` is the packaging output directory; `target/` is Cargo build output and should stay in `.gitignore`.

## Build, Test, and Development Commands

- `cargo build -p app --release` builds the main executable in `target/release/app.exe`.
- `tools/scripts/build_win.bat` builds CPU-only binaries (app + backends).
- `tools/scripts/build_cuda.bat` builds CUDA-enabled binaries (requires CUDA + cuDNN).
- `tools/scripts/package_release.bat [cuda|cpu]` builds and produces a zip in `dist/`.
- `cargo test -p app` runs unit/integration tests for the app crate.

## Coding Style & Naming Conventions

- Rust 2021 edition; prefer `anyhow` for errors and keep FFI types in `app-core`.
- Use `snake_case` for functions/modules and `CamelCase` for types.
- Default to `rustfmt` formatting (`cargo fmt`) before committing.
- Backend DLL exports must match the FFI interface in `app-core`.

## Testing Guidelines

- Tests live under `apps/app/tests/` and should be named `*_tests.rs`.
- Favor lightweight unit tests; keep GPU/CUDA tests optional.
- For manual QA, verify setup wizard, model download, hotkeys, and tray UI on Windows.

## Commit & Pull Request Guidelines

- There is no established commit message convention yet; use short, imperative summaries (e.g., “Fix CUDA path detection”).
- PRs should describe user impact, link any issues, and include screenshots for UI changes.

## Configuration & Runtime Notes

- Build environment is configured in `.cargo/config.toml` (toolchain, LLVM, CUDA paths).
- Runtime config is stored next to the executable as `config.json`; logs write to `app.log`.
- Models are downloaded into a `models/` folder next to the executable.
