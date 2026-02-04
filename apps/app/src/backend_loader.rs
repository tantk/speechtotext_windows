//! Dynamic backend loader for speech-to-text plugins
//!
//! Loads backend DLLs at runtime using libloading, allowing the main app
//! to work with different speech recognition backends.

use anyhow::{Context, Result};
use libloading::Library;
use serde::{Deserialize, Serialize};
use app_core::*;
#[allow(unused_imports)]
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

/// Information about a model from manifest.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestModel {
    pub id: String,
    pub display_name: String,
    pub folder_name: String,
    pub size_mb: u32,
    pub hf_repo: String,
    pub download_url: String,
    pub files: Vec<String>,
    pub is_english_only: bool,
    /// Optional SHA256 checksums for file verification
    /// Map of filename -> "sha256:hash" or just hash
    #[serde(default)]
    pub checksums: Option<std::collections::HashMap<String, String>>,
}

/// Backend capabilities from manifest.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestCapabilities {
    pub supports_cuda: bool,
    pub supports_multilingual: bool,
}

/// Backend manifest loaded from manifest.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendManifest {
    pub id: String,
    pub display_name: String,
    pub dll_name: String,
    pub version: String,
    pub models: Vec<ManifestModel>,
    pub capabilities: ManifestCapabilities,
}

impl BackendManifest {
    /// Load manifest from a JSON file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read manifest: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse manifest: {}", path.display()))
    }
}

/// A loaded backend DLL with its function table
pub struct LoadedBackend {
    _library: Library,
    #[allow(dead_code)]
    pub id: String,
    pub display_name: String,
    #[allow(dead_code)]
    pub manifest: BackendManifest,
    vtable: BackendVTable,
}

impl LoadedBackend {
    /// Load a backend from a directory containing manifest.json and the DLL
    pub fn load(backend_dir: &Path) -> Result<Self> {
        // Load manifest
        let manifest_path = backend_dir.join("manifest.json");
        let manifest = BackendManifest::load(&manifest_path)?;

        // Load DLL
        let dll_path = backend_dir.join(&manifest.dll_name);
        let library = unsafe {
            Library::new(&dll_path)
                .with_context(|| format!("Failed to load DLL: {}", dll_path.display()))?
        };

        // Load function pointers
        let vtable = unsafe {
            BackendVTable {
                get_backend_info: *library
                    .get::<GetBackendInfoFn>(b"get_backend_info\0")
                    .context("Missing get_backend_info export")?,
                create_model: *library
                    .get::<CreateModelFn>(b"create_model\0")
                    .context("Missing create_model export")?,
                destroy_model: *library
                    .get::<DestroyModelFn>(b"destroy_model\0")
                    .context("Missing destroy_model export")?,
                transcribe: *library
                    .get::<TranscribeFn>(b"transcribe\0")
                    .context("Missing transcribe export")?,
                free_result: *library
                    .get::<FreeResultFn>(b"free_result\0")
                    .context("Missing free_result export")?,
                get_last_error: *library
                    .get::<GetLastErrorFn>(b"get_last_error\0")
                    .context("Missing get_last_error export")?,
            }
        };

        // Verify API version
        let info = unsafe { (vtable.get_backend_info)() };
        if info.api_version != API_VERSION {
            anyhow::bail!(
                "Backend API version mismatch: expected {}, got {}",
                API_VERSION,
                info.api_version
            );
        }

        // Extract info strings
        let id = unsafe { CStr::from_ptr(info.id) }
            .to_str()
            .unwrap_or("unknown")
            .to_string();
        let display_name = unsafe { CStr::from_ptr(info.display_name) }
            .to_str()
            .unwrap_or("Unknown Backend")
            .to_string();

        Ok(Self {
            _library: library,
            id,
            display_name,
            manifest,
            vtable,
        })
    }

    /// Create a model instance from this backend
    pub fn create_model(&self, model_path: &Path, use_gpu: bool) -> Result<Model> {
        let model_path_cstring = CString::new(model_path.to_string_lossy().as_ref())
            .context("Invalid model path")?;

        let config = ModelConfig {
            model_path: model_path_cstring.as_ptr(),
            use_gpu,
            language: ptr::null(),
        };

        let handle = unsafe { (self.vtable.create_model)(&config) };

        if handle.is_null() {
            let error = self.get_last_error();
            anyhow::bail!("Failed to create model: {}", error.unwrap_or("Unknown error".to_string()));
        }

        Ok(Model {
            handle,
            vtable: self.vtable.clone(),
        })
    }

    /// Get the last error message from the backend
    pub fn get_last_error(&self) -> Option<String> {
        let ptr = unsafe { (self.vtable.get_last_error)() };
        if ptr.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_str()
                .ok()
                .map(|s| s.to_string())
        }
    }

    /// Check if this backend supports CUDA
    #[allow(dead_code)]
    pub fn supports_cuda(&self) -> bool {
        self.manifest.capabilities.supports_cuda
    }

    /// Check CUDA support as reported by the loaded DLL (compile-time feature)
    pub fn supports_cuda_runtime(&self) -> bool {
        let info = unsafe { (self.vtable.get_backend_info)() };
        info.supports_cuda
    }

    /// Get available models for this backend
    #[allow(dead_code)]
    pub fn models(&self) -> &[ManifestModel] {
        &self.manifest.models
    }
}

/// A loaded model instance
pub struct Model {
    handle: *mut ModelHandle,
    vtable: BackendVTable,
}

// Safety: Model is Send + Sync because:
// - The handle is only accessed through FFI functions
// - The backend guarantees thread-safe access
unsafe impl Send for Model {}
unsafe impl Sync for Model {}

impl Model {
    /// Transcribe audio samples
    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        if audio.is_empty() {
            return Ok(String::new());
        }

        let options = TranscribeOptions::default();
        let mut result = unsafe {
            (self.vtable.transcribe)(self.handle, audio.as_ptr(), audio.len(), &options)
        };

        if result.code != SttResult::Ok {
            let error = if !result.text.is_null() {
                unsafe { CStr::from_ptr(result.text) }
                    .to_str()
                    .unwrap_or("Unknown error")
                    .to_string()
            } else {
                "Transcription failed".to_string()
            };
            unsafe { (self.vtable.free_result)(&mut result) };
            anyhow::bail!("{}", error);
        }

        let text = if !result.text.is_null() {
            unsafe { CStr::from_ptr(result.text) }
                .to_str()
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        // Free the result
        unsafe { (self.vtable.free_result)(&mut result) };

        Ok(text)
    }

    /// Get the device being used (CPU/CUDA)
    #[allow(dead_code)]
    pub fn device_used(&self) -> Option<String> {
        // Note: This would require storing the device info from the last transcription
        // For now, return None and we can enhance this later
        None
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.vtable.destroy_model)(self.handle) };
        }
    }
}

/// Discover available backends in a directory
pub fn discover_backends(backends_dir: &Path) -> Vec<PathBuf> {
    let mut backends = Vec::new();

    if let Ok(entries) = std::fs::read_dir(backends_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("manifest.json").exists() {
                backends.push(path);
            }
        }
    }

    backends
}

/// Get the backends directory (next to exe)
pub fn get_backends_dir() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not get exe directory"))?;
    Ok(exe_dir.join("backends"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_manifest_serialization() {
        let manifest = BackendManifest {
            id: "test_backend".to_string(),
            display_name: "Test Backend".to_string(),
            dll_name: "test_backend.dll".to_string(),
            version: "1.0.0".to_string(),
            models: vec![
                ManifestModel {
                    id: "model1".to_string(),
                    display_name: "Model 1".to_string(),
                    folder_name: "model1".to_string(),
                    size_mb: 50,
                    hf_repo: "test/model1".to_string(),
                    download_url: "https://example.com/model1.bin".to_string(),
                    files: vec!["model1.bin".to_string()],
                    is_english_only: true,
                    checksums: None,
                }
            ],
            capabilities: ManifestCapabilities {
                supports_cuda: true,
                supports_multilingual: true,
            },
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        assert!(json.contains("test_backend"));
        assert!(json.contains("test_backend.dll"));
        assert!(json.contains("supports_cuda"));

        // Deserialize and verify
        let deserialized: BackendManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test_backend");
        assert_eq!(deserialized.models.len(), 1);
        assert!(deserialized.capabilities.supports_cuda);
    }

    #[test]
    fn test_manifest_deserialization_from_json() {
        let json = r#"{
            "id": "whisper-cpp",
            "display_name": "Whisper (whisper.cpp)",
            "dll_name": "whisper_cpp.dll",
            "version": "0.1.0",
            "models": [
                {
                    "id": "ggml-tiny",
                    "display_name": "Whisper Tiny",
                    "folder_name": "ggml-tiny",
                    "size_mb": 75,
                    "hf_repo": "ggerganov/whisper.cpp",
                    "download_url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
                    "files": ["ggml-tiny.bin"],
                    "is_english_only": false
                }
            ],
            "capabilities": {
                "supports_cuda": true,
                "supports_multilingual": true
            }
        }"#;

        let manifest: BackendManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "whisper-cpp");
        assert_eq!(manifest.models.len(), 1);
        
        let model = &manifest.models[0];
        assert_eq!(model.id, "ggml-tiny");
    }

    #[test]
    fn test_discover_backends() {
        let temp_dir = std::env::temp_dir().join("app_test_backends");
        
        // Create test directory structure
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        
        let backend1 = temp_dir.join("backend1");
        let backend2 = temp_dir.join("backend2");
        let no_manifest = temp_dir.join("no_manifest");
        
        std::fs::create_dir(&backend1).unwrap();
        std::fs::create_dir(&backend2).unwrap();
        std::fs::create_dir(&no_manifest).unwrap();
        
        // Create manifest files
        File::create(backend1.join("manifest.json")).unwrap();
        File::create(backend2.join("manifest.json")).unwrap();
        // no_manifest doesn't have a manifest.json
        
        let discovered = discover_backends(&temp_dir);
        
        // Should find 2 backends (those with manifest.json)
        assert_eq!(discovered.len(), 2);
        
        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_discover_backends_empty_dir() {
        let temp_dir = std::env::temp_dir().join("app_test_backends_empty");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        
        let discovered = discover_backends(&temp_dir);
        assert!(discovered.is_empty());
        
        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_manifest_load_from_file() {
        let temp_dir = std::env::temp_dir().join("app_test_manifest");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        
        let manifest_path = temp_dir.join("manifest.json");
        let manifest_json = r#"{
            "id": "test-backend",
            "display_name": "Test Backend",
            "dll_name": "test.dll",
            "version": "1.0.0",
            "models": [],
            "capabilities": {
                "supports_cuda": false,
                "supports_multilingual": false
            }
        }"#;
        
        let mut file = File::create(&manifest_path).unwrap();
        file.write_all(manifest_json.as_bytes()).unwrap();
        
        let manifest = BackendManifest::load(&manifest_path).unwrap();
        assert_eq!(manifest.id, "test-backend");
        assert!(manifest.models.is_empty());
        
        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_manifest_load_missing_file() {
        let temp_path = std::env::temp_dir().join("nonexistent_manifest.json");
        let result = BackendManifest::load(&temp_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_load_invalid_json() {
        let temp_dir = std::env::temp_dir().join("app_test_manifest_invalid");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        
        let manifest_path = temp_dir.join("manifest.json");
        let mut file = File::create(&manifest_path).unwrap();
        file.write_all(b"invalid json {{[").unwrap();
        
        let result = BackendManifest::load(&manifest_path);
        assert!(result.is_err());
        
        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ============================================
    // GPU Capability Tests
    // ============================================

    #[test]
    fn test_manifest_capabilities_cuda_support() {
        let manifest = BackendManifest {
            id: "test_backend".to_string(),
            display_name: "Test Backend".to_string(),
            dll_name: "test_backend.dll".to_string(),
            version: "1.0.0".to_string(),
            models: vec![],
            capabilities: ManifestCapabilities {
                supports_cuda: true,
                supports_multilingual: true,
            },
        };

        assert!(manifest.capabilities.supports_cuda);
        assert!(manifest.capabilities.supports_multilingual);
    }

    #[test]
    fn test_manifest_capabilities_no_cuda() {
        let manifest = BackendManifest {
            id: "test_backend_cpu".to_string(),
            display_name: "Test Backend CPU".to_string(),
            dll_name: "test_backend_cpu.dll".to_string(),
            version: "1.0.0".to_string(),
            models: vec![],
            capabilities: ManifestCapabilities {
                supports_cuda: false,
                supports_multilingual: true,
            },
        };

        assert!(!manifest.capabilities.supports_cuda);
        assert!(manifest.capabilities.supports_multilingual);
    }

    #[test]
    fn test_manifest_capabilities_serialization() {
        let capabilities = ManifestCapabilities {
            supports_cuda: true,
            supports_multilingual: false,
        };

        let manifest = BackendManifest {
            id: "test".to_string(),
            display_name: "Test".to_string(),
            dll_name: "test.dll".to_string(),
            version: "1.0.0".to_string(),
            models: vec![],
            capabilities,
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        
        // Verify capabilities are in JSON
        assert!(json.contains("supports_cuda"));
        assert!(json.contains("supports_multilingual"));
        assert!(json.contains("true")); // CUDA support
        assert!(json.contains("false")); // No multilingual

        // Deserialize and verify
        let loaded: BackendManifest = serde_json::from_str(&json).unwrap();
        assert!(loaded.capabilities.supports_cuda);
        assert!(!loaded.capabilities.supports_multilingual);
    }

    #[test]
    fn test_gpu_backend_manifest_from_real_configs() {
        // Test whisper-cpp manifest (supports CUDA)
        let whisper_cpp_json = r#"{
            "id": "whisper_cpp",
            "display_name": "Whisper (whisper.cpp)",
            "dll_name": "whisper_cpp.dll",
            "version": "0.1.0",
            "models": [],
            "capabilities": {
                "supports_cuda": true,
                "supports_multilingual": true
            }
        }"#;

        let cpp_manifest: BackendManifest = serde_json::from_str(whisper_cpp_json).unwrap();
        assert!(cpp_manifest.capabilities.supports_cuda);
        assert!(cpp_manifest.capabilities.supports_multilingual);

        // Test whisper-ct2 manifest (supports CUDA)
        let whisper_ct2_json = r#"{
            "id": "whisper_ct2",
            "display_name": "Faster Whisper",
            "dll_name": "whisper_ct2.dll",
            "version": "0.1.0",
            "models": [],
            "capabilities": {
                "supports_cuda": true,
                "supports_multilingual": true
            }
        }"#;

        let ct2_manifest: BackendManifest = serde_json::from_str(whisper_ct2_json).unwrap();
        assert!(ct2_manifest.capabilities.supports_cuda);
        assert!(ct2_manifest.capabilities.supports_multilingual);
    }

    #[test]
    fn test_model_size_variants() {
        // Test that different model sizes are correctly parsed
        let sizes = vec![75, 150, 500, 1500, 3000];
        
        for (i, size) in sizes.iter().enumerate() {
            let model = ManifestModel {
                id: format!("model-{}", i),
                display_name: format!("Model {}", i),
                folder_name: format!("model-{}", i),
                size_mb: *size,
                hf_repo: "test/repo".to_string(),
                download_url: "https://example.com/model.bin".to_string(),
                files: vec!["model.bin".to_string()],
                is_english_only: false,
                checksums: None,
            };
            
            assert_eq!(model.size_mb, *size);
        }
    }

    #[test]
    fn test_english_only_models() {
        let english_model = ManifestModel {
            id: "tiny_en".to_string(),
            display_name: "Tiny (English)".to_string(),
            folder_name: "tiny.en".to_string(),
            size_mb: 75,
            hf_repo: "test/repo".to_string(),
            download_url: "https://example.com/model.bin".to_string(),
            files: vec!["model.bin".to_string()],
            is_english_only: true,
            checksums: None,
        };

        let multilingual_model = ManifestModel {
            id: "tiny".to_string(),
            display_name: "Tiny".to_string(),
            folder_name: "tiny".to_string(),
            size_mb: 75,
            hf_repo: "test/repo".to_string(),
            download_url: "https://example.com/model.bin".to_string(),
            files: vec!["model.bin".to_string()],
            is_english_only: false,
            checksums: None,
        };

        assert!(english_model.is_english_only);
        assert!(!multilingual_model.is_english_only);
    }

    #[test]
    fn test_backend_id_consistency() {
        // Test that backend IDs follow expected patterns
        let whisper_cpp = BackendManifest {
            id: "whisper-cpp".to_string(),
            display_name: "Whisper (whisper.cpp)".to_string(),
            dll_name: "whisper_cpp.dll".to_string(),
            version: "0.1.0".to_string(),
            models: vec![],
            capabilities: ManifestCapabilities {
                supports_cuda: true,
                supports_multilingual: true,
            },
        };

        // ID should be kebab-case (using hyphens)
        assert!(whisper_cpp.id.contains("-"));
        // DLL name should use underscores
        assert!(whisper_cpp.dll_name.contains("_"));
        // DLL should have .dll extension
        assert!(whisper_cpp.dll_name.ends_with(".dll"));
    }

    // ============================================
    // Backend DLL Loading Tests (Manual/Integration)
    // ============================================

    /// Test loading whisper-cpp backend DLL
    /// 
    /// Run with: cargo test test_whisper_cpp_backend_load -- --ignored
    /// Requires: 
    ///   - target/release/whisper_cpp.dll built
    ///   - crates/backends/whisper-cpp/manifest.json exists
    #[test]
    #[ignore = "Requires built DLL - run manually after building backends"]
    fn test_whisper_cpp_backend_load() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        
        let backend_dir = project_root.join("crates/backends/whisper-cpp");
        let dll_path = project_root.join("target/release/whisper_cpp.dll");
        
        // Verify DLL exists
        assert!(dll_path.exists(), "whisper_cpp.dll not found. Build with: cargo build --release -p whisper-cpp");
        
        // Copy DLL to backend directory temporarily
        let dest_dll = backend_dir.join("whisper_cpp.dll");
        std::fs::copy(&dll_path, &dest_dll).expect("Failed to copy DLL");
        
        // Load the backend
        let backend = LoadedBackend::load(&backend_dir);
        
        // Cleanup
        let _ = std::fs::remove_file(&dest_dll);
        
        // Verify backend loaded successfully
        let backend = backend.expect("Failed to load whisper-cpp backend");
        assert_eq!(backend.id, "whisper-cpp");
        assert_eq!(backend.display_name, "Whisper (whisper.cpp)");
        assert!(backend.supports_cuda(), "Backend should report CUDA support");
        
        println!("✓ whisper-cpp backend loaded successfully");
        println!("  ID: {}", backend.id);
        println!("  Name: {}", backend.display_name);
        println!("  Supports CUDA: {}", backend.supports_cuda());
    }

    /// Test loading whisper-ct2 backend DLL
    ///
    /// Run with: cargo test test_whisper_ct2_backend_load -- --ignored
    #[test]
    #[ignore = "Requires built DLL - run manually after building backends"]
    fn test_whisper_ct2_backend_load() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        
        let backend_dir = project_root.join("crates/backends/whisper-ct2");
        let dll_path = project_root.join("target/release/whisper_ct2.dll");
        
        assert!(dll_path.exists(), "whisper_ct2.dll not found. Build with: cargo build --release -p whisper-ct2");
        
        let dest_dll = backend_dir.join("whisper_ct2.dll");
        std::fs::copy(&dll_path, &dest_dll).expect("Failed to copy DLL");
        
        let backend = LoadedBackend::load(&backend_dir);
        let _ = std::fs::remove_file(&dest_dll);
        
        let backend = backend.expect("Failed to load whisper-ct2 backend");
        assert_eq!(backend.id, "whisper-ct2");
        assert!(backend.supports_cuda());
        
        println!("✓ whisper-ct2 backend loaded successfully");
    }

    /// Test creating a CPU model with whisper-cpp
    ///
    /// Run with: cargo test test_whisper_cpp_create_model_cpu -- --ignored
    /// Requires:
    ///   - Built whisper_cpp.dll
    ///   - target/release/models/ggml-tiny.bin model file
    #[test]
    #[ignore = "Requires DLL and model file - run manually"]
    fn test_whisper_cpp_create_model_cpu() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        
        let backend_dir = project_root.join("crates/backends/whisper-cpp");
        let model_path = project_root.join("target/release/models/ggml-tiny.bin");
        
        assert!(model_path.exists(), "Model file not found at target/release/models/ggml-tiny.bin");
        
        // Setup backend
        let dll_path = project_root.join("target/release/whisper_cpp.dll");
        let dest_dll = backend_dir.join("whisper_cpp.dll");
        std::fs::copy(&dll_path, &dest_dll).unwrap();
        
        let backend = LoadedBackend::load(&backend_dir).expect("Failed to load backend");
        
        // Create CPU model
        let model = backend.create_model(&model_path, false)
            .expect("Failed to create CPU model");
        
        println!("✓ CPU model created successfully");
        
        // Test transcription with silence
        let silence = vec![0.0f32; 16000]; // 1 second
        let result = model.transcribe(&silence);
        println!("  Transcription result: {:?}", result);
        
        // Cleanup
        let _ = std::fs::remove_file(&dest_dll);
    }

    /// Test creating a GPU model with whisper-cpp
    ///
    /// Run with: cargo test test_whisper_cpp_create_model_gpu -- --ignored
    /// Requires:
    ///   - Built whisper_cpp.dll with CUDA support
    ///   - CUDA installed and available
    ///   - target/release/models/ggml-tiny.bin model file
    #[test]
    #[ignore = "Requires CUDA and GPU-enabled DLL - run manually"]
    fn test_whisper_cpp_create_model_gpu() {
        // Verify CUDA is available
        let cuda_path = std::env::var("CUDA_PATH").expect("CUDA_PATH not set");
        println!("CUDA_PATH: {}", cuda_path);
        
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        
        let backend_dir = project_root.join("crates/backends/whisper-cpp");
        let model_path = project_root.join("target/release/models/ggml-tiny.bin");
        
        assert!(model_path.exists(), "Model file not found");
        
        // Setup backend with CUDA in PATH
        let dll_path = project_root.join("target/release/whisper_cpp.dll");
        let dest_dll = backend_dir.join("whisper_cpp.dll");
        std::fs::copy(&dll_path, &dest_dll).unwrap();
        
        // Add CUDA to PATH for this test
        let cuda_bin = PathBuf::from(&cuda_path).join("bin");
        let path = std::env::var("PATH").unwrap();
        std::env::set_var("PATH", format!("{};{}", cuda_bin.display(), path));
        
        let backend = LoadedBackend::load(&backend_dir).expect("Failed to load backend");
        
        // Create GPU model
        println!("Creating GPU model...");
        let model = backend.create_model(&model_path, true)
            .expect("Failed to create GPU model");
        
        println!("✓ GPU model created successfully");
        
        // Test transcription
        let silence = vec![0.0f32; 16000];
        let result = model.transcribe(&silence);
        println!("  Transcription result: {:?}", result);
        
        // Cleanup
        let _ = std::fs::remove_file(&dest_dll);
    }

    /// Compare CPU vs GPU transcription results
    ///
    /// Run with: cargo test test_cpu_gpu_transcription_compare -- --ignored
    /// Verifies that CPU and GPU produce consistent results
    #[test]
    #[ignore = "Requires CUDA - run manually"]
    fn test_cpu_gpu_transcription_compare() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        
        let backend_dir = project_root.join("crates/backends/whisper-cpp");
        let model_path = project_root.join("target/release/models/ggml-tiny.bin");
        
        assert!(model_path.exists(), "Model file not found");
        
        // Setup
        let dll_path = project_root.join("target/release/whisper_cpp.dll");
        let dest_dll = backend_dir.join("whisper_cpp.dll");
        std::fs::copy(&dll_path, &dest_dll).unwrap();
        
        // Setup CUDA PATH
        if let Ok(cuda_path) = std::env::var("CUDA_PATH") {
            let cuda_bin = PathBuf::from(&cuda_path).join("bin");
            let path = std::env::var("PATH").unwrap();
            std::env::set_var("PATH", format!("{};{}", cuda_bin.display(), path));
        }
        
        let backend = LoadedBackend::load(&backend_dir).unwrap();
        
        // Create test audio (sine wave)
        let sample_rate = 16000;
        let audio: Vec<f32> = (0..sample_rate)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5
            })
            .collect();
        
        // Test CPU
        println!("Testing CPU...");
        let cpu_model = backend.create_model(&model_path, false).unwrap();
        let cpu_result = cpu_model.transcribe(&audio);
        println!("  CPU result: {:?}", cpu_result);
        
        // Test GPU
        println!("Testing GPU...");
        let gpu_model = backend.create_model(&model_path, true).unwrap();
        let gpu_result = gpu_model.transcribe(&audio);
        println!("  GPU result: {:?}", gpu_result);
        
        // Both should succeed
        assert!(cpu_result.is_ok(), "CPU transcription failed");
        assert!(gpu_result.is_ok(), "GPU transcription failed");
        
        println!("✓ Both CPU and GPU transcription succeeded");
        
        // Cleanup
        let _ = std::fs::remove_file(&dest_dll);
    }
}
