use anyhow::Result;
use image::GenericImageView;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

// Embed icon files at compile time
const ICON_GRAY: &[u8] = include_bytes!("../assets/mic_gray.png");
const ICON_RED: &[u8] = include_bytes!("../assets/mic_red.png");
const ICON_YELLOW: &[u8] = include_bytes!("../assets/mic_yellow.png");
const ICON_GREEN: &[u8] = include_bytes!("../assets/mic_green.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppStatus {
    Idle,
    Recording,
    Processing,
    AlwaysListening,
}

pub struct TrayManager {
    tray: TrayIcon,
    pub show_overlay_id: MenuId,
    pub settings_id: MenuId,
    pub exit_id: MenuId,
    icons: TrayIcons,
}

struct TrayIcons {
    idle: Icon,
    recording: Icon,
    processing: Icon,
    always_listening: Icon,
}

impl TrayManager {
    pub fn new() -> Result<Self> {
        let icons = TrayIcons::new()?;

        let show_overlay_item = MenuItem::new("Show/Hide Overlay", true, None);
        let settings_item = MenuItem::new("Settings", true, None);
        let exit_item = MenuItem::new("Exit", true, None);

        let show_overlay_id = show_overlay_item.id().clone();
        let settings_id = settings_item.id().clone();
        let exit_id = exit_item.id().clone();

        let menu = Menu::new();
        menu.append(&show_overlay_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&settings_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&exit_item)?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("Speech to Text - Idle")
            .with_icon(icons.idle.clone())
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create tray icon: {}", e))?;

        Ok(Self {
            tray,
            show_overlay_id,
            settings_id,
            exit_id,
            icons,
        })
    }

    pub fn set_status(&mut self, status: AppStatus) {
        let (icon, tooltip) = match status {
            AppStatus::Idle => (&self.icons.idle, "Speech to Text - Idle"),
            AppStatus::Recording => (&self.icons.recording, "Speech to Text - Recording..."),
            AppStatus::Processing => (&self.icons.processing, "Speech to Text - Processing..."),
            AppStatus::AlwaysListening => {
                (&self.icons.always_listening, "Speech to Text - Listening...")
            }
        };

        let _ = self.tray.set_icon(Some(icon.clone()));
        let _ = self.tray.set_tooltip(Some(tooltip));
    }

    pub fn menu_receiver() -> crossbeam_channel::Receiver<MenuEvent> {
        MenuEvent::receiver().clone()
    }
}

impl TrayIcons {
    fn new() -> Result<Self> {
        Ok(Self {
            idle: load_png_icon(ICON_GRAY)?,
            recording: load_png_icon(ICON_RED)?,
            processing: load_png_icon(ICON_YELLOW)?,
            always_listening: load_png_icon(ICON_GREEN)?,
        })
    }
}

/// Load an icon from embedded PNG data
fn load_png_icon(png_data: &[u8]) -> Result<Icon> {
    let img = image::load_from_memory(png_data)
        .map_err(|e| anyhow::anyhow!("Failed to decode PNG: {}", e))?;

    // Resize to 32x32 for system tray
    let img = img.resize_exact(32, 32, image::imageops::FilterType::Lanczos3);

    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8().into_raw();

    Icon::from_rgba(rgba, width, height)
        .map_err(|e| anyhow::anyhow!("Failed to create icon: {}", e))
}
