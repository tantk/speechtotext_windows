use crate::backend_loader::{discover_backends, get_backends_dir, BackendManifest, ManifestModel};
use crate::config::{
    detect_cuda_path, detect_cudnn_path, get_models_dir, validate_cuda_path, validate_cudnn_path,
    AudioSource, Config,
};
use crate::downloader::{self, DownloadProgress};
use crate::tray::WHISPER_LANGUAGES;
use cpal::traits::{DeviceTrait, HostTrait};
use eframe::egui;
use std::sync::Arc;

const DEFAULT_DEVICE_LABEL: &str = "<Default device>";

/// Unified model entry combining backend and model info
#[derive(Debug, Clone)]
struct UnifiedModel {
    backend_id: String,
    backend_name: String,
    model: ManifestModel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Model,
    Audio,
    Hotkeys,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HotkeyCapture {
    Idle,
    CapturingPushToTalk,
    CapturingToggleListen,
}

struct SetupApp {
    // Tab state
    current_tab: Tab,

    // Backend info
    available_backends: Vec<BackendManifest>,

    // Models
    all_models: Vec<UnifiedModel>,
    selected_model: Option<usize>,

    // Audio
    audio_source: AudioSource,
    input_devices: Vec<String>,
    selected_input_device: Option<String>,

    // Language
    input_language: String,
    target_language: String,

    // Hotkeys
    push_to_talk_hotkey: String,
    toggle_listening_hotkey: String,
    hotkey_capture: HotkeyCapture,
    silence_timeout_ms: u64,
    streaming_interval_ms: u64,

    // GPU/CUDA
    use_gpu: bool,
    cuda_path: Option<std::path::PathBuf>,
    cudnn_path: Option<std::path::PathBuf>,
    cuda_valid: bool,
    cudnn_valid: bool,

    // Download state
    status: String,
    download_progress: Option<Arc<DownloadProgress>>,
    model_downloaded: bool,

    // Overlay settings (preserved from existing config)
    overlay_visible: bool,
    overlay_x: Option<i32>,
    overlay_y: Option<i32>,

    // Should exit
    should_start: bool,
}

impl SetupApp {
    fn new() -> Self {
        let existing_config = Config::load().ok();

        // Load audio input devices
        let mut input_devices: Vec<String> = vec![DEFAULT_DEVICE_LABEL.to_string()];
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
        let available_backends: Vec<BackendManifest> =
            if let Ok(backends_dir) = get_backends_dir() {
                let backend_paths = discover_backends(&backends_dir);
                backend_paths
                    .iter()
                    .filter_map(|p| BackendManifest::load(&p.join("manifest.json")).ok())
                    .collect()
            } else {
                Vec::new()
            };

        // Create unified model list
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

        // Resolve saved model selection
        let mut selected_model: Option<usize> = None;
        if let Some(ref cfg) = existing_config {
            if let Some(idx) = all_models
                .iter()
                .position(|u| u.backend_id == cfg.backend_id && u.model.id == cfg.model_name)
            {
                selected_model = Some(idx);
            } else if let Some(model_folder) =
                cfg.model_path.file_name().and_then(|n| n.to_str())
            {
                if let Some(idx) = all_models
                    .iter()
                    .position(|u| u.model.folder_name == model_folder)
                {
                    selected_model = Some(idx);
                }
            }
        }

        let model_downloaded = selected_model
            .and_then(|idx| all_models.get(idx))
            .map(is_model_downloaded)
            .unwrap_or(false);

        let status = if selected_model.is_some() && model_downloaded {
            "Model ready! Click Start.".to_string()
        } else if selected_model.is_some() {
            "Model selected. Click Download.".to_string()
        } else {
            "Select a model to get started".to_string()
        };

        let use_gpu = existing_config.as_ref().map(|c| c.use_gpu).unwrap_or(false);
        let cuda_path = existing_config
            .as_ref()
            .and_then(|c| c.cuda_path.clone())
            .or_else(detect_cuda_path);
        let cudnn_path = existing_config
            .as_ref()
            .and_then(|c| c.cudnn_path.clone())
            .or_else(detect_cudnn_path);
        let cuda_valid = cuda_path
            .as_ref()
            .map(|p| validate_cuda_path(p))
            .unwrap_or(false);
        let cudnn_valid = cudnn_path
            .as_ref()
            .map(|p| validate_cudnn_path(p))
            .unwrap_or(false);

        Self {
            current_tab: Tab::Model,
            available_backends,
            all_models,
            selected_model,
            audio_source: existing_config
                .as_ref()
                .map(|c| c.audio_source)
                .unwrap_or(AudioSource::Microphone),
            input_devices,
            selected_input_device,
            input_language: existing_config
                .as_ref()
                .map(|c| c.input_language.clone())
                .unwrap_or_else(|| "auto".to_string()),
            target_language: existing_config
                .as_ref()
                .map(|c| c.target_language.clone())
                .unwrap_or_else(|| "original".to_string()),
            push_to_talk_hotkey: existing_config
                .as_ref()
                .map(|c| c.hotkey_push_to_talk.clone())
                .unwrap_or_else(|| "Backquote".to_string()),
            toggle_listening_hotkey: existing_config
                .as_ref()
                .map(|c| c.hotkey_always_listen.clone())
                .unwrap_or_else(|| "Control+Backquote".to_string()),
            hotkey_capture: HotkeyCapture::Idle,
            silence_timeout_ms: existing_config
                .as_ref()
                .map(|c| c.silence_timeout_ms)
                .unwrap_or(2000),
            streaming_interval_ms: existing_config
                .as_ref()
                .map(|c| c.streaming_interval_ms)
                .unwrap_or(1200),
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
            should_start: false,
        }
    }

    fn selected_backend_id(&self) -> Option<&str> {
        self.selected_model
            .and_then(|idx| self.all_models.get(idx))
            .map(|u| u.backend_id.as_str())
    }

    fn backend_display_name(&self, backend_id: &str) -> Option<&str> {
        self.available_backends
            .iter()
            .find(|b| b.id == backend_id)
            .map(|b| b.display_name.as_str())
    }

    fn check_model_exists(&self) -> bool {
        self.selected_model
            .and_then(|idx| self.all_models.get(idx))
            .map(is_model_downloaded)
            .unwrap_or(false)
    }

    fn build_config(&self) -> Option<Config> {
        let idx = self.selected_model?;
        let unified = self.all_models.get(idx)?;
        let models_dir = get_models_dir().ok()?;
        let model_path = models_dir.join(&unified.model.folder_name);

        let mut config = Config::for_model(
            &unified.backend_id,
            &unified.model.id,
            model_path,
            &self.push_to_talk_hotkey,
            &self.toggle_listening_hotkey,
            self.use_gpu,
            self.cuda_path.clone(),
            self.cudnn_path.clone(),
            self.selected_input_device.clone(),
            self.silence_timeout_ms,
            self.streaming_interval_ms,
        );
        config.overlay_visible = self.overlay_visible;
        config.overlay_x = self.overlay_x;
        config.overlay_y = self.overlay_y;
        config.audio_source = self.audio_source;
        config.input_language = self.input_language.clone();
        config.target_language = self.target_language.clone();
        config.translate_mode = self.target_language != "original";
        Some(config)
    }
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll download progress
        if let Some(ref progress) = self.download_progress {
            if progress.is_finished() {
                if let Some(err) = progress.get_error() {
                    self.status = format!("Download failed: {}", err);
                } else {
                    self.status = "Download complete!".to_string();
                    self.model_downloaded = true;
                }
                self.download_progress = None;
            } else {
                let (downloaded, total) = progress.get_progress();
                let (current_file, total_files) = progress.get_file_progress();
                if total > 0 {
                    let percent = (downloaded as f64 / total as f64 * 100.0) as u32;
                    let mb_downloaded = downloaded as f64 / 1_000_000.0;
                    let mb_total = total as f64 / 1_000_000.0;
                    self.status = format!(
                        "File {}/{}: {:.1}/{:.1} MB ({}%)",
                        current_file, total_files, mb_downloaded, mb_total, percent
                    );
                } else {
                    self.status =
                        format!("Downloading file {}/{}...", current_file, total_files);
                }
            }
            ctx.request_repaint();
        }

        // Handle hotkey capture via egui input
        if self.hotkey_capture != HotkeyCapture::Idle {
            ctx.input(|input| {
                for event in &input.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    {
                        if matches!(
                            key,
                            egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                        ) {
                            // Allow arrow keys as hotkeys
                        }
                        let key_str = format_egui_key(*key, modifiers);
                        if let Some(hotkey) = key_str {
                            match self.hotkey_capture {
                                HotkeyCapture::CapturingPushToTalk => {
                                    self.push_to_talk_hotkey = hotkey;
                                }
                                HotkeyCapture::CapturingToggleListen => {
                                    self.toggle_listening_hotkey = hotkey;
                                }
                                HotkeyCapture::Idle => {}
                            }
                            self.hotkey_capture = HotkeyCapture::Idle;
                        }
                    }
                }
            });
        }

        // Handle start action (deferred to avoid borrow issues)
        if self.should_start {
            if let Some(config) = self.build_config() {
                if let Err(e) = config.save() {
                    self.status = format!("Error saving config: {}", e);
                    self.should_start = false;
                } else {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    std::process::exit(0);
                }
            } else {
                self.status = "Error building config".to_string();
                self.should_start = false;
            }
        }

        // Dark theme
        ctx.set_visuals(egui::Visuals::dark());

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Model, "Model");
                ui.selectable_value(&mut self.current_tab, Tab::Audio, "Audio");
                ui.selectable_value(&mut self.current_tab, Tab::Hotkeys, "Hotkeys");
                ui.selectable_value(&mut self.current_tab, Tab::Gpu, "GPU");
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.add_space(4.0);

            // Download progress bar
            if let Some(ref progress) = self.download_progress {
                let (downloaded, total) = progress.get_progress();
                if total > 0 {
                    let fraction = downloaded as f32 / total as f32;
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                } else {
                    ui.add(egui::ProgressBar::new(0.0).animate(true));
                }
                ui.add_space(4.0);
            }

            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can_start = self.selected_model.is_some()
                        && self.model_downloaded
                        && self.download_progress.is_none();
                    if ui
                        .add_enabled(can_start, egui::Button::new("Start"))
                        .clicked()
                    {
                        self.should_start = true;
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_tab {
                Tab::Model => self.render_model_tab(ui),
                Tab::Audio => self.render_audio_tab(ui),
                Tab::Hotkeys => self.render_hotkeys_tab(ui),
                Tab::Gpu => self.render_gpu_tab(ui),
            }
        });
    }
}

impl SetupApp {
    fn render_model_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Model Selection");
        ui.add_space(4.0);

        // Backend display (read-only)
        ui.horizontal(|ui| {
            ui.label("Backend:");
            let backend_text = self
                .selected_backend_id()
                .and_then(|id| self.backend_display_name(id))
                .unwrap_or("(auto-selected from model)");
            ui.label(
                egui::RichText::new(backend_text)
                    .color(if self.selected_backend_id().is_some() {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::GRAY
                    }),
            );
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        if self.all_models.is_empty() {
            ui.label(
                egui::RichText::new("No models found! Check backends/ folder.")
                    .color(egui::Color32::RED),
            );
            return;
        }

        // Model list
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 50.0)
            .show(ui, |ui| {
                let mut new_selection = self.selected_model;
                for (idx, unified) in self.all_models.iter().enumerate() {
                    let is_selected = self.selected_model == Some(idx);
                    let downloaded = is_model_downloaded(unified);

                    let label = format!(
                        "{} ({}) - {} MB{}",
                        unified.model.display_name,
                        unified.backend_name,
                        unified.model.size_mb,
                        if downloaded { " [Downloaded]" } else { "" }
                    );

                    let response = ui.selectable_label(is_selected, &label);
                    if response.clicked() {
                        new_selection = Some(idx);
                    }
                }
                if new_selection != self.selected_model {
                    self.selected_model = new_selection;
                    self.model_downloaded = self.check_model_exists();
                    if self.model_downloaded {
                        self.status = "Model ready! Click Start.".to_string();
                    } else {
                        self.status = "Click Download to get this model.".to_string();
                    }
                }
            });

        ui.add_space(4.0);

        // Download / Open Link buttons
        ui.horizontal(|ui| {
            let can_download = self.selected_model.is_some()
                && !self.model_downloaded
                && self.download_progress.is_none();
            if ui
                .add_enabled(can_download, egui::Button::new("Download"))
                .clicked()
            {
                self.start_download();
            }

            if let Some(idx) = self.selected_model {
                if let Some(unified) = self.all_models.get(idx) {
                    if !unified.model.download_url.is_empty()
                        && ui.button("Open Model Page").clicked()
                    {
                        let _ = open::that(&unified.model.download_url);
                    }
                }
            }
        });
    }

    fn render_audio_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Audio Settings");
        ui.add_space(8.0);

        // Audio source
        ui.label("Audio Source:");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.audio_source, AudioSource::Microphone, "Microphone");
            ui.radio_value(
                &mut self.audio_source,
                AudioSource::SystemAudio,
                "System Audio (Loopback)",
            );
        });

        ui.add_space(12.0);

        // Input device selection
        ui.label("Input Device:");
        let current_device = self
            .selected_input_device
            .as_deref()
            .unwrap_or(DEFAULT_DEVICE_LABEL);
        egui::ComboBox::from_id_salt("input_device")
            .selected_text(current_device)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for device_name in &self.input_devices.clone() {
                    let is_default = device_name == DEFAULT_DEVICE_LABEL;
                    let selected = if is_default {
                        self.selected_input_device.is_none()
                    } else {
                        self.selected_input_device.as_deref() == Some(device_name)
                    };
                    if ui.selectable_label(selected, device_name).clicked() {
                        if is_default {
                            self.selected_input_device = None;
                        } else {
                            self.selected_input_device = Some(device_name.clone());
                        }
                    }
                }
            });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Language settings
        ui.heading("Language");
        ui.add_space(4.0);

        // Input language
        ui.label("Input Language:");
        let input_lang_display = if self.input_language == "auto" {
            "Auto (detect)".to_string()
        } else {
            language_display_name(&self.input_language)
        };
        egui::ComboBox::from_id_salt("input_language")
            .selected_text(&input_lang_display)
            .width(250.0)
            .show_ui(ui, |ui| {
                // Auto option
                ui.selectable_value(&mut self.input_language, "auto".to_string(), "Auto (detect)");
                ui.separator();
                // All Whisper languages
                for &(code, name) in WHISPER_LANGUAGES {
                    ui.selectable_value(
                        &mut self.input_language,
                        code.to_string(),
                        name,
                    );
                }
            });

        ui.add_space(8.0);

        // Target language
        ui.label("Target Language:");
        egui::ComboBox::from_id_salt("target_language")
            .selected_text(if self.target_language == "original" {
                "Original Language"
            } else {
                "English (Translation)"
            })
            .width(250.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.target_language,
                    "original".to_string(),
                    "Original Language",
                );
                ui.selectable_value(
                    &mut self.target_language,
                    "en".to_string(),
                    "English (Translation)",
                );
            });

        if self.target_language == "en" {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Whisper translates any language to English.")
                    .color(egui::Color32::GRAY)
                    .small(),
            );
        }
    }

    fn render_hotkeys_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Hotkey Settings");
        ui.add_space(8.0);

        // Push-to-Talk
        ui.label("Push-to-Talk Hotkey:");
        ui.horizontal(|ui| {
            let display = format_hotkey_display(&self.push_to_talk_hotkey);
            if self.hotkey_capture == HotkeyCapture::CapturingPushToTalk {
                ui.label(
                    egui::RichText::new("Press any key...")
                        .color(egui::Color32::YELLOW)
                        .strong(),
                );
            } else {
                ui.monospace(&display);
            }
            if self.hotkey_capture == HotkeyCapture::CapturingPushToTalk {
                if ui.button("Cancel").clicked() {
                    self.hotkey_capture = HotkeyCapture::Idle;
                }
            } else {
                if ui.button("Change").clicked() {
                    self.hotkey_capture = HotkeyCapture::CapturingPushToTalk;
                }
            }
        });
        ui.label(
            egui::RichText::new("Hold to record, release to transcribe")
                .color(egui::Color32::GRAY)
                .small(),
        );

        ui.add_space(16.0);

        // Toggle Listen
        ui.label("Toggle Always-Listen Hotkey:");
        ui.horizontal(|ui| {
            let display = format_hotkey_display(&self.toggle_listening_hotkey);
            if self.hotkey_capture == HotkeyCapture::CapturingToggleListen {
                ui.label(
                    egui::RichText::new("Press any key...")
                        .color(egui::Color32::YELLOW)
                        .strong(),
                );
            } else {
                ui.monospace(&display);
            }
            if self.hotkey_capture == HotkeyCapture::CapturingToggleListen {
                if ui.button("Cancel").clicked() {
                    self.hotkey_capture = HotkeyCapture::Idle;
                }
            } else {
                if ui.button("Change").clicked() {
                    self.hotkey_capture = HotkeyCapture::CapturingToggleListen;
                }
            }
        });
        ui.label(
            egui::RichText::new("Toggle continuous speech detection")
                .color(egui::Color32::GRAY)
                .small(),
        );

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(8.0);

        // Always-listen tuning
        ui.heading("Always-Listen Settings");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("Silence timeout:");
            if ui.button("-").clicked() && self.silence_timeout_ms > 100 {
                self.silence_timeout_ms = self.silence_timeout_ms.saturating_sub(100);
            }
            ui.monospace(format!("{:.1}s", self.silence_timeout_ms as f64 / 1000.0));
            if ui.button("+").clicked() && self.silence_timeout_ms < 5000 {
                self.silence_timeout_ms = self.silence_timeout_ms.saturating_add(100);
            }
        });
        ui.label(
            egui::RichText::new("How long to wait after speech stops before transcribing")
                .color(egui::Color32::GRAY)
                .small(),
        );

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Streaming interval:");
            if ui.button("-").clicked() && self.streaming_interval_ms > 200 {
                self.streaming_interval_ms = self.streaming_interval_ms.saturating_sub(100);
            }
            ui.monospace(format!(
                "{:.1}s",
                self.streaming_interval_ms as f64 / 1000.0
            ));
            if ui.button("+").clicked() && self.streaming_interval_ms < 3000 {
                self.streaming_interval_ms = self.streaming_interval_ms.saturating_add(100);
            }
        });
        ui.label(
            egui::RichText::new("How often to send partial transcriptions while speaking")
                .color(egui::Color32::GRAY)
                .small(),
        );
    }

    fn render_gpu_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("GPU Acceleration");
        ui.add_space(8.0);

        ui.checkbox(&mut self.use_gpu, "Enable GPU acceleration (CUDA)");

        if !self.use_gpu {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("GPU is disabled. Transcription will use CPU only.")
                    .color(egui::Color32::GRAY),
            );
            return;
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // CUDA path
        ui.label("CUDA Toolkit Path:");
        ui.horizontal(|ui| {
            let path_text = self
                .cuda_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(not set)".to_string());
            let color = if self.cuda_valid {
                egui::Color32::LIGHT_GREEN
            } else if self.cuda_path.is_some() {
                egui::Color32::LIGHT_RED
            } else {
                egui::Color32::GRAY
            };
            ui.label(egui::RichText::new(&path_text).color(color));
        });
        ui.horizontal(|ui| {
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select CUDA Toolkit Directory")
                    .pick_folder()
                {
                    self.cuda_valid = validate_cuda_path(&path);
                    self.cuda_path = Some(path);
                    self.status = if self.cuda_valid {
                        "CUDA path set!".to_string()
                    } else {
                        "Warning: No cudart DLL found in bin/".to_string()
                    };
                }
            }
            if ui.button("Auto-detect").clicked() {
                self.cuda_path = detect_cuda_path();
                self.cuda_valid = self
                    .cuda_path
                    .as_ref()
                    .map(|p| validate_cuda_path(p))
                    .unwrap_or(false);
                if self.cuda_valid {
                    self.status = "CUDA detected!".to_string();
                } else {
                    self.status = "CUDA not found.".to_string();
                }
            }
        });
        if self.cuda_valid {
            ui.label(egui::RichText::new("CUDA OK").color(egui::Color32::LIGHT_GREEN));
        }

        ui.add_space(12.0);

        // cuDNN path
        ui.label("cuDNN Path:");
        ui.horizontal(|ui| {
            let path_text = self
                .cudnn_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(not set)".to_string());
            let color = if self.cudnn_valid {
                egui::Color32::LIGHT_GREEN
            } else if self.cudnn_path.is_some() {
                egui::Color32::LIGHT_RED
            } else {
                egui::Color32::GRAY
            };
            ui.label(egui::RichText::new(&path_text).color(color));
        });
        ui.horizontal(|ui| {
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select cuDNN Directory")
                    .pick_folder()
                {
                    self.cudnn_valid = validate_cudnn_path(&path);
                    self.cudnn_path = Some(path);
                    self.status = if self.cudnn_valid {
                        "cuDNN path set!".to_string()
                    } else {
                        "Warning: No cudnn DLL found in bin/".to_string()
                    };
                }
            }
            if ui.button("Auto-detect").clicked() {
                self.cudnn_path = detect_cudnn_path();
                self.cudnn_valid = self
                    .cudnn_path
                    .as_ref()
                    .map(|p| validate_cudnn_path(p))
                    .unwrap_or(false);
                if self.cudnn_valid {
                    self.status = "cuDNN detected!".to_string();
                } else {
                    self.status = "cuDNN not found.".to_string();
                }
            }
        });
        if self.cudnn_valid {
            ui.label(egui::RichText::new("cuDNN OK").color(egui::Color32::LIGHT_GREEN));
        }
    }

    fn start_download(&mut self) {
        let idx = match self.selected_model {
            Some(idx) => idx,
            None => {
                self.status = "Select a model first!".to_string();
                return;
            }
        };
        if self.download_progress.is_some() || self.model_downloaded {
            return;
        }
        let unified = &self.all_models[idx];
        if let Ok(models_dir) = get_models_dir() {
            let dest_folder = models_dir.join(&unified.model.folder_name);
            self.status = "Starting download...".to_string();
            self.download_progress = Some(downloader::start_manifest_model_download(
                &unified.backend_id,
                &unified.model,
                dest_folder,
            ));
        }
    }
}

fn is_model_downloaded(unified: &UnifiedModel) -> bool {
    if let Ok(models_dir) = get_models_dir() {
        let model_folder = models_dir.join(&unified.model.folder_name);
        if let Some(last_file) = unified.model.files.last() {
            model_folder.join(last_file).exists()
        } else {
            model_folder.exists()
        }
    } else {
        false
    }
}

fn language_display_name(code: &str) -> String {
    WHISPER_LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| code.to_string())
}

fn format_hotkey_display(key: &str) -> String {
    key.replace("Control", "Ctrl")
        .replace("Backquote", "`")
        .replace("Minus", "-")
        .replace("Equal", "=")
        .replace("BracketLeft", "[")
        .replace("BracketRight", "]")
        .replace("Backslash", "\\")
        .replace("Semicolon", ";")
        .replace("Quote", "'")
        .replace("Comma", ",")
        .replace("Period", ".")
        .replace("Slash", "/")
        .replace("Key", "")
        .replace("Digit", "")
        .replace("Arrow", "")
}

/// Convert an egui Key event to the same hotkey string format used by global-hotkey
fn format_egui_key(key: egui::Key, modifiers: &egui::Modifiers) -> Option<String> {
    // Skip pure modifier presses
    let key_name = match key {
        egui::Key::A => "KeyA",
        egui::Key::B => "KeyB",
        egui::Key::C => "KeyC",
        egui::Key::D => "KeyD",
        egui::Key::E => "KeyE",
        egui::Key::F => "KeyF",
        egui::Key::G => "KeyG",
        egui::Key::H => "KeyH",
        egui::Key::I => "KeyI",
        egui::Key::J => "KeyJ",
        egui::Key::K => "KeyK",
        egui::Key::L => "KeyL",
        egui::Key::M => "KeyM",
        egui::Key::N => "KeyN",
        egui::Key::O => "KeyO",
        egui::Key::P => "KeyP",
        egui::Key::Q => "KeyQ",
        egui::Key::R => "KeyR",
        egui::Key::S => "KeyS",
        egui::Key::T => "KeyT",
        egui::Key::U => "KeyU",
        egui::Key::V => "KeyV",
        egui::Key::W => "KeyW",
        egui::Key::X => "KeyX",
        egui::Key::Y => "KeyY",
        egui::Key::Z => "KeyZ",
        egui::Key::Num0 => "Digit0",
        egui::Key::Num1 => "Digit1",
        egui::Key::Num2 => "Digit2",
        egui::Key::Num3 => "Digit3",
        egui::Key::Num4 => "Digit4",
        egui::Key::Num5 => "Digit5",
        egui::Key::Num6 => "Digit6",
        egui::Key::Num7 => "Digit7",
        egui::Key::Num8 => "Digit8",
        egui::Key::Num9 => "Digit9",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        egui::Key::Enter => "Enter",
        egui::Key::Backspace => "Backspace",
        egui::Key::Space => "Space",
        egui::Key::Tab => "Tab",
        egui::Key::Escape => "Escape",
        egui::Key::Insert => "Insert",
        egui::Key::Delete => "Delete",
        egui::Key::Home => "Home",
        egui::Key::End => "End",
        egui::Key::PageUp => "PageUp",
        egui::Key::PageDown => "PageDown",
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::Backtick => "Backquote",
        egui::Key::Minus => "Minus",
        egui::Key::Equals => "Equal",
        egui::Key::OpenBracket => "BracketLeft",
        egui::Key::CloseBracket => "BracketRight",
        egui::Key::Backslash => "Backslash",
        egui::Key::Semicolon => "Semicolon",
        egui::Key::Quote => "Quote",
        egui::Key::Comma => "Comma",
        egui::Key::Period => "Period",
        egui::Key::Slash => "Slash",
        _ => return None,
    };

    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Control");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.mac_cmd || modifiers.command {
        parts.push("Super");
    }
    parts.push(key_name);
    Some(parts.join("+"))
}

/// Run the setup wizard. This function never returns - it either:
/// 1. Saves config, spawns app.exe, and exits (user clicks Start), or
/// 2. User closes the window and exits
pub fn run_setup() -> ! {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 600.0])
            .with_min_inner_size([400.0, 500.0]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Speech-to-Text Setup",
        options,
        Box::new(|_cc| Ok(Box::new(SetupApp::new()))),
    );

    std::process::exit(0);
}
