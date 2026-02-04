//! Shared FFI types for app backend plugins
//!
//! This crate defines the C-compatible interface that all speech-to-text
//! backend DLLs must implement.

use std::ffi::c_char;

/// API version for compatibility checking
pub const API_VERSION: u32 = 1;

/// Result codes for backend operations
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttResult {
    Ok = 0,
    InvalidParam = 1,
    ModelNotLoaded = 2,
    TranscriptionFailed = 3,
    OutOfMemory = 4,
    UnsupportedDevice = 5,
    UnknownError = 99,
}

/// Configuration for creating a model
#[repr(C)]
pub struct ModelConfig {
    /// Path to the model directory (null-terminated UTF-8)
    pub model_path: *const c_char,
    /// Whether to use GPU acceleration
    pub use_gpu: bool,
    /// Language code (e.g., "en") or null for auto-detect
    pub language: *const c_char,
}

/// Options for transcription
#[repr(C)]
pub struct TranscribeOptions {
    /// Language code (e.g., "en") or null for auto-detect
    pub language: *const c_char,
    /// Whether to include timestamps
    pub timestamps: bool,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            language: std::ptr::null(),
            timestamps: false,
        }
    }
}

/// Result of a transcription operation
#[repr(C)]
pub struct TranscribeResult {
    /// Result code
    pub code: SttResult,
    /// Transcribed text (null-terminated UTF-8, owned by backend)
    pub text: *const c_char,
    /// Length of text in bytes (excluding null terminator)
    pub text_len: usize,
    /// Device used for transcription ("CPU", "CUDA", etc.)
    pub device_used: *const c_char,
}

/// Information about a backend
#[repr(C)]
pub struct BackendInfo {
    /// API version this backend implements
    pub api_version: u32,
    /// Backend identifier (e.g., "whisper-ct2")
    pub id: *const c_char,
    /// Human-readable name (e.g., "Whisper (CTranslate2)")
    pub display_name: *const c_char,
    /// Backend version string
    pub version: *const c_char,
    /// Whether this backend supports CUDA
    pub supports_cuda: bool,
}

/// Opaque handle to a loaded model
#[repr(C)]
pub struct ModelHandle {
    _opaque: [u8; 0],
}

// Function pointer types for backend exports

/// Get information about this backend
pub type GetBackendInfoFn = unsafe extern "C" fn() -> BackendInfo;

/// Create a new model instance
/// Returns null on failure (call get_last_error for details)
pub type CreateModelFn = unsafe extern "C" fn(config: *const ModelConfig) -> *mut ModelHandle;

/// Destroy a model instance
pub type DestroyModelFn = unsafe extern "C" fn(handle: *mut ModelHandle);

/// Transcribe audio samples
/// Audio must be f32 samples at 16kHz mono
pub type TranscribeFn = unsafe extern "C" fn(
    handle: *mut ModelHandle,
    audio: *const f32,
    audio_len: usize,
    options: *const TranscribeOptions,
) -> TranscribeResult;

/// Free a transcription result
pub type FreeResultFn = unsafe extern "C" fn(result: *mut TranscribeResult);

/// Get the last error message (null-terminated UTF-8)
/// Returns null if no error
pub type GetLastErrorFn = unsafe extern "C" fn() -> *const c_char;

/// VTable containing all backend function pointers
#[derive(Clone)]
pub struct BackendVTable {
    pub get_backend_info: GetBackendInfoFn,
    pub create_model: CreateModelFn,
    pub destroy_model: DestroyModelFn,
    pub transcribe: TranscribeFn,
    pub free_result: FreeResultFn,
    pub get_last_error: GetLastErrorFn,
}

// Helper functions for backends to create FFI strings

/// Helper to create a static C string from a Rust string literal
#[macro_export]
macro_rules! cstr {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const std::ffi::c_char
    };
}

/// Helper trait for setting thread-local error messages
pub trait SetLastError {
    fn set_last_error(msg: &str);
    fn clear_last_error();
}
