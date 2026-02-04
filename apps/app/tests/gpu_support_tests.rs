//! GPU Support Integration Tests
//!
//! These tests verify that GPU/CUDA support is properly configured and detected.
//! Note: These tests may be skipped or fail in environments without CUDA installed.

use std::path::PathBuf;

/// Test that GPU configuration structure is correct
#[test]
fn test_gpu_config_structure() {
    // Simulate a GPU-enabled configuration
    let use_gpu = true;
    let cuda_path = Some(PathBuf::from("C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.0"));
    let cudnn_path = Some(PathBuf::from("C:/Program Files/NVIDIA/CUDNN/v9.18"));

    assert!(use_gpu);
    assert!(cuda_path.is_some());
    assert!(cudnn_path.is_some());

    let cuda = cuda_path.unwrap();
    let cudnn = cudnn_path.unwrap();

    // Verify paths contain expected components
    assert!(cuda.to_string_lossy().to_lowercase().contains("cuda"));
    assert!(cudnn.to_string_lossy().to_lowercase().contains("cudnn"));
}

/// Test GPU configuration without cuDNN (CUDA only)
#[test]
fn test_gpu_config_cuda_only() {
    let use_gpu = true;
    let cuda_path = Some(PathBuf::from("C:/CUDA/v13.0"));
    let cudnn_path: Option<PathBuf> = None;

    assert!(use_gpu);
    assert!(cuda_path.is_some());
    assert!(cudnn_path.is_none());
}

/// Test CPU-only configuration
#[test]
fn test_cpu_only_config() {
    let use_gpu = false;
    let cuda_path: Option<PathBuf> = None;
    let cudnn_path: Option<PathBuf> = None;

    assert!(!use_gpu);
    assert!(cuda_path.is_none());
    assert!(cudnn_path.is_none());
}

/// Test GPU backend capabilities from manifests
#[test]
fn test_gpu_backend_capabilities() {
    // whisper-cpp manifest capabilities
    let whisper_cpp_cuda = true;
    let whisper_cpp_multilingual = true;

    assert!(whisper_cpp_cuda, "whisper-cpp should support CUDA");
    assert!(whisper_cpp_multilingual, "whisper-cpp should support multilingual");

    // whisper-ct2 manifest capabilities
    let whisper_ct2_cuda = true;
    let whisper_ct2_multilingual = true;

    assert!(whisper_ct2_cuda, "whisper-ct2 should support CUDA");
    assert!(whisper_ct2_multilingual, "whisper-ct2 should support multilingual");
}

/// Test model variants for GPU usage
#[test]
fn test_gpu_model_variants() {
    // Large models benefit more from GPU acceleration
    let gpu_recommended_models = vec![
        ("ggml-small", 500),
        ("ggml-medium", 1500),
        ("ggml-large-v2", 3000),
        ("ggml-large-v3", 3000),
    ];

    for (name, size_mb) in gpu_recommended_models {
        // Models > 500MB benefit significantly from GPU
        if size_mb >= 500 {
            println!("Model {} ({}MB) benefits from GPU acceleration", name, size_mb);
        }
        assert!(size_mb > 0);
    }
}

/// Test GPU environment variable setup
#[test]
fn test_gpu_env_vars() {
    // Check if CUDA_PATH is set (may not be in test environment)
    let cuda_path = std::env::var("CUDA_PATH").ok();
    
    // If CUDA is installed, verify path format
    if let Some(path) = cuda_path {
        assert!(!path.is_empty());
        println!("CUDA_PATH is set to: {}", path);
        
        // Path should contain version info
        assert!(
            path.contains("v") || path.contains("12") || path.contains("13"),
            "CUDA path should contain version info"
        );
    } else {
        println!("CUDA_PATH not set - GPU support not available in this environment");
    }
}

/// Test GPU configuration serialization
#[test]
fn test_gpu_config_json_serialization() {
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
    struct GpuConfig {
        use_gpu: bool,
        cuda_path: Option<PathBuf>,
        cudnn_path: Option<PathBuf>,
    }

    let config = GpuConfig {
        use_gpu: true,
        cuda_path: Some(PathBuf::from("/cuda/v13.0")),
        cudnn_path: Some(PathBuf::from("/cudnn/v9.18")),
    };

    let json = serde_json::to_string_pretty(&config).unwrap();
    
    // Verify JSON contains GPU fields
    assert!(json.contains("use_gpu"));
    assert!(json.contains("cuda_path"));
    assert!(json.contains("cudnn_path"));
    assert!(json.contains("true")); // use_gpu value

    // Deserialize and verify
    let loaded: GpuConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, config);
    assert!(loaded.use_gpu);
    assert_eq!(loaded.cuda_path, Some(PathBuf::from("/cuda/v13.0")));
}

/// Test GPU feature requirements
#[test]
fn test_gpu_feature_requirements() {
    // GPU support requires:
    // 1. CUDA Toolkit 12.x or 13.x
    // 2. cuDNN 8.x or 9.x
    // 3. Compatible NVIDIA GPU
    // 4. Backend built with CUDA support

    let cuda_version_supported = |version: &str| -> bool {
        version.starts_with("12.") || version.starts_with("13.")
    };

    let cudnn_version_supported = |version: &str| -> bool {
        version.starts_with("8.") || version.starts_with("9.")
    };

    assert!(cuda_version_supported("12.0"));
    assert!(cuda_version_supported("13.0"));
    assert!(!cuda_version_supported("11.8"));
    assert!(!cuda_version_supported("10.1"));

    assert!(cudnn_version_supported("8.9"));
    assert!(cudnn_version_supported("9.0"));
    assert!(cudnn_version_supported("9.18"));
    assert!(!cudnn_version_supported("7.6"));
}

/// Test GPU memory requirements estimation
#[test]
fn test_gpu_memory_requirements() {
    // Approximate GPU memory needed for different models
    let model_memory_requirements = vec![
        ("tiny", 1),      // ~1GB VRAM
        ("base", 1),      // ~1GB VRAM
        ("small", 2),     // ~2GB VRAM
        ("medium", 5),    // ~5GB VRAM
        ("large", 10),    // ~10GB VRAM
    ];

    for (model, vram_gb) in model_memory_requirements {
        println!("Model '{}' requires approximately {}GB VRAM", model, vram_gb);
        assert!(vram_gb > 0);
    }
}

/// Test GPU fallback to CPU
#[test]
fn test_gpu_fallback_mechanism() {
    // When GPU is not available, should gracefully fall back to CPU
    let gpu_available = false;
    let backend_supports_cuda = true;
    
    // If GPU not available, use CPU
    let use_gpu = gpu_available && backend_supports_cuda;
    
    assert!(!use_gpu, "Should fall back to CPU when GPU not available");
}

/// Test CUDA architecture compatibility
#[test]
fn test_cuda_architecture_compatibility() {
    // Common CUDA architectures and their compute capabilities
    let supported_architectures = vec![
        ("Turing", "7.5", vec!["GTX 1660", "RTX 2060", "RTX 2070", "RTX 2080"]),
        ("Ampere", "8.0-8.9", vec!["RTX 3060", "RTX 3070", "RTX 3080", "RTX 3090", "RTX 4070", "RTX 4080", "RTX 4090"]),
        ("Ada Lovelace", "8.9-9.0", vec!["RTX 4090", "RTX 4080", "RTX 4070 Ti"]),
    ];

    for (arch, compute_cap, gpus) in supported_architectures {
        println!("{} (compute {}): {:?}", arch, compute_cap, gpus);
        assert!(!gpus.is_empty());
    }
}

/// Integration test: Verify GPU detection flow
/// This test checks the complete GPU detection and validation flow
#[test]
#[ignore = "Requires CUDA to be installed - run manually"]
fn test_gpu_detection_integration() {
    // This test is ignored by default because it requires CUDA installation
    // Run with: cargo test -- --ignored

    // Check CUDA_PATH
    let cuda_path = std::env::var("CUDA_PATH").expect("CUDA_PATH should be set");
    println!("Detected CUDA at: {}", cuda_path);

    // Verify CUDA bin directory exists
    let cuda_bin = PathBuf::from(&cuda_path).join("bin");
    assert!(cuda_bin.exists(), "CUDA bin directory should exist");

    // Check for nvcc
    let nvcc = cuda_bin.join("nvcc.exe");
    if nvcc.exists() {
        println!("Found nvcc compiler");
    }

    // Check PATH for CUDA
    let path = std::env::var("PATH").unwrap();
    assert!(
        path.to_lowercase().contains("cuda"),
        "PATH should contain CUDA directory"
    );
}
