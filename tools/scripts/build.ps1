# App Build Script (PowerShell)
# Usage: .\build.ps1 [command]
# Commands: all, build, run, test, package, clean, help

param(
    [Parameter(Position=0)]
    [string]$Command = "all"
)

$ErrorActionPreference = "Stop"
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location (Join-Path $scriptRoot "..\\..")

# Configuration
$VERSION = "0.1.0"
$RELEASE_DIR = "target\release"
$PACKAGE_DIR = "dist\app-v$VERSION-cuda"

# CUDA settings (using 8.3 short paths to avoid CMake issues)
$env:CUDA_PATH = "C:/PROGRA~1/NVIDIA~2/CUDA/v13.0"
$env:CUDA_ARCH_LIST = "89"
$env:CMAKE = "C:/dev/speechwindows/tools/cmake/bin/cmake.exe"

function Write-Header($text) {
    Write-Host "=========================================" -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host "=========================================" -ForegroundColor Cyan
}

function Write-Step($step, $text) {
    Write-Host "[$step] $text" -ForegroundColor Yellow
}

function Write-OK($text) {
    Write-Host "[OK] $text" -ForegroundColor Green
}

function Write-Error($text) {
    Write-Host "[ERROR] $text" -ForegroundColor Red
}

function Build-App {
    Write-Step "1/3" "Building main application..."
    cargo build -p app --release
    if ($LASTEXITCODE -ne 0) { throw "Failed to build app" }
    Write-OK "app.exe"
}

function Build-WhisperCpp {
    Write-Step "2/3" "Building whisper-cpp backend (CUDA)..."
    cargo build -p whisper-cpp --release --features cuda
    if ($LASTEXITCODE -ne 0) { throw "Failed to build whisper-cpp" }
    Write-OK "whisper_cpp.dll"
}

function Build-WhisperCt2 {
    Write-Step "3/3" "Building whisper-ct2 backend (CUDA)..."
    $env:RUSTFLAGS = "-C target-feature=+crt-static"
    cargo build -p whisper-ct2 --release --features cuda
    $env:RUSTFLAGS = ""
    if ($LASTEXITCODE -ne 0) { throw "Failed to build whisper-ct2" }
    Write-OK "whisper_ct2.dll"
}

function Setup-Backends {
    Write-Host "Setting up backends folder structure..." -ForegroundColor Yellow

    # Create release directories
    New-Item -ItemType Directory -Force -Path "$RELEASE_DIR\backends\whisper-cpp" | Out-Null
    New-Item -ItemType Directory -Force -Path "$RELEASE_DIR\backends\whisper-ct2" | Out-Null
    New-Item -ItemType Directory -Force -Path "$RELEASE_DIR\models" | Out-Null

    # Copy whisper-cpp files
    Copy-Item "crates\backends\whisper-cpp\manifest.json" "$RELEASE_DIR\backends\whisper-cpp\" -Force
    Copy-Item "$RELEASE_DIR\whisper_cpp.dll" "$RELEASE_DIR\backends\whisper-cpp\" -Force

    # Copy whisper-ct2 files
    Copy-Item "crates\backends\whisper-ct2\manifest.json" "$RELEASE_DIR\backends\whisper-ct2\" -Force
    Copy-Item "$RELEASE_DIR\whisper_ct2.dll" "$RELEASE_DIR\backends\whisper-ct2\" -Force

    Write-OK "Release backends ready"

    # Also set up debug folder (for development)
    $DEBUG_DIR = "target\debug"
    if (Test-Path "$DEBUG_DIR") {
        Write-Host "Setting up debug backends..." -ForegroundColor Yellow
        New-Item -ItemType Directory -Force -Path "$DEBUG_DIR\backends\whisper-cpp" | Out-Null
        New-Item -ItemType Directory -Force -Path "$DEBUG_DIR\backends\whisper-ct2" | Out-Null
        New-Item -ItemType Directory -Force -Path "$DEBUG_DIR\models" | Out-Null

        Copy-Item "crates\backends\whisper-cpp\manifest.json" "$DEBUG_DIR\backends\whisper-cpp\" -Force
        Copy-Item "crates\backends\whisper-ct2\manifest.json" "$DEBUG_DIR\backends\whisper-ct2\" -Force

        # Copy release DLLs to debug (they work for both)
        if (Test-Path "$RELEASE_DIR\whisper_cpp.dll") {
            Copy-Item "$RELEASE_DIR\whisper_cpp.dll" "$DEBUG_DIR\backends\whisper-cpp\" -Force
        }
        if (Test-Path "$RELEASE_DIR\whisper_ct2.dll") {
            Copy-Item "$RELEASE_DIR\whisper_ct2.dll" "$DEBUG_DIR\backends\whisper-ct2\" -Force
        }
        Write-OK "Debug backends ready"
    }
}

function Build-All {
    Write-Header "Building App (CUDA)"
    Build-App
    Build-WhisperCpp
    Build-WhisperCt2
    Setup-Backends
    Write-Header "Build Complete!"
}

function Build-Cpu {
    Write-Header "Building App (CPU only)"
    Write-Step "1/3" "Building main application..."
    cargo build -p app --release

    Write-Step "2/3" "Building whisper-cpp backend..."
    cargo build -p whisper-cpp --release

    Write-Step "3/3" "Building whisper-ct2 backend..."
    $env:RUSTFLAGS = "-C target-feature=+crt-static"
    cargo build -p whisper-ct2 --release
    $env:RUSTFLAGS = ""

    Setup-Backends
    Write-Header "CPU Build Complete!"
}

function Run-App {
    Build-All
    Write-Host "Starting app..." -ForegroundColor Yellow
    & "$RELEASE_DIR\app.exe"
}

function Run-Tests {
    Write-Header "Running Tests"
    cargo test -p app
}

function Run-AllTests {
    Write-Header "Running All Tests"
    cargo test --workspace
}

function Clean-Build {
    Write-Header "Cleaning Build"
    cargo clean
    if (Test-Path "$RELEASE_DIR\backends") {
        Remove-Item -Recurse -Force "$RELEASE_DIR\backends"
    }
    Write-OK "Clean complete"
}

function Clean-Ct2 {
    Write-Host "Cleaning ct2rs cache..." -ForegroundColor Yellow
    Get-ChildItem -Path "target\release\build" -Filter "ct2rs*" -Directory | Remove-Item -Recurse -Force
    Write-OK "ct2rs cache cleaned"
}

function Create-Package {
    Build-All

    Write-Header "Creating Release Package"

    # Remove old package
    if (Test-Path $PACKAGE_DIR) {
        Remove-Item -Recurse -Force $PACKAGE_DIR
    }

    # Create directories
    New-Item -ItemType Directory -Force -Path "$PACKAGE_DIR\backends\whisper-cpp" | Out-Null
    New-Item -ItemType Directory -Force -Path "$PACKAGE_DIR\backends\whisper-ct2" | Out-Null
    New-Item -ItemType Directory -Force -Path "$PACKAGE_DIR\models" | Out-Null

    # Copy files
    Copy-Item "$RELEASE_DIR\app.exe" $PACKAGE_DIR
    Copy-Item "$RELEASE_DIR\backends\whisper-cpp\*" "$PACKAGE_DIR\backends\whisper-cpp\"
    Copy-Item "$RELEASE_DIR\backends\whisper-ct2\*" "$PACKAGE_DIR\backends\whisper-ct2\"

    # Create README
    @"
App v$VERSION (CUDA)

Requirements:
- Windows 10/11
- NVIDIA GPU with CUDA support (for GPU acceleration)

Usage:
1. Run app.exe
2. Select and download a model
3. Press backtick (``) to record, release to transcribe

Hotkeys:
- Backtick (``) - Push-to-talk
- Ctrl+Backtick - Toggle always-listen mode
"@ | Out-File -FilePath "$PACKAGE_DIR\README.txt" -Encoding UTF8

    Write-OK "Package created: $PACKAGE_DIR"
    Get-ChildItem $PACKAGE_DIR -Recurse | Format-Table Name, Length
}

function Create-Zip {
    Create-Package

    Write-Host "Creating zip archive..." -ForegroundColor Yellow
    $zipPath = "dist\app-v$VERSION-cuda.zip"

    if (Test-Path $zipPath) {
        Remove-Item $zipPath
    }

    Compress-Archive -Path $PACKAGE_DIR -DestinationPath $zipPath
    Write-OK "Created: $zipPath"
}

function Show-Help {
    Write-Host @"
App Build Script

Usage: .\build.ps1 [command]

Commands:
  all            Build everything and set up backends (default)
  build          Same as 'all'
  build-cpu      Build CPU-only version (no CUDA)
  run            Build and run the application
  test           Run unit tests
  test-all       Run all workspace tests
  package        Create release package
  zip            Create release zip archive
  clean          Clean all build artifacts
  clean-ct2      Clean ct2rs cache (for rebuild)
  help           Show this help

Examples:
  .\build.ps1              # Build everything
  .\build.ps1 run          # Build and run
  .\build.ps1 package      # Create release package
  .\build.ps1 clean-ct2    # Clean ct2rs cache before rebuild
"@
}

# Main
switch ($Command.ToLower()) {
    "all"       { Build-All }
    "build"     { Build-All }
    "build-cpu" { Build-Cpu }
    "run"       { Run-App }
    "test"      { Run-Tests }
    "test-all"  { Run-AllTests }
    "package"   { Create-Package }
    "zip"       { Create-Zip }
    "clean"     { Clean-Build }
    "clean-ct2" { Clean-Ct2 }
    "help"      { Show-Help }
    default     {
        Write-Error "Unknown command: $Command"
        Show-Help
        exit 1
    }
}
