@echo off
REM Download Voxtral model from HuggingFace for mistral.rs backend
REM Model: mistralai/Voxtral-Mini-4B-Realtime-2602 (~8.2GB)

setlocal enabledelayedexpansion

REM Set model directory
set MODEL_DIR=%~dp0models\Voxtral-Mini-4B-Realtime-2602
echo Creating model directory: %MODEL_DIR%
mkdir "%MODEL_DIR%" 2>nul

REM Check if huggingface-cli is installed
where huggingface-cli >nul 2>nul
if %errorlevel% neq 0 (
    echo Installing huggingface_hub...
    pip install huggingface_hub
)

REM Download model files
echo.
echo Downloading Voxtral-Mini-4B-Realtime-2602 from HuggingFace...
echo This will download ~8.2GB of data.
echo.

cd /d "%MODEL_DIR%"

REM Download required files
huggingface-cli download mistralai/Voxtral-Mini-4B-Realtime-2602 ^
    tokenizer.json ^
    tekken.json ^
    params.json ^
    consolidated.safetensors ^
    --local-dir . ^
    --resume-download

if %errorlevel% neq 0 (
    echo.
    echo ERROR: Download failed!
    echo Please check your internet connection and try again.
    exit /b 1
)

echo.
echo ========================================
echo Download complete!
echo Model saved to: %MODEL_DIR%
echo ========================================
echo.
echo To use this model:
echo 1. Copy the model folder to your app directory
echo 2. Select "Voxtral Mini 4B Realtime" in the setup wizard
echo.

endlocal
