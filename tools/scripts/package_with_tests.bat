@echo off
REM Package script that includes running tests

setlocal enabledelayedexpansion
set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

set VERSION=0.1.0
set RELEASE_NAME=app-v%VERSION%-cuda
set RELEASE_DIR=dist\%RELEASE_NAME%

echo ========================================
echo  App Build with Tests
echo  Version: %VERSION%
echo ========================================
echo.

REM Step 1: Run all tests
echo [1/5] Running tests...
cargo test -p app --tests --quiet
if %errorlevel% neq 0 (
    echo [ERROR] Tests failed! Aborting build.
    exit /b 1
)
echo [OK] All tests passed
echo.

REM Step 2: Build main application
echo [2/5] Building main application...
cargo build -p app --release --quiet
echo [OK] app.exe
echo.

REM Step 3: Build whisper-cpp backend with CUDA
echo [3/5] Building whisper-cpp backend (CUDA)...
cargo build -p whisper-cpp --release --features cuda --quiet
echo [OK] whisper_cpp.dll
echo.

REM Step 4: Create release directory
echo [4/5] Creating release package...
if exist dist rmdir /s /q dist
mkdir %RELEASE_DIR%
mkdir %RELEASE_DIR%\backends\whisper-cpp
mkdir %RELEASE_DIR%\models

REM Copy main executable
copy target\release\app.exe %RELEASE_DIR%\

REM Copy whisper-cpp backend
copy target\release\whisper_cpp.dll %RELEASE_DIR%\backends\whisper-cpp\
copy crates\backends\whisper-cpp\manifest.json %RELEASE_DIR%\backends\whisper-cpp\

REM Copy documentation
copy README.md %RELEASE_DIR%\ 2>nul
copy docs\BACKEND_TESTING.md %RELEASE_DIR%\ 2>nul
copy docs\UI_TESTING.md %RELEASE_DIR%\ 2>nul

REM Create default config
echo { > %RELEASE_DIR%\config.json
echo   "backend_id": "whisper-cpp", >> %RELEASE_DIR%\config.json
echo   "model_name": "ggml-tiny", >> %RELEASE_DIR%\config.json
echo   "model_path": "models/ggml-tiny", >> %RELEASE_DIR%\config.json
echo   "use_gpu": true, >> %RELEASE_DIR%\config.json
echo   "cuda_path": null, >> %RELEASE_DIR%\config.json
echo   "cudnn_path": null, >> %RELEASE_DIR%\config.json
echo   "hotkey_push_to_talk": "Backquote", >> %RELEASE_DIR%\config.json
echo   "hotkey_always_listen": "Control+Backquote", >> %RELEASE_DIR%\config.json
echo   "overlay_visible": true, >> %RELEASE_DIR%\config.json
echo   "overlay_x": null, >> %RELEASE_DIR%\config.json
echo   "overlay_y": null >> %RELEASE_DIR%\config.json
echo } >> %RELEASE_DIR%\config.json

REM Create batch file for easy launch
echo @echo off > %RELEASE_DIR%\App.bat
echo REM Launch App >> %RELEASE_DIR%\App.bat
echo. >> %RELEASE_DIR%\App.bat
echo REM Set CUDA paths if available >> %RELEASE_DIR%\App.bat
echo if exist "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin" ( >> %RELEASE_DIR%\App.bat
echo     set PATH=%%PATH%%;C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin >> %RELEASE_DIR%\App.bat
echo ) >> %RELEASE_DIR%\App.bat
echo. >> %RELEASE_DIR%\App.bat
echo start app.exe >> %RELEASE_DIR%\App.bat

echo [OK] Package created at %RELEASE_DIR%
echo.

REM Step 5: Summary
echo [5/5] Build Summary:
echo   - Main executable: %RELEASE_DIR%\app.exe
echo   - Backend: %RELEASE_DIR%\backends\whisper-cpp\
echo   - Models folder: %RELEASE_DIR%\models\
echo   - Config: %RELEASE_DIR%\config.json
echo.

REM File sizes
echo File sizes:
for %%f in (%RELEASE_DIR%\app.exe) do echo   app.exe: %%~zf bytes
for %%f in (%RELEASE_DIR%\backends\whisper-cpp\whisper_cpp.dll) do echo   whisper_cpp.dll: %%~zf bytes
echo.

echo ========================================
echo  Build Complete!
echo ========================================
echo.
echo To run:
echo   1. Download a model to %RELEASE_DIR%\models\
echo   2. Run %RELEASE_DIR%\App.bat
echo.

endlocal
popd
