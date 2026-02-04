@echo off
REM Release packaging script for app
REM Creates a distributable zip with all components

setlocal enabledelayedexpansion
set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

REM Configuration
set VERSION=0.1.0
set RELEASE_NAME=app-v%VERSION%
set BUILD_TYPE=%1

if "%BUILD_TYPE%"=="" set BUILD_TYPE=cuda

echo ========================================
echo  Packaging app Release
echo  Version: %VERSION%
echo  Build: %BUILD_TYPE%
echo ========================================
echo.

REM Setup Visual Studio environment
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
set CMAKE_GENERATOR=Visual Studio 17 2022

REM CUDA paths (for CUDA build)
if "%BUILD_TYPE%"=="cuda" (
    echo [INFO] Building with CUDA support...
    set CUDA_PATH=C:/PROGRA~1/NVIDIA~2/CUDA/v13.0
    set CMAKE_INCLUDE_PATH=C:/PROGRA~1/NVIDIA/CUDNN/v9.18/include/13.1
    set CMAKE_LIBRARY_PATH=C:/PROGRA~1/NVIDIA/CUDNN/v9.18/lib/13.1/x64
    set CUDA_ARCH_LIST=89
) else (
    echo [INFO] Building CPU-only version...
)

REM Create release directory
set RELEASE_DIR=dist\%RELEASE_NAME%
if exist dist rmdir /s /q dist
mkdir %RELEASE_DIR%
mkdir %RELEASE_DIR%\backends\whisper-cpp
mkdir %RELEASE_DIR%\backends\whisper-ct2

echo.
echo [1/4] Building main application...
cargo build -p app --release || goto :error
echo [OK] app.exe

echo.
echo [2/4] Building whisper-cpp backend...
if "%BUILD_TYPE%"=="cuda" (
    cargo build -p whisper-cpp --release --features cuda || goto :error
) else (
    cargo build -p whisper-cpp --release || goto :error
)
echo [OK] whisper_cpp.dll

echo.
echo [3/4] Building whisper-ct2 backend...
set RUSTFLAGS=-C target-feature=+crt-static
if "%BUILD_TYPE%"=="cuda" (
    cargo build -p whisper-ct2 --release --features cuda || goto :error
) else (
    cargo build -p whisper-ct2 --release || goto :error
)
set RUSTFLAGS=
echo [OK] whisper_ct2.dll

echo.
echo [4/4] Packaging release...

REM Copy main executable
copy target\release\app.exe %RELEASE_DIR%\ || goto :error

REM Copy whisper-cpp backend
copy target\release\whisper_cpp.dll %RELEASE_DIR%\backends\whisper-cpp\ || goto :error
copy crates\backends\whisper-cpp\manifest.json %RELEASE_DIR%\backends\whisper-cpp\ || goto :error

REM Copy whisper-ct2 backend
copy target\release\whisper_ct2.dll %RELEASE_DIR%\backends\whisper-ct2\ || goto :error
copy crates\backends\whisper-ct2\manifest.json %RELEASE_DIR%\backends\whisper-ct2\ || goto :error

REM Create README
(
echo App
echo ===
echo Version: %VERSION%
echo.
echo Quick Start:
echo 1. Run app.exe
echo 2. Select a model and click Download
echo 3. Click Start
echo.
echo Hotkeys ^(default^):
echo - Push-to-Talk: ` ^(backtick^)
echo - Toggle Listening: Ctrl+`
echo.
echo GPU Support:
if "%BUILD_TYPE%"=="cuda" (
echo This build includes CUDA support for NVIDIA GPUs.
echo To use GPU acceleration:
echo 1. Install CUDA Toolkit 13.0+ from NVIDIA
echo 2. Install cuDNN 9.x from NVIDIA
echo 3. Check "Use GPU" in the setup wizard
) else (
echo This is a CPU-only build.
echo For GPU support, download the CUDA version.
)
echo.
echo Models are downloaded next to the executable in \models\
echo Configuration is saved next to the executable in config.json
echo.
echo For more info: https://github.com/user/app
) > %RELEASE_DIR%\README.txt

REM Create zip file using PowerShell
echo.
echo Creating zip archive...
powershell -Command "Compress-Archive -Path '%RELEASE_DIR%\*' -DestinationPath 'dist\%RELEASE_NAME%.zip' -Force"

echo.
echo ========================================
echo  Release package created successfully!
echo ========================================
echo.
echo Location: dist\%RELEASE_NAME%.zip
echo.
echo Contents:
dir /b %RELEASE_DIR%
echo.
echo backends\whisper-cpp:
dir /b %RELEASE_DIR%\backends\whisper-cpp
echo.
echo backends\whisper-ct2:
dir /b %RELEASE_DIR%\backends\whisper-ct2
echo.

REM Show file sizes
echo File sizes:
for %%F in (%RELEASE_DIR%\app.exe) do echo   app.exe: %%~zF bytes
for %%F in (%RELEASE_DIR%\backends\whisper-cpp\whisper_cpp.dll) do echo   whisper_cpp.dll: %%~zF bytes
for %%F in (%RELEASE_DIR%\backends\whisper-ct2\whisper_ct2.dll) do echo   whisper_ct2.dll: %%~zF bytes
for %%F in (dist\%RELEASE_NAME%.zip) do echo   Total zip: %%~zF bytes

goto :end

:error
echo.
echo ========================================
echo  BUILD FAILED!
echo ========================================
exit /b 1

:end
endlocal
popd
