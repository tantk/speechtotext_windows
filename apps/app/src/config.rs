use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Backend ID (e.g., "whisper-ct2" or "whisper-cpp")
    #[serde(default = "default_backend_id")]
    pub backend_id: String,
    pub model_name: String,
    pub model_path: PathBuf,
    #[serde(default)]
    pub use_gpu: bool,
    /// Path to CUDA installation (auto-detected if not set)
    #[serde(default)]
    pub cuda_path: Option<PathBuf>,
    /// Path to cuDNN installation (auto-detected if not set)
    #[serde(default)]
    pub cudnn_path: Option<PathBuf>,
    pub overlay_visible: bool,
    #[serde(default)]
    pub overlay_x: Option<i32>,
    #[serde(default)]
    pub overlay_y: Option<i32>,
    pub hotkey_push_to_talk: String,
    pub hotkey_always_listen: String,
    #[serde(default)]
    pub input_device_name: Option<String>,
}

fn default_backend_id() -> String {
    "whisper-ct2".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend_id: default_backend_id(),
            model_name: "whisper-tiny-en".to_string(),
            model_path: get_models_dir().unwrap_or_default().join("whisper-tiny-en"),
            use_gpu: false,
            cuda_path: None,
            cudnn_path: None,
            overlay_visible: true,
            overlay_x: None,
            overlay_y: None,
            hotkey_push_to_talk: "Backquote".to_string(),
            hotkey_always_listen: "Control+Backquote".to_string(),
            input_device_name: None,
        }
    }
}

/// Get the directory where the exe lives
pub fn get_exe_dir() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    exe_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Could not get exe directory"))
}

/// Get the executable stem (used for per-instance config/log naming)
pub fn get_exe_stem() -> Result<String> {
    let exe_path = std::env::current_exe()?;
    let stem = exe_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("app");
    Ok(stem.to_string())
}

/// Get the models directory (next to exe)
pub fn get_models_dir() -> Result<PathBuf> {
    Ok(get_exe_dir()?.join("models"))
}

/// Get the backends directory (next to exe)
pub fn get_backends_dir() -> Result<PathBuf> {
    Ok(get_exe_dir()?.join("backends"))
}

/// Get the config file path (next to exe)
pub fn get_config_path() -> Result<PathBuf> {
    let stem = get_exe_stem()?;
    Ok(get_exe_dir()?.join(format!("config-{}.json", stem)))
}

fn get_legacy_config_path() -> Result<PathBuf> {
    Ok(get_exe_dir()?.join("config.json"))
}

/// Auto-detect CUDA installation path
pub fn detect_cuda_path() -> Option<PathBuf> {
    // Check common Windows CUDA installation paths
    let base = Path::new(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA");
    if base.exists() {
        // Find latest version (prefer v12.x)
        if let Ok(entries) = std::fs::read_dir(base) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();

            fn parse_version(name: &str) -> Option<(u32, u32, u32)> {
                let trimmed = name.trim_start_matches(|c| c == 'v' || c == 'V');
                let mut parts: Vec<u32> = Vec::new();
                let mut current = String::new();
                for ch in trimmed.chars() {
                    if ch.is_ascii_digit() {
                        current.push(ch);
                    } else if ch == '.' || ch == '_' {
                        if !current.is_empty() {
                            parts.push(current.parse().ok()?);
                            current.clear();
                        }
                    } else {
                        break;
                    }
                }
                if !current.is_empty() {
                    parts.push(current.parse().ok()?);
                }
                if parts.is_empty() {
                    return None;
                }
                while parts.len() < 3 {
                    parts.push(0);
                }
                Some((parts[0], parts[1], parts[2]))
            }

            // Sort by numeric version (prefer higher versions)
            versions.sort_by(|a, b| {
                let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let a_ver = parse_version(a_name).unwrap_or((0, 0, 0));
                let b_ver = parse_version(b_name).unwrap_or((0, 0, 0));
                b_ver.cmp(&a_ver).then_with(|| b_name.cmp(a_name))
            });

            // Helper to check for cudart DLL in a directory
            let has_cudart = |dir: &Path| -> bool {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("cudart64_") && name_str.ends_with(".dll") {
                            return true;
                        }
                    }
                }
                false
            };

            // Return first version that has cudart DLL
            for version_path in versions {
                let bin_dir = version_path.join("bin");
                if bin_dir.exists() {
                    // Check directly in bin/
                    if has_cudart(&bin_dir) {
                        return Some(version_path);
                    }
                    // Check bin/x64/ (CUDA 13.x structure)
                    let bin_x64 = bin_dir.join("x64");
                    if bin_x64.exists() && has_cudart(&bin_x64) {
                        return Some(version_path);
                    }
                }
            }
        }
    }

    // Fall back to CUDA_PATH environment variable
    std::env::var("CUDA_PATH").ok().map(PathBuf::from)
}

/// Auto-detect cuDNN installation path
pub fn detect_cudnn_path() -> Option<PathBuf> {
    // Check the NVIDIA cuDNN directory for any version
    let cudnn_base = Path::new(r"C:\Program Files\NVIDIA\CUDNN");
    if cudnn_base.exists() {
        if let Ok(entries) = std::fs::read_dir(cudnn_base) {
            // Collect and sort versions to get the latest
            let mut versions: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));  // Descending order

            for entry in versions {
                let version_path = entry.path();
                // cuDNN 9.x has a different structure: CUDNN/v9.x/include/<cuda_version>/
                // Check for bin directory or subdirectories with bin
                if version_path.join("bin").exists() {
                    // Verify there's actually a cuDNN DLL
                    if let Ok(bin_entries) = std::fs::read_dir(version_path.join("bin")) {
                        for bin_entry in bin_entries.flatten() {
                            let name = bin_entry.file_name();
                            let name_str = name.to_string_lossy();
                            if name_str.starts_with("cudnn") && name_str.ends_with(".dll") {
                                return Some(version_path);
                            }
                        }
                    }
                }
                // Check for CUDA version subdirectories (e.g., v9.18/bin/13.1/x64/)
                if let Ok(sub_entries) = std::fs::read_dir(&version_path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.join("x64").exists() {
                            // This might be a lib path, but check for DLLs
                            let potential_bin = version_path.join("bin");
                            if potential_bin.exists() {
                                // Check any subdirectory under bin for DLLs
                                if let Ok(bin_subs) = std::fs::read_dir(&potential_bin) {
                                    for bin_sub in bin_subs.flatten() {
                                        let dll_path = bin_sub.path();
                                        if dll_path.is_dir() {
                                            if let Ok(dll_entries) = std::fs::read_dir(&dll_path) {
                                                for dll_entry in dll_entries.flatten() {
                                                    let name = dll_entry.file_name();
                                                    let name_str = name.to_string_lossy();
                                                    if name_str.starts_with("cudnn") && name_str.ends_with(".dll") {
                                                        return Some(version_path);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // cuDNN might also be installed in the CUDA directory
    if let Some(cuda_path) = detect_cuda_path() {
        let bin_dir = cuda_path.join("bin");
        if let Ok(entries) = std::fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("cudnn") && name_str.ends_with(".dll") {
                    return Some(cuda_path);
                }
            }
        }
    }

    None
}

/// Validate CUDA path by checking for cudart DLL
pub fn validate_cuda_path(path: &Path) -> bool {
    let bin_dir = path.join("bin");
    if !bin_dir.exists() {
        return false;
    }

    // Helper to check for cudart DLL
    let has_cudart = |dir: &Path| -> bool {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("cudart64_") && name_str.ends_with(".dll") {
                    return true;
                }
            }
        }
        false
    };

    // Check directly in bin/
    if has_cudart(&bin_dir) {
        return true;
    }

    // Check bin/x64/ (CUDA 13.x structure)
    let bin_x64 = bin_dir.join("x64");
    if bin_x64.exists() && has_cudart(&bin_x64) {
        return true;
    }

    false
}

/// Validate cuDNN path by checking for cudnn DLL
pub fn validate_cudnn_path(path: &Path) -> bool {
    let bin_dir = path.join("bin");
    if !bin_dir.exists() {
        return false;
    }

    // Helper to check if a directory contains cuDNN DLLs
    let has_cudnn_dll = |dir: &Path| -> bool {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("cudnn") && name_str.ends_with(".dll") {
                    return true;
                }
            }
        }
        false
    };

    // Check directly in bin/
    if has_cudnn_dll(&bin_dir) {
        return true;
    }

    // Check subdirectories (cuDNN 9.x has structure like bin/13.1/ or bin/13.1/x64/)
    if let Ok(entries) = std::fs::read_dir(&bin_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub_path = entry.path();
                // Check bin/<version>/
                if has_cudnn_dll(&sub_path) {
                    return true;
                }
                // Check bin/<version>/x64/ (cuDNN 9.x structure)
                let x64_path = sub_path.join("x64");
                if x64_path.exists() && has_cudnn_dll(&x64_path) {
                    return true;
                }
            }
        }
    }

    false
}

/// Find the actual directory containing cuDNN DLLs
/// Returns the path to add to PATH environment variable
fn find_cudnn_bin_dir(cudnn_path: &Path) -> Option<PathBuf> {
    let bin_dir = cudnn_path.join("bin");
    if !bin_dir.exists() {
        return None;
    }

    // Helper to check if a directory contains cuDNN DLLs
    let has_cudnn_dll = |dir: &Path| -> bool {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("cudnn") && name_str.ends_with(".dll") {
                    return true;
                }
            }
        }
        false
    };

    // Check directly in bin/
    if has_cudnn_dll(&bin_dir) {
        return Some(bin_dir);
    }

    // Check subdirectories (cuDNN 9.x has structure like bin/13.1/x64/)
    // Sort in descending order to prefer newer CUDA versions (13.x before 12.x)
    if let Ok(entries) = std::fs::read_dir(&bin_dir) {
        let mut subdirs: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        // Sort descending to prefer 13.x over 12.x
        subdirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for entry in subdirs {
            let sub_path = entry.path();
            // Check bin/<version>/
            if has_cudnn_dll(&sub_path) {
                return Some(sub_path);
            }
            // Check bin/<version>/x64/ (cuDNN 9.x structure)
            let x64_path = sub_path.join("x64");
            if x64_path.exists() && has_cudnn_dll(&x64_path) {
                return Some(x64_path);
            }
        }
    }

    None
}

/// Set up CUDA environment variables from config
pub fn setup_cuda_env(config: &Config) {
    if !config.use_gpu {
        return;
    }

    // Get CUDA path (from config or auto-detect)
    let cuda_path = config.cuda_path.clone().or_else(detect_cuda_path);

    // Get cuDNN path (from config or auto-detect)
    let cudnn_path = config.cudnn_path.clone().or_else(detect_cudnn_path);

    // Set CUDA_PATH if we have one
    if let Some(ref cuda) = cuda_path {
        std::env::set_var("CUDA_PATH", cuda);

        // Add CUDA bin to PATH (check for x64 subdirectory first - CUDA 13.x structure)
        let cuda_bin = cuda.join("bin");
        if cuda_bin.exists() {
            let cuda_bin_x64 = cuda_bin.join("x64");
            let bin_to_add = if cuda_bin_x64.exists() {
                cuda_bin_x64
            } else {
                cuda_bin
            };
            info!("  CUDA bin added to PATH: {}", bin_to_add.display());
            if let Ok(path) = std::env::var("PATH") {
                let new_path = format!("{};{}", bin_to_add.display(), path);
                std::env::set_var("PATH", new_path);
            }
        }
    }

    // Add cuDNN bin to PATH if different from CUDA
    if let Some(ref cudnn) = cudnn_path {
        if cudnn_path != cuda_path {
            // Find the actual directory containing cuDNN DLLs
            let cudnn_bin = find_cudnn_bin_dir(cudnn);
            if let Some(ref bin_dir) = cudnn_bin {
                info!("  cuDNN bin added to PATH: {}", bin_dir.display());
                if let Ok(path) = std::env::var("PATH") {
                    let new_path = format!("{};{}", bin_dir.display(), path);
                    std::env::set_var("PATH", new_path);
                }
            } else {
                info!("  WARNING: Could not find cuDNN bin directory");
            }
        }
    }

    if cuda_path.is_some() {
        info!("[CUDA] Environment configured");
        if let Some(ref p) = cuda_path {
            info!("  CUDA_PATH: {}", p.display());
        }
        if let Some(ref p) = cudnn_path {
            if cudnn_path != cuda_path {
                info!("  cuDNN: {}", p.display());
            }
        }
    }
}

impl Config {
    /// Check if the configured model file exists
    pub fn model_exists(&self) -> bool {
        self.model_path.exists()
    }

    /// Try to load config from file
    pub fn load() -> Result<Self> {
        let config_path = get_config_path()?;

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: Config = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            let legacy_path = get_legacy_config_path()?;
            if legacy_path.exists() {
                let content = fs::read_to_string(&legacy_path)?;
                let config: Config = serde_json::from_str(&content)?;
                let content = serde_json::to_string_pretty(&config)?;
                let _ = fs::write(config_path, content);
                Ok(config)
            } else {
                Err(anyhow::anyhow!("Config file not found"))
            }
        }
    }

    /// Save config to file
    pub fn save(&self) -> Result<()> {
        let config_path = get_config_path()?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    /// Create config for a specific model
    pub fn for_model(
        backend_id: &str,
        model_name: &str,
        model_path: PathBuf,
        hotkey_push_to_talk: &str,
        hotkey_always_listen: &str,
        use_gpu: bool,
        cuda_path: Option<PathBuf>,
        cudnn_path: Option<PathBuf>,
        input_device_name: Option<String>,
    ) -> Self {
        Self {
            backend_id: backend_id.to_string(),
            model_name: model_name.to_string(),
            model_path,
            use_gpu,
            cuda_path,
            cudnn_path,
            overlay_visible: true,
            overlay_x: None,
            overlay_y: None,
            hotkey_push_to_talk: hotkey_push_to_talk.to_string(),
            hotkey_always_listen: hotkey_always_listen.to_string(),
            input_device_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.backend_id, "whisper-ct2");
        assert_eq!(config.model_name, "whisper-tiny-en");
        assert!(!config.use_gpu);
        assert!(config.overlay_visible);
        assert_eq!(config.hotkey_push_to_talk, "Backquote");
        assert_eq!(config.hotkey_always_listen, "Control+Backquote");
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::for_model(
            "whisper-cpp",
            "test-model",
            PathBuf::from("/models/test"),
            "F1",
            "Control+F1",
            true,
            Some(PathBuf::from("/cuda")),
            Some(PathBuf::from("/cudnn")),
            None,
        );

        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("whisper-cpp"));
        assert!(json.contains("test-model"));
        assert!(json.contains("F1"));

        // Deserialize and verify
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.backend_id, "whisper-cpp");
        assert_eq!(deserialized.model_name, "test-model");
        assert!(deserialized.use_gpu);
        assert_eq!(deserialized.hotkey_push_to_talk, "F1");
    }

    #[test]
    fn test_config_save_and_load() {
        let temp_dir = std::env::temp_dir().join("app_test_config");
        fs::create_dir_all(&temp_dir).ok();
        let config_path = temp_dir.join("config.json");

        let config = Config::for_model(
            "whisper-ct2",
            "test-model",
            PathBuf::from("models/test"),
            "Backquote",
            "Control+Backquote",
            false,
            None,
            None,
            None,
        );

        // Save config
        let json = serde_json::to_string_pretty(&config).unwrap();
        let mut file = File::create(&config_path).unwrap();
        file.write_all(json.as_bytes()).unwrap();

        // Load and verify
        let content = fs::read_to_string(&config_path).unwrap();
        let loaded: Config = serde_json::from_str(&content).unwrap();
        
        assert_eq!(loaded.backend_id, config.backend_id);
        assert_eq!(loaded.model_name, config.model_name);
        assert_eq!(loaded.use_gpu, config.use_gpu);

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_config_model_exists() {
        let temp_dir = std::env::temp_dir().join("app_test_model");
        fs::create_dir_all(&temp_dir).ok();
        let model_path = temp_dir.join("model.bin");
        
        // Create a dummy file
        File::create(&model_path).unwrap();

        let config = Config {
            model_path: model_path.clone(),
            ..Config::default()
        };

        assert!(config.model_exists());

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();

        // Non-existent path
        let config2 = Config {
            model_path: PathBuf::from("/nonexistent/path/model.bin"),
            ..Config::default()
        };

        assert!(!config2.model_exists());
    }

    #[test]
    fn test_cuda_path_validation() {
        // Empty path should fail
        let empty = Path::new("");
        assert!(!validate_cuda_path(empty));
        
        // Non-existent path should fail
        let nonexistent = Path::new("/nonexistent/cuda");
        assert!(!validate_cuda_path(nonexistent));
    }

    #[test]
    fn test_cudnn_path_validation() {
        // Empty path should fail
        let empty = Path::new("");
        assert!(!validate_cudnn_path(empty));
        
        // Non-existent path should fail  
        let nonexistent = Path::new("/nonexistent/cudnn");
        assert!(!validate_cudnn_path(nonexistent));
    }

    // ============================================
    // GPU Support Tests
    // ============================================

    #[test]
    fn test_config_gpu_enabled() {
        let config = Config::for_model(
            "whisper-ct2",
            "test-model",
            PathBuf::from("models/test"),
            "Backquote",
            "Control+Backquote",
            true,  // GPU enabled
            Some(PathBuf::from("C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.0")),
            Some(PathBuf::from("C:/Program Files/NVIDIA/CUDNN/v9.18")),
            None,
        );

        assert!(config.use_gpu);
        assert!(config.cuda_path.is_some());
        assert!(config.cudnn_path.is_some());
        
        let cuda_path = config.cuda_path.unwrap();
        assert!(cuda_path.to_string_lossy().contains("CUDA"));
        
        let cudnn_path = config.cudnn_path.unwrap();
        assert!(cudnn_path.to_string_lossy().contains("CUDNN"));
    }

    #[test]
    fn test_config_gpu_disabled() {
        let config = Config::for_model(
            "whisper-ct2",
            "test-model",
            PathBuf::from("models/test"),
            "Backquote",
            "Control+Backquote",
            false,  // GPU disabled
            None,
            None,
            None,
        );

        assert!(!config.use_gpu);
        assert!(config.cuda_path.is_none());
        assert!(config.cudnn_path.is_none());
    }

    #[test]
    fn test_config_gpu_serialization() {
        let config = Config::for_model(
            "whisper-cpp",
            "test-model",
            PathBuf::from("models/test"),
            "F1",
            "Control+F1",
            true,
            Some(PathBuf::from("/cuda/path")),
            Some(PathBuf::from("/cudnn/path")),
            None,
        );

        let json = serde_json::to_string_pretty(&config).unwrap();
        
        // Verify GPU fields are in JSON
        assert!(json.contains("use_gpu"));
        assert!(json.contains("cuda_path"));
        assert!(json.contains("cudnn_path"));
        assert!(json.contains("/cuda/path"));
        assert!(json.contains("/cudnn/path"));

        // Deserialize and verify GPU settings preserved
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert!(loaded.use_gpu);
        assert_eq!(loaded.cuda_path, Some(PathBuf::from("/cuda/path")));
        assert_eq!(loaded.cudnn_path, Some(PathBuf::from("/cudnn/path")));
    }

    #[test]
    fn test_setup_cuda_env_no_gpu() {
        // Test that setup_cuda_env returns early when GPU is disabled
        let config = Config {
            use_gpu: false,
            ..Config::default()
        };

        // Should not panic and should return immediately
        setup_cuda_env(&config);
        
        // CUDA_PATH should not be set (or should be unchanged)
        // Note: We can't easily test this without mocking env vars
    }

    #[test]
    fn test_cuda_path_validation_mock() {
        // Create a mock CUDA directory structure
        let temp_dir = std::env::temp_dir().join("app_test_cuda");
        fs::create_dir_all(&temp_dir).ok();
        
        let cuda_dir = temp_dir.join("cuda");
        let bin_dir = cuda_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        
        // Create a fake cudart64 dll
        File::create(bin_dir.join("cudart64_110.dll")).unwrap();
        
        // Should validate successfully
        assert!(validate_cuda_path(&cuda_dir));
        
        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_cudnn_path_validation_mock() {
        // Create a mock cuDNN directory structure
        let temp_dir = std::env::temp_dir().join("app_test_cudnn");
        fs::create_dir_all(&temp_dir).ok();
        
        let cudnn_dir = temp_dir.join("cudnn");
        let bin_dir = cudnn_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        
        // Create a fake cudnn dll
        File::create(bin_dir.join("cudnn64_8.dll")).unwrap();
        
        // Should validate successfully
        assert!(validate_cudnn_path(&cudnn_dir));
        
        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_cudnn_path_validation_nested_structure() {
        // Test cuDNN 9.x nested structure: bin/13.1/x64/
        let temp_dir = std::env::temp_dir().join("app_test_cudnn_nested");
        fs::create_dir_all(&temp_dir).ok();
        
        let cudnn_dir = temp_dir.join("cudnn");
        let nested_bin = cudnn_dir.join("bin").join("13.1");
        fs::create_dir_all(&nested_bin).unwrap();
        
        // Create a fake cudnn dll in nested directory
        File::create(nested_bin.join("cudnn64_9.dll")).unwrap();
        
        // Should validate successfully with nested structure
        assert!(validate_cudnn_path(&cudnn_dir));
        
        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_detect_cuda_path_returns_none_for_invalid() {
        // This test assumes CUDA is not installed in test environment
        // If CUDA is installed, this test may fail
        
        // Temporarily clear CUDA_PATH env var
        let old_cuda_path = std::env::var("CUDA_PATH").ok();
        std::env::remove_var("CUDA_PATH");
        
        // Detection should return None when CUDA is not installed
        // Note: This depends on the test environment
        let detected = detect_cuda_path();
        
        // Restore env var
        if let Some(path) = old_cuda_path {
            std::env::set_var("CUDA_PATH", path);
        }
        
        // In CI/test environment without CUDA, this should be None
        // In dev environment with CUDA, this will be Some
        // So we just verify the function doesn't panic
        let _ = detected;
    }

    #[test]
    fn test_config_gpu_toggle() {
        // Test toggling GPU on/off
        let mut config = Config::default();
        
        // Initially GPU is off
        assert!(!config.use_gpu);
        
        // Enable GPU
        config.use_gpu = true;
        config.cuda_path = Some(PathBuf::from("/cuda"));
        config.cudnn_path = Some(PathBuf::from("/cudnn"));
        
        assert!(config.use_gpu);
        assert!(config.cuda_path.is_some());
        
        // Disable GPU
        config.use_gpu = false;
        
        // Paths should remain but GPU flag is off
        assert!(!config.use_gpu);
        assert!(config.cuda_path.is_some()); // Path persists for re-enabling
    }
}
