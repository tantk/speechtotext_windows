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

echo [1/3] Building main application...
cargo build -p app --release || exit /b 1
echo [OK] app.exe

echo [2/3] Building whisper-cpp backend (CUDA)...
cargo build -p whisper-cpp --release --features cuda || exit /b 1
echo [OK] whisper_cpp.dll

echo [3/3] Building whisper-ct2 backend (CUDA)...
set RUSTFLAGS=-C target-feature=+crt-static
cargo build -p whisper-ct2 --release --features cuda || exit /b 1
set RUSTFLAGS=
echo [OK] whisper_ct2.dll

echo.
echo ========================================
echo  CUDA Build complete!
echo ========================================

popd
