@echo off
REM Build script for app (CPU only)

set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

echo ========================================
echo  Building app (CPU)
echo ========================================
echo.

call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
set CMAKE_GENERATOR=Visual Studio 17 2022

echo [1/3] Building main application...
cargo build -p app --release || exit /b 1
echo [OK] app.exe

echo [2/3] Building whisper-cpp backend...
cargo build -p whisper-cpp --release || exit /b 1
echo [OK] whisper_cpp.dll

echo [3/3] Building whisper-ct2 backend...
set RUSTFLAGS=-C target-feature=+crt-static
cargo build -p whisper-ct2 --release || exit /b 1
set RUSTFLAGS=
echo [OK] whisper_ct2.dll

echo.
echo Build complete!

popd
