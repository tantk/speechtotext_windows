# Backend DLL Testing Guide

This document describes how to test the speech-to-text backend DLLs (whisper_cpp.dll and whisper_ct2.dll) for GPU and CPU functionality.

## Overview

The project includes two backend DLLs:
- **whisper_cpp.dll** - Uses whisper.cpp (GGML format models)
- **whisper_ct2.dll** - Uses CTranslate2 (Faster Whisper format models)

Both support GPU acceleration via CUDA when built with the `cuda` feature.

---

## Quick Test Summary

| Test Suite | Command | Description |
|------------|---------|-------------|
| Unit tests | `cargo test -p app` | 56 tests covering manifest parsing, config, etc. |
| Integration tests | `cargo test -p app --test backend_dll_tests` | 10 tests for DLL/manifest existence |
| GPU config tests | `cargo test -p app --test gpu_support_tests` | 11 tests for GPU configuration |
| Backend loading | `cargo test -p app -- --ignored` | Manual tests requiring DLLs and models |

---

## Automated Tests (Always Run)

These tests don't require built DLLs or model files:

```bash
# Run all automated tests
cargo test -p app
```

Results:
- **56 unit tests** - Backend manifest, config, hotkeys, audio, always-listen
- **10 integration tests** - DLL existence, manifest validation, API compatibility
- **11 GPU config tests** - GPU path detection, environment setup
- **2 doc tests**

**Total: 79 tests passing**

---

## Manual Backend Loading Tests

These tests require built DLLs and optionally model files. They are marked with `#[ignore]` and must be run manually.

### Prerequisites

1. **Build the backends** (choose one):
   ```bash
   # CPU only
   cargo build --release -p whisper-cpp
   cargo build --release -p whisper-ct2
   
   # With CUDA support
   tools/scripts/build_cuda.bat
   ```

2. **Verify DLLs exist**:
   ```bash
   ls target/release/*.dll
   # Should show: whisper_cpp.dll, whisper_ct2.dll
   ```

3. **Download model** (for transcription tests):
   ```bash
   # For whisper-cpp tests
   curl -L -o target/release/models/ggml-tiny.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
   ```

### Test 1: Load whisper-cpp Backend

```bash
cargo test -p app test_whisper_cpp_backend_load -- --ignored --nocapture
```

Expected output:
```
✓ whisper-cpp backend loaded successfully
  ID: whisper-cpp
  Name: Whisper (whisper.cpp)
  Supports CUDA: true
```

### Test 2: Load whisper-ct2 Backend

```bash
cargo test -p app test_whisper_ct2_backend_load -- --ignored --nocapture
```

### Test 3: Create CPU Model (whisper-cpp)

Requires `target/release/models/ggml-tiny.bin`:

```bash
cargo test -p app test_whisper_cpp_create_model_cpu -- --ignored --nocapture
```

Expected output:
```
✓ CPU model created successfully
  Transcription result: Ok("")  # Empty for silence input
```

### Test 4: Create GPU Model (whisper-cpp)

Requires:
- CUDA installed
- whisper_cpp.dll built with CUDA support
- Model file

```bash
cargo test -p app test_whisper_cpp_create_model_gpu -- --ignored --nocapture
```

Expected output:
```
CUDA_PATH: C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0
Creating GPU model...
✓ GPU model created successfully
  Transcription result: Ok("")
```

### Test 5: CPU vs GPU Comparison

```bash
cargo test -p app test_cpu_gpu_transcription_compare -- --ignored --nocapture
```

This test:
1. Creates both CPU and GPU models
2. Runs transcription on the same audio
3. Verifies both produce results

---

## End-to-End Testing via Application

### 1. Setup

```bash
# Build everything with CUDA
tools/scripts/build_cuda.bat

# Or build just the main app (uses existing DLLs)
cargo build --release -p app
```

### 2. Configure

Create `config.json` next to the executable (e.g., `target/release/config.json`):
```json
{
  "backend_id": "whisper-cpp",
  "model_name": "ggml-tiny",
  "model_path": "target/release/models/ggml-tiny.bin",
  "use_gpu": true,
  "cuda_path": "C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.0",
  "cudnn_path": "C:\\Program Files\\NVIDIA\\CUDNN\\v9.18",
  "hotkey_push_to_talk": "Ctrl+F1",
  "hotkey_always_listen": "Ctrl+F2"
}
```

### 3. Run and Check Logs

```bash
.\target\release\app.exe
```

Watch for these log messages:

**Successful Backend Loading:**
```
INFO  app > Loading backend from: C:\dev\speechwindows\target\release\backends\whisper-cpp
INFO  app > Backend loaded: whisper-cpp (Whisper (whisper.cpp))
```

**Successful GPU Model Creation:**
```
INFO  app > Creating model with GPU enabled
INFO  app > Model created successfully on device: CUDA
```

**Transcription:**
```
INFO  app > Transcription result: "hello world"
```

---

## Verifying GPU Usage

### Method 1: Check Transcription Result

The `TranscribeResult` includes a `device_used` field:
- `"CUDA"` - GPU was used
- `"CPU"` - CPU was used (possibly due to fallback)

### Method 2: GPU Monitoring

During transcription, check GPU utilization:

```bash
# Using nvidia-smi
nvidia-smi -l 1

# Or watch GPU in Task Manager > Performance > GPU
```

### Method 3: Backend Info

Call `get_backend_info()` from the DLL:
- `supports_cuda: true` - Backend was compiled with CUDA support
- `supports_cuda: false` - CPU-only build

---

## Troubleshooting

### "Failed to load DLL"

**Cause:** Missing dependencies (CUDA DLLs not in PATH)

**Fix:**
```powershell
$env:PATH = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin;" + $env:PATH
```

### "Failed to create model" (GPU)

**Causes:**
1. CUDA not available - Check `echo $env:CUDA_PATH`
2. Out of GPU memory - Try smaller model (tiny instead of base)
3. Backend compiled without CUDA - Rebuild with `tools/scripts/build_cuda.bat`

**Fix:**
```bash
# Test with CPU first
config.use_gpu = false
```

### "Model file not found"

**Fix:**
```bash
mkdir -p models
curl -L -o target/release/models/ggml-tiny.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
```

### Different results between CPU and GPU

**Note:** Minor differences are expected due to floating-point precision. Both should produce similar quality transcriptions.

---

## Test Checklist

Before releasing a GPU-enabled build:

- [ ] Unit tests pass (`cargo test -p app`)
- [ ] whisper_cpp.dll builds with CUDA
- [ ] whisper_ct2.dll builds with CUDA  
- [ ] `test_whisper_cpp_backend_load` passes
- [ ] `test_whisper_cpp_create_model_cpu` passes
- [ ] `test_whisper_cpp_create_model_gpu` passes (if CUDA available)
- [ ] Application starts with GPU enabled
- [ ] Transcription produces results with GPU
- [ ] GPU utilization visible during transcription
- [ ] Fallback to CPU works when GPU unavailable

---

## CI/CD Considerations

GPU tests are marked with `#[ignore]` because:
1. CUDA may not be installed in CI environment
2. GPU hardware may not be available
3. Model files are large and not stored in git

For CI, run only the automated tests:
```bash
cargo test -p app  # Excludes ignored tests
```

For release validation, run all tests on a machine with:
- NVIDIA GPU
- CUDA 12.x or 13.x
- cuDNN 8.x or 9.x
- Model files downloaded
