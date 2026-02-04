use crate::backend_loader::{discover_backends, get_backends_dir, BackendManifest, ManifestModel};
use crate::config::{detect_cuda_path, detect_cudnn_path, get_models_dir, validate_cuda_path, validate_cudnn_path, Config};
use crate::downloader::{self, DownloadProgress};
use cpal::traits::HostTrait;
use image::GenericImageView;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use tao::dpi::LogicalSize;
use tao::event::{ElementState, Event, MouseButton, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::keyboard::{KeyCode, ModifiersState};
use tao::window::{Icon, WindowBuilder};

const WINDOW_WIDTH: u32 = 500;
const WINDOW_HEIGHT: u32 = 500;
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/mic_gray.png");

// Colors
const BG_COLOR: u32 = 0xFF1a1a2e;
const HEADER_BG: u32 = 0xFF16213e;
const TEXT_COLOR: u32 = 0xFFe8e8e8;
const DIM_TEXT: u32 = 0xFF888888;
const ACCENT_COLOR: u32 = 0xFF4a9eff;
const BUTTON_COLOR: u32 = 0xFF2d4a6f;
const BUTTON_HOVER: u32 = 0xFF3d5a8f;
const SELECTED_COLOR: u32 = 0xFF0f3460;
const PROGRESS_BG: u32 = 0xFF2a2a4a;
const PROGRESS_FG: u32 = 0xFF4ade80;
const FIELD_BG: u32 = 0xFF252545;
const CAPTURE_BG: u32 = 0xFF1a3a5a;

// Pages in the setup wizard
#[derive(Debug, Clone, PartialEq)]
enum SetupPage {
    Home,
    ModelSelection,
    HotkeyConfig(HotkeyTarget),
    CudaConfig,
    AudioConfig,
}

/// Unified model entry combining backend and model info
#[derive(Debug, Clone)]
struct UnifiedModel {
    backend_id: String,
    backend_name: String,
    model: ManifestModel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HotkeyTarget {
    PushToTalk,
    ToggleListening,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HotkeyCapture {
    Idle,
    WaitingForKey,
}

struct SetupState {
    current_page: SetupPage,

    // Backend info (for looking up DLL paths, etc.)
    available_backends: Vec<BackendManifest>,

    // Unified model list (all models from all backends)
    all_models: Vec<UnifiedModel>,
    selected_model: Option<usize>,
    model_scroll_offset: usize,
    // Audio input devices
    input_devices: Vec<String>,
    selected_input_device: Option<String>,
    device_scroll_offset: usize,

    // Auto-selected backend (based on model choice)
    selected_backend_id: Option<String>,

    // Hotkey configuration
    push_to_talk_hotkey: Option<String>,
    toggle_listening_hotkey: Option<String>,
    hotkey_capture: HotkeyCapture,
    captured_key: Option<String>,
    current_modifiers: ModifiersState,

    // GPU/CUDA settings
    use_gpu: bool,
    cuda_path: Option<std::path::PathBuf>,
    cudnn_path: Option<std::path::PathBuf>,
    cuda_valid: bool,
    cudnn_valid: bool,

    // Download state
    status: String,
    download_progress: Option<Arc<DownloadProgress>>,
    model_downloaded: bool,
    // Overlay settings (persisted from config)
    overlay_visible: bool,
    overlay_x: Option<i32>,
    overlay_y: Option<i32>,

    // UI state
    hovered_button: Option<Button>,
    mouse_pos: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Button {
    // Home page
    SelectModel,
    ConfigureMic,
    ConfigurePushToTalk,
    ConfigureToggleListen,
    GpuToggle,
    ConfigureCuda,
    Start,

    // Model selection page
    Model(usize),
    Download,
    OpenLink,
    ModelScrollUp,
    ModelScrollDown,
    Back,

    // Hotkey config page
    SetHotkey,
    ConfirmHotkey,
    ClearHotkey,

    // CUDA config page
    DetectCuda,
    BrowseCuda,
    BrowseCudnn,

    // Audio config page
    Device(usize),
    DeviceScrollUp,
    DeviceScrollDown,
    ConfirmDevice,
}

struct ButtonRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    button: Button,
}

const VISIBLE_MODELS: usize = 6;
const VISIBLE_DEVICES: usize = 6;
const DEFAULT_DEVICE_LABEL: &str = "<Default device>";

impl SetupState {
    fn new() -> Self {
        let existing_config = Config::load().ok();

        // Load audio input devices
        let mut input_devices: Vec<String> = Vec::new();
        input_devices.push(DEFAULT_DEVICE_LABEL.to_string());
        if let Ok(mut devices) = cpal::default_host().input_devices() {
            for dev in devices.by_ref() {
                if let Ok(name) = dev.name() {
                    input_devices.push(name);
                }
            }
        }

        let selected_input_device = existing_config
            .as_ref()
            .and_then(|c| c.input_device_name.clone());

        // Load available backends
        let available_backends: Vec<BackendManifest> = if let Ok(backends_dir) = get_backends_dir() {
            let backend_paths = discover_backends(&backends_dir);
            backend_paths
                .iter()
                .filter_map(|p| {
                    BackendManifest::load(&p.join("manifest.json")).ok()
                })
                .collect()
        } else {
            Vec::new()
        };

        // Create unified model list from all backends
        let mut all_models: Vec<UnifiedModel> = Vec::new();
        for backend in &available_backends {
            for model in &backend.models {
                all_models.push(UnifiedModel {
                    backend_id: backend.id.clone(),
                    backend_name: backend.display_name.clone(),
                    model: model.clone(),
                });
            }
        }

        // Resolve saved model selection from config (if any).
        let mut selected_model: Option<usize> = None;
        let mut selected_backend_id: Option<String> = None;
        if let Some(ref cfg) = existing_config {
            if let Some(idx) = all_models.iter().position(|u| {
                u.backend_id == cfg.backend_id && u.model.id == cfg.model_name
            }) {
                selected_model = Some(idx);
                selected_backend_id = Some(cfg.backend_id.clone());
            } else if let Some(model_folder) = cfg.model_path.file_name().and_then(|n| n.to_str()) {
                if let Some(idx) = all_models.iter().position(|u| u.model.folder_name == model_folder) {
                    selected_model = Some(idx);
                    selected_backend_id = Some(all_models[idx].backend_id.clone());
                }
            }
        }

        let model_downloaded = selected_model
            .and_then(|idx| all_models.get(idx))
            .map(is_unified_model_downloaded)
            .unwrap_or(false);

        let status = if selected_model.is_some() && model_downloaded {
            "Model ready! Click Start.".to_string()
        } else if selected_model.is_some() {
            "Model selected. Click Download.".to_string()
        } else {
            "Select a model to get started".to_string()
        };

        // Load saved GPU settings if available, otherwise auto-detect.
        let use_gpu = existing_config.as_ref().map(|c| c.use_gpu).unwrap_or(false);
        let cuda_path = existing_config
            .as_ref()
            .and_then(|c| c.cuda_path.clone())
            .or_else(detect_cuda_path);
        let cudnn_path = existing_config
            .as_ref()
            .and_then(|c| c.cudnn_path.clone())
            .or_else(detect_cudnn_path);
        let cuda_valid = cuda_path.as_ref().map(|p| validate_cuda_path(p)).unwrap_or(false);
        let cudnn_valid = cudnn_path.as_ref().map(|p| validate_cudnn_path(p)).unwrap_or(false);

        Self {
            current_page: SetupPage::Home,
            available_backends,
            all_models,
            selected_model,
            model_scroll_offset: 0,
            selected_backend_id,
            input_devices,
            selected_input_device,
            device_scroll_offset: 0,
            push_to_talk_hotkey: Some(
                existing_config
                    .as_ref()
                    .map(|c| c.hotkey_push_to_talk.clone())
                    .unwrap_or_else(|| "Backquote".to_string()),
            ),
            toggle_listening_hotkey: Some(
                existing_config
                    .as_ref()
                    .map(|c| c.hotkey_always_listen.clone())
                    .unwrap_or_else(|| "Control+Backquote".to_string()),
            ),
            hotkey_capture: HotkeyCapture::Idle,
            captured_key: None,
            current_modifiers: ModifiersState::default(),
            use_gpu,
            cuda_path,
            cudnn_path,
            cuda_valid,
            cudnn_valid,
            status,
            download_progress: None,
            model_downloaded,
            overlay_visible: existing_config
                .as_ref()
                .map(|c| c.overlay_visible)
                .unwrap_or(true),
            overlay_x: existing_config.as_ref().and_then(|c| c.overlay_x),
            overlay_y: existing_config.as_ref().and_then(|c| c.overlay_y),
            hovered_button: None,
            mouse_pos: (0.0, 0.0),
        }
    }

    fn selected_unified_model(&self) -> Option<&UnifiedModel> {
        self.selected_model.map(|idx| &self.all_models[idx])
    }

    fn selected_model_info(&self) -> Option<&ManifestModel> {
        self.selected_unified_model().map(|u| &u.model)
    }

    fn get_backend_display_name(&self, backend_id: &str) -> Option<&str> {
        self.available_backends
            .iter()
            .find(|b| b.id == backend_id)
            .map(|b| b.display_name.as_str())
    }

    #[allow(dead_code)]
    fn selected_backend_info(&self) -> Option<&BackendManifest> {
        self.selected_backend_id.as_ref().and_then(|id| {
            self.available_backends.iter().find(|b| &b.id == id)
        })
    }

    fn check_model_exists(&self) -> bool {
        if let (Some(unified), Ok(models_dir)) =
            (self.selected_unified_model(), get_models_dir())
        {
            let model_folder = models_dir.join(&unified.model.folder_name);
            // Check if last file in the list exists
            if let Some(last_file) = unified.model.files.last() {
                model_folder.join(last_file).exists()
            } else {
                model_folder.exists()
            }
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn get_current_hotkey(&self, target: HotkeyTarget) -> Option<&String> {
        match target {
            HotkeyTarget::PushToTalk => self.push_to_talk_hotkey.as_ref(),
            HotkeyTarget::ToggleListening => self.toggle_listening_hotkey.as_ref(),
        }
    }

    fn set_hotkey(&mut self, target: HotkeyTarget, key: Option<String>) {
        match target {
            HotkeyTarget::PushToTalk => self.push_to_talk_hotkey = key,
            HotkeyTarget::ToggleListening => self.toggle_listening_hotkey = key,
        }
    }
}

/// Check if a unified model is downloaded
fn is_unified_model_downloaded(unified: &UnifiedModel) -> bool {
    if let Ok(models_dir) = get_models_dir() {
        let model_folder = models_dir.join(&unified.model.folder_name);
        // Check if last file in the list exists
        if let Some(last_file) = unified.model.files.last() {
            model_folder.join(last_file).exists()
        } else {
            model_folder.exists()
        }
    } else {
        false
    }
}

fn load_window_icon() -> Option<Icon> {
    let img = image::load_from_memory(WINDOW_ICON_PNG).ok()?;
    let img = img.resize_exact(32, 32, image::imageops::FilterType::Lanczos3);
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8().into_raw();
    Icon::from_rgba(rgba, width, height).ok()
}

/// Run the setup wizard. This function never returns - it either:
/// 1. Spawns a new process with the config and exits, or
/// 2. User closes the window and exits
pub fn run_setup() -> ! {
    let event_loop = EventLoopBuilder::<SetupEvent>::with_user_event().build();
    let window_icon = load_window_icon();

    let window = WindowBuilder::new()
        .with_title("Speech-to-Text Setup")
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_min_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_window_icon(window_icon)
        .with_resizable(true)
        .build(&event_loop)
        .expect("Failed to create setup window");

    let window = Rc::new(window);
    let context =
        softbuffer::Context::new(window.clone()).expect("Failed to create softbuffer context");
    let mut surface =
        softbuffer::Surface::new(&context, window.clone()).expect("Failed to create softbuffer surface");

    let mut state = SetupState::new();

    let proxy = event_loop.create_proxy();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        // Check download progress
        if let Some(ref progress) = state.download_progress {
            if progress.is_finished() {
                if let Some(err) = progress.get_error() {
                    state.status = format!("Download failed: {}", err);
                } else {
                    state.status = "Download complete!".to_string();
                    state.model_downloaded = true;
                }
                state.download_progress = None;
                window.request_redraw();
            } else {
                let (downloaded, total) = progress.get_progress();
                let (current_file, total_files) = progress.get_file_progress();
                if total > 0 {
                    let percent = (downloaded as f64 / total as f64 * 100.0) as u32;
                    let mb_downloaded = downloaded as f64 / 1_000_000.0;
                    let mb_total = total as f64 / 1_000_000.0;
                    state.status = format!(
                        "File {}/{}: {:.1}/{:.1} MB ({}%)",
                        current_file, total_files, mb_downloaded, mb_total, percent
                    );
                } else {
                    state.status = format!("Downloading file {}/{}...", current_file, total_files);
                }
                window.request_redraw();
            }
        }

        match event {
            Event::UserEvent(SetupEvent::Exit(_config)) => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(modifiers),
                ..
            } => {
                state.current_modifiers = modifiers;
            }
            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event: key_event, .. },
                ..
            } => {
                // Handle hotkey capture
                if state.hotkey_capture == HotkeyCapture::WaitingForKey {
                    if key_event.state == ElementState::Pressed {
                        use tao::keyboard::Key;
                        
                        // Build modifier prefix first
                        let mut parts = Vec::new();
                        if state.current_modifiers.control_key() {
                            parts.push("Control".to_string());
                        }
                        if state.current_modifiers.alt_key() {
                            parts.push("Alt".to_string());
                        }
                        if state.current_modifiers.shift_key() {
                            parts.push("Shift".to_string());
                        }
                        if state.current_modifiers.super_key() {
                            parts.push("Super".to_string());
                        }
                        
                        // Get key name based on the Key variant
                        let key_name = match &key_event.logical_key {
                            Key::Character(c) => c.to_uppercase().to_string(),
                            Key::Enter => "Enter".to_string(),
                            Key::Tab => "Tab".to_string(),
                            Key::Space => "Space".to_string(),
                            Key::Backspace => "Backspace".to_string(),
                            Key::Escape => "Escape".to_string(),
                            Key::ArrowUp => "ArrowUp".to_string(),
                            Key::ArrowDown => "ArrowDown".to_string(),
                            Key::ArrowLeft => "ArrowLeft".to_string(),
                            Key::ArrowRight => "ArrowRight".to_string(),
                            Key::Home => "Home".to_string(),
                            Key::End => "End".to_string(),
                            Key::PageUp => "PageUp".to_string(),
                            Key::PageDown => "PageDown".to_string(),
                            Key::Insert => "Insert".to_string(),
                            Key::Delete => "Delete".to_string(),
                            Key::CapsLock => "CapsLock".to_string(),
                            Key::F1 => "F1".to_string(),
                            Key::F2 => "F2".to_string(),
                            Key::F3 => "F3".to_string(),
                            Key::F4 => "F4".to_string(),
                            Key::F5 => "F5".to_string(),
                            Key::F6 => "F6".to_string(),
                            Key::F7 => "F7".to_string(),
                            Key::F8 => "F8".to_string(),
                            Key::F9 => "F9".to_string(),
                            Key::F10 => "F10".to_string(),
                            Key::F11 => "F11".to_string(),
                            Key::F12 => "F12".to_string(),
                            // Modifier keys - ignore them as standalone keys
                            Key::Control | Key::Shift | Key::Alt | Key::Super | Key::AltGraph => return,
                            // Ignore other keys we don't handle
                            _ => return,
                        };
                        
                        parts.push(key_name);
                        let key_str = parts.join("+");
                        
                        eprintln!("DEBUG: Captured hotkey: {}", key_str);
                        state.captured_key = Some(key_str);
                        state.hotkey_capture = HotkeyCapture::Idle;
                        window.request_redraw();
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } => {
                state.mouse_pos = (position.x, position.y);
                let buttons = get_button_rects(&state);
                let old_hover = state.hovered_button;
                state.hovered_button = None;
                for btn in &buttons {
                    if is_inside(state.mouse_pos, btn) {
                        state.hovered_button = Some(btn.button);
                        break;
                    }
                }
                if old_hover != state.hovered_button {
                    window.request_redraw();
                }
            }
            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta, .. },
                ..
            } => {
                // Handle scroll on model selection page
                if state.current_page == SetupPage::ModelSelection {
                    let scroll_amount = match delta {
                        tao::event::MouseScrollDelta::LineDelta(_, y) => -y as i32,
                        tao::event::MouseScrollDelta::PixelDelta(pos) => -(pos.y / 20.0) as i32,
                        _ => 0,
                    };
                    let model_count = state.all_models.len();
                    let new_offset = (state.model_scroll_offset as i32 + scroll_amount)
                        .max(0)
                        .min((model_count.saturating_sub(VISIBLE_MODELS)) as i32);
                    state.model_scroll_offset = new_offset as usize;
                    window.request_redraw();
                } else if state.current_page == SetupPage::AudioConfig {
                    let scroll_amount = match delta {
                        tao::event::MouseScrollDelta::LineDelta(_, y) => -y as i32,
                        tao::event::MouseScrollDelta::PixelDelta(pos) => -(pos.y / 20.0) as i32,
                        _ => 0,
                    };
                    let device_count = state.input_devices.len();
                    let new_offset = (state.device_scroll_offset as i32 + scroll_amount)
                        .max(0)
                        .min((device_count.saturating_sub(VISIBLE_DEVICES)) as i32);
                    state.device_scroll_offset = new_offset as usize;
                    window.request_redraw();
                }
            }
            Event::WindowEvent {
                event:
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                        ..
                    },
                ..
            } => {
                let buttons = get_button_rects(&state);
                for btn in &buttons {
                    if is_inside(state.mouse_pos, btn) {
                        let old_capture = state.hotkey_capture.clone();
                        if let Some(config) = handle_click(&mut state, btn.button) {
                            let _ = proxy.send_event(SetupEvent::Exit(config));
                        }
                        // Request focus when entering hotkey capture mode
                        if old_capture != HotkeyCapture::WaitingForKey && state.hotkey_capture == HotkeyCapture::WaitingForKey {
                            window.set_focus();
                        }
                        window.request_redraw();
                        break;
                    }
                }
            }
            Event::RedrawRequested(_) => {
                let size = window.inner_size();
                if let (Some(width), Some(height)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    surface.resize(width, height).ok();
                    if let Ok(mut buffer) = surface.buffer_mut() {
                        render(&state, &mut buffer, size.width, size.height);
                        buffer.present().ok();
                    }
                }
            }
            Event::MainEventsCleared => {
                if state.download_progress.is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                window.request_redraw();
            }
            _ => {}
        }
    })
    // event_loop.run() never returns
}

#[derive(Debug, Clone)]
enum SetupEvent {
    Exit(Config),
}

#[allow(dead_code)]
fn is_modifier_key(keycode: KeyCode) -> bool {
    matches!(keycode,
        KeyCode::ControlLeft | KeyCode::ControlRight |
        KeyCode::ShiftLeft | KeyCode::ShiftRight |
        KeyCode::AltLeft | KeyCode::AltRight |
        KeyCode::SuperLeft | KeyCode::SuperRight
    )
}

#[allow(dead_code)]
fn keycode_to_string(keycode: KeyCode, modifiers: &ModifiersState) -> String {
    let mut parts = Vec::new();

    if modifiers.control_key() {
        parts.push("Control");
    }
    if modifiers.alt_key() {
        parts.push("Alt");
    }
    if modifiers.shift_key() {
        parts.push("Shift");
    }
    if modifiers.super_key() {
        parts.push("Super");
    }

    let key_name = match keycode {
        KeyCode::Backquote => "Backquote",
        KeyCode::Digit1 => "Digit1",
        KeyCode::Digit2 => "Digit2",
        KeyCode::Digit3 => "Digit3",
        KeyCode::Digit4 => "Digit4",
        KeyCode::Digit5 => "Digit5",
        KeyCode::Digit6 => "Digit6",
        KeyCode::Digit7 => "Digit7",
        KeyCode::Digit8 => "Digit8",
        KeyCode::Digit9 => "Digit9",
        KeyCode::Digit0 => "Digit0",
        KeyCode::KeyA => "KeyA",
        KeyCode::KeyB => "KeyB",
        KeyCode::KeyC => "KeyC",
        KeyCode::KeyD => "KeyD",
        KeyCode::KeyE => "KeyE",
        KeyCode::KeyF => "KeyF",
        KeyCode::KeyG => "KeyG",
        KeyCode::KeyH => "KeyH",
        KeyCode::KeyI => "KeyI",
        KeyCode::KeyJ => "KeyJ",
        KeyCode::KeyK => "KeyK",
        KeyCode::KeyL => "KeyL",
        KeyCode::KeyM => "KeyM",
        KeyCode::KeyN => "KeyN",
        KeyCode::KeyO => "KeyO",
        KeyCode::KeyP => "KeyP",
        KeyCode::KeyQ => "KeyQ",
        KeyCode::KeyR => "KeyR",
        KeyCode::KeyS => "KeyS",
        KeyCode::KeyT => "KeyT",
        KeyCode::KeyU => "KeyU",
        KeyCode::KeyV => "KeyV",
        KeyCode::KeyW => "KeyW",
        KeyCode::KeyX => "KeyX",
        KeyCode::KeyY => "KeyY",
        KeyCode::KeyZ => "KeyZ",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Space => "Space",
        KeyCode::Tab => "Tab",
        KeyCode::CapsLock => "CapsLock",
        KeyCode::Escape => "Escape",
        KeyCode::Insert => "Insert",
        KeyCode::Delete => "Delete",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::ArrowUp => "ArrowUp",
        KeyCode::ArrowDown => "ArrowDown",
        KeyCode::ArrowLeft => "ArrowLeft",
        KeyCode::ArrowRight => "ArrowRight",
        KeyCode::Numpad0 => "Numpad0",
        KeyCode::Numpad1 => "Numpad1",
        KeyCode::Numpad2 => "Numpad2",
        KeyCode::Numpad3 => "Numpad3",
        KeyCode::Numpad4 => "Numpad4",
        KeyCode::Numpad5 => "Numpad5",
        KeyCode::Numpad6 => "Numpad6",
        KeyCode::Numpad7 => "Numpad7",
        KeyCode::Numpad8 => "Numpad8",
        KeyCode::Numpad9 => "Numpad9",
        KeyCode::NumpadAdd => "NumpadAdd",
        KeyCode::NumpadSubtract => "NumpadSubtract",
        KeyCode::NumpadMultiply => "NumpadMultiply",
        KeyCode::NumpadDivide => "NumpadDivide",
        KeyCode::NumpadEnter => "NumpadEnter",
        KeyCode::NumpadDecimal => "NumpadDecimal",
        _ => "Unknown",
    };

    parts.push(key_name);
    parts.join("+")
}

fn get_button_rects(state: &SetupState) -> Vec<ButtonRect> {
    match &state.current_page {
        SetupPage::Home => get_home_buttons(state),
        SetupPage::ModelSelection => get_model_page_buttons(state),
        SetupPage::HotkeyConfig(_) => get_hotkey_page_buttons(state),
        SetupPage::CudaConfig => get_cuda_page_buttons(state),
        SetupPage::AudioConfig => get_audio_page_buttons(state),
    }
}

fn get_home_buttons(state: &SetupState) -> Vec<ButtonRect> {
    let mut buttons = Vec::new();

    // Layout constants - MUST match render_home_page exactly!
    const FIELD_HEIGHT: u32 = 28;
    const ROW_SPACING: u32 = 50;
    const LABEL_FIELD_GAP: u32 = 15;
    
    // Match render_home_page positioning exactly
    // render_home_page: y starts at 65, does label, then y += LABEL_FIELD_GAP, then draws button
    let mut y: u32 = 65;

    // Backend section (no button - display only)
    y += LABEL_FIELD_GAP;  // y = 80 - backend field row
    y += ROW_SPACING;      // y = 130 - move to next row

    // Select Model button (at y=145 in render - after label gap)
    y += LABEL_FIELD_GAP;  // y = 145 - button row
    buttons.push(ButtonRect {
        x: 380,
        y,
        width: 90,
        height: FIELD_HEIGHT,
        button: Button::SelectModel,
    });
    y += ROW_SPACING;      // y = 195 - move to next row

    // Configure Mic button (at y=210 in render)
    y += LABEL_FIELD_GAP;  // y = 210 - button row
    buttons.push(ButtonRect {
        x: 380,
        y,
        width: 90,
        height: FIELD_HEIGHT,
        button: Button::ConfigureMic,
    });
    y += ROW_SPACING;      // y = 260 - move to next row

    // Configure Push-to-Talk button (at y=210 in render)
    y += LABEL_FIELD_GAP;  // y = 210 - button row
    buttons.push(ButtonRect {
        x: 380,
        y,
        width: 90,
        height: FIELD_HEIGHT,
        button: Button::ConfigurePushToTalk,
    });
    y += ROW_SPACING;      // y = 260 - move to next row

    // Configure Toggle Listen button (at y=275 in render)
    y += LABEL_FIELD_GAP;  // y = 275 - button row
    buttons.push(ButtonRect {
        x: 380,
        y,
        width: 90,
        height: FIELD_HEIGHT,
        button: Button::ConfigureToggleListen,
    });
    y += ROW_SPACING;      // y = 325 - move to next row

    // GPU toggle button (at y=390 in render)
    buttons.push(ButtonRect {
        x: 30,
        y,
        width: 250,
        height: FIELD_HEIGHT,
        button: Button::GpuToggle,
    });

    // Configure CUDA button (only if GPU enabled) - same row as GPU toggle
    if state.use_gpu {
        buttons.push(ButtonRect {
            x: 290,
            y,
            width: 90,
            height: FIELD_HEIGHT,
            button: Button::ConfigureCuda,
        });
    }

    // Start button - fixed position at bottom (matches render at y=440)
    buttons.push(ButtonRect {
        x: 175,
        y: 440,
        width: 150,
        height: 45,
        button: Button::Start,
    });

    buttons
}

fn get_cuda_page_buttons(_state: &SetupState) -> Vec<ButtonRect> {
    let mut buttons = Vec::new();

    // Back button
    buttons.push(ButtonRect {
        x: 400,
        y: 10,
        width: 80,
        height: 30,
        button: Button::Back,
    });

    // Browse CUDA button
    buttons.push(ButtonRect {
        x: 380,
        y: 90,
        width: 90,
        height: 28,
        button: Button::BrowseCuda,
    });

    // Browse cuDNN button
    buttons.push(ButtonRect {
        x: 380,
        y: 160,
        width: 90,
        height: 28,
        button: Button::BrowseCudnn,
    });

    // Auto-detect button
    buttons.push(ButtonRect {
        x: 175,
        y: 320,
        width: 150,
        height: 35,
        button: Button::DetectCuda,
    });

    buttons
}

fn get_audio_page_buttons(state: &SetupState) -> Vec<ButtonRect> {
    let mut buttons = Vec::new();

    // Back button
    buttons.push(ButtonRect {
        x: 400,
        y: 10,
        width: 80,
        height: 30,
        button: Button::Back,
    });

    // Confirm button
    buttons.push(ButtonRect {
        x: 300,
        y: 440,
        width: 150,
        height: 35,
        button: Button::ConfirmDevice,
    });

    // Scroll buttons
    buttons.push(ButtonRect {
        x: 450,
        y: 80,
        width: 30,
        height: 30,
        button: Button::DeviceScrollUp,
    });
    buttons.push(ButtonRect {
        x: 450,
        y: 360,
        width: 30,
        height: 30,
        button: Button::DeviceScrollDown,
    });

    // Device list buttons
    let start_y: u32 = 110;
    for i in 0..VISIBLE_DEVICES {
        let device_idx = state.device_scroll_offset + i;
        if device_idx >= state.input_devices.len() {
            break;
        }
        buttons.push(ButtonRect {
            x: 30,
            y: start_y + (i as u32 * 45),
            width: 400,
            height: 35,
            button: Button::Device(device_idx),
        });
    }

    buttons
}

fn get_model_page_buttons(state: &SetupState) -> Vec<ButtonRect> {
    let mut buttons = Vec::new();

    // Back button
    buttons.push(ButtonRect {
        x: 400,
        y: 10,
        width: 80,
        height: 30,
        button: Button::Back,
    });

    // Get model count from unified list
    let model_count = state.all_models.len();

    // Model list items
    let end_idx = (state.model_scroll_offset + VISIBLE_MODELS).min(model_count);
    for (display_idx, model_idx) in (state.model_scroll_offset..end_idx).enumerate() {
        buttons.push(ButtonRect {
            x: 30,
            y: 60 + (display_idx as u32 * 40),
            width: 440,
            height: 35,
            button: Button::Model(model_idx),
        });
    }

    // Scroll buttons (if needed)
    if state.model_scroll_offset > 0 {
        buttons.push(ButtonRect {
            x: 450,
            y: 55,
            width: 30,
            height: 20,
            button: Button::ModelScrollUp,
        });
    }
    if end_idx < model_count {
        buttons.push(ButtonRect {
            x: 450,
            y: 280,
            width: 30,
            height: 20,
            button: Button::ModelScrollDown,
        });
    }

    // Download button
    buttons.push(ButtonRect {
        x: 30,
        y: 310,
        width: 120,
        height: 35,
        button: Button::Download,
    });

    // Open Link button
    buttons.push(ButtonRect {
        x: 160,
        y: 310,
        width: 120,
        height: 35,
        button: Button::OpenLink,
    });

    buttons
}

fn get_hotkey_page_buttons(_state: &SetupState) -> Vec<ButtonRect> {
    let mut buttons = Vec::new();

    // Back button
    buttons.push(ButtonRect {
        x: 400,
        y: 10,
        width: 80,
        height: 30,
        button: Button::Back,
    });

    // Set Hotkey button
    buttons.push(ButtonRect {
        x: 150,
        y: 200,
        width: 200,
        height: 40,
        button: Button::SetHotkey,
    });

    // Confirm button
    buttons.push(ButtonRect {
        x: 150,
        y: 260,
        width: 95,
        height: 35,
        button: Button::ConfirmHotkey,
    });

    // Clear button
    buttons.push(ButtonRect {
        x: 255,
        y: 260,
        width: 95,
        height: 35,
        button: Button::ClearHotkey,
    });

    buttons
}

fn is_inside(pos: (f64, f64), btn: &ButtonRect) -> bool {
    pos.0 >= btn.x as f64
        && pos.0 <= (btn.x + btn.width) as f64
        && pos.1 >= btn.y as f64
        && pos.1 <= (btn.y + btn.height) as f64
}

fn handle_click(state: &mut SetupState, button: Button) -> Option<Config> {
    match button {
        // Home page
        Button::SelectModel => {
            if state.all_models.is_empty() {
                state.status = "No models found! Check backends/ folder.".to_string();
                return None;
            }
            state.current_page = SetupPage::ModelSelection;
            state.model_scroll_offset = 0;
            if state.selected_model.is_none() {
                state.status = "Select a model from the list".to_string();
            }
            None
        }
        Button::ConfigurePushToTalk => {
            state.current_page = SetupPage::HotkeyConfig(HotkeyTarget::PushToTalk);
            state.captured_key = state.push_to_talk_hotkey.clone();
            state.hotkey_capture = HotkeyCapture::Idle;
            None
        }
        Button::ConfigureToggleListen => {
            state.current_page = SetupPage::HotkeyConfig(HotkeyTarget::ToggleListening);
            state.captured_key = state.toggle_listening_hotkey.clone();
            state.hotkey_capture = HotkeyCapture::Idle;
            None
        }
        Button::ConfigureMic => {
            state.current_page = SetupPage::AudioConfig;
            None
        }
        Button::GpuToggle => {
            state.use_gpu = !state.use_gpu;
            None
        }
        Button::ConfigureCuda => {
            state.current_page = SetupPage::CudaConfig;
            None
        }
        Button::Start => {
            if state.selected_model.is_none() {
                state.status = "Please select a model first!".to_string();
                return None;
            }
            if state.selected_backend_id.is_none() {
                state.status = "No backend available for selected model!".to_string();
                return None;
            }
            if !state.model_downloaded {
                state.status = "Please download the model first!".to_string();
                return None;
            }
            if let (Ok(models_dir), Some(unified), Some(backend_id)) = (
                get_models_dir(),
                state.selected_unified_model(),
                state.selected_backend_id.as_ref(),
            ) {
                let model_path = models_dir.join(&unified.model.folder_name);
                let mut config = Config::for_model(
                    backend_id,
                    &unified.model.id,
                    model_path,
                    state.push_to_talk_hotkey.as_deref().unwrap_or("Backquote"),
                    state.toggle_listening_hotkey.as_deref().unwrap_or("Control+Backquote"),
                    state.use_gpu,
                    state.cuda_path.clone(),
                    state.cudnn_path.clone(),
                    state.selected_input_device.clone(),
                );
                config.overlay_visible = state.overlay_visible;
                config.overlay_x = state.overlay_x;
                config.overlay_y = state.overlay_y;
                if let Err(e) = config.save() {
                    state.status = format!("Error saving config: {}", e);
                    return None;
                }
                // Re-launch the app
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
                Some(config)
            } else {
                state.status = "Error: Could not get models directory".to_string();
                None
            }
        }

        // Model selection page
        Button::Model(idx) => {
            state.selected_model = Some(idx);
            // Auto-select backend based on chosen model
            let unified = &state.all_models[idx];
            state.selected_backend_id = Some(unified.backend_id.clone());
            state.model_downloaded = state.check_model_exists();
            if state.model_downloaded {
                state.status = "Model ready! Click Back then Start.".to_string();
            } else {
                state.status = "Click Download to get this model.".to_string();
            }
            None
        }
        Button::Download => {
            if state.selected_model.is_none() {
                state.status = "Select a model first!".to_string();
                return None;
            }
            if state.download_progress.is_some() {
                return None;
            }
            if state.model_downloaded {
                state.status = "Model already downloaded!".to_string();
                return None;
            }
            // Extract data before modifying state
            let download_info = {
                if let (Ok(models_dir), Some(unified)) = (
                    get_models_dir(),
                    state.selected_unified_model(),
                ) {
                    Some((
                        models_dir.join(&unified.model.folder_name),
                        unified.backend_id.clone(),
                        unified.model.clone(),
                    ))
                } else {
                    None
                }
            };
            if let Some((dest_folder, backend_id, model)) = download_info {
                state.status = "Starting download...".to_string();
                state.download_progress = Some(downloader::start_manifest_model_download(
                    &backend_id,
                    &model,
                    dest_folder,
                ));
            }
            None
        }
        Button::OpenLink => {
            if let Some(model) = state.selected_model_info() {
                let _ = open::that(&model.download_url);
            }
            None
        }
        Button::ModelScrollUp => {
            if state.model_scroll_offset > 0 {
                state.model_scroll_offset -= 1;
            }
            None
        }
        Button::ModelScrollDown => {
            let model_count = state.all_models.len();
            let max_offset = model_count.saturating_sub(VISIBLE_MODELS);
            if state.model_scroll_offset < max_offset {
                state.model_scroll_offset += 1;
            }
            None
        }
        Button::DeviceScrollUp => {
            if state.device_scroll_offset > 0 {
                state.device_scroll_offset -= 1;
            }
            None
        }
        Button::DeviceScrollDown => {
            let device_count = state.input_devices.len();
            let max_offset = device_count.saturating_sub(VISIBLE_DEVICES);
            if state.device_scroll_offset < max_offset {
                state.device_scroll_offset += 1;
            }
            None
        }
        Button::Device(idx) => {
            if let Some(name) = state.input_devices.get(idx) {
                if name == DEFAULT_DEVICE_LABEL {
                    state.selected_input_device = None;
                } else {
                    state.selected_input_device = Some(name.clone());
                }
                state.status = "Microphone selection updated.".to_string();
            }
            None
        }
        Button::Back => {
            state.current_page = SetupPage::Home;
            state.hotkey_capture = HotkeyCapture::Idle;
            // Update status for home page
            if state.selected_model.is_some() && state.model_downloaded {
                state.status = "Ready! Click Start to begin.".to_string();
            } else if state.selected_model.is_some() {
                state.status = "Download the model, then click Start.".to_string();
            } else {
                state.status = "Select a model to get started.".to_string();
            }
            None
        }
        Button::ConfirmDevice => {
            if let Ok(mut config) = Config::load() {
                config.input_device_name = state.selected_input_device.clone();
                if let Err(e) = config.save() {
                    state.status = format!("Error saving microphone: {}", e);
                }
            }
            state.current_page = SetupPage::Home;
            None
        }

        // CUDA config page
        Button::DetectCuda => {
            state.cuda_path = detect_cuda_path();
            state.cudnn_path = detect_cudnn_path();
            state.cuda_valid = state.cuda_path.as_ref().map(|p| validate_cuda_path(p)).unwrap_or(false);
            state.cudnn_valid = state.cudnn_path.as_ref().map(|p| validate_cudnn_path(p)).unwrap_or(false);
            if state.cuda_valid && state.cudnn_valid {
                state.status = "CUDA and cuDNN detected!".to_string();
            } else if state.cuda_valid {
                state.status = "CUDA found. cuDNN not detected.".to_string();
            } else {
                state.status = "CUDA not found. Install CUDA Toolkit.".to_string();
            }
            None
        }
        Button::BrowseCuda => {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Select CUDA Toolkit Directory")
                .pick_folder()
            {
                state.cuda_path = Some(path);
                state.cuda_valid = state.cuda_path.as_ref().map(|p| validate_cuda_path(p)).unwrap_or(false);
                if state.cuda_valid {
                    state.status = "CUDA path set successfully!".to_string();
                } else {
                    state.status = "Warning: No cudart DLL found in bin/".to_string();
                }
            }
            None
        }
        Button::BrowseCudnn => {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Select cuDNN Directory")
                .pick_folder()
            {
                state.cudnn_path = Some(path);
                state.cudnn_valid = state.cudnn_path.as_ref().map(|p| validate_cudnn_path(p)).unwrap_or(false);
                if state.cudnn_valid {
                    state.status = "cuDNN path set successfully!".to_string();
                } else {
                    state.status = "Warning: No cudnn DLL found in bin/".to_string();
                }
            }
            None
        }

        // Hotkey config page
        Button::SetHotkey => {
            state.hotkey_capture = HotkeyCapture::WaitingForKey;
            // Reset captured key when entering capture mode
            state.captured_key = None;
            None
        }
        Button::ConfirmHotkey => {
            if let SetupPage::HotkeyConfig(target) = state.current_page {
                state.set_hotkey(target, state.captured_key.clone());
            }
            if let Ok(mut config) = Config::load() {
                config.hotkey_push_to_talk = state
                    .push_to_talk_hotkey
                    .clone()
                    .unwrap_or_else(|| "Backquote".to_string());
                config.hotkey_always_listen = state
                    .toggle_listening_hotkey
                    .clone()
                    .unwrap_or_else(|| "Control+Backquote".to_string());
                if let Err(e) = config.save() {
                    state.status = format!("Error saving hotkeys: {}", e);
                }
            }
            state.current_page = SetupPage::Home;
            state.hotkey_capture = HotkeyCapture::Idle;
            // Update status
            if state.selected_model.is_some() && state.model_downloaded {
                state.status = "Ready! Click Start to begin.".to_string();
            }
            None
        }
        Button::ClearHotkey => {
            state.captured_key = None;
            None
        }
    }
}

fn render(state: &SetupState, buffer: &mut [u32], width: u32, height: u32) {
    // Clear background
    for pixel in buffer.iter_mut() {
        *pixel = BG_COLOR;
    }

    match &state.current_page {
        SetupPage::Home => render_home_page(state, buffer, width, height),
        SetupPage::ModelSelection => render_model_page(state, buffer, width, height),
        SetupPage::HotkeyConfig(target) => render_hotkey_page(state, buffer, width, height, *target),
        SetupPage::CudaConfig => render_cuda_page(state, buffer, width, height),
        SetupPage::AudioConfig => render_audio_page(state, buffer, width, height),
    }
}

fn render_home_page(state: &SetupState, buffer: &mut [u32], width: u32, _height: u32) {
    // Header
    draw_rect(buffer, width, 0, 0, width, 50, HEADER_BG);
    draw_text(buffer, width, 20, 20, "Speech-to-Text Setup", TEXT_COLOR);

    // Field dimensions - increased for better visibility
    const FIELD_HEIGHT: u32 = 28;
    const TEXT_OFFSET: u32 = 10;  // Vertical offset for text within fields
    const ROW_SPACING: u32 = 50;  // Space between rows
    const LABEL_FIELD_GAP: u32 = 15;  // Gap between label and field

    let mut y: u32 = 65;

    // Backend section (read-only - auto-selected from model)
    draw_text(buffer, width, 30, y, "Backend:", TEXT_COLOR);
    y += LABEL_FIELD_GAP;
    draw_rect(buffer, width, 30, y, 440, FIELD_HEIGHT, FIELD_BG);
    let backend_text = state.selected_backend_id
        .as_ref()
        .and_then(|id| state.get_backend_display_name(id))
        .unwrap_or("(auto-selected from model)");
    draw_text(buffer, width, 40, y + TEXT_OFFSET, backend_text, if state.selected_backend_id.is_some() { TEXT_COLOR } else { DIM_TEXT });
    y += ROW_SPACING;

    // Model section
    draw_text(buffer, width, 30, y, "Model:", TEXT_COLOR);
    y += LABEL_FIELD_GAP;
    draw_rect(buffer, width, 30, y, 340, FIELD_HEIGHT, FIELD_BG);
    let model_text = state.selected_model_info()
        .map(|m| m.display_name.as_str())
        .unwrap_or("None selected");
    draw_text(buffer, width, 40, y + TEXT_OFFSET, model_text, if state.selected_model.is_some() { TEXT_COLOR } else { DIM_TEXT });

    // Select Model button
    let select_bg = if state.hovered_button == Some(Button::SelectModel) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, y, 90, FIELD_HEIGHT, select_bg);
    draw_text(buffer, width, 400, y + TEXT_OFFSET, "Select", TEXT_COLOR);
    y += ROW_SPACING;

    // Microphone section
    draw_text(buffer, width, 30, y, "Microphone:", TEXT_COLOR);
    y += LABEL_FIELD_GAP;
    draw_rect(buffer, width, 30, y, 340, FIELD_HEIGHT, FIELD_BG);
    let device_text = state
        .selected_input_device
        .as_deref()
        .unwrap_or(DEFAULT_DEVICE_LABEL);
    draw_text(buffer, width, 40, y + TEXT_OFFSET, device_text, TEXT_COLOR);

    // Configure Microphone button
    let mic_btn_bg = if state.hovered_button == Some(Button::ConfigureMic) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, y, 90, FIELD_HEIGHT, mic_btn_bg);
    draw_text(buffer, width, 392, y + TEXT_OFFSET, "Change", TEXT_COLOR);
    y += ROW_SPACING;

    // Push-to-Talk section
    draw_text(buffer, width, 30, y, "Push-to-Talk:", TEXT_COLOR);
    y += LABEL_FIELD_GAP;
    draw_rect(buffer, width, 30, y, 340, FIELD_HEIGHT, FIELD_BG);
    let ptt_text = state.push_to_talk_hotkey.as_deref()
        .map(format_hotkey_display)
        .unwrap_or_else(|| "None".to_string());
    draw_text(buffer, width, 40, y + TEXT_OFFSET, &ptt_text, if state.push_to_talk_hotkey.is_some() { TEXT_COLOR } else { DIM_TEXT });

    // Configure PTT button
    let ptt_btn_bg = if state.hovered_button == Some(Button::ConfigurePushToTalk) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, y, 90, FIELD_HEIGHT, ptt_btn_bg);
    draw_text(buffer, width, 390, y + TEXT_OFFSET, "Configure", TEXT_COLOR);
    y += ROW_SPACING;

    // Toggle Listening section
    draw_text(buffer, width, 30, y, "Toggle Listen:", TEXT_COLOR);
    y += LABEL_FIELD_GAP;
    draw_rect(buffer, width, 30, y, 340, FIELD_HEIGHT, FIELD_BG);
    let toggle_text = state.toggle_listening_hotkey.as_deref()
        .map(format_hotkey_display)
        .unwrap_or_else(|| "None".to_string());
    draw_text(buffer, width, 40, y + TEXT_OFFSET, &toggle_text, if state.toggle_listening_hotkey.is_some() { TEXT_COLOR } else { DIM_TEXT });

    // Configure Toggle button
    let toggle_btn_bg = if state.hovered_button == Some(Button::ConfigureToggleListen) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, y, 90, FIELD_HEIGHT, toggle_btn_bg);
    draw_text(buffer, width, 390, y + TEXT_OFFSET, "Configure", TEXT_COLOR);
    y += ROW_SPACING;

    // GPU toggle
    let gpu_bg = if state.hovered_button == Some(Button::GpuToggle) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 30, y, 250, FIELD_HEIGHT, gpu_bg);
    let gpu_indicator = if state.use_gpu { "[x]" } else { "[ ]" };
    let gpu_text = format!("{} Use GPU (CUDA)", gpu_indicator);
    draw_text(buffer, width, 40, y + TEXT_OFFSET, &gpu_text, TEXT_COLOR);

    // Configure CUDA button (only when GPU enabled)
    if state.use_gpu {
        let cuda_btn_bg = if state.hovered_button == Some(Button::ConfigureCuda) { BUTTON_HOVER } else { BUTTON_COLOR };
        draw_rect(buffer, width, 290, y, 90, FIELD_HEIGHT, cuda_btn_bg);
        let cuda_status = if state.cuda_valid { "OK" } else { "Setup" };
        draw_text(buffer, width, 310, y + TEXT_OFFSET, cuda_status, if state.cuda_valid { PROGRESS_FG } else { TEXT_COLOR });
    }
    y += 35;

    // CUDA status (when GPU enabled)
    if state.use_gpu {
        let cuda_status = if state.cuda_valid {
            "CUDA: Ready"
        } else {
            "CUDA: Not configured"
        };
        draw_text(buffer, width, 30, y, cuda_status, if state.cuda_valid { PROGRESS_FG } else { DIM_TEXT });
        y += 25;
    }

    // Status text
    y += 10;
    draw_text(buffer, width, 30, y, &state.status, DIM_TEXT);

    // Start button - fixed position at bottom
    let can_start = state.selected_model.is_some() && state.model_downloaded;
    let start_bg = if state.hovered_button == Some(Button::Start) {
        if can_start { ACCENT_COLOR } else { BUTTON_HOVER }
    } else if can_start {
        BUTTON_COLOR
    } else {
        0xFF333355
    };
    draw_rect(buffer, width, 175, 440, 150, 45, start_bg);
    draw_text(buffer, width, 222, 458, "Start", TEXT_COLOR);
}


fn render_cuda_page(state: &SetupState, buffer: &mut [u32], width: u32, _height: u32) {
    // Header
    draw_rect(buffer, width, 0, 0, width, 50, HEADER_BG);
    draw_text(buffer, width, 20, 20, "CUDA Configuration", TEXT_COLOR);

    // Back button
    let back_bg = if state.hovered_button == Some(Button::Back) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 400, 10, 80, 30, back_bg);
    draw_text(buffer, width, 420, 20, "Back", TEXT_COLOR);

    // CUDA path
    draw_text(buffer, width, 30, 70, "CUDA Toolkit Path:", TEXT_COLOR);
    draw_rect(buffer, width, 30, 90, 340, 28, FIELD_BG);
    let cuda_text = state.cuda_path.as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Not detected".to_string());
    let cuda_text_short = if cuda_text.len() > 40 { format!("...{}", &cuda_text[cuda_text.len()-37..]) } else { cuda_text };
    draw_text(buffer, width, 40, 100, &cuda_text_short, if state.cuda_valid { TEXT_COLOR } else { DIM_TEXT });

    // CUDA status indicator
    let cuda_status = if state.cuda_valid { "[OK]" } else { "[!]" };
    draw_text(buffer, width, 340, 100, cuda_status, if state.cuda_valid { PROGRESS_FG } else { 0xFFFF6666 });

    // Browse CUDA button
    let browse_cuda_bg = if state.hovered_button == Some(Button::BrowseCuda) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, 90, 90, 28, browse_cuda_bg);
    draw_text(buffer, width, 398, 100, "Browse", TEXT_COLOR);

    // cuDNN path
    draw_text(buffer, width, 30, 140, "cuDNN Path:", TEXT_COLOR);
    draw_rect(buffer, width, 30, 160, 340, 28, FIELD_BG);
    let cudnn_text = state.cudnn_path.as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Not detected".to_string());
    let cudnn_text_short = if cudnn_text.len() > 40 { format!("...{}", &cudnn_text[cudnn_text.len()-37..]) } else { cudnn_text };
    draw_text(buffer, width, 40, 170, &cudnn_text_short, if state.cudnn_valid { TEXT_COLOR } else { DIM_TEXT });

    // cuDNN status indicator
    let cudnn_status = if state.cudnn_valid { "[OK]" } else { "[!]" };
    draw_text(buffer, width, 340, 170, cudnn_status, if state.cudnn_valid { PROGRESS_FG } else { 0xFFFF6666 });

    // Browse cuDNN button
    let browse_cudnn_bg = if state.hovered_button == Some(Button::BrowseCudnn) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 380, 160, 90, 28, browse_cudnn_bg);
    draw_text(buffer, width, 398, 170, "Browse", TEXT_COLOR);

    // Instructions
    draw_text(buffer, width, 30, 220, "Click Browse to manually select folders,", DIM_TEXT);
    draw_text(buffer, width, 30, 240, "or Auto-Detect to find installed paths.", DIM_TEXT);
    draw_text(buffer, width, 30, 270, "Install CUDA Toolkit and cuDNN from NVIDIA", DIM_TEXT);
    draw_text(buffer, width, 30, 290, "if not already installed.", DIM_TEXT);

    // Detect button
    let detect_bg = if state.hovered_button == Some(Button::DetectCuda) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 175, 320, 150, 35, detect_bg);
    draw_text(buffer, width, 195, 332, "Auto-Detect", TEXT_COLOR);

    // Status
    draw_text(buffer, width, 30, 380, &state.status, DIM_TEXT);
}

fn render_audio_page(state: &SetupState, buffer: &mut [u32], width: u32, _height: u32) {
    // Header
    draw_rect(buffer, width, 0, 0, width, 50, HEADER_BG);
    draw_text(buffer, width, 20, 20, "Microphone Selection", TEXT_COLOR);

    // Back button
    let back_bg = if state.hovered_button == Some(Button::Back) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 400, 10, 80, 30, back_bg);
    draw_text(buffer, width, 420, 20, "Back", TEXT_COLOR);

    // Scroll buttons
    let up_bg = if state.hovered_button == Some(Button::DeviceScrollUp) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 450, 80, 30, 30, up_bg);
    draw_text(buffer, width, 460, 88, "^", TEXT_COLOR);

    let down_bg = if state.hovered_button == Some(Button::DeviceScrollDown) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 450, 360, 30, 30, down_bg);
    draw_text(buffer, width, 460, 368, "v", TEXT_COLOR);

    // Device list
    let start_y: u32 = 110;
    for i in 0..VISIBLE_DEVICES {
        let device_idx = state.device_scroll_offset + i;
        if device_idx >= state.input_devices.len() {
            break;
        }
        let device_name = &state.input_devices[device_idx];
        let is_selected = state
            .selected_input_device
            .as_deref()
            .unwrap_or(DEFAULT_DEVICE_LABEL)
            == device_name.as_str();

        let bg = if state.hovered_button == Some(Button::Device(device_idx)) {
            BUTTON_HOVER
        } else if is_selected {
            SELECTED_COLOR
        } else {
            FIELD_BG
        };

        draw_rect(buffer, width, 30, start_y + (i as u32 * 45), 400, 35, bg);
        draw_text(buffer, width, 40, start_y + (i as u32 * 45) + 12, device_name, TEXT_COLOR);
    }

    // Confirm button
    let confirm_bg = if state.hovered_button == Some(Button::ConfirmDevice) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 300, 440, 150, 35, confirm_bg);
    draw_text(buffer, width, 330, 450, "Use Selected", TEXT_COLOR);
}

fn render_model_page(state: &SetupState, buffer: &mut [u32], width: u32, _height: u32) {
    // Header
    draw_rect(buffer, width, 0, 0, width, 50, HEADER_BG);
    draw_text(buffer, width, 20, 15, "Select Model", TEXT_COLOR);

    // Back button
    let back_bg = if state.hovered_button == Some(Button::Back) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 400, 10, 80, 30, back_bg);
    draw_text(buffer, width, 420, 18, "Back", TEXT_COLOR);

    if state.all_models.is_empty() {
        draw_text(buffer, width, 30, 100, "No models found!", TEXT_COLOR);
        draw_text(buffer, width, 30, 130, "Check backends/ folder for manifest.json", DIM_TEXT);
        return;
    }

    // Model list (unified from all backends)
    let model_count = state.all_models.len();
    let end_idx = (state.model_scroll_offset + VISIBLE_MODELS).min(model_count);
    for (display_idx, model_idx) in (state.model_scroll_offset..end_idx).enumerate() {
        let y = 60 + (display_idx as u32 * 40);
        let unified = &state.all_models[model_idx];
        let is_selected = state.selected_model == Some(model_idx);
        let is_hovered = state.hovered_button == Some(Button::Model(model_idx));

        let bg = if is_selected {
            SELECTED_COLOR
        } else if is_hovered {
            BUTTON_HOVER
        } else {
            BUTTON_COLOR
        };

        draw_rect(buffer, width, 30, y, 440, 35, bg);

        let indicator = if is_selected { "[*]" } else { "[ ]" };
        let downloaded = is_unified_model_downloaded(unified);
        let status = if downloaded { " [OK]" } else { "" };
        // Show backend name with model (truncate if too long)
        let backend_short = if unified.backend_name.len() > 12 {
            &unified.backend_name[..12]
        } else {
            &unified.backend_name
        };
        let label = format!("{} {} ({}MB) [{}]{}",
            indicator,
            unified.model.display_name,
            unified.model.size_mb,
            backend_short,
            status
        );
        draw_text(buffer, width, 40, y + 10, &label, TEXT_COLOR);
    }

    // Scroll indicators
    if state.model_scroll_offset > 0 {
        draw_text(buffer, width, 455, 58, "^", ACCENT_COLOR);
    }
    if end_idx < model_count {
        draw_text(buffer, width, 455, 283, "v", ACCENT_COLOR);
    }

    // Download button
    let download_bg = if state.hovered_button == Some(Button::Download) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 30, 310, 120, 35, download_bg);
    draw_text(buffer, width, 55, 320, "Download", TEXT_COLOR);

    // Open Link button
    let link_bg = if state.hovered_button == Some(Button::OpenLink) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 160, 310, 120, 35, link_bg);
    draw_text(buffer, width, 180, 320, "Open Link", TEXT_COLOR);

    // Status text
    draw_text(buffer, width, 30, 360, &state.status, DIM_TEXT);

    // Progress bar
    if let Some(ref progress) = state.download_progress {
        let (downloaded, total) = progress.get_progress();
        draw_rect(buffer, width, 30, 375, 440, 15, PROGRESS_BG);
        if total > 0 {
            let fill_width = ((downloaded as f64 / total as f64) * 440.0) as u32;
            draw_rect(buffer, width, 30, 375, fill_width, 15, PROGRESS_FG);
        }
    }
}

fn render_hotkey_page(state: &SetupState, buffer: &mut [u32], width: u32, _height: u32, target: HotkeyTarget) {
    // Header
    draw_rect(buffer, width, 0, 0, width, 50, HEADER_BG);
    let title = match target {
        HotkeyTarget::PushToTalk => "Configure Push-to-Talk",
        HotkeyTarget::ToggleListening => "Configure Toggle Listening",
    };
    draw_text(buffer, width, 20, 15, title, TEXT_COLOR);

    // Back button
    let back_bg = if state.hovered_button == Some(Button::Back) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 400, 10, 80, 30, back_bg);
    draw_text(buffer, width, 420, 18, "Back", TEXT_COLOR);

    // Current hotkey display
    draw_text(buffer, width, 150, 80, "Current Hotkey:", TEXT_COLOR);

    // Large display box
    let display_bg = if state.hotkey_capture == HotkeyCapture::WaitingForKey { CAPTURE_BG } else { FIELD_BG };
    draw_rect(buffer, width, 100, 100, 300, 60, display_bg);

    let display_text = if state.hotkey_capture == HotkeyCapture::WaitingForKey {
        "Press any key...".to_string()
    } else {
        state.captured_key.as_deref()
            .map(format_hotkey_display)
            .unwrap_or_else(|| "None".to_string())
    };

    // Center the text in the box
    let text_x = 250 - (display_text.len() as u32 * 4);
    draw_text(buffer, width, text_x, 125, &display_text,
        if state.hotkey_capture == HotkeyCapture::WaitingForKey { ACCENT_COLOR } else { TEXT_COLOR });

    // Set Hotkey button
    let set_bg = if state.hovered_button == Some(Button::SetHotkey) { BUTTON_HOVER } else { BUTTON_COLOR };
    let set_bg = if state.hotkey_capture == HotkeyCapture::WaitingForKey { SELECTED_COLOR } else { set_bg };
    draw_rect(buffer, width, 150, 200, 200, 40, set_bg);
    let set_text = if state.hotkey_capture == HotkeyCapture::WaitingForKey { "Listening..." } else { "Set Hotkey" };
    draw_text(buffer, width, 205, 215, set_text, TEXT_COLOR);

    // Confirm button
    let confirm_bg = if state.hovered_button == Some(Button::ConfirmHotkey) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 150, 260, 95, 35, confirm_bg);
    draw_text(buffer, width, 170, 270, "Confirm", TEXT_COLOR);

    // Clear button
    let clear_bg = if state.hovered_button == Some(Button::ClearHotkey) { BUTTON_HOVER } else { BUTTON_COLOR };
    draw_rect(buffer, width, 255, 260, 95, 35, clear_bg);
    draw_text(buffer, width, 280, 270, "Clear", TEXT_COLOR);

    // Instructions
    draw_text(buffer, width, 100, 320, "Click 'Set Hotkey' then press any key", DIM_TEXT);
    draw_text(buffer, width, 100, 340, "Supports modifiers: Ctrl, Alt, Shift", DIM_TEXT);
}

fn format_hotkey_display(key: &str) -> String {
    // Convert internal format to user-friendly display
    key.replace("Control", "Ctrl")
       .replace("Backquote", "`")
       .replace("Key", "")
       .replace("Digit", "")
       .replace("Arrow", "")
}

fn draw_rect(buffer: &mut [u32], buf_width: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    for dy in 0..h {
        for dx in 0..w {
            let px = x + dx;
            let py = y + dy;
            if px < buf_width {
                let idx = (py * buf_width + px) as usize;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
    }
}

fn draw_text(buffer: &mut [u32], buf_width: u32, x: u32, y: u32, text: &str, color: u32) {
    let chars: Vec<char> = text.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        draw_char(buffer, buf_width, x + (i as u32 * 8), y, *ch, color);
    }
}

fn draw_char(buffer: &mut [u32], buf_width: u32, x: u32, y: u32, ch: char, color: u32) {
    let bitmap = get_char_bitmap(ch);
    for (row, bits) in bitmap.iter().enumerate() {
        for col in 0..6 {
            if (bits >> (5 - col)) & 1 == 1 {
                let px = x + col;
                let py = y + row as u32;
                if px < buf_width {
                    let idx = (py * buf_width + px) as usize;
                    if idx < buffer.len() {
                        buffer[idx] = color;
                    }
                }
            }
        }
    }
}

fn get_char_bitmap(ch: char) -> [u8; 7] {
    match ch {
        'A' => [0x1E, 0x21, 0x21, 0x3F, 0x21, 0x21, 0x21],
        'B' => [0x3E, 0x21, 0x21, 0x3E, 0x21, 0x21, 0x3E],
        'C' => [0x1E, 0x21, 0x20, 0x20, 0x20, 0x21, 0x1E],
        'D' => [0x3C, 0x22, 0x21, 0x21, 0x21, 0x22, 0x3C],
        'E' => [0x3F, 0x20, 0x20, 0x3E, 0x20, 0x20, 0x3F],
        'F' => [0x3F, 0x20, 0x20, 0x3E, 0x20, 0x20, 0x20],
        'G' => [0x1E, 0x21, 0x20, 0x27, 0x21, 0x21, 0x1E],
        'H' => [0x21, 0x21, 0x21, 0x3F, 0x21, 0x21, 0x21],
        'I' => [0x1C, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1C],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x21, 0x21, 0x1E],
        'K' => [0x21, 0x22, 0x24, 0x38, 0x24, 0x22, 0x21],
        'L' => [0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x3F],
        'M' => [0x21, 0x33, 0x2D, 0x21, 0x21, 0x21, 0x21],
        'N' => [0x21, 0x31, 0x29, 0x25, 0x23, 0x21, 0x21],
        'O' => [0x1E, 0x21, 0x21, 0x21, 0x21, 0x21, 0x1E],
        'P' => [0x3E, 0x21, 0x21, 0x3E, 0x20, 0x20, 0x20],
        'Q' => [0x1E, 0x21, 0x21, 0x21, 0x25, 0x22, 0x1D],
        'R' => [0x3E, 0x21, 0x21, 0x3E, 0x24, 0x22, 0x21],
        'S' => [0x1E, 0x21, 0x20, 0x1E, 0x01, 0x21, 0x1E],
        'T' => [0x3F, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08],
        'U' => [0x21, 0x21, 0x21, 0x21, 0x21, 0x21, 0x1E],
        'V' => [0x21, 0x21, 0x21, 0x21, 0x12, 0x12, 0x0C],
        'W' => [0x21, 0x21, 0x21, 0x21, 0x2D, 0x33, 0x21],
        'X' => [0x21, 0x12, 0x0C, 0x0C, 0x0C, 0x12, 0x21],
        'Y' => [0x21, 0x21, 0x12, 0x0C, 0x08, 0x08, 0x08],
        'Z' => [0x3F, 0x02, 0x04, 0x08, 0x10, 0x20, 0x3F],
        'a' => [0x00, 0x00, 0x1E, 0x01, 0x1F, 0x21, 0x1F],
        'b' => [0x20, 0x20, 0x3E, 0x21, 0x21, 0x21, 0x3E],
        'c' => [0x00, 0x00, 0x1E, 0x20, 0x20, 0x20, 0x1E],
        'd' => [0x01, 0x01, 0x1F, 0x21, 0x21, 0x21, 0x1F],
        'e' => [0x00, 0x00, 0x1E, 0x21, 0x3F, 0x20, 0x1E],
        'f' => [0x06, 0x08, 0x1E, 0x08, 0x08, 0x08, 0x08],
        'g' => [0x00, 0x1F, 0x21, 0x21, 0x1F, 0x01, 0x1E],
        'h' => [0x20, 0x20, 0x3E, 0x21, 0x21, 0x21, 0x21],
        'i' => [0x08, 0x00, 0x18, 0x08, 0x08, 0x08, 0x1C],
        'j' => [0x02, 0x00, 0x06, 0x02, 0x02, 0x22, 0x1C],
        'k' => [0x20, 0x20, 0x22, 0x24, 0x38, 0x24, 0x22],
        'l' => [0x18, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1C],
        'm' => [0x00, 0x00, 0x36, 0x2D, 0x21, 0x21, 0x21],
        'n' => [0x00, 0x00, 0x3E, 0x21, 0x21, 0x21, 0x21],
        'o' => [0x00, 0x00, 0x1E, 0x21, 0x21, 0x21, 0x1E],
        'p' => [0x00, 0x3E, 0x21, 0x21, 0x3E, 0x20, 0x20],
        'q' => [0x00, 0x1F, 0x21, 0x21, 0x1F, 0x01, 0x01],
        'r' => [0x00, 0x00, 0x2E, 0x30, 0x20, 0x20, 0x20],
        's' => [0x00, 0x00, 0x1E, 0x20, 0x1E, 0x01, 0x3E],
        't' => [0x08, 0x08, 0x1E, 0x08, 0x08, 0x08, 0x06],
        'u' => [0x00, 0x00, 0x21, 0x21, 0x21, 0x21, 0x1F],
        'v' => [0x00, 0x00, 0x21, 0x21, 0x12, 0x12, 0x0C],
        'w' => [0x00, 0x00, 0x21, 0x21, 0x21, 0x2D, 0x12],
        'x' => [0x00, 0x00, 0x21, 0x12, 0x0C, 0x12, 0x21],
        'y' => [0x00, 0x21, 0x21, 0x1F, 0x01, 0x21, 0x1E],
        'z' => [0x00, 0x00, 0x3F, 0x02, 0x0C, 0x10, 0x3F],
        '0' => [0x1E, 0x21, 0x23, 0x25, 0x29, 0x31, 0x1E],
        '1' => [0x08, 0x18, 0x08, 0x08, 0x08, 0x08, 0x1C],
        '2' => [0x1E, 0x21, 0x01, 0x0E, 0x10, 0x20, 0x3F],
        '3' => [0x1E, 0x21, 0x01, 0x0E, 0x01, 0x21, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x3F, 0x02, 0x02],
        '5' => [0x3F, 0x20, 0x3E, 0x01, 0x01, 0x21, 0x1E],
        '6' => [0x0E, 0x10, 0x20, 0x3E, 0x21, 0x21, 0x1E],
        '7' => [0x3F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x1E, 0x21, 0x21, 0x1E, 0x21, 0x21, 0x1E],
        '9' => [0x1E, 0x21, 0x21, 0x1F, 0x01, 0x02, 0x1C],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x10],
        ':' => [0x00, 0x00, 0x08, 0x00, 0x00, 0x08, 0x00],
        '!' => [0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x08],
        '?' => [0x1E, 0x21, 0x01, 0x0E, 0x08, 0x00, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1E, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3F],
        '(' => [0x04, 0x08, 0x10, 0x10, 0x10, 0x08, 0x04],
        ')' => [0x10, 0x08, 0x04, 0x04, 0x04, 0x08, 0x10],
        '[' => [0x1C, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1C],
        ']' => [0x1C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1C],
        '*' => [0x00, 0x08, 0x2A, 0x1C, 0x2A, 0x08, 0x00],
        '/' => [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x00],
        '%' => [0x31, 0x32, 0x04, 0x08, 0x13, 0x23, 0x00],
        '`' => [0x10, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00],
        '\'' => [0x08, 0x08, 0x10, 0x00, 0x00, 0x00, 0x00],
        '"' => [0x14, 0x14, 0x28, 0x00, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x08, 0x08, 0x3E, 0x08, 0x08, 0x00],
        '=' => [0x00, 0x00, 0x3E, 0x00, 0x3E, 0x00, 0x00],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
        '^' => [0x08, 0x14, 0x22, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

// ============================================
// UI Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================
    // Helper Functions Tests
    // ============================================

    #[test]
    fn test_format_hotkey_display() {
        // Test Control modifier
        assert_eq!(format_hotkey_display("Control+KeyA"), "Ctrl+A");
        assert_eq!(format_hotkey_display("Control+Backquote"), "Ctrl+`");
        
        // Test Alt modifier
        assert_eq!(format_hotkey_display("Alt+KeyF"), "Alt+F");
        
        // Test Shift modifier  
        assert_eq!(format_hotkey_display("Shift+KeyZ"), "Shift+Z");
        
        // Test combined modifiers
        assert_eq!(format_hotkey_display("Control+Shift+KeyA"), "Ctrl+Shift+A");
        
        // Test Arrow keys
        assert_eq!(format_hotkey_display("ArrowUp"), "Up");
        assert_eq!(format_hotkey_display("ArrowDown"), "Down");
        
        // Test simple key
        assert_eq!(format_hotkey_display("KeyA"), "A");
        
        // Test Backquote
        assert_eq!(format_hotkey_display("Backquote"), "`");
    }

    #[test]
    fn test_keycode_to_string_simple() {
        use tao::keyboard::KeyCode;
        
        let modifiers = ModifiersState::default();
        // keycode_to_string returns the internal format (with Key prefix)
        assert_eq!(keycode_to_string(KeyCode::KeyA, &modifiers), "KeyA");
        assert_eq!(keycode_to_string(KeyCode::KeyZ, &modifiers), "KeyZ");
        assert_eq!(keycode_to_string(KeyCode::Digit1, &modifiers), "Digit1");
        assert_eq!(keycode_to_string(KeyCode::Space, &modifiers), "Space");
        assert_eq!(keycode_to_string(KeyCode::Backquote, &modifiers), "Backquote");
    }

    #[test]
    fn test_keycode_to_string_with_modifiers() {
        use tao::keyboard::KeyCode;
        
        let mut modifiers = ModifiersState::default();
        modifiers.set(ModifiersState::CONTROL, true);
        assert_eq!(keycode_to_string(KeyCode::KeyA, &modifiers), "Control+KeyA");
        
        modifiers = ModifiersState::default();
        modifiers.set(ModifiersState::ALT, true);
        assert_eq!(keycode_to_string(KeyCode::KeyF, &modifiers), "Alt+KeyF");
        
        modifiers = ModifiersState::default();
        modifiers.set(ModifiersState::SHIFT, true);
        assert_eq!(keycode_to_string(KeyCode::KeyZ, &modifiers), "Shift+KeyZ");
        
        modifiers = ModifiersState::default();
        modifiers.set(ModifiersState::CONTROL, true);
        modifiers.set(ModifiersState::SHIFT, true);
        assert_eq!(keycode_to_string(KeyCode::KeyA, &modifiers), "Control+Shift+KeyA");
    }

    #[test]
    fn test_is_modifier_key() {
        use tao::keyboard::KeyCode;
        
        assert!(is_modifier_key(KeyCode::ControlLeft));
        assert!(is_modifier_key(KeyCode::ControlRight));
        assert!(is_modifier_key(KeyCode::ShiftLeft));
        assert!(is_modifier_key(KeyCode::ShiftRight));
        assert!(is_modifier_key(KeyCode::AltLeft));
        assert!(is_modifier_key(KeyCode::AltRight));
        assert!(is_modifier_key(KeyCode::SuperLeft));
        assert!(is_modifier_key(KeyCode::SuperRight));
        
        assert!(!is_modifier_key(KeyCode::KeyA));
        assert!(!is_modifier_key(KeyCode::Digit1));
        assert!(!is_modifier_key(KeyCode::Space));
    }

    // ============================================
    // Drawing Functions Tests
    // ============================================

    #[test]
    fn test_draw_rect() {
        let mut buffer = vec![0u32; 100 * 100]; // 100x100 buffer
        
        // Draw a red rectangle
        draw_rect(&mut buffer, 100, 10, 10, 20, 20, 0xFFFF0000);
        
        // Check corners
        assert_eq!(buffer[10 * 100 + 10], 0xFFFF0000); // Top-left
        assert_eq!(buffer[10 * 100 + 29], 0xFFFF0000); // Top-right
        assert_eq!(buffer[29 * 100 + 10], 0xFFFF0000); // Bottom-left
        assert_eq!(buffer[29 * 100 + 29], 0xFFFF0000); // Bottom-right
        
        // Check outside rectangle is unchanged
        assert_eq!(buffer[9 * 100 + 9], 0);
        assert_eq!(buffer[30 * 100 + 30], 0);
    }

    #[test]
    fn test_draw_text() {
        let mut buffer = vec![0u32; 200 * 50];
        
        // Draw "ABC" at position (10, 10)
        draw_text(&mut buffer, 200, 10, 10, "ABC", 0xFFFFFFFF);
        
        // Just verify it doesn't panic - visual verification would need screenshot
        // Check that some pixels were written
        let has_content = buffer.iter().any(|&p| p != 0);
        assert!(has_content, "Text rendering should write pixels");
    }

    #[test]
    fn test_get_char_bitmap_coverage() {
        // Test all printable ASCII characters
        for ch in ' '..='~' {
            let bitmap = get_char_bitmap(ch);
            // Bitmap should be valid (we just verify no panic)
            assert_eq!(bitmap.len(), 7);
        }
    }

    // ============================================
    // Button Geometry Tests
    // ============================================

    #[test]
    fn test_is_inside() {
        let btn = ButtonRect {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
            button: Button::Start,
        };
        
        // Inside
        assert!(is_inside((50.0, 30.0), &btn));
        assert!(is_inside((10.0, 20.0), &btn)); // Top-left corner
        assert!(is_inside((109.0, 69.0), &btn)); // Bottom-right corner
        
        // Outside
        assert!(!is_inside((5.0, 30.0), &btn)); // Left
        assert!(!is_inside((150.0, 30.0), &btn)); // Right
        assert!(!is_inside((50.0, 10.0), &btn)); // Top
        assert!(!is_inside((50.0, 80.0), &btn)); // Bottom
    }

    #[test]
    fn test_button_rect_consistency() {
        // Create a mock state for button rect generation
        let state = SetupState {
            current_page: SetupPage::Home,
            available_backends: vec![],
            all_models: vec![],
            selected_model: None,
            model_scroll_offset: 0,
            selected_backend_id: None,
            input_devices: vec![DEFAULT_DEVICE_LABEL.to_string()],
            selected_input_device: None,
            device_scroll_offset: 0,
            push_to_talk_hotkey: Some("Backquote".to_string()),
            toggle_listening_hotkey: Some("Control+Backquote".to_string()),
            hotkey_capture: HotkeyCapture::Idle,
            captured_key: None,
            current_modifiers: ModifiersState::default(),
            use_gpu: false,
            cuda_path: None,
            cudnn_path: None,
            cuda_valid: false,
            cudnn_valid: false,
            status: "Test".to_string(),
            download_progress: None,
            model_downloaded: false,
            overlay_visible: true,
            overlay_x: None,
            overlay_y: None,
            hovered_button: None,
            mouse_pos: (0.0, 0.0),
        };
        
        // Get home page buttons
        let buttons = get_home_buttons(&state);
        
        // Verify we have expected buttons
        let has_start = buttons.iter().any(|b| matches!(b.button, Button::Start));
        let has_select_model = buttons.iter().any(|b| matches!(b.button, Button::SelectModel));
        
        assert!(has_start, "Home page should have Start button");
        assert!(has_select_model, "Home page should have SelectModel button");
        
        // Verify button rects are valid (non-zero size)
        for btn in &buttons {
            assert!(btn.width > 0, "Button width should be > 0");
            assert!(btn.height > 0, "Button height should be > 0");
        }
    }

    // ============================================
    // Hotkey Target Tests
    // ============================================

    #[test]
    fn test_hotkey_target_variants() {
        // Verify both hotkey targets exist
        let ptt = HotkeyTarget::PushToTalk;
        let toggle = HotkeyTarget::ToggleListening;
        
        // They should be different
        assert_ne!(std::mem::discriminant(&ptt), std::mem::discriminant(&toggle));
    }

    #[test]
    fn test_hotkey_capture_states() {
        let idle = HotkeyCapture::Idle;
        let waiting = HotkeyCapture::WaitingForKey;
        
        assert_ne!(std::mem::discriminant(&idle), std::mem::discriminant(&waiting));
    }

    // ============================================
    // Setup Page Tests
    // ============================================

    #[test]
    fn test_setup_page_navigation() {
        let pages = vec![
            SetupPage::Home,
            SetupPage::ModelSelection,
            SetupPage::HotkeyConfig(HotkeyTarget::PushToTalk),
            SetupPage::HotkeyConfig(HotkeyTarget::ToggleListening),
            SetupPage::CudaConfig,
            SetupPage::AudioConfig,
        ];
        
        // Verify all pages are distinct
        for (i, page1) in pages.iter().enumerate() {
            for (j, page2) in pages.iter().enumerate() {
                if i != j {
                    // Note: HotkeyConfig variants are compared by their target
                    match (page1, page2) {
                        (SetupPage::HotkeyConfig(t1), SetupPage::HotkeyConfig(t2)) => {
                            assert_ne!(t1, t2);
                        }
                        _ => assert_ne!(page1, page2),
                    }
                }
            }
        }
    }

    // ============================================
    // Color Constants Tests
    // ============================================

    #[test]
    fn test_color_format() {
        // Colors should be in 0xAARRGGBB format
        let colors = vec![
            BG_COLOR,
            HEADER_BG,
            TEXT_COLOR,
            ACCENT_COLOR,
            BUTTON_COLOR,
            BUTTON_HOVER,
            SELECTED_COLOR,
            PROGRESS_BG,
            PROGRESS_FG,
        ];
        
        for color in colors {
            // Should have alpha channel set (non-zero in top byte)
            let alpha = (color >> 24) & 0xFF;
            assert!(alpha > 0, "Color {:#010x} should have non-zero alpha", color);
        }
    }

    // ============================================
    // Window Dimensions Tests
    // ============================================

    #[test]
    fn test_window_dimensions() {
        // Window should have reasonable dimensions
        assert!(WINDOW_WIDTH >= 400, "Window width should be at least 400px");
        assert!(WINDOW_HEIGHT >= 400, "Window height should be at least 400px");
        assert!(WINDOW_WIDTH <= 1920, "Window width should be at most 1920px");
        assert!(WINDOW_HEIGHT <= 1080, "Window height should be at most 1080px");
    }

    #[test]
    fn test_visible_models_constant() {
        // VISIBLE_MODELS should be reasonable
        assert!(VISIBLE_MODELS > 0, "Should show at least 1 model");
        assert!(VISIBLE_MODELS <= 20, "Should not show more than 20 models at once");
    }
}
