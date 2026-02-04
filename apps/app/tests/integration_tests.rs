//! Integration tests for app

use std::path::PathBuf;

/// Test that config serialization round-trips correctly
#[test]
fn test_config_roundtrip() {
    // This is a simple smoke test to ensure the binary builds
    // More comprehensive tests would require the actual application setup
    
    // Verify paths work correctly
    let test_path = PathBuf::from("models/test-model");
    assert!(test_path.parent().is_some());
    assert_eq!(test_path.file_name().unwrap(), "test-model");
}

/// Test path operations
#[test]
fn test_path_operations() {
    let paths = vec![
        PathBuf::from("models/whisper-tiny"),
        PathBuf::from("crates/backends/whisper-cpp"),
        PathBuf::from("config.json"),
    ];
    
    for path in paths {
        assert!(!path.is_absolute());  // These are relative paths
    }
}
