use std::path::Path;

fn main() {
    // Force CMake to use Visual Studio 2022 (not the year from system date)
    // This fixes the "Visual Studio 18 2026" error on systems with dates >= 2026
    if std::env::var("CMAKE_GENERATOR").is_err() {
        println!("cargo:rustc-env=CMAKE_GENERATOR=Visual Studio 17 2022");
    }

    // Check if model exists, print helpful message if not
    let model_path = Path::new("models/ggml-tiny.bin");

    if !model_path.exists() {
        println!("cargo:warning=Whisper model not found at models/ggml-tiny.bin");
        println!("cargo:warning=Download from: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin");
        println!("cargo:warning=Place the file in the models/ directory before running");
    }

    // Link Windows libraries
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=shell32");
    }
}
