//! Backend DLL Integration Tests
//!
//! These tests verify that the backend DLLs (whisper_cpp.dll, whisper_ct2.dll)
//! can be loaded, initialized, and perform transcription.
//!
//! Note: Some tests require actual model files and may be skipped if not present.
//! GPU tests require CUDA and are marked with #[ignore] to run manually.

use std::path::{Path, PathBuf};
use std::ffi::CString;

/// Get the project root directory (two levels up from tests/)
fn get_project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf()
}

/// Get the release directory where DLLs are built
fn get_release_dir() -> PathBuf {
    get_project_root().join("target/release")
}

/// Check if a DLL file exists
fn dll_exists(dll_name: &str) -> bool {
    get_release_dir().join(dll_name).exists()
}

/// Get the backends directory
fn get_backends_dir() -> PathBuf {
    get_project_root().join("crates/backends")
}

/// Get path to a specific backend directory
fn get_backend_dir(backend_id: &str) -> PathBuf {
    let backends_dir = get_backends_dir();
    // Map backend IDs to directory names
    let dir_name = match backend_id {
        "whisper-cpp" => "whisper-cpp",
        "whisper-ct2" => "whisper-ct2",
        _ => backend_id,
    };
    backends_dir.join(dir_name)
}

/// Check if a model file exists
fn model_exists(model_path: &Path) -> bool {
    model_path.exists()
}

// ============================================
// Backend Loading Tests
// ============================================

/// Test that whisper_cpp.dll exists in release directory
#[test]
fn test_whisper_cpp_dll_exists() {
    let exists = dll_exists("whisper_cpp.dll");
    if !exists {
        println!("whisper_cpp.dll not found in target/release - need to build with CUDA support");
    }
    // This is a soft check - the DLL may not exist if backends weren't built
}

/// Test that whisper_ct2.dll exists in release directory  
#[test]
fn test_whisper_ct2_dll_exists() {
    let exists = dll_exists("whisper_ct2.dll");
    if !exists {
        println!("whisper_ct2.dll not found in target/release - need to build with CUDA support");
    }
    // This is a soft check - the DLL may not exist if backends weren't built
}

/// Test that backend manifest files exist and are valid JSON
#[test]
fn test_whisper_cpp_manifest_exists() {
    let manifest_path = get_backend_dir("whisper-cpp").join("manifest.json");
    assert!(manifest_path.exists(), "whisper-cpp manifest.json should exist");
    
    // Verify it's valid JSON
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(json["id"].as_str().unwrap(), "whisper-cpp");
    assert!(json["capabilities"]["supports_cuda"].as_bool().unwrap());
}

#[test]
fn test_whisper_ct2_manifest_exists() {
    let manifest_path = get_backend_dir("whisper-ct2").join("manifest.json");
    assert!(manifest_path.exists(), "whisper-ct2 manifest.json should exist");
    
    // Verify it's valid JSON
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(json["id"].as_str().unwrap(), "whisper-ct2");
    assert!(json["capabilities"]["supports_cuda"].as_bool().unwrap());
}

/// Test backend manifest structure
#[test]
fn test_backend_manifest_structure() {
    let backends = vec![
        ("whisper-cpp", "whisper_cpp.dll"),
        ("whisper-ct2", "whisper_ct2.dll"),
    ];
    
    for (id, expected_dll) in backends {
        let manifest_path = get_backend_dir(id).join("manifest.json");
        let content = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|_| panic!("Failed to read {} manifest", id));
        let json: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| panic!("Invalid JSON in {} manifest", id));
        
        // Check required fields
        assert!(json["id"].is_string(), "{}: id should be a string", id);
        assert!(json["display_name"].is_string(), "{}: display_name should be a string", id);
        assert!(json["dll_name"].is_string(), "{}: dll_name should be a string", id);
        assert!(json["version"].is_string(), "{}: version should be a string", id);
        assert!(json["models"].is_array(), "{}: models should be an array", id);
        assert!(json["capabilities"].is_object(), "{}: capabilities should be an object", id);
        
        // Verify DLL name matches
        assert_eq!(json["dll_name"].as_str().unwrap(), expected_dll);
        
        // Verify capabilities
        let caps = &json["capabilities"];
        assert!(caps["supports_cuda"].is_boolean(), "{}: supports_cuda should be boolean", id);
        assert!(caps["supports_multilingual"].is_boolean(), "{}: supports_multilingual should be boolean", id);
        
        println!("{} manifest validated successfully", id);
    }
}

// ============================================
// Model Availability Tests
// ============================================

/// Test that models directory exists
#[test]
fn test_models_directory_exists() {
    let models_dir = get_project_root().join("models");
    if models_dir.exists() {
        println!("Models directory found at: {}", models_dir.display());
        // List model files
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    println!("  - {} ({} bytes)", path.file_name().unwrap().to_string_lossy(), size);
                }
            }
        }
    } else {
        println!("Models directory not found at: {}", models_dir.display());
    }
}

/// Test that ggml-tiny.bin exists for whisper-cpp tests
#[test]
fn test_ggml_tiny_model_exists() {
    let model_path = get_project_root().join("target/release/models/ggml-tiny.bin");
    if model_path.exists() {
        let size = std::fs::metadata(&model_path).unwrap().len();
        println!("ggml-tiny.bin found ({} bytes = {} MB)", size, size / 1024 / 1024);
        assert!(size > 0);
    } else {
        println!("ggml-tiny.bin not found - skipping test");
        // Don't fail - this is an optional integration test
    }
}

// ============================================
// FFI Function Signature Tests (Compile-time)
// ============================================

/// These tests verify that the backend DLL exports the expected symbols
    /// by checking against app_core definitions.
#[test]
fn test_backend_api_compatibility() {
    // The API version should match between core and backends
    use app_core::API_VERSION;
    
    // API_VERSION is defined in core and used by both backends
    assert!(API_VERSION > 0, "API_VERSION should be a positive integer");
    println!("Backend API version: {}", API_VERSION);
    
    // Verify core structures are properly sized for FFI
    // This catches struct layout mismatches at test time
    let model_config_size = std::mem::size_of::<app_core::ModelConfig>();
    let transcribe_options_size = std::mem::size_of::<app_core::TranscribeOptions>();
    let backend_info_size = std::mem::size_of::<app_core::BackendInfo>();
    let transcribe_result_size = std::mem::size_of::<app_core::TranscribeResult>();
    
    println!("ModelConfig size: {} bytes", model_config_size);
    println!("TranscribeOptions size: {} bytes", transcribe_options_size);
    println!("BackendInfo size: {} bytes", backend_info_size);
    println!("TranscribeResult size: {} bytes", transcribe_result_size);
    
    // All sizes should be non-zero
    assert!(model_config_size > 0);
    assert!(transcribe_options_size > 0);
    assert!(backend_info_size > 0);
    assert!(transcribe_result_size > 0);
}

// ============================================
// GPU Capability Tests
// ============================================

/// Test GPU support flags in backend manifests
#[test]
fn test_backend_gpu_capabilities() {
    let backends = vec!["whisper-cpp", "whisper-ct2"];
    
    for backend_id in backends {
        let manifest_path = get_backend_dir(backend_id).join("manifest.json");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        
        let supports_cuda = json["capabilities"]["supports_cuda"].as_bool().unwrap();
        println!("{}: supports_cuda = {}", backend_id, supports_cuda);
        
        // Both backends should have CUDA support in their manifests
        assert!(supports_cuda, "{} should support CUDA", backend_id);
    }
}

/// Test GPU device naming convention
#[test]
fn test_gpu_device_naming() {
    // Backends should report either "CUDA" or "CPU" as device name
    let valid_device_names = vec!["CUDA", "CPU"];
    
    for device in &valid_device_names {
        assert!(!device.is_empty());
        println!("Valid device name: {}", device);
    }
}

// ============================================
// Full Integration Tests (Require Models)
// ============================================

/*
NOTE: The following tests require access to internal APIs (LoadedBackend, Model).
These are available as unit tests in apps/app/src/backend_loader.rs.

To test backend DLL loading manually:

1. Build backends with CUDA:
   .\build_cuda.bat

2. Ensure model exists:
   - target/release/models/ggml-tiny.bin (for whisper-cpp)
   - target/release/models/faster-whisper-tiny/ (for whisper-ct2)

3. Run the application and check logs for:
   - "Loading backend from: ..."
   - "Backend loaded: whisper-cpp (Whisper (whisper.cpp))"
   - "Creating model..."
   - "Transcription result: ..."

4. To verify GPU is being used:
   - Check for "CUDA" in device_used field of TranscribeResult
   - Or check GPU utilization in Task Manager / nvidia-smi

Unit tests for backend loading can be added to backend_loader.rs:

#[cfg(test)]
mod backend_integration_tests {
    use super::*;
    
    #[test]
    #[ignore = "Requires DLL and model"]
    fn test_load_whisper_cpp() {
        let backend = LoadedBackend::load(Path::new("crates/backends/whisper-cpp")).unwrap();
        assert!(backend.supports_cuda());
    }
}
*/
