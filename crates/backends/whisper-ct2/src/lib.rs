//! CTranslate2-based Whisper backend for app
//!
//! This backend uses the ct2rs crate (CTranslate2 Rust bindings) for
//! fast Whisper inference. Supports models from Systran/faster-whisper.

use ct2rs::{Config, Device, Whisper, WhisperOptions};
use app_core::*;
use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

// Thread-local storage for error messages
thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = RefCell::new(None);
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

fn clear_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

/// Internal model state
struct WhisperModel {
    whisper: Whisper,
    device_name: CString,
}

// Static strings for backend info
const BACKEND_ID: &[u8] = b"whisper-ct2\0";
const BACKEND_NAME: &[u8] = b"Whisper (CTranslate2)\0";
const BACKEND_VERSION: &[u8] = b"0.1.0\0";

/// Get information about this backend
#[no_mangle]
pub extern "C" fn get_backend_info() -> BackendInfo {
    BackendInfo {
        api_version: API_VERSION,
        id: BACKEND_ID.as_ptr() as *const c_char,
        display_name: BACKEND_NAME.as_ptr() as *const c_char,
        version: BACKEND_VERSION.as_ptr() as *const c_char,
        #[cfg(feature = "cuda")]
        supports_cuda: true,
        #[cfg(not(feature = "cuda"))]
        supports_cuda: false,
    }
}

/// Create a new model instance
#[no_mangle]
pub extern "C" fn create_model(config: *const ModelConfig) -> *mut ModelHandle {
    clear_error();

    if config.is_null() {
        set_error("Config is null");
        return ptr::null_mut();
    }

    let config = unsafe { &*config };

    // Get model path
    let model_path = if config.model_path.is_null() {
        set_error("Model path is null");
        return ptr::null_mut();
    } else {
        match unsafe { CStr::from_ptr(config.model_path) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_error("Invalid UTF-8 in model path");
                return ptr::null_mut();
            }
        }
    };

    // Determine device and create model
    if config.use_gpu {
        #[cfg(feature = "cuda")]
        {
            // GPU requested - try CUDA only, no fallback
            match try_create_whisper(model_path, Device::CUDA) {
                Ok(whisper) => {
                    let model = Box::new(WhisperModel {
                        whisper,
                        device_name: CString::new("CUDA").unwrap(),
                    });
                    return Box::into_raw(model) as *mut ModelHandle;
                }
                Err(e) => {
                    set_error(&format!("CUDA initialization failed: {}. Check CUDA/cuDNN paths in config.", e));
                    return ptr::null_mut();
                }
            }
        }
        #[cfg(not(feature = "cuda"))]
        {
            set_error("GPU requested but CUDA support not compiled in this build");
            return ptr::null_mut();
        }
    }

    // CPU mode
    match try_create_whisper(model_path, Device::CPU) {
        Ok(whisper) => {
            let model = Box::new(WhisperModel {
                whisper,
                device_name: CString::new("CPU").unwrap(),
            });
            Box::into_raw(model) as *mut ModelHandle
        }
        Err(e) => {
            set_error(&format!("Failed to load model: {}", e));
            ptr::null_mut()
        }
    }
}

fn try_create_whisper(model_path: &str, device: Device) -> Result<Whisper, String> {
    let config = Config {
        device,
        ..Default::default()
    };
    Whisper::new(model_path, config).map_err(|e| format!("{:?}: {}", device, e))
}

/// Destroy a model instance
#[no_mangle]
pub extern "C" fn destroy_model(handle: *mut ModelHandle) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle as *mut WhisperModel));
        }
    }
}

/// Transcribe audio samples
#[no_mangle]
pub extern "C" fn transcribe(
    handle: *mut ModelHandle,
    audio: *const f32,
    audio_len: usize,
    options: *const TranscribeOptions,
) -> TranscribeResult {
    clear_error();

    if handle.is_null() {
        set_error("Model handle is null");
        return TranscribeResult {
            code: SttResult::ModelNotLoaded,
            text: ptr::null(),
            text_len: 0,
            device_used: ptr::null(),
        };
    }

    if audio.is_null() || audio_len == 0 {
        // Empty audio is OK, just return empty string
        let empty = CString::new("").unwrap();
        let text_ptr = empty.as_ptr();
        std::mem::forget(empty);

        let model = unsafe { &*(handle as *const WhisperModel) };
        return TranscribeResult {
            code: SttResult::Ok,
            text: text_ptr,
            text_len: 0,
            device_used: model.device_name.as_ptr(),
        };
    }

    let model = unsafe { &*(handle as *const WhisperModel) };
    let audio_slice = unsafe { std::slice::from_raw_parts(audio, audio_len) };

    // Get language from options
    let language = if !options.is_null() {
        let opts = unsafe { &*options };
        if !opts.language.is_null() {
            unsafe { CStr::from_ptr(opts.language) }.to_str().ok()
        } else {
            Some("en") // Default to English
        }
    } else {
        Some("en")
    };

    // Perform transcription
    match model.whisper.generate(
        audio_slice,
        language,
        false, // timestamps
        &WhisperOptions::default(),
    ) {
        Ok(results) => {
            let text = results.join(" ").trim().to_string();
            let text_len = text.len();
            let text_cstring = CString::new(text).unwrap();
            let text_ptr = text_cstring.as_ptr();
            std::mem::forget(text_cstring); // Caller must free via free_result

            TranscribeResult {
                code: SttResult::Ok,
                text: text_ptr,
                text_len,
                device_used: model.device_name.as_ptr(),
            }
        }
        Err(e) => {
            set_error(&format!("Transcription failed: {}", e));
            TranscribeResult {
                code: SttResult::TranscriptionFailed,
                text: ptr::null(),
                text_len: 0,
                device_used: model.device_name.as_ptr(),
            }
        }
    }
}

/// Free a transcription result
#[no_mangle]
pub extern "C" fn free_result(result: *mut TranscribeResult) {
    if !result.is_null() {
        let result = unsafe { &mut *result };
        if !result.text.is_null() {
            unsafe {
                drop(CString::from_raw(result.text as *mut c_char));
            }
            result.text = ptr::null();
        }
    }
}

/// Get the last error message
#[no_mangle]
pub extern "C" fn get_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        match e.borrow().as_ref() {
            Some(s) => s.as_ptr(),
            None => ptr::null(),
        }
    })
}
