@echo off
REM Fast CUDA build for whisper-ct2 - targets only RTX 4070 Ti (arch 8.9)

set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

echo ========================================
echo  Fast whisper-ct2 CUDA Build
echo  Target: RTX 4070 Ti (arch 8.9)
echo ========================================
echo.

REM Setup Visual Studio
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
set LIBCLANG_PATH=C:\Program Files\LLVM\bin

REM CUDA paths (using short names to avoid CMake issues)
set CUDA_PATH=C:\PROGRA~1\NVIDIA~2\CUDA\v13.0
set CUDA_TOOLKIT_ROOT_DIR=C:/PROGRA~1/NVIDIA~2/CUDA/v13.0

REM IMPORTANT: Set specific architecture to 8.9 (RTX 4070 Ti)
REM "Common" would build for ALL architectures (50,60,70,80,90) - very slow!
set CUDA_ARCH_LIST=8.9

echo [INFO] CUDA Architecture: 8.9 (RTX 4070 Ti)
echo [INFO] CUDA Path: %CUDA_PATH%
echo.

REM Clean previous builds
echo [1/3] Cleaning previous builds...
cargo clean -p whisper-ct2 2>nul
cargo clean -p ct2rs 2>nul
echo [OK] Cleaned
echo.

REM Static CRT linking required for Windows
echo [2/3] Building whisper-ct2 with CUDA...
set RUSTFLAGS=-C target-feature=+crt-static

REM Add CUDA to PATH for NVCC
set PATH=%CUDA_PATH%\bin;%PATH%

cargo build -p whisper-ct2 --release --features cuda,cudnn,cuda-dynamic-loading 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Build failed!
    exit /b 1
)

echo [OK] Build complete!
echo.

REM Show file size
for %%F in (target\release\whisper_ct2.dll) do (
    echo File: %%F
    echo Size: %%~zF bytes
)

echo.
echo ========================================
echo  Build Complete!
echo ========================================
echo.
pause

popd
