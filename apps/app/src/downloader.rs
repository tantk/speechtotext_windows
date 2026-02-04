use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::info;

use crate::backend_loader::ManifestModel;

/// Download progress tracking
pub struct DownloadProgress {
    pub downloaded: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub current_file: Arc<AtomicUsize>,
    pub total_files: usize,
    pub finished: Arc<AtomicBool>,
    pub error: Arc<parking_lot::Mutex<Option<String>>>,
}

impl DownloadProgress {
    pub fn new(total_files: usize) -> Self {
        Self {
            downloaded: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
            current_file: Arc::new(AtomicUsize::new(0)),
            total_files,
            finished: Arc::new(AtomicBool::new(false)),
            error: Arc::new(parking_lot::Mutex::new(None)),
        }
    }

    pub fn get_progress(&self) -> (u64, u64) {
        (
            self.downloaded.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }

    pub fn get_file_progress(&self) -> (usize, usize) {
        (
            self.current_file.load(Ordering::Relaxed),
            self.total_files,
        )
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn get_error(&self) -> Option<String> {
        self.error.lock().clone()
    }
}

/// Download a single file with progress tracking
fn download_file(url: &str, dest: &Path, progress: &DownloadProgress) -> Result<()> {
    // Create parent directory if needed
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).context("Failed to create model directory")?;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3600)) // 1 hour timeout for large models
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(url)
        .send()
        .context("Failed to connect to download server")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Download failed with status: {}",
            response.status()
        ));
    }

    let content_length = response.content_length().unwrap_or(0);
    progress.total.fetch_add(content_length, Ordering::Relaxed);

    let mut file = File::create(dest).context("Failed to create file")?;

    // Stream download to disk to avoid loading large files into memory
    let mut buffer = [0u8; 64 * 1024];
    let mut reader = response;
    loop {
        let read = reader.read(&mut buffer).context("Failed to read response")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).context("Failed to write to file")?;
        progress.downloaded.fetch_add(read as u64, Ordering::Relaxed);
    }

    file.flush().context("Failed to flush file")?;

    Ok(())
}

/// Get file download URL based on backend type
fn get_preprocessor_repo(model: &ManifestModel) -> Option<String> {
    let folder = model.folder_name.to_lowercase();
    let is_english = folder.ends_with(".en") || model.id.ends_with("-en");

    let base = if folder.contains("large-v3") {
        "openai/whisper-large-v3"
    } else if folder.contains("large-v2") {
        "openai/whisper-large-v2"
    } else if folder.contains("large") {
        "openai/whisper-large"
    } else if folder.contains("medium") {
        "openai/whisper-medium"
    } else if folder.contains("small") {
        "openai/whisper-small"
    } else if folder.contains("base") {
        "openai/whisper-base"
    } else if folder.contains("tiny") {
        "openai/whisper-tiny"
    } else {
        return None;
    };

    if is_english {
        Some(format!("{base}.en"))
    } else {
        Some(base.to_string())
    }
}

fn get_file_url(backend_id: &str, model: &ManifestModel, filename: &str) -> String {
    match backend_id {
        // CTranslate2 models use the standard HuggingFace resolve URL
        "whisper-ct2" => {
            if filename == "preprocessor_config.json" {
                if let Some(repo) = get_preprocessor_repo(model) {
                    return format!(
                        "https://huggingface.co/{}/resolve/main/{}",
                        repo, filename
                    );
                }
            }
            format!(
                "https://huggingface.co/{}/resolve/main/{}",
                model.hf_repo, filename
            )
        }
        // whisper.cpp models - the download_url in manifest points directly to the file
        "whisper-cpp" => {
            // For whisper.cpp, the download_url is the direct file URL
            // The filename in files list is the actual filename to save as
            model.download_url.clone()
        }
        // Default: assume HuggingFace-style URLs
        _ => {
            format!(
                "https://huggingface.co/{}/resolve/main/{}",
                model.hf_repo, filename
            )
        }
    }
}

/// Validate filename to prevent path traversal attacks
fn validate_filename(filename: &str) -> Result<()> {
    // Check for path separators
    if filename.contains('/') || filename.contains('\\') {
        return Err(anyhow::anyhow!(
            "Invalid filename '{}' contains path separators",
            filename
        ));
    }
    // Check for parent directory references
    if filename == ".." || filename.contains("../") || filename.contains("..\\") {
        return Err(anyhow::anyhow!(
            "Invalid filename '{}' contains parent directory references",
            filename
        ));
    }
    // Check for current directory reference (not dangerous but suspicious)
    if filename == "." {
        return Err(anyhow::anyhow!("Invalid filename '.'"));
    }
    // Ensure filename is not empty
    if filename.is_empty() {
        return Err(anyhow::anyhow!("Empty filename"));
    }
    Ok(())
}

/// Download all files for a model from manifest
pub fn download_manifest_model(
    backend_id: &str,
    model: &ManifestModel,
    dest_dir: &Path,
    progress: Arc<DownloadProgress>,
) -> Result<()> {
    // Create model directory
    fs::create_dir_all(dest_dir).context("Failed to create models directory")?;

    for (i, filename) in model.files.iter().enumerate() {
        // Validate filename for path traversal
        validate_filename(filename)?;

        progress.current_file.store(i + 1, Ordering::Relaxed);

        let url = get_file_url(backend_id, model, filename);
        let dest_path = dest_dir.join(filename);

        // Double-check the resolved path is within dest_dir
        // Check by comparing parent directories since file doesn't exist yet
        let dest_parent = dest_path.parent().ok_or_else(|| {
            anyhow::anyhow!("Invalid destination path: no parent directory")
        })?;
        
        // Canonicalize only the base directory (which exists)
        let canonical_base = dest_dir.canonicalize()
            .unwrap_or_else(|_| dest_dir.to_path_buf());
        let canonical_parent = dest_parent.canonicalize()
            .unwrap_or_else(|_| dest_parent.to_path_buf());
            
        if !canonical_parent.starts_with(&canonical_base) {
            return Err(anyhow::anyhow!(
                "Path traversal detected: '{}' resolves outside of destination directory",
                filename
            ));
        }

        download_file(&url, &dest_path, &progress)?;
    }

    progress.finished.store(true, Ordering::Relaxed);
    Ok(())
}

/// Start model download in a background thread (for manifest models)
pub fn start_manifest_model_download(
    backend_id: &str,
    model: &ManifestModel,
    dest_dir: PathBuf,
) -> Arc<DownloadProgress> {
    let progress = Arc::new(DownloadProgress::new(model.files.len()));
    let progress_clone = Arc::clone(&progress);

    let backend_id = backend_id.to_string();
    let model_clone = model.clone();

    std::thread::spawn(move || {
        if let Err(e) = download_manifest_model(
            &backend_id,
            &model_clone,
            &dest_dir,
            Arc::clone(&progress_clone),
        ) {
            *progress_clone.error.lock() = Some(e.to_string());
            progress_clone.finished.store(true, Ordering::Relaxed);
            return;
        }
        
        if model_clone.checksums.is_some() {
            info!("Checksum verification disabled; skipping.");
        }
    });

    progress
}
