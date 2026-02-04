// Windows subsystem disabled in debug builds for console output
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod always_listen;
mod audio;
mod backend_loader;
mod config;
mod downloader;
mod hotkeys;
mod overlay;
mod setup;
mod tray;
mod typer;

use anyhow::Result;
use backend_loader::LoadedBackend;
use config::{get_exe_stem, setup_cuda_env, Config};
use cpal::traits::StreamTrait;
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
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;

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

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    let _instance_lock = {
        let lock = acquire_instance_lock()?;
        if lock.is_none() {
            show_error_dialog(
                "Already Running",
                "Another instance with the same executable name is already running.",
            );
            return Ok(());
        }
        // Keep lock alive for the lifetime of the process.
        lock
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

/// Transcription worker that processes audio and types the result
fn transcribe_and_type(
    audio_data: Vec<f32>,
    model: Arc<backend_loader::Model>,
    typer: Arc<Mutex<typer::Typer>>,
    _state: Arc<Mutex<AppMode>>,
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
    app_status: AppStatus,
) {
    std::thread::spawn(move || {
        info!(
            "Transcribing {} samples (~{:.1}s of audio)...",
            audio_data.len(),
            audio_data.len() as f32 / 16000.0
        );

        match model.transcribe(&audio_data) {
            Ok(text) => {
                if !text.is_empty() {
                    info!("Result: \"{}\"", text);
                    info!("Typing into active window...");
                    if let Err(e) = typer.lock().type_text(&text) {
                        error!("Failed to type: {}", e);
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

fn run_app(mut config: Config) -> Result<()> {
    // Set up CUDA environment if GPU is enabled
    setup_cuda_env(&config);

    // Initialize audio capture
    let audio_capture = match audio::AudioCapture::new_with_device(config.input_device_name.as_deref()) {
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
    };

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

    // Log model input state before creation
    info!(
        "Model load request (path={}, use_gpu={}, backend_cuda={})",
        config.model_path.display(),
        config.use_gpu,
        backend.supports_cuda_runtime()
    );

    for filename in [
        "model.bin",
        "config.json",
        "preprocessor_config.json",
        "tokenizer.json",
        "vocabulary.txt",
    ] {
        let path = config.model_path.join(filename);
        info!("Model file check: {} exists={}", path.display(), path.exists());
    }

    // Create model (with GPU->CPU fallback)
    let model = match backend.create_model(&config.model_path, config.use_gpu) {
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
                match backend.create_model(&config.model_path, false) {
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
                                config.model_path.display(),
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
                        config.model_path.display(),
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
    let mut tray_manager = match tray::TrayManager::new() {
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
    let settings_id = tray_manager.settings_id.clone();
    let exit_id = tray_manager.exit_id.clone();

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

    info!("Overlay window created");
    info!("System tray icon created");
    info!("========================================");
    info!("  READY!");
    info!("  - Right-click tray icon for menu");
    info!("========================================");

    // App state
    let state = Arc::new(Mutex::new(AppMode::Idle));
    let running = Arc::new(AtomicBool::new(true));

    // Always-listen state
    let always_listen_active = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<f32>>(100);
    let (result_tx, _result_rx) = crossbeam_channel::bounded::<Vec<f32>>(10);

    // Spawn always-listen processing thread
    let always_listen_running = Arc::clone(&running);
    let always_listen_active_thread = Arc::clone(&always_listen_active);
    let al_proxy = proxy.clone();
    
    std::thread::spawn(move || {
        use always_listen::{AlwaysListenConfig, AlwaysListenController, AlwaysListenState};
        
        let config = AlwaysListenConfig::default();
        let controller = AlwaysListenController::new(config, audio_rx, result_tx);
        
        while always_listen_running.load(Ordering::SeqCst) {
            // Only process when always-listen is active
            if always_listen_active_thread.load(Ordering::SeqCst) {
                if controller.state() == AlwaysListenState::Paused {
                    let _ = controller.start();
                }
                
                // Check for transcription results
                if let Some(audio_data) = controller.try_recv_result() {
                    debug!("Received {} samples from always-listen", audio_data.len());
                    
                    // Send event to main thread for transcription
                    let _ = al_proxy.send_event(UserEvent::AlwaysListenAudio(audio_data));
                }
            } else {
                if controller.state() != AlwaysListenState::Paused {
                    let _ = controller.stop();
                }
            }
            
            std::thread::sleep(Duration::from_millis(10));
        }
        
        let _ = controller.stop();
    });

    // Create always-listen audio stream (will be started/stopped based on always_listen_active)
    let always_listen_stream_running = Arc::new(AtomicBool::new(false));
    let al_stream_running = Arc::clone(&always_listen_stream_running);
    let al_stream_audio_tx = audio_tx.clone();
    
    let always_listen_stream = match audio_capture.lock().create_always_listen_stream(
        al_stream_audio_tx,
        al_stream_running,
    ) {
        Ok(stream) => {
            info!("Always-listen audio stream created");
            Some(stream)
        }
        Err(e) => {
            error!("Failed to create always-listen audio stream: {}", e);
            None
        }
    };

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
    std::thread::spawn(move || {
        while running_menu.load(Ordering::SeqCst) {
            if let Ok(event) = menu_receiver.recv_timeout(Duration::from_millis(100)) {
                let _ = proxy_menu.send_event(UserEvent::Menu(event.id));
            }
        }
    });

    // Clone for event loop
    let always_listen_stream_for_loop = always_listen_stream;
    let always_listen_stream_running_for_loop = always_listen_stream_running;

    // Run event loop
    event_loop.run(move |event, _, control_flow| {
        // Rename for convenience in the loop
        let always_listen_stream = &always_listen_stream_for_loop;
        let always_listen_stream_running = &always_listen_stream_running_for_loop;
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(user_event) => match user_event {
                UserEvent::Hotkey(action) => {
                    let mut mode = state.lock();
                    match action {
                        HotkeyAction::PushToTalk => match *mode {
                            AppMode::Idle => {
                                // Start recording
                                info!("RECORDING... (press hotkey to stop)");
                                if let Err(e) = audio_capture.lock().start_recording() {
                                    error!("Failed to start recording: {}", e);
                                    return;
                                }
                                *mode = AppMode::Recording;
                                tray_manager.set_status(AppStatus::Recording);
                                overlay.set_status(AppStatus::Recording);
                            }
                            AppMode::Recording => {
                                // Stop recording and transcribe
                                info!("Stopped. Processing...");
                                let audio_data = audio_capture.lock().stop_recording();

                                *mode = AppMode::Processing;
                                drop(mode);

                                // Transcribe in background
                                transcribe_and_type(
                                    audio_data,
                                    Arc::clone(&model),
                                    Arc::clone(&typer),
                                    Arc::clone(&state),
                                    proxy.clone(),
                                    AppStatus::Idle,
                                );
                            }
                            AppMode::Processing => {
                                info!("Still processing, please wait...");
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
                        },
                        HotkeyAction::AlwaysListen => {
                            // Toggle always-listen mode
                            match *mode {
                                AppMode::Idle => {
                                    info!("Starting always-listen mode...");
                                    always_listen_active.store(true, Ordering::SeqCst);
                                    always_listen_stream_running.store(true, Ordering::SeqCst);
                                    // Start the audio stream if available
                                    if let Some(ref stream) = always_listen_stream {
                                        if let Err(e) = stream.play() {
                                            error!("Failed to start always-listen audio stream: {}", e);
                                            always_listen_active.store(false, Ordering::SeqCst);
                                            always_listen_stream_running.store(false, Ordering::SeqCst);
                                            return;
                                        }
                                    }
                                    *mode = AppMode::AlwaysListening;
                                    tray_manager.set_status(AppStatus::AlwaysListening);
                                    overlay.set_status(AppStatus::AlwaysListening);
                                }
                                AppMode::AlwaysListening => {
                                    info!("Stopping always-listen mode...");
                                    always_listen_active.store(false, Ordering::SeqCst);
                                    always_listen_stream_running.store(false, Ordering::SeqCst);
                                    // Pause the audio stream
                                    if let Some(ref stream) = always_listen_stream {
                                        let _ = stream.pause();
                                    }
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
                UserEvent::AlwaysListenAudio(audio_data) => {
                    // Handle always-listen audio for transcription
                    *state.lock() = AppMode::Processing;
                    tray_manager.set_status(AppStatus::Processing);
                    overlay.set_status(AppStatus::Processing);

                    // Transcribe the audio
                    transcribe_and_type(
                        audio_data,
                        Arc::clone(&model),
                        Arc::clone(&typer),
                        Arc::clone(&state),
                        proxy.clone(),
                        AppStatus::AlwaysListening,
                    );
                }
                UserEvent::Menu(menu_id) => {
                    if menu_id == show_overlay_id {
                        overlay.toggle_visibility();
                    } else if menu_id == settings_id {
                        // Save state and launch setup wizard
                        info!("Opening settings...");
                        let (x, y) = overlay.get_position();
                        config.overlay_x = Some(x);
                        config.overlay_y = Some(y);
                        if let Err(e) = config.save() {
                            error!("Failed to save config: {}", e);
                        }
                        // Stop always-listen before launching setup
                        always_listen_active.store(false, Ordering::SeqCst);
                        always_listen_stream_running.store(false, Ordering::SeqCst);
                        if let Some(ref stream) = always_listen_stream {
                            let _ = stream.pause();
                        }
                        // Launch setup wizard (will restart app - this never returns)
                        setup::run_setup();
                    } else if menu_id == exit_id {
                        info!("Exiting...");
                        // Stop always-listen
                        always_listen_active.store(false, Ordering::SeqCst);
                        always_listen_stream_running.store(false, Ordering::SeqCst);
                        if let Some(ref stream) = always_listen_stream {
                            let _ = stream.pause();
                        }
                        // Save overlay position before exit
                        let (x, y) = overlay.get_position();
                        config.overlay_x = Some(x);
                        config.overlay_y = Some(y);
                        if let Err(e) = config.save() {
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
                }
            }
            Event::RedrawRequested(window_id) => {
                if window_id == overlay.window_id() {
                    overlay.handle_redraw();
                }
            }
            _ => {}
        }
    });
}

#[derive(Debug, Clone)]
enum UserEvent {
    Hotkey(HotkeyAction),
    Menu(tray_icon::menu::MenuId),
    TranscriptionComplete(AppStatus),
    AlwaysListenAudio(Vec<f32>),
}
