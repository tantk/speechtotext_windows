@echo off
REM Build script for app (with CUDA for RTX 4070 Ti)

set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

echo ========================================
echo  Building app (CUDA)
echo ========================================
echo.

call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
set CMAKE_GENERATOR=Visual Studio 17 2022

REM Use CMake 3.29 to avoid FindCUDA path escaping bug in CMake 3.31
set CMAKE=C:\dev\speechwindows\tools\cmake\bin\cmake.exe

REM CUDA paths (using 8.3 short paths to avoid CMake escape issues)
set CUDA_PATH=C:/PROGRA~1/NVIDIA~2/CUDA/v13.0
set CMAKE_INCLUDE_PATH=C:/PROGRA~1/NVIDIA/CUDNN/v9.18/include/13.1
set CMAKE_LIBRARY_PATH=C:/PROGRA~1/NVIDIA/CUDNN/v9.18/lib/13.1/x64
set CUDA_ARCH_LIST=89

echo [1/4] Building main application...
cargo build -p app --release || exit /b 1
echo [OK] app.exe

echo [2/4] Building whisper-cpp backend (CUDA)...
cargo build -p whisper-cpp --release --features cuda || exit /b 1
echo [OK] whisper_cpp.dll

echo [3/4] Building whisper-ct2 backend (CUDA)...
set RUSTFLAGS=-C target-feature=+crt-static
cargo build -p whisper-ct2 --release --features cuda || exit /b 1
set RUSTFLAGS=
echo [OK] whisper_ct2.dll

echo [4/4] Building mistralrs Voxtral backend (CUDA)...
REM Set CUDA paths for mistralrs build (must use CUDA 13.0, not 10.1)
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0
set CUDA_HOME=%CUDA_PATH%
set CUDA_ROOT=%CUDA_PATH%
REM Update PATH to put CUDA 13.0 first
set PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin;%PATH%
cargo build -p mistralrs-backend --release --features cuda || exit /b 1
echo [OK] mistralrs_backend.dll

echo.
echo ========================================
echo  CUDA Build complete!
echo ========================================

popd
