@echo off
REM Quick packaging script - assumes binaries already built
REM Use this after running build_cuda.bat or build_win.bat

setlocal enabledelayedexpansion
set SCRIPT_DIR=%~dp0
pushd %SCRIPT_DIR%\..\..

set VERSION=0.1.0
set RELEASE_NAME=app-v%VERSION%
set RELEASE_DIR=dist\%RELEASE_NAME%

echo ========================================
echo  Packaging app (no build)
echo ========================================
echo.

REM Check if binaries exist
if not exist target\release\app.exe (
    echo ERROR: app.exe not found!
    echo Run tools/scripts/build_cuda.bat or tools/scripts/build_win.bat first.
    exit /b 1
)
if not exist target\release\whisper_cpp.dll (
    echo ERROR: whisper_cpp.dll not found!
    echo Run tools/scripts/build_cuda.bat or tools/scripts/build_win.bat first.
    exit /b 1
)
if not exist target\release\whisper_ct2.dll (
    echo ERROR: whisper_ct2.dll not found!
    echo Run tools/scripts/build_cuda.bat or tools/scripts/build_win.bat first.
    exit /b 1
)

REM Create release directory
if exist dist rmdir /s /q dist
mkdir %RELEASE_DIR%
mkdir %RELEASE_DIR%\backends\whisper-cpp
mkdir %RELEASE_DIR%\backends\whisper-ct2

echo Copying files...

REM Copy main executable
copy target\release\app.exe %RELEASE_DIR%\

REM Copy whisper-cpp backend
copy target\release\whisper_cpp.dll %RELEASE_DIR%\backends\whisper-cpp\
copy crates\backends\whisper-cpp\manifest.json %RELEASE_DIR%\backends\whisper-cpp\

REM Copy whisper-ct2 backend
copy target\release\whisper_ct2.dll %RELEASE_DIR%\backends\whisper-ct2\
copy crates\backends\whisper-ct2\manifest.json %RELEASE_DIR%\backends\whisper-ct2\

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
echo To use GPU acceleration, install from NVIDIA:
echo - CUDA Toolkit 13.0+
echo - cuDNN 9.x
echo Then check "Use GPU" in the setup wizard.
echo.
echo Models: .\models\
echo Config: .\config.json
) > %RELEASE_DIR%\README.txt

REM Create zip
echo Creating zip...
powershell -Command "Compress-Archive -Path '%RELEASE_DIR%\*' -DestinationPath 'dist\%RELEASE_NAME%.zip' -Force"

echo.
echo ========================================
echo  Package created: dist\%RELEASE_NAME%.zip
echo ========================================
echo.

REM Show sizes
echo File sizes:
for %%F in (%RELEASE_DIR%\app.exe) do set /a SIZE=%%~zF/1024/1024 & echo   app.exe: !SIZE! MB
for %%F in (%RELEASE_DIR%\backends\whisper-cpp\whisper_cpp.dll) do set /a SIZE=%%~zF/1024/1024 & echo   whisper_cpp.dll: !SIZE! MB
for %%F in (%RELEASE_DIR%\backends\whisper-ct2\whisper_ct2.dll) do set /a SIZE=%%~zF/1024/1024 & echo   whisper_ct2.dll: !SIZE! MB
for %%F in (dist\%RELEASE_NAME%.zip) do set /a SIZE=%%~zF/1024/1024 & echo   Total zip: !SIZE! MB

endlocal
popd
