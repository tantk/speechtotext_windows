//! CTranslate2-based Whisper backend for app
//!
//! This backend uses ct2rs (CTranslate2 Rust bindings) via the sys API
//! for direct control over Whisper prompts (transcribe vs translate, timestamps).

use app_core::*;
use ct2rs::sys::{self, WhisperOptions};
use ct2rs::tokenizers::hf;
use ct2rs::Tokenizer;
use mel_spec::mel::{log_mel_spectrogram, mel, norm_mel};
use mel_spec::stft::Spectrogram;
use ndarray::{s, stack, Array2, Axis};
use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
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

/// Preprocessor config loaded from the model directory
#[allow(dead_code)]
struct PreprocessorConfig {
    feature_size: usize,
    hop_length: usize,
    n_fft: usize,
    n_samples: usize,
    nb_max_frames: usize,
    sampling_rate: usize,
    mel_filters: Array2<f64>,
}

impl PreprocessorConfig {
    fn read(path: &Path) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Aux {
            feature_size: usize,
            hop_length: usize,
            n_fft: usize,
            n_samples: usize,
            nb_max_frames: usize,
            sampling_rate: usize,
            mel_filters: Option<Vec<Vec<f64>>>,
        }

        let file = File::open(path).map_err(|e| format!("Failed to open preprocessor config: {}", e))?;
        let reader = BufReader::new(file);
        let aux: Aux = serde_json::from_reader(reader)
            .map_err(|e| format!("Failed to parse preprocessor config: {}", e))?;

        let mel_filters = if let Some(filters) = aux.mel_filters {
            let rows = filters.len();
            let cols = filters.first().map(|row| row.len()).unwrap_or_default();
            Array2::from_shape_vec((rows, cols), filters.into_iter().flatten().collect())
                .map_err(|e| format!("Failed to build mel filter array: {}", e))?
        } else {
            mel(
                aux.sampling_rate as f64,
                aux.n_fft,
                aux.feature_size,
                None,
                None,
                false,
                true,
            )
        };

        Ok(Self {
            feature_size: aux.feature_size,
            hop_length: aux.hop_length,
            n_fft: aux.n_fft,
            n_samples: aux.n_samples,
            nb_max_frames: aux.nb_max_frames,
            sampling_rate: aux.sampling_rate,
            mel_filters,
        })
    }
}

/// Internal model state
struct WhisperModel {
    whisper: sys::Whisper,
    tokenizer: hf::Tokenizer,
    config: PreprocessorConfig,
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

    let model_dir = Path::new(model_path);

    // Load preprocessor config
    let preprocess_config = match PreprocessorConfig::read(&model_dir.join("preprocessor_config.json")) {
        Ok(c) => c,
        Err(e) => {
            set_error(&format!("Failed to load preprocessor config: {}", e));
            return ptr::null_mut();
        }
    };

    // Load tokenizer
    let tokenizer = match hf::Tokenizer::new(model_dir) {
        Ok(t) => t,
        Err(e) => {
            set_error(&format!("Failed to load tokenizer: {}", e));
            return ptr::null_mut();
        }
    };

    // Determine device and create model
    if config.use_gpu {
        #[cfg(feature = "cuda")]
        {
            match try_create_whisper(model_path, sys::Device::CUDA) {
                Ok(whisper) => {
                    let model = Box::new(WhisperModel {
                        whisper,
                        tokenizer,
                        config: preprocess_config,
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
    match try_create_whisper(model_path, sys::Device::CPU) {
        Ok(whisper) => {
            let model = Box::new(WhisperModel {
                whisper,
                tokenizer,
                config: preprocess_config,
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

fn try_create_whisper(model_path: &str, device: sys::Device) -> Result<sys::Whisper, String> {
    let config = sys::Config {
        device,
        ..Default::default()
    };
    sys::Whisper::new(model_path, config).map_err(|e| format!("{:?}: {}", device, e))
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

/// Compute mel spectrogram from audio samples, returning the ndarray and batch size.
/// The caller must create the StorageView from the returned array to keep the borrow alive.
fn compute_mel_spectrogram(
    samples: &[f32],
    config: &PreprocessorConfig,
) -> Result<(ndarray::Array3<f32>, usize), String> {
    let mut stft = Spectrogram::new(config.n_fft, config.hop_length);

    let mut mel_spectrogram_vec = vec![];
    for chunk in samples.chunks(config.n_samples) {
        let mut mel_spectrogram_per_chunk =
            Array2::zeros((config.feature_size, config.nb_max_frames));
        for (i, frame) in chunk.chunks(config.hop_length).enumerate() {
            if let Some(fft_frame) = stft.add(frame) {
                let mel_frame = norm_mel(&log_mel_spectrogram(&fft_frame, &config.mel_filters))
                    .mapv(|v| v as f32);
                mel_spectrogram_per_chunk
                    .slice_mut(s![.., i])
                    .assign(&mel_frame.slice(s![.., 0]));
            }
        }
        mel_spectrogram_vec.push(mel_spectrogram_per_chunk);
    }

    let batch_size = mel_spectrogram_vec.len();
    let mut mel_spectrogram = stack(
        Axis(0),
        &mel_spectrogram_vec
            .iter()
            .map(|a| a.view())
            .collect::<Vec<_>>(),
    )
    .map_err(|e| format!("Failed to stack mel spectrograms: {}", e))?;

    if !mel_spectrogram.is_standard_layout() {
        mel_spectrogram = mel_spectrogram.as_standard_layout().into_owned();
    }

    Ok((mel_spectrogram, batch_size))
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
        } else if opts.translate {
            None // Auto-detect when translating (source language unknown)
        } else {
            Some("en") // Default to English for transcription
        }
    } else {
        Some("en")
    };

    // Check options
    let (use_timestamps, use_translate) = if !options.is_null() {
        let opts = unsafe { &*options };
        (opts.timestamps, opts.translate)
    } else {
        (false, false)
    };

    // Compute mel spectrogram
    let (mut mel_spectrogram, batch_size) = match compute_mel_spectrogram(audio_slice, &model.config) {
        Ok(v) => v,
        Err(e) => {
            set_error(&format!("Mel spectrogram failed: {}", e));
            return TranscribeResult {
                code: SttResult::TranscriptionFailed,
                text: ptr::null(),
                text_len: 0,
                device_used: model.device_name.as_ptr(),
            };
        }
    };

    let shape = mel_spectrogram.shape().to_vec();
    let storage_view = match sys::StorageView::new(
        &shape,
        mel_spectrogram.as_slice_mut().unwrap(),
        Default::default(),
    ) {
        Ok(sv) => sv,
        Err(e) => {
            set_error(&format!("Failed to create storage view: {}", e));
            return TranscribeResult {
                code: SttResult::TranscriptionFailed,
                text: ptr::null(),
                text_len: 0,
                device_used: model.device_name.as_ptr(),
            };
        }
    };

    // Detect or set language token
    let lang_token = match language {
        Some(lang) => format!("<|{}|>", lang),
        None => {
            match model.whisper.detect_language(&storage_view) {
                Ok(detection) => {
                    detection
                        .into_iter()
                        .next()
                        .and_then(|v| v.into_iter().next())
                        .map(|d| d.language)
                        .unwrap_or_else(|| "<|en|>".to_string())
                }
                Err(e) => {
                    set_error(&format!("Language detection failed: {}", e));
                    return TranscribeResult {
                        code: SttResult::TranscriptionFailed,
                        text: ptr::null(),
                        text_len: 0,
                        device_used: model.device_name.as_ptr(),
                    };
                }
            }
        }
    };

    // Build prompt with configurable task token
    let task_token = if use_translate {
        "<|translate|>"
    } else {
        "<|transcribe|>"
    };

    let mut prompt = vec!["<|startoftranscript|>", &lang_token, task_token];
    if !use_timestamps {
        prompt.push("<|notimestamps|>");
    }

    // Perform transcription
    match model.whisper.generate(
        &storage_view,
        &vec![prompt; batch_size],
        &WhisperOptions::default(),
    ) {
        Ok(results) => {
            let text = results
                .into_iter()
                .filter_map(|res| {
                    res.sequences
                        .into_iter()
                        .next()
                        .and_then(|tokens| model.tokenizer.decode(tokens).ok())
                })
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();

            let text_len = text.len();
            let text_cstring = CString::new(text).unwrap();
            let text_ptr = text_cstring.as_ptr();
            std::mem::forget(text_cstring);

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
