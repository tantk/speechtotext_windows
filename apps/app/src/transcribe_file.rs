//! File transcription orchestrator
//!
//! Decodes audio files, processes them in 30-second chunks through
//! the backend, and generates SRT subtitle output.

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;

use crate::audio_file;
use crate::backend_loader::LoadedBackend;
use crate::config::{self, Config};
use crate::srt;

/// Samples per 30-second chunk at 16kHz
const CHUNK_SAMPLES: usize = 16000 * 30;

/// Transcribe an audio file and write SRT output.
pub fn transcribe_file(input: &Path, output: &Path, config: &Config) -> Result<()> {
    // 1. Decode audio file
    println!("Decoding audio file: {}", input.display());
    let (samples, _sample_rate) = audio_file::decode_audio_file(input)?;
    let duration_secs = samples.len() as f64 / 16000.0;
    println!(
        "Audio: {:.1}s ({} samples)",
        duration_secs,
        samples.len()
    );

    // 2. Load backend and model
    println!("Loading model...");
    config::setup_cuda_env(config);

    let backend_dir = config::get_backends_dir()?.join(&config.backend_id);
    let backend = LoadedBackend::load(&backend_dir)
        .with_context(|| format!("Failed to load backend: {}", config.backend_id))?;

    let model_load_path = resolve_model_load_path(config, &backend);
    let model = backend
        .create_model(&model_load_path, config.use_gpu)
        .context("Failed to create model")?;
    println!("Model loaded ({}).", backend.display_name);

    // 3. Split into chunks and transcribe
    let chunks: Vec<&[f32]> = samples.chunks(CHUNK_SAMPLES).collect();
    let total_chunks = chunks.len();
    println!("Processing {} chunks...", total_chunks);

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks ({percent}%) ETA: {eta}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let mut all_segments: Vec<(f64, f64, String)> = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_offset = i as f64 * 30.0;

        let lang = if config.input_language == "auto" { None } else { Some(config.input_language.as_str()) };
        match model.transcribe_with_timestamps(chunk, lang) {
            Ok(text) => {
                let segments = srt::parse_timestamped_text(&text, chunk_offset);
                all_segments.extend(segments);
            }
            Err(e) => {
                eprintln!("Warning: chunk {} failed: {}", i + 1, e);
            }
        }

        pb.inc(1);
    }

    pb.finish_with_message("done");

    // 4. Write SRT output
    let srt_content = srt::generate_srt(&all_segments);
    std::fs::write(output, &srt_content)
        .with_context(|| format!("Failed to write SRT file: {}", output.display()))?;

    println!(
        "\nWrote {} segments to {}",
        all_segments.len(),
        output.display()
    );

    Ok(())
}

/// Resolve the model path (same logic as main app).
fn resolve_model_load_path(config: &Config, backend: &LoadedBackend) -> std::path::PathBuf {
    if let Some(model) = backend
        .manifest
        .models
        .iter()
        .find(|m| m.id == config.model_name)
    {
        if config.model_path.is_dir() && model.files.len() == 1 {
            let candidate = config.model_path.join(&model.files[0]);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    config.model_path.clone()
}
