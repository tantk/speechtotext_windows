// Windows subsystem disabled in debug builds for console output
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod always_listen;
mod audio;
mod audio_file;
mod backend_loader;
mod config;
mod downloader;
mod hotkeys;
mod local_agreement;
mod overlay;
mod setup;
mod srt;
mod subtitle;
mod transcribe_file;
mod tray;
mod typer;

use anyhow::Result;
use backend_loader::LoadedBackend;
use config::{get_exe_stem, setup_cuda_env, Config};
use cpal::traits::StreamTrait;
use cpal::Stream;
use hotkeys::{check_hotkey_event, HotkeyAction, HotkeyManager};
use overlay::Overlay;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tao::event::{ElementState, Event, MouseButton, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tray::AppStatus;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, POINT,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, AppendMenuW, TrackPopupMenu, DestroyMenu,
    MF_STRING, TPM_RIGHTALIGN, TPM_BOTTOMALIGN, TPM_RETURNCMD,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Idle,
    Recording,
    Processing,
    AlwaysListening,
}

/// Initialize logging with file output (and console in debug builds)
fn init_logging(file_writer: tracing_appender::non_blocking::NonBlocking) {
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false); // No ANSI colors in file

    #[cfg(debug_assertions)]
    {
        // Debug: log to both console and file
        let console_layer = tracing_subscriber::fmt::layer();
        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .init();
    }

    #[cfg(not(debug_assertions))]
    {
        // Release: log to file only (no console available)
        tracing_subscriber::registry()
            .with(file_layer)
            .init();
    }
}

#[cfg(target_os = "windows")]
struct InstanceLock {
    handle: HANDLE,
}

#[cfg(target_os = "windows")]
impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
fn acquire_instance_lock() -> Result<Option<InstanceLock>> {
    let stem = get_exe_stem()?;
    let mutex_name = format!("Global\\app-{}", stem);
    let mut wide: Vec<u16> = mutex_name.encode_utf16().collect();
    wide.push(0);

    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr()))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(None);
        }
        Ok(Some(InstanceLock { handle }))
    }
}

// Context menu item IDs for overlay right-click menu
#[cfg(target_os = "windows")]
const MENU_SHOW_OVERLAY: u32 = 1;
#[cfg(target_os = "windows")]
const MENU_SETTINGS: u32 = 2;
#[cfg(target_os = "windows")]
const MENU_EXIT: u32 = 3;

/// Show a context menu at the current cursor position and return the selected item ID
#[cfg(target_os = "windows")]
fn show_overlay_context_menu(hwnd: HWND) -> Option<u32> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;

        let show_overlay_text: Vec<u16> = "Show/Hide Overlay\0".encode_utf16().collect();
        let settings_text: Vec<u16> = "Settings\0".encode_utf16().collect();
        let exit_text: Vec<u16> = "Exit\0".encode_utf16().collect();

        let _ = AppendMenuW(menu, MF_STRING, MENU_SHOW_OVERLAY as usize, PCWSTR(show_overlay_text.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS as usize, PCWSTR(settings_text.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT as usize, PCWSTR(exit_text.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        let cmd = TrackPopupMenu(
            menu,
            TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );

        let _ = DestroyMenu(menu);

        if cmd.0 != 0 {
            Some(cmd.0 as u32)
        } else {
            None
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let setup_mode = args.iter().any(|arg| arg == "--setup");

    // Check for --transcribe mode
    let transcribe_file_path = args
        .iter()
        .position(|arg| arg == "--transcribe")
        .and_then(|i| args.get(i + 1))
        .map(|s| std::path::PathBuf::from(s));

    let output_path = args
        .iter()
        .position(|arg| arg == "--output")
        .and_then(|i| args.get(i + 1))
        .map(|s| std::path::PathBuf::from(s));

    // In --transcribe mode, attach a console for stdout in release builds
    if transcribe_file_path.is_some() {
        #[cfg(all(target_os = "windows", not(debug_assertions)))]
        {
            use windows::Win32::System::Console::AllocConsole;
            unsafe {
                let _ = AllocConsole();
            }
        }

        // Initialize minimal logging to stderr
        let log_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let log_name = format!("app-{}.log", get_exe_stem().unwrap_or_else(|_| "app".to_string()));
        let file_appender = tracing_appender::rolling::never(&log_dir, log_name);
        let (file_writer, _log_guard) = tracing_appender::non_blocking(file_appender);
        init_logging(file_writer);

        let input = transcribe_file_path.unwrap();
        if !input.exists() {
            eprintln!("Error: file not found: {}", input.display());
            std::process::exit(1);
        }

        let output = output_path.unwrap_or_else(|| input.with_extension("srt"));

        let config = Config::load().unwrap_or_else(|_| {
            eprintln!("Error: no config found. Run the app normally first to set up a model.");
            std::process::exit(1);
        });

        return transcribe_file::transcribe_file(&input, &output, &config);
    }

    #[cfg(target_os = "windows")]
    let _instance_lock = {
        if setup_mode {
            // Setup mode skips the instance lock because the main app may still
            // be shutting down when it spawns `app.exe --setup`.
            None
        } else {
            let lock = acquire_instance_lock()?;
            if lock.is_none() {
                show_error_dialog(
                    "Already Running",
                    "Another instance with the same executable name is already running.",
                );
                return Ok(());
            }
            lock
        }
    };

    // Initialize logging with file output
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let log_name = format!("app-{}.log", get_exe_stem().unwrap_or_else(|_| "app".to_string()));
    // Create a file appender that writes to app-<exe>.log
    let file_appender = tracing_appender::rolling::never(&log_dir, log_name.clone());
    let (file_writer, _log_guard) = tracing_appender::non_blocking(file_appender);

    // Set up logging with both console (for debug builds) and file output
    // Note: _log_guard must be kept alive for the duration of the program
    init_logging(file_writer);

    info!("========================================");
    info!("  Speech-to-Text for Windows");
    info!("========================================");
    info!("Log file: {}", log_dir.join(log_name).display());

    // --setup flag: run setup wizard then exit (used for settings flow)
    if setup_mode {
        info!("Running setup wizard (--setup mode)...");
        // run_setup() never returns - it saves config, spawns app.exe, and exits
        setup::run_setup();
    }

    // Check if config exists and model is available
    let config = match Config::load() {
        Ok(cfg) => {
            let model_complete = cfg.model_exists() && model_files_complete(&cfg).unwrap_or(false);
            if model_complete {
                info!("Config loaded. Backend: {}", cfg.backend_id);
                info!("Model: {:?}", cfg.model_path);
                cfg
            } else {
                warn!("Model files missing or incomplete: {:?}", cfg.model_path);
                info!("Launching setup wizard...");
                run_setup_and_get_config()?
            }
        }
        Err(_) => {
            info!("No config found. Launching setup wizard...");
            run_setup_and_get_config()?
        }
    };

    run_app(config)
}

fn run_setup_and_get_config() -> Result<Config> {
    // run_setup() never returns - it either spawns a new process or exits
    setup::run_setup()
}

fn model_files_complete(config: &Config) -> Result<bool> {
    let backend_dir = config::get_backends_dir()?.join(&config.backend_id);
    let manifest_path = backend_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(true);
    }

    let manifest = backend_loader::BackendManifest::load(&manifest_path)?;
    let model = match manifest.models.iter().find(|m| m.id == config.model_name) {
        Some(model) => model,
        None => {
            warn!(
                "Model id '{}' not found in manifest: {}",
                config.model_name,
                manifest_path.display()
            );
            return Ok(true);
        }
    };

    for filename in &model.files {
        let file_path = config.model_path.join(filename);
        if !file_path.exists() {
            warn!("Missing model file: {}", file_path.display());
            return Ok(false);
        }
    }

    Ok(true)
}

fn resolve_model_load_path(config: &Config, backend: &LoadedBackend) -> std::path::PathBuf {
    // whisper.cpp expects a model file path (e.g. ggml-base.bin), while other backends
    // may expect a model directory.
    if let Some(model) = backend
        .manifest
        .models
        .iter()
        .find(|m| m.id == config.model_name)
    {
        if config.model_path.is_dir() && model.files.len() == 1 {
            let candidate = config.model_path.join(&model.files[0]);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    config.model_path.clone()
}

/// Show an error dialog to the user (Windows native message box)
#[cfg(windows)]
fn show_error_dialog(title: &str, message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK,
    };
    use windows::core::HSTRING;

    let title_wide = HSTRING::from(title);
    let message_wide = HSTRING::from(message);

    unsafe {
        let _ = MessageBoxW(
            None,
            &message_wide,
            &title_wide,
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Non-Windows fallback just logs the error
#[cfg(not(windows))]
fn show_error_dialog(title: &str, message: &str) {
    error!("{}: {}", title, message);
}

/// Determine if Whisper translate mode should be used.
/// Only translates when target is "en" and input is not English.
fn should_translate(input_lang: &str, target_lang: &str) -> bool {
    if target_lang == "original" {
        return false;
    }
    // Whisper only supports translating TO English
    if target_lang != "en" {
        return false;
    }
    // No point translating English to English
    if input_lang == "en" {
        return false;
    }
    true
}

/// Transcription worker that processes audio and types the result
fn transcribe_and_type(
    audio_data: Vec<f32>,
    model: Arc<backend_loader::Model>,
    typer: Arc<Mutex<typer::Typer>>,
    _state: Arc<Mutex<AppMode>>,
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
    app_status: AppStatus,
    partial_state: Option<Arc<Mutex<PartialState>>>,
    is_partial: bool,
    translate: bool,
    type_to_window: bool,
    agreement: Option<Arc<Mutex<local_agreement::LocalAgreement>>>,
    trim_tx: Option<crossbeam_channel::Sender<always_listen::AlwaysListenCommand>>,
    input_language: Option<String>,
) {
    std::thread::spawn(move || {
        // Basic silence gating to avoid hallucinations on empty/near-silent audio.
        if !should_transcribe(&audio_data) {
            let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
            return;
        }

        info!(
            "{} {} samples (~{:.1}s of audio)...",
            if translate { "Translating" } else { "Transcribing" },
            audio_data.len(),
            audio_data.len() as f32 / 16000.0
        );

        let lang_ref = input_language.as_deref();
        let use_timestamps = translate && agreement.is_some();
        let result = if translate {
            if use_timestamps {
                model.translate_with_timestamps(&audio_data, lang_ref)
            } else {
                model.translate(&audio_data, lang_ref)
            }
        } else {
            model.transcribe(&audio_data, lang_ref)
        };
        match result {
            Ok(text) => {
                if !text.is_empty() {
                    let trimmed = text.trim().to_string();
                    if trimmed.is_empty() {
                        info!("No speech detected");
                        let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                        return;
                    }
                    if is_placeholder_transcript(&trimmed) {
                        info!("No speech detected (placeholder transcript)");
                        let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                        return;
                    }

                    // --- Translate mode with LocalAgreement ---
                    // Uses word-level comparison + timestamps for buffer trimming.
                    // Partials: run through LocalAgreement, confirmed sentences get typed.
                    // Final: type whatever hasn't been confirmed yet.
                    if translate {
                        if let Some(ref ag) = agreement {
                            let mut la = ag.lock();
                            if is_partial {
                                let ar = la.process(&trimmed);

                                // Show full text on subtitle (replace mode)
                                let display = if ar.full_text.is_empty() { trimmed.clone() } else { ar.full_text };
                                let _ = proxy.send_event(UserEvent::SubtitleReplace(display));

                                // Type confirmed sentences
                                if !ar.confirmed_new.is_empty() {
                                    info!("LA confirmed: \"{}\"", ar.confirmed_new);
                                    if type_to_window {
                                        if let Err(e) = typer.lock().type_text(&ar.confirmed_new) {
                                            error!("Failed to type confirmed: {}", e);
                                        }
                                    }
                                    let _ = proxy.send_event(UserEvent::SubtitleText(ar.confirmed_new));
                                }

                                // Send trim command if we have a trim point
                                if let Some(ts) = ar.trim_to {
                                    if let Some(ref tx) = trim_tx {
                                        let _ = tx.send(always_listen::AlwaysListenCommand::TrimBuffer(ts));
                                    }
                                }

                                let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                                return;
                            } else {
                                // Final result — type the unconfirmed remainder
                                let confirmed = la.confirmed_text();
                                // Strip timestamp tokens for clean final text
                                let clean = strip_timestamp_tokens(&trimmed);
                                let remainder = if !confirmed.is_empty() && clean.starts_with(&confirmed) {
                                    clean[confirmed.len()..].trim().to_string()
                                } else {
                                    clean
                                };

                                if !remainder.is_empty() {
                                    info!("Final translate (remainder): \"{}\"", remainder);
                                    if type_to_window {
                                        if let Err(e) = typer.lock().type_text(&remainder) {
                                            error!("Failed to type final: {}", e);
                                        }
                                    }
                                }
                                let _ = proxy.send_event(UserEvent::SubtitleText(trimmed.clone()));
                                la.reset();
                            }
                        } else if let Some(state) = partial_state.as_ref() {
                            // Fallback: old PartialState approach (push-to-talk translate)
                            let _ = proxy.send_event(UserEvent::SubtitleText(trimmed.clone()));
                            if !is_partial {
                                info!("Final translate: \"{}\"", trimmed);
                                if type_to_window {
                                    if let Err(e) = typer.lock().type_text(&trimmed) {
                                        error!("Failed to type: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    // --- Regular transcription: suffix-based partial dedup ---
                    else if let Some(state) = partial_state.as_ref() {
                        let _ = proxy.send_event(UserEvent::SubtitleText(trimmed.clone()));
                        let mut st = state.lock();
                        if is_partial {
                            let normalized = normalize_partial(&trimmed);
                            if normalized.is_empty()
                                || looks_like_url(&normalized)
                                || is_symbol_heavy(&trimmed)
                            {
                                info!("Partial filtered: \"{}\"", trimmed);
                                let _ =
                                    proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                                return;
                            }

                            if normalized == st.last_candidate {
                                st.candidate_count = st.candidate_count.saturating_add(1);
                            } else {
                                st.last_candidate = normalized.clone();
                                st.candidate_count = 1;
                            }

                            if st.candidate_count < 2 {
                                info!("Partial unstable, waiting: \"{}\"", trimmed);
                                let _ =
                                    proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                                return;
                            }

                            if !st.last_typed.is_empty() && trimmed.starts_with(&st.last_typed) {
                                let suffix = &trimmed[st.last_typed.len()..];
                                if !suffix.is_empty() {
                                    info!("Typing partial suffix...");
                                    if type_to_window {
                                        if let Err(e) = typer.lock().type_text(suffix) {
                                            error!("Failed to type partial suffix: {}", e);
                                        }
                                    }
                                }
                            } else if st.last_typed.is_empty() {
                                info!("Typing partial text...");
                                if type_to_window {
                                    if let Err(e) = typer.lock().type_text(&trimmed) {
                                        error!("Failed to type partial text: {}", e);
                                    }
                                }
                            } else {
                                info!("Partial changed, skipping typing this chunk");
                            }

                            st.last_typed = trimmed;
                            let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
                            return;
                        } else {
                            if !st.last_typed.is_empty() && trimmed.starts_with(&st.last_typed) {
                                let suffix = &trimmed[st.last_typed.len()..];
                                if !suffix.is_empty() {
                                    info!("Typing final suffix...");
                                    if type_to_window {
                                        if let Err(e) = typer.lock().type_text(suffix) {
                                            error!("Failed to type final suffix: {}", e);
                                        }
                                    }
                                }
                            } else {
                                info!("Final result: \"{}\"", trimmed);
                                if type_to_window {
                                    if let Err(e) = typer.lock().type_text(&trimmed) {
                                        error!("Failed to type: {}", e);
                                    }
                                }
                            }
                            st.last_typed.clear();
                            st.last_candidate.clear();
                            st.candidate_count = 0;
                        }
                    } else {
                        let _ = proxy.send_event(UserEvent::SubtitleText(trimmed.clone()));
                        info!("Result: \"{}\"", trimmed);
                        if type_to_window {
                            if let Err(e) = typer.lock().type_text(&trimmed) {
                                error!("Failed to type: {}", e);
                            }
                        }
                    }
                } else {
                    info!("No speech detected");
                }
            }
            Err(e) => {
                error!("Transcription error: {}", e);
            }
        }

        let _ = proxy.send_event(UserEvent::TranscriptionComplete(app_status));
    });
}

#[derive(Default)]
struct PartialState {
    last_typed: String,
    last_candidate: String,
    candidate_count: u8,
}

fn normalize_partial(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Remove Whisper timestamp tokens like `<|0.00|>` from text
fn strip_timestamp_tokens(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();
    while pos < len {
        if pos + 5 < len && &text[pos..pos + 2] == "<|" {
            if let Some(end_pipe) = text[pos + 2..].find("|>") {
                let ts_str = &text[pos + 2..pos + 2 + end_pipe];
                if ts_str.parse::<f64>().is_ok() {
                    pos = pos + 2 + end_pipe + 2;
                    continue;
                }
            }
        }
        result.push(bytes[pos] as char);
        pos += 1;
    }
    // Clean up multiple spaces
    let mut clean = String::with_capacity(result.len());
    let mut last_space = false;
    for ch in result.trim().chars() {
        if ch == ' ' {
            if !last_space {
                clean.push(ch);
            }
            last_space = true;
        } else {
            clean.push(ch);
            last_space = false;
        }
    }
    clean
}

fn is_placeholder_transcript(text: &str) -> bool {
    const PLACEHOLDERS: [&str; 4] = ["[BLANK_AUDIO]", "[NO_SPEECH]", "<|NOSPEECH|>", "[SILENCE]"];

    let compact = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();

    if compact.is_empty() {
        return false;
    }

    let mut remaining = compact.as_str();
    loop {
        let mut matched = false;
        for token in PLACEHOLDERS {
            if let Some(rest) = remaining.strip_prefix(token) {
                remaining = rest;
                matched = true;
                break;
            }
        }

        if !matched {
            break;
        }
    }

    remaining.is_empty()
}

fn looks_like_url(text: &str) -> bool {
    let t = text.trim();
    if t.starts_with("www.") || t.starts_with("http://") || t.starts_with("https://") {
        return true;
    }
    if t.contains(".com") || t.contains(".org") || t.contains(".net") {
        return true;
    }
    false
}

fn is_symbol_heavy(text: &str) -> bool {
    let mut symbol_count = 0usize;
    let mut alnum_count = 0usize;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            alnum_count += 1;
        } else if !ch.is_whitespace() {
            symbol_count += 1;
        }
    }
    if alnum_count < 3 {
        return true;
    }
    symbol_count * 2 >= (alnum_count + symbol_count)
}

fn should_transcribe(audio_data: &[f32]) -> bool {
    let sample_count = audio_data.len();
    if sample_count == 0 {
        warn!("Skipping transcription: no audio captured");
        return false;
    }

    let duration_s = sample_count as f32 / 16000.0;
    let max_val = audio_data
        .iter()
        .map(|x| x.abs())
        .fold(0.0f32, f32::max);
    let rms = (audio_data.iter().map(|x| x * x).sum::<f32>() / sample_count as f32).sqrt();

    info!(
        "Audio check: {:.2}s, {} samples, max={:.4}, rms={:.6}",
        duration_s, sample_count, max_val, rms
    );

    if duration_s < 0.15 {
        warn!("Skipping transcription: audio too short ({:.3}s)", duration_s);
        return false;
    }

    if max_val < 0.01 && rms < 0.004 {
        warn!(
            "Skipping transcription: audio too quiet (max={:.4}, rms={:.4})",
            max_val, rms
        );
        return false;
    }

    true
}

fn run_app(mut config: Config) -> Result<()> {
    // Set up CUDA environment if GPU is enabled
    setup_cuda_env(&config);

    // Initialize audio capture based on audio source config
    info!(
        "Audio source: {:?}, Input language: {}, Target language: {}",
        config.audio_source, config.input_language, config.target_language
    );
    let audio_capture = match config.audio_source {
        config::AudioSource::SystemAudio => {
            match audio::AudioCapture::new_loopback() {
                Ok(cap) => {
                    info!("System audio (loopback) capture ready");
                    Arc::new(Mutex::new(cap))
                }
                Err(e) => {
                    error!("Failed to initialize loopback capture: {}", e);
                    show_error_dialog(
                        "Audio Error",
                        &format!("Failed to initialize system audio capture:\n{}\n\nFalling back to microphone.", e),
                    );
                    // Fall back to microphone
                    let cap = audio::AudioCapture::new_with_device(config.input_device_name.as_deref())?;
                    config.audio_source = config::AudioSource::Microphone;
                    Arc::new(Mutex::new(cap))
                }
            }
        }
        config::AudioSource::Microphone => {
            match audio::AudioCapture::new_with_device(config.input_device_name.as_deref()) {
                Ok(cap) => {
                    info!("Audio capture ready");
                    Arc::new(Mutex::new(cap))
                }
                Err(e) => {
                    error!("Failed to initialize audio capture: {}", e);
                    show_error_dialog(
                        "Audio Error",
                        &format!("Failed to initialize audio capture:\n{}\n\nPlease check your microphone settings.", e),
                    );
                    return Err(e);
                }
            }
        }
    };

    // Track language settings at runtime (can be changed from tray)
    let input_language = Arc::new(Mutex::new(config.input_language.clone()));
    let target_language = Arc::new(Mutex::new(config.target_language.clone()));
    let type_to_window = Arc::new(AtomicBool::new(config.type_to_window));

    // Load backend
    let backend_dir = config::get_backends_dir()?.join(&config.backend_id);
    info!("Loading backend from: {}", backend_dir.display());

    let backend = match LoadedBackend::load(&backend_dir) {
        Ok(be) => {
            info!("Backend loaded: {}", be.display_name);
            be
        }
        Err(e) => {
            error!("Failed to load backend: {}", e);
            show_error_dialog(
                "Backend Error",
                &format!(
                    "Failed to load backend '{}':\n{}\n\nPlease ensure the backend files are in:\n{}",
                    config.backend_id,
                    e,
                    backend_dir.display()
                ),
            );
            return Err(e);
        }
    };

    // Verify CUDA support at runtime before creating the model
    if config.use_gpu && !backend.supports_cuda_runtime() {
        warn!("GPU requested but backend was built without CUDA support");
        show_error_dialog(
            "CUDA Error",
            "GPU was requested, but the selected backend was built without CUDA support.\n\nRebuild the backend with --features cuda or disable GPU.",
        );
        config.use_gpu = false;
    }

    let model_load_path = resolve_model_load_path(&config, &backend);

    // Log model input state before creation
    info!(
        "Model load request (path={}, use_gpu={}, backend_cuda={})",
        model_load_path.display(),
        config.use_gpu,
        backend.supports_cuda_runtime()
    );

    if let Some(model) = backend
        .manifest
        .models
        .iter()
        .find(|m| m.id == config.model_name)
    {
        for filename in &model.files {
            let path = config.model_path.join(filename);
            info!("Model file check: {} exists={}", path.display(), path.exists());
        }
    } else {
        info!(
            "Model file check: {} exists={}",
            model_load_path.display(),
            model_load_path.exists()
        );
    }

    // Create model (with GPU->CPU fallback)
    let model = match backend.create_model(&model_load_path, config.use_gpu) {
        Ok(m) => {
            let device_used = if config.use_gpu { "CUDA" } else { "CPU" };
            info!(
                "Model ready (use_gpu={}, backend_cuda={}, device_used={})",
                config.use_gpu,
                backend.supports_cuda_runtime(),
                device_used
            );
            Arc::new(m)
        }
        Err(e) => {
            if config.use_gpu {
                warn!(
                    "GPU model load failed: {}. Retrying on CPU...",
                    e
                );
                match backend.create_model(&model_load_path, false) {
                    Ok(m) => {
                        config.use_gpu = false;
                        info!(
                            "Model ready (use_gpu=false, backend_cuda={}, device_used=CPU)",
                            backend.supports_cuda_runtime()
                        );
                        Arc::new(m)
                    }
                    Err(cpu_e) => {
                        error!("Failed to create model (GPU then CPU): {}", cpu_e);
                        show_error_dialog(
                            "Model Error",
                            &format!(
                                "Failed to load model '{}'.\n\nGPU error:\n{}\n\nCPU error:\n{}\n\nPlease try re-downloading the model from settings.",
                                model_load_path.display(),
                                e,
                                cpu_e
                            ),
                        );
                        return Err(cpu_e);
                    }
                }
            } else {
                error!("Failed to create model: {}", e);
                show_error_dialog(
                    "Model Error",
                    &format!(
                        "Failed to load model '{}':\n{}\n\nPlease try re-downloading the model from settings.",
                        model_load_path.display(),
                        e
                    ),
                );
                return Err(e);
            }
        }
    };

    let typer = match typer::Typer::new() {
        Ok(t) => {
            info!("Keyboard typer ready");
            Arc::new(Mutex::new(t))
        }
        Err(e) => {
            error!("Failed to initialize typer: {}", e);
            show_error_dialog(
                "Keyboard Error",
                &format!("Failed to initialize keyboard simulation:\n{}\n\nSome features may not work.", e),
            );
            return Err(e);
        }
    };

    // Create event loop
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Initialize hotkeys from config
    info!(
        "Registering hotkeys: PTT='{}', AlwaysListen='{}'",
        config.hotkey_push_to_talk, config.hotkey_always_listen
    );
    let hotkey_manager = match HotkeyManager::from_config(
        &config.hotkey_push_to_talk,
        &config.hotkey_always_listen,
    ) {
        Ok(hm) => {
            info!("Hotkey manager ready");
            hm
        }
        Err(e) => {
            error!("Failed to initialize hotkey manager: {}", e);
            show_error_dialog(
                "Hotkey Error",
                &format!(
                    "Failed to register hotkeys:\n{}\n\nDefault hotkeys will be used instead.",
                    e
                ),
            );
            // Fall back to default hotkeys
            HotkeyManager::from_config("Backquote", "Control+Backquote")?
        }
    };
    let push_to_talk_id = hotkey_manager.push_to_talk_id();
    let always_listen_id = hotkey_manager.always_listen_id();
    let hotkey_receiver = HotkeyManager::receiver();

    // Initialize tray
    let mut tray_manager = match tray::TrayManager::new(
        config.audio_source == config::AudioSource::SystemAudio,
        &config.input_language,
        &config.target_language,
        config.subtitle_visible,
        config.type_to_window,
    ) {
        Ok(tm) => tm,
        Err(e) => {
            error!("Failed to initialize tray: {}", e);
            // Non-fatal - we can run without tray
            show_error_dialog(
                "Tray Icon Error",
                &format!("Failed to create system tray icon:\n{}\n\nThe app will continue running.", e),
            );
            return Err(e);
        }
    };
    let menu_receiver = tray::TrayManager::menu_receiver();
    let show_overlay_id = tray_manager.show_overlay_id.clone();
    let show_subtitle_id = tray_manager.show_subtitle_id.clone();
    let type_to_window_id = tray_manager.type_to_window_id.clone();
    let settings_id = tray_manager.settings_id.clone();
    let exit_id = tray_manager.exit_id.clone();
    let mic_source_id = tray_manager.mic_source_id.clone();
    let system_audio_source_id = tray_manager.system_audio_source_id.clone();

    // Initialize overlay with saved position
    let mut overlay = match Overlay::new(&event_loop, config.overlay_x, config.overlay_y) {
        Ok(ov) => ov,
        Err(e) => {
            error!("Failed to create overlay: {}", e);
            // Non-fatal - we can run without overlay
            show_error_dialog(
                "Overlay Error",
                &format!("Failed to create status overlay:\n{}\n\nThe app will run without overlay.", e),
            );
            return Err(e);
        }
    };
    overlay.set_status(AppStatus::Idle);

    // Initialize subtitle bar
    let mut subtitle_bar = match subtitle::SubtitleBar::new(
        &event_loop,
        config.subtitle_x,
        config.subtitle_y,
        config.subtitle_width,
        config.subtitle_height,
        &config.subtitle_font,
        config.subtitle_font_size,
    ) {
        Ok(sb) => {
            info!("Subtitle bar created");
            sb
        }
        Err(e) => {
            error!("Failed to create subtitle bar: {}", e);
            return Err(e);
        }
    };
    if config.subtitle_visible {
        subtitle_bar.show();
    }

    // Build subtitle bar right-click context menu (Font / Size submenus)
    use tray_icon::menu::{ContextMenu, Menu as TrayMenu, MenuId, MenuItem as TrayMenuItem, Submenu};

    let sub_font_submenu = Submenu::new("Font", true);
    let mut sub_font_items: Vec<(TrayMenuItem, String)> = Vec::new();
    let mut sub_font_ids: Vec<(MenuId, String)> = Vec::new();
    for &font in subtitle::FONTS {
        let marker = if font == config.subtitle_font { " *" } else { "" };
        let item = TrayMenuItem::new(format!("{}{}", font, marker), true, None);
        sub_font_ids.push((item.id().clone(), font.to_string()));
        sub_font_items.push((item.clone(), font.to_string()));
        let _ = sub_font_submenu.append(&item);
    }

    let sub_size_submenu = Submenu::new("Size", true);
    let mut sub_size_items: Vec<(TrayMenuItem, u32)> = Vec::new();
    let mut sub_size_ids: Vec<(MenuId, u32)> = Vec::new();
    for &sz in subtitle::FONT_SIZES {
        let marker = if sz == config.subtitle_font_size { " *" } else { "" };
        let item = TrayMenuItem::new(format!("{}{}", sz, marker), true, None);
        sub_size_ids.push((item.id().clone(), sz));
        sub_size_items.push((item.clone(), sz));
        let _ = sub_size_submenu.append(&item);
    }

    let sub_ctx_menu = TrayMenu::new();
    let _ = sub_ctx_menu.append(&sub_font_submenu);
    let _ = sub_ctx_menu.append(&sub_size_submenu);

    info!("Overlay window created");
    info!("System tray icon created");
    info!("========================================");
    info!("  READY!");
    info!("  - Right-click tray icon for menu");
    info!("========================================");

    // App state
    let state = Arc::new(Mutex::new(AppMode::Idle));
    let always_listen_partial = Arc::new(Mutex::new(PartialState::default()));
    let always_listen_agreement = Arc::new(Mutex::new(local_agreement::LocalAgreement::new()));
    let running = Arc::new(AtomicBool::new(true));

    // Always-listen state
    let always_listen_active = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<f32>>(100);
    let (result_tx, _result_rx) =
        crossbeam_channel::bounded::<always_listen::AlwaysListenResult>(10);

    // Channel for sending trim commands from transcription threads to always-listen controller
    let (trim_cmd_tx, trim_cmd_rx) = crossbeam_channel::bounded::<always_listen::AlwaysListenCommand>(10);

    // Spawn always-listen processing thread
    let always_listen_running = Arc::clone(&running);
    let always_listen_active_thread = Arc::clone(&always_listen_active);
    let al_proxy = proxy.clone();
    let silence_timeout_ms = config.silence_timeout_ms;
    let streaming_interval_ms = config.streaming_interval_ms;

    std::thread::spawn(move || {
        use always_listen::{AlwaysListenConfig, AlwaysListenController, AlwaysListenState};

        let mut al_config = AlwaysListenConfig::default();
        al_config.post_silence_duration_ms = silence_timeout_ms;
        al_config.streaming_interval_ms = streaming_interval_ms;
        let controller = AlwaysListenController::new(al_config, audio_rx, result_tx);

        // Track previous state to detect changes
        let mut last_was_recording = false;

        while always_listen_running.load(Ordering::SeqCst) {
            // Only process when always-listen is active
            if always_listen_active_thread.load(Ordering::SeqCst) {
                if controller.state() == AlwaysListenState::Paused {
                    let _ = controller.start();
                }

                // Check for state changes (recording vs listening)
                let current_state = controller.state();
                let is_recording = matches!(current_state, AlwaysListenState::Recording { .. });

                if is_recording != last_was_recording {
                    // State changed - notify main thread
                    let _ = al_proxy.send_event(UserEvent::AlwaysListenStateChange(is_recording));
                    last_was_recording = is_recording;
                }

                // Process trim commands from transcription threads
                while let Ok(cmd) = trim_cmd_rx.try_recv() {
                    if let always_listen::AlwaysListenCommand::TrimBuffer(ts) = cmd {
                        let _ = controller.trim_buffer(ts);
                    }
                }

                // Check for transcription results
                if let Some(result) = controller.try_recv_result() {
                    debug!(
                        "Received {} samples from always-listen (partial={})",
                        result.audio.len(),
                        result.is_partial
                    );

                    // Send event to main thread for transcription
                    let _ = al_proxy.send_event(UserEvent::AlwaysListenAudio(result));
                }
            } else {
                if controller.state() != AlwaysListenState::Paused {
                    let _ = controller.stop();
                }
                last_was_recording = false;
            }

            std::thread::sleep(Duration::from_millis(10));
        }

        let _ = controller.stop();
    });

    // Always-listen audio stream — created lazily on toggle to avoid holding
    // a WASAPI session on the output device when not in use (which can block
    // DirectShow-based video players like MPC).
    let always_listen_stream_running = Arc::new(AtomicBool::new(false));
    let always_listen_stream: Option<Stream> = None;

    // Spawn hotkey listener thread
    let proxy_hotkey = proxy.clone();
    let running_hotkey = Arc::clone(&running);
    std::thread::spawn(move || {
        while running_hotkey.load(Ordering::SeqCst) {
            if let Ok(event) = hotkey_receiver.recv_timeout(Duration::from_millis(100)) {
                if let Some(action) = check_hotkey_event(&event, push_to_talk_id, always_listen_id)
                {
                    let _ = proxy_hotkey.send_event(UserEvent::Hotkey(action));
                }
            }
        }
    });

    // Keep hotkey_manager alive
    let _hotkey_manager = hotkey_manager;

    // Spawn menu listener thread
    let proxy_menu = proxy.clone();
    let running_menu = Arc::clone(&running);
    let sub_font_ids_thread = sub_font_ids.clone();
    let sub_size_ids_thread = sub_size_ids.clone();
    std::thread::spawn(move || {
        while running_menu.load(Ordering::SeqCst) {
            if let Ok(event) = menu_receiver.recv_timeout(Duration::from_millis(100)) {
                // Check subtitle font/size menus first
                let mut handled = false;
                for (fid, name) in &sub_font_ids_thread {
                    if event.id == *fid {
                        let _ = proxy_menu.send_event(UserEvent::SetSubtitleFont(String::clone(name)));
                        handled = true;
                        break;
                    }
                }
                if !handled {
                    for (sid, sz) in &sub_size_ids_thread {
                        if event.id == *sid {
                            let _ = proxy_menu.send_event(UserEvent::SetSubtitleFontSize(*sz));
                            handled = true;
                            break;
                        }
                    }
                }
                if !handled {
                    let _ = proxy_menu.send_event(UserEvent::Menu(event.id));
                }
            }
        }
    });

    // Move into event loop
    let mut always_listen_stream = always_listen_stream;
    let always_listen_stream_running_for_loop = always_listen_stream_running;

    // Run event loop
    event_loop.run(move |event, _, control_flow| {
        let always_listen_stream_running = &always_listen_stream_running_for_loop;
        // Use WaitUntil so we can periodically check subtitle fade
        *control_flow = ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_secs(1));

        // Periodically check subtitle fade
        subtitle_bar.check_fade();

        match event {
            Event::UserEvent(user_event) => match user_event {
                UserEvent::Hotkey(action) => {
                    let mut mode = state.lock();
                    match action {
                        HotkeyAction::PushToTalkPressed => match *mode {
                            AppMode::Idle => {
                                // Start recording (hold to record)
                                info!("RECORDING... (release to stop)");
                                if let Err(e) = audio_capture.lock().start_recording() {
                                    error!("Failed to start recording: {}", e);
                                    return;
                                }
                                *mode = AppMode::Recording;
                                tray_manager.set_status(AppStatus::Recording);
                                overlay.set_status(AppStatus::Recording);
                            }
                            AppMode::AlwaysListening => {
                                // In always-listening mode, push-to-talk temporarily pauses it
                                info!("Push-to-talk activated while in always-listen mode - pausing");
                                always_listen_active.store(false, Ordering::SeqCst);

                                // Start push-to-talk recording
                                if let Err(e) = audio_capture.lock().start_recording() {
                                    error!("Failed to start recording: {}", e);
                                    return;
                                }
                                *mode = AppMode::Recording;
                                tray_manager.set_status(AppStatus::Recording);
                                overlay.set_status(AppStatus::Recording);
                            }
                            _ => {
                                // Already recording or processing, ignore
                            }
                        },
                        HotkeyAction::PushToTalkReleased => {
                            if *mode == AppMode::Recording {
                                // Stop recording and transcribe
                                info!("Released. Processing...");
                                let audio_data = audio_capture.lock().stop_recording();

                                *mode = AppMode::Processing;
                                drop(mode);

                                // Transcribe in background
                                let tgt = target_language.lock().clone();
                                let inp = input_language.lock().clone();
                                let is_translating = should_translate(&inp, &tgt);
                                let lang = if inp == "auto" { None } else { Some(inp) };
                                transcribe_and_type(
                                    audio_data,
                                    Arc::clone(&model),
                                    Arc::clone(&typer),
                                    Arc::clone(&state),
                                    proxy.clone(),
                                    AppStatus::Idle,
                                    None,
                                    false,
                                    is_translating,
                                    type_to_window.load(Ordering::SeqCst),
                                    None,
                                    None,
                                    lang,
                                );
                            }
                        }
                        HotkeyAction::AlwaysListenToggle => {
                            // Toggle always-listen mode
                            match *mode {
                                AppMode::Idle => {
                                    info!("Starting always-listen mode...");
                                    // Create the audio stream on demand
                                    let al_running = Arc::clone(always_listen_stream_running);
                                    always_listen_stream_running.store(true, Ordering::SeqCst);
                                    match audio_capture.lock().create_always_listen_stream(
                                        audio_tx.clone(),
                                        al_running,
                                    ) {
                                        Ok(stream) => {
                                            if let Err(e) = stream.play() {
                                                error!("Failed to start always-listen audio stream: {}", e);
                                                always_listen_active.store(false, Ordering::SeqCst);
                                                always_listen_stream_running.store(false, Ordering::SeqCst);
                                                return;
                                            }
                                            always_listen_stream = Some(stream);
                                            always_listen_active.store(true, Ordering::SeqCst);
                                            *mode = AppMode::AlwaysListening;
                                            tray_manager.set_status(AppStatus::AlwaysListening);
                                            overlay.set_status(AppStatus::AlwaysListening);
                                        }
                                        Err(e) => {
                                            error!("Failed to create always-listen audio stream: {}", e);
                                            always_listen_stream_running.store(false, Ordering::SeqCst);
                                        }
                                    }
                                }
                                AppMode::AlwaysListening => {
                                    info!("Stopping always-listen mode...");
                                    always_listen_active.store(false, Ordering::SeqCst);
                                    always_listen_stream_running.store(false, Ordering::SeqCst);
                                    // Drop the stream to release the WASAPI session
                                    always_listen_stream = None;
                                    *mode = AppMode::Idle;
                                    tray_manager.set_status(AppStatus::Idle);
                                    overlay.set_status(AppStatus::Idle);
                                }
                                _ => {
                                    warn!("Cannot toggle always-listen mode while recording or processing");
                                }
                            }
                        }
                    }
                }
                UserEvent::AlwaysListenAudio(result) => {
                    // Handle always-listen audio for transcription
                    info!(
                        "Always-listen audio received: {} samples ({:.1}s), partial={}",
                        result.audio.len(),
                        result.audio.len() as f32 / 16000.0,
                        result.is_partial
                    );
                    *state.lock() = AppMode::Processing;
                    tray_manager.set_status(AppStatus::Processing);
                    overlay.set_status(AppStatus::Processing);

                    let tgt = target_language.lock().clone();
                    let inp = input_language.lock().clone();
                    let is_translating = should_translate(&inp, &tgt);
                    let lang = if inp == "auto" { None } else { Some(inp) };
                    transcribe_and_type(
                        result.audio,
                        Arc::clone(&model),
                        Arc::clone(&typer),
                        Arc::clone(&state),
                        proxy.clone(),
                        AppStatus::AlwaysListening,
                        Some(Arc::clone(&always_listen_partial)),
                        result.is_partial,
                        is_translating,
                        type_to_window.load(Ordering::SeqCst),
                        if is_translating { Some(Arc::clone(&always_listen_agreement)) } else { None },
                        if is_translating { Some(trim_cmd_tx.clone()) } else { None },
                        lang,
                    );
                }
                UserEvent::AlwaysListenStateChange(is_recording) => {
                    // Update UI when always-listen starts/stops recording speech
                    let mode = *state.lock();
                    if mode == AppMode::AlwaysListening {
                        if is_recording {
                            tray_manager.set_status(AppStatus::AlwaysListeningRecording);
                            overlay.set_status(AppStatus::AlwaysListeningRecording);
                        } else {
                            tray_manager.set_status(AppStatus::AlwaysListening);
                            overlay.set_status(AppStatus::AlwaysListening);
                        }
                    }
                }
                UserEvent::SubtitleText(text) => {
                    subtitle_bar.append_text(&text);
                }
                UserEvent::SubtitleReplace(text) => {
                    subtitle_bar.set_text(&text);
                }
                UserEvent::SetSubtitleFont(ref name) => {
                    subtitle_bar.set_font(name);
                    if let Ok(mut cfg) = config::Config::load() {
                        cfg.subtitle_font = name.clone();
                        let _ = cfg.save();
                    }
                    for (item, font) in &sub_font_items {
                        let marker = if font == name { " *" } else { "" };
                        item.set_text(format!("{}{}", font, marker));
                    }
                    info!("Subtitle font: {}", name);
                }
                UserEvent::SetSubtitleFontSize(sz) => {
                    subtitle_bar.set_font_size(sz);
                    if let Ok(mut cfg) = config::Config::load() {
                        cfg.subtitle_font_size = sz;
                        let _ = cfg.save();
                    }
                    for (item, size) in &sub_size_items {
                        let marker = if *size == sz { " *" } else { "" };
                        item.set_text(format!("{}{}", size, marker));
                    }
                    info!("Subtitle font size: {}", sz);
                }
                UserEvent::Menu(menu_id) => {
                    if menu_id == show_overlay_id {
                        overlay.toggle_visibility();
                    } else if menu_id == show_subtitle_id {
                        subtitle_bar.toggle_visibility();
                        tray_manager.set_subtitle_visible(subtitle_bar.is_visible());
                        // Save to config
                        if let Ok(mut cfg) = config::Config::load() {
                            cfg.subtitle_visible = subtitle_bar.is_visible();
                            let _ = cfg.save();
                        }
                    } else if menu_id == type_to_window_id {
                        let current = type_to_window.load(Ordering::SeqCst);
                        let new_val = !current;
                        type_to_window.store(new_val, Ordering::SeqCst);
                        tray_manager.set_type_to_window(new_val);
                        info!("Type to window: {}", if new_val { "ON" } else { "OFF" });
                        if let Ok(mut cfg) = config::Config::load() {
                            cfg.type_to_window = new_val;
                            let _ = cfg.save();
                        }
                    } else if menu_id == settings_id {
                        // Save state, launch setup wizard, and exit
                        info!("Opening settings (restarting into setup mode)...");
                        // Stop always-listen
                        always_listen_active.store(false, Ordering::SeqCst);
                        always_listen_stream_running.store(false, Ordering::SeqCst);
                        always_listen_stream = None;
                        if let Err(e) = save_window_positions(
                            overlay.get_position(),
                            subtitle_bar.get_position(),
                            subtitle_bar.get_size(),
                            subtitle_bar.is_visible(),
                        ) {
                            error!("Failed to save config: {}", e);
                        }
                        // Spawn setup wizard and exit current process
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(exe)
                                .arg("--setup")
                                .spawn();
                        }
                        running.store(false, Ordering::SeqCst);
                        *control_flow = ControlFlow::Exit;
                    } else if menu_id == mic_source_id {
                        info!("Switching to microphone input");
                        tray_manager.set_audio_source_microphone();
                        // Recreate audio capture with microphone
                        match audio::AudioCapture::new_with_device(None) {
                            Ok(cap) => {
                                *audio_capture.lock() = cap;
                                // Save to config
                                if let Ok(mut cfg) = config::Config::load() {
                                    cfg.audio_source = config::AudioSource::Microphone;
                                    let _ = cfg.save();
                                }
                            }
                            Err(e) => error!("Failed to switch to microphone: {}", e),
                        }
                    } else if menu_id == system_audio_source_id {
                        info!("Switching to system audio (loopback)");
                        tray_manager.set_audio_source_system_audio();
                        // Recreate audio capture with loopback
                        match audio::AudioCapture::new_loopback() {
                            Ok(cap) => {
                                *audio_capture.lock() = cap;
                                // Save to config
                                if let Ok(mut cfg) = config::Config::load() {
                                    cfg.audio_source = config::AudioSource::SystemAudio;
                                    let _ = cfg.save();
                                }
                            }
                            Err(e) => error!("Failed to switch to system audio: {}", e),
                        }
                    } else if let Some(lang) = tray_manager.input_language_for_menu_id(&menu_id) {
                        tray_manager.set_input_language(&lang);
                        *input_language.lock() = lang.clone();
                        info!("Input language: {}", lang);
                        if let Ok(mut cfg) = config::Config::load() {
                            cfg.input_language = lang;
                            let _ = cfg.save();
                        }
                    } else if let Some(lang) = tray_manager.target_language_for_menu_id(&menu_id) {
                        tray_manager.set_target_language(&lang);
                        *target_language.lock() = lang.clone();
                        let is_translating = lang != "original";
                        info!("Target language: {} (translate={})", lang, is_translating);
                        if let Ok(mut cfg) = config::Config::load() {
                            cfg.target_language = lang;
                            cfg.translate_mode = is_translating;
                            let _ = cfg.save();
                        }
                    } else if menu_id == exit_id {
                        info!("Exiting...");
                        // Stop always-listen
                        always_listen_active.store(false, Ordering::SeqCst);
                        always_listen_stream_running.store(false, Ordering::SeqCst);
                        always_listen_stream = None;
                        // Save window positions before exit
                        if let Err(e) = save_window_positions(
                            overlay.get_position(),
                            subtitle_bar.get_position(),
                            subtitle_bar.get_size(),
                            subtitle_bar.is_visible(),
                        ) {
                            error!("Failed to save config: {}", e);
                        }
                        running.store(false, Ordering::SeqCst);
                        *control_flow = ControlFlow::Exit;
                    }
                }
                UserEvent::TranscriptionComplete(target_status) => {
                    let mode = *state.lock();
                    if mode == AppMode::Processing {
                        // Return to previous state
                        if target_status == AppStatus::AlwaysListening {
                            *state.lock() = AppMode::AlwaysListening;
                            tray_manager.set_status(AppStatus::AlwaysListening);
                            overlay.set_status(AppStatus::AlwaysListening);
                        } else {
                            *state.lock() = AppMode::Idle;
                            tray_manager.set_status(AppStatus::Idle);
                            overlay.set_status(AppStatus::Idle);
                        }
                    }
                    info!("Ready for next recording");
                }
            },
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                if window_id == overlay.window_id() {
                    overlay.set_visible(false);
                } else if window_id == subtitle_bar.window_id() {
                    subtitle_bar.hide();
                    tray_manager.set_subtitle_visible(false);
                }
            }
            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                        ..
                    },
                window_id,
                ..
            } => {
                if window_id == overlay.window_id() {
                    overlay.start_drag();
                } else if window_id == subtitle_bar.window_id() {
                    subtitle_bar.start_drag();
                }
            }
            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Right,
                        ..
                    },
                window_id,
                ..
            } => {
                if window_id == overlay.window_id() {
                    #[cfg(target_os = "windows")]
                    {
                        let hwnd = HWND(overlay.hwnd() as *mut std::ffi::c_void);
                        if let Some(cmd) = show_overlay_context_menu(hwnd) {
                            match cmd {
                                MENU_SHOW_OVERLAY => {
                                    overlay.toggle_visibility();
                                }
                                MENU_SETTINGS => {
                                    // Save state, launch setup wizard, and exit
                                    info!("Opening settings from overlay (restarting into setup mode)...");
                                    // Stop always-listen
                                    always_listen_active.store(false, Ordering::SeqCst);
                                    always_listen_stream_running.store(false, Ordering::SeqCst);
                                    always_listen_stream = None;
                                    if let Err(e) = save_window_positions(
                                        overlay.get_position(),
                                        subtitle_bar.get_position(),
                                        subtitle_bar.get_size(),
                                        subtitle_bar.is_visible(),
                                    ) {
                                        error!("Failed to save config: {}", e);
                                    }
                                    // Spawn setup wizard and exit current process
                                    if let Ok(exe) = std::env::current_exe() {
                                        let _ = std::process::Command::new(exe)
                                            .arg("--setup")
                                            .spawn();
                                    }
                                    running.store(false, Ordering::SeqCst);
                                    *control_flow = ControlFlow::Exit;
                                }
                                MENU_EXIT => {
                                    info!("Exiting from overlay menu...");
                                    // Stop always-listen
                                    always_listen_active.store(false, Ordering::SeqCst);
                                    always_listen_stream_running.store(false, Ordering::SeqCst);
                                    always_listen_stream = None;
                                    if let Err(e) = save_window_positions(
                                        overlay.get_position(),
                                        subtitle_bar.get_position(),
                                        subtitle_bar.get_size(),
                                        subtitle_bar.is_visible(),
                                    ) {
                                        error!("Failed to save config: {}", e);
                                    }
                                    running.store(false, Ordering::SeqCst);
                                    *control_flow = ControlFlow::Exit;
                                }
                                _ => {}
                            }
                        }
                    }
                } else if window_id == subtitle_bar.window_id() {
                    sub_ctx_menu.show_context_menu_for_hwnd(subtitle_bar.hwnd(), None);
                }
            }
            Event::RedrawRequested(window_id) => {
                if window_id == overlay.window_id() {
                    overlay.handle_redraw();
                } else if window_id == subtitle_bar.window_id() {
                    subtitle_bar.handle_redraw();
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                window_id,
                ..
            } => {
                if window_id == subtitle_bar.window_id() {
                    subtitle_bar.handle_redraw();
                }
            }
            _ => {}
        }
    });
}

fn sanitize_overlay_position(x: i32, y: i32) -> Option<(i32, i32)> {
    // Windows reports minimized windows around (-32000, -32000); never persist that.
    if x <= -30_000 || y <= -30_000 {
        None
    } else {
        Some((x, y))
    }
}

fn save_window_positions(
    overlay_pos: (i32, i32),
    subtitle_pos: (i32, i32),
    subtitle_size: (u32, u32),
    subtitle_visible: bool,
) -> Result<()> {
    let mut cfg = Config::load()?;
    if let Some((x, y)) = sanitize_overlay_position(overlay_pos.0, overlay_pos.1) {
        cfg.overlay_x = Some(x);
        cfg.overlay_y = Some(y);
    } else {
        cfg.overlay_x = None;
        cfg.overlay_y = None;
    }
    if let Some((x, y)) = sanitize_overlay_position(subtitle_pos.0, subtitle_pos.1) {
        cfg.subtitle_x = Some(x);
        cfg.subtitle_y = Some(y);
    } else {
        cfg.subtitle_x = None;
        cfg.subtitle_y = None;
    }
    cfg.subtitle_width = Some(subtitle_size.0);
    cfg.subtitle_height = Some(subtitle_size.1);
    cfg.subtitle_visible = subtitle_visible;
    cfg.save()
}

#[derive(Debug, Clone)]
enum UserEvent {
    Hotkey(HotkeyAction),
    Menu(tray_icon::menu::MenuId),
    TranscriptionComplete(AppStatus),
    AlwaysListenAudio(always_listen::AlwaysListenResult),
    AlwaysListenStateChange(bool), // true = recording, false = listening
    SubtitleText(String),
    SubtitleReplace(String),
    SetSubtitleFont(String),
    SetSubtitleFontSize(u32),
}
