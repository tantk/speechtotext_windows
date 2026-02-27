//! mistral.rs-based Voxtral backend for speech-to-text
//!
//! This backend provides speech-to-text transcription using Mistral's Voxtral models.
//! Voxtral is a multimodal LLM with a built-in audio encoder for realtime speech recognition.

use app_core::*;
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use mistralrs::{
    AudioInput, AutoDeviceMapParams, DeviceMapSetting, IsqType, TextMessageRole,
    VisionModelBuilder, VisionMessages,
};

/// Global error storage for FFI
static LAST_ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn get_last_error_storage() -> &'static Mutex<Option<String>> {
    LAST_ERROR.get_or_init(|| Mutex::new(None))
}

fn set_last_error(msg: &str) {
    if let Ok(mut guard) = get_last_error_storage().lock() {
        *guard = Some(msg.to_string());
    }
}

fn clear_last_error() {
    if let Ok(mut guard) = get_last_error_storage().lock() {
        *guard = None;
    }
}

/// Internal model wrapper
struct VoxtralModel {
    model: mistralrs::Model,
    runtime: tokio::runtime::Runtime,
    device_used: String,
}

/// Opaque handle for FFI
pub struct ModelHandle {
    inner: VoxtralModel,
}

// Version string for FFI
const VERSION: &str = env!("CARGO_PKG_VERSION");
static VERSION_CSTRING: OnceLock<CString> = OnceLock::new();

fn get_version_ptr() -> *const i8 {
    let cstring = VERSION_CSTRING.get_or_init(|| CString::new(VERSION).unwrap());
    cstring.as_ptr()
}

// ============ FFI Exports ============

#[no_mangle]
pub extern "C" fn get_backend_info() -> BackendInfo {
    BackendInfo {
        api_version: API_VERSION,
        id: cstr!("mistralrs"),
        display_name: cstr!("Voxtral (mistral.rs)"),
        version: get_version_ptr(),
        supports_cuda: cfg!(feature = "cuda"),
    }
}

#[no_mangle]
pub extern "C" fn create_model(config: *const ModelConfig) -> *mut ModelHandle {
    clear_last_error();

    if config.is_null() {
        set_last_error("Null config pointer");
        return std::ptr::null_mut();
    }

    let config = unsafe { &*config };

    // Parse model path
    let model_path = unsafe {
        CStr::from_ptr(config.model_path)
            .to_str()
            .unwrap_or("")
    };

    // Determine if using GPU
    let use_gpu = config.use_gpu;

    // Validate model path exists
    let path = PathBuf::from(model_path);
    if !path.exists() {
        set_last_error(&format!("Model path does not exist: {}", model_path));
        return std::ptr::null_mut();
    }

    // Load Voxtral model using mistral.rs
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(&format!("Failed to create tokio runtime: {}", e));
            return std::ptr::null_mut();
        }
    };

    let result = runtime.block_on(async {
        // Determine quantization type from path or default to Q4K
        let path_str = path.to_string_lossy();
        let isq_type = if path_str.contains("Q8") {
            IsqType::Q8K
        } else if path_str.contains("Q5") {
            IsqType::Q5K
        } else {
            IsqType::Q4K  // Default for Voxtral
        };

        // Build model using VisionModelBuilder for Voxtral
        // Use HF model ID so mistral.rs can resolve the architecture,
        // but override the local cache path to our downloaded model
        let mut builder = VisionModelBuilder::new("mistralai/Voxtral-Mini-4B-Realtime-2602")
            .from_hf_cache_pathf(PathBuf::from(model_path))
            .with_isq(isq_type)
            .with_logging();

        // Map model layers to GPU when requested
        if use_gpu {
            builder = builder.with_device_mapping(
                DeviceMapSetting::Auto(AutoDeviceMapParams::default_vision()),
            );
        }

        let model = builder
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("mistral.rs model load failed: {:?}", e))?;

        let device_used = if use_gpu { "CUDA" } else { "CPU" }.to_string();

        Ok::<(mistralrs::Model, String), anyhow::Error>((model, device_used))
    });

    let result = result.map(|(model, device_used)| VoxtralModel {
        model,
        runtime,
        device_used,
    });

    match result {
        Ok(model) => Box::into_raw(Box::new(ModelHandle { inner: model })),
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn destroy_model(handle: *mut ModelHandle) {
    if !handle.is_null() {
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}

#[no_mangle]
pub extern "C" fn transcribe(
    handle: *mut ModelHandle,
    audio: *const f32,
    audio_len: usize,
    options: *const TranscribeOptions,
) -> TranscribeResult {
    clear_last_error();

    if handle.is_null() || audio.is_null() {
        set_last_error("Null handle or audio pointer");
        return TranscribeResult {
            code: SttResult::InvalidParam,
            text: cstr!(""),
            text_len: 0,
            device_used: cstr!(""),
        };
    }

    let handle = unsafe { &*handle };
    let audio_slice = unsafe { std::slice::from_raw_parts(audio, audio_len) };
    let _options = if options.is_null() {
        &TranscribeOptions::default()
    } else {
        unsafe { &*options }
    };

    // Transcribe using mistral.rs Voxtral
    let result = transcribe_with_voxtral(&handle.inner, audio_slice);

    match result {
        Ok(text) => {
            let text_cstring = CString::new(text.clone()).unwrap_or_default();
            let text_ptr = text_cstring.into_raw();
            let device_cstring = CString::new(handle.inner.device_used.clone()).unwrap_or_default();
            let device_ptr = device_cstring.into_raw();

            TranscribeResult {
                code: SttResult::Ok,
                text: text_ptr,
                text_len: text.len(),
                device_used: device_ptr,
            }
        }
        Err(e) => {
            set_last_error(&e.to_string());
            TranscribeResult {
                code: SttResult::TranscriptionFailed,
                text: cstr!(""),
                text_len: 0,
                device_used: cstr!(""),
            }
        }
    }
}

/// Transcribe audio using Voxtral via mistral.rs
fn transcribe_with_voxtral(model: &VoxtralModel, audio: &[f32]) -> Result<String> {
    model.runtime.block_on(async {
        // Create AudioInput from f32 samples @ 16kHz mono
        // Voxtral expects 16kHz audio
        let audio_input = AudioInput {
            samples: audio.to_vec(),
            sample_rate: 16000,
            channels: 1,
        };

        // Create vision messages with audio
        let messages = VisionMessages::new()
            .add_audio_message(
                TextMessageRole::User,
                "Transcribe this audio.",
                vec![audio_input],
                &model.model,
            )
            .context("Failed to create audio message")?;

        // Send chat request and get transcription
        let response = model
            .model
            .send_chat_request(messages)
            .await
            .context("Failed to get transcription from Voxtral")?;

        // Extract transcription from response
        // Response structure: response.choices[0].message.content
        let transcription = response.choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|c| c.as_str())
            .unwrap_or("[No transcription]");

        Ok(transcription.to_string())
    })
}

#[no_mangle]
pub extern "C" fn free_result(result: *mut TranscribeResult) {
    if !result.is_null() {
        unsafe {
            let r = &mut *result;

            // Free text if not empty
            if !r.text.is_null() && r.text_len > 0 {
                let _ = CString::from_raw(r.text as *mut _);
            }

            // Free device_used if not empty
            if !r.device_used.is_null() {
                let _ = CString::from_raw(r.device_used as *mut _);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn get_last_error() -> *const i8 {
    if let Ok(guard) = get_last_error_storage().lock() {
        if let Some(ref msg) = *guard {
            if let Ok(cstring) = CString::new(msg.as_str()) {
                return cstring.into_raw();
            }
        }
    }
    std::ptr::null()
}
