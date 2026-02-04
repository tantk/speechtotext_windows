#!/bin/bash
# Release packaging script for app (bash version)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}/../.." || exit 1

VERSION="0.1.0"
RELEASE_NAME="app-v${VERSION}"
RELEASE_DIR="dist/${RELEASE_NAME}"

echo "========================================"
echo " Packaging app Release"
echo " Version: ${VERSION}"
echo "========================================"
echo

# Check if binaries exist
if [ ! -f "target/release/app.exe" ]; then
    echo "ERROR: app.exe not found!"
    echo "Run tools/scripts/build_cuda.bat or tools/scripts/build_win.bat first."
    exit 1
fi

if [ ! -f "target/release/whisper_cpp.dll" ]; then
    echo "ERROR: whisper_cpp.dll not found!"
    exit 1
fi

if [ ! -f "target/release/whisper_ct2.dll" ]; then
    echo "ERROR: whisper_ct2.dll not found!"
    exit 1
fi

# Clean and create release directory
rm -rf dist
mkdir -p "${RELEASE_DIR}/backends/whisper-cpp"
mkdir -p "${RELEASE_DIR}/backends/whisper-ct2"

echo "Copying files..."

# Copy main executable
cp target/release/app.exe "${RELEASE_DIR}/"

# Copy whisper-cpp backend
cp target/release/whisper_cpp.dll "${RELEASE_DIR}/backends/whisper-cpp/"
cp crates/backends/whisper-cpp/manifest.json "${RELEASE_DIR}/backends/whisper-cpp/"

# Copy whisper-ct2 backend
cp target/release/whisper_ct2.dll "${RELEASE_DIR}/backends/whisper-ct2/"
cp crates/backends/whisper-ct2/manifest.json "${RELEASE_DIR}/backends/whisper-ct2/"

# Create README
cat > "${RELEASE_DIR}/README.txt" << 'EOF'
App
===
Version: 0.1.0

Quick Start:
1. Run app.exe
2. Select a model and click Download
3. Click Start

Hotkeys (default):
- Push-to-Talk: ` (backtick)
- Toggle Listening: Ctrl+`

GPU Support:
This build includes CUDA support for NVIDIA GPUs.
To use GPU acceleration:
1. Install CUDA Toolkit 13.0+ from NVIDIA
2. Install cuDNN 9.x from NVIDIA
3. Check "Use GPU" in the setup wizard

If CUDA is not installed, the app will automatically use CPU.

Models: ./models
Config: ./config.json

Included Backends:
- whisper-cpp (whisper.cpp) - GGML format models
- whisper-ct2 (CTranslate2) - Faster-whisper format models
EOF

# Create zip
echo "Creating zip..."
cd dist
powershell -Command "Compress-Archive -Path '${RELEASE_NAME}/*' -DestinationPath '${RELEASE_NAME}.zip' -Force"
cd ..

echo
echo "========================================"
echo " Package created successfully!"
echo "========================================"
echo
echo "Location: dist/${RELEASE_NAME}.zip"
echo
echo "Contents:"
ls -lh "${RELEASE_DIR}/"
echo
echo "backends/whisper-cpp:"
ls -lh "${RELEASE_DIR}/backends/whisper-cpp/"
echo
echo "backends/whisper-ct2:"
ls -lh "${RELEASE_DIR}/backends/whisper-ct2/"
echo
echo "Zip size:"
ls -lh "dist/${RELEASE_NAME}.zip"
