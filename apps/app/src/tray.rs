use anyhow::Result;
use image::GenericImageView;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
    Icon, TrayIcon, TrayIconBuilder,
};

// Embed icon files at compile time
const ICON_GRAY: &[u8] = include_bytes!("../assets/mic_gray.png");
const ICON_RED: &[u8] = include_bytes!("../assets/mic_red.png");
const ICON_YELLOW: &[u8] = include_bytes!("../assets/mic_yellow.png");
const ICON_GREEN: &[u8] = include_bytes!("../assets/mic_green.png");

/// All Whisper-supported languages: (code, display_name)
pub const WHISPER_LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("am", "Amharic"),
    ("ar", "Arabic"),
    ("as", "Assamese"),
    ("az", "Azerbaijani"),
    ("ba", "Bashkir"),
    ("be", "Belarusian"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("bo", "Tibetan"),
    ("br", "Breton"),
    ("bs", "Bosnian"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("cy", "Welsh"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("eu", "Basque"),
    ("fa", "Persian"),
    ("fi", "Finnish"),
    ("fo", "Faroese"),
    ("fr", "French"),
    ("gl", "Galician"),
    ("gu", "Gujarati"),
    ("ha", "Hausa"),
    ("haw", "Hawaiian"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("ht", "Haitian Creole"),
    ("hu", "Hungarian"),
    ("hy", "Armenian"),
    ("id", "Indonesian"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("jw", "Javanese"),
    ("ka", "Georgian"),
    ("kk", "Kazakh"),
    ("km", "Khmer"),
    ("kn", "Kannada"),
    ("ko", "Korean"),
    ("la", "Latin"),
    ("lb", "Luxembourgish"),
    ("ln", "Lingala"),
    ("lo", "Lao"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("mg", "Malagasy"),
    ("mi", "Maori"),
    ("mk", "Macedonian"),
    ("ml", "Malayalam"),
    ("mn", "Mongolian"),
    ("mr", "Marathi"),
    ("ms", "Malay"),
    ("mt", "Maltese"),
    ("my", "Myanmar"),
    ("ne", "Nepali"),
    ("nl", "Dutch"),
    ("nn", "Nynorsk"),
    ("no", "Norwegian"),
    ("oc", "Occitan"),
    ("pa", "Punjabi"),
    ("pl", "Polish"),
    ("ps", "Pashto"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sa", "Sanskrit"),
    ("sd", "Sindhi"),
    ("si", "Sinhala"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sn", "Shona"),
    ("so", "Somali"),
    ("sq", "Albanian"),
    ("sr", "Serbian"),
    ("su", "Sundanese"),
    ("sv", "Swedish"),
    ("sw", "Swahili"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("tg", "Tajik"),
    ("th", "Thai"),
    ("tk", "Turkmen"),
    ("tl", "Tagalog"),
    ("tr", "Turkish"),
    ("tt", "Tatar"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("yi", "Yiddish"),
    ("yo", "Yoruba"),
    ("yue", "Cantonese"),
    ("zh", "Chinese"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppStatus {
    Idle,
    Recording,
    Processing,
    AlwaysListening,
    AlwaysListeningRecording, // Active speech detected in always-listen mode
}

/// A submenu of language CheckMenuItems acting as radio buttons
struct LanguageMenu {
    items: Vec<(CheckMenuItem, String)>, // (menu_item, language_code)
}

impl LanguageMenu {
    /// Find the language code for a given menu ID
    fn find_by_menu_id(&self, id: &MenuId) -> Option<String> {
        self.items
            .iter()
            .find(|(item, _)| *item.id() == *id)
            .map(|(_, code)| code.clone())
    }

    /// Set the selected language (unchecks all others)
    fn set_selected(&self, code: &str) {
        for (item, item_code) in &self.items {
            item.set_checked(item_code == code);
        }
    }
}

pub struct TrayManager {
    tray: TrayIcon,
    pub show_overlay_id: MenuId,
    pub show_subtitle_id: MenuId,
    pub type_to_window_id: MenuId,
    pub settings_id: MenuId,
    pub exit_id: MenuId,
    pub mic_source_id: MenuId,
    pub system_audio_source_id: MenuId,
    mic_source_item: CheckMenuItem,
    system_audio_source_item: CheckMenuItem,
    show_subtitle_item: CheckMenuItem,
    type_to_window_item: CheckMenuItem,
    input_language_menu: LanguageMenu,
    target_language_menu: LanguageMenu,
    icons: TrayIcons,
}

struct TrayIcons {
    idle: Icon,
    recording: Icon,
    processing: Icon,
    always_listening: Icon,
}

impl TrayManager {
    pub fn new(
        system_audio: bool,
        input_language: &str,
        target_language: &str,
        subtitle_visible: bool,
        type_to_window: bool,
    ) -> Result<Self> {
        let icons = TrayIcons::new()?;

        let show_overlay_item = MenuItem::new("Show/Hide Overlay", true, None);
        let show_subtitle_item =
            CheckMenuItem::new("Subtitle Bar", true, subtitle_visible, None);
        let type_to_window_item =
            CheckMenuItem::new("Type to Window", true, type_to_window, None);
        let settings_item = MenuItem::new("Settings", true, None);
        let exit_item = MenuItem::new("Exit", true, None);

        // Audio source submenu
        let mic_source_item = CheckMenuItem::new("Microphone", true, !system_audio, None);
        let system_audio_source_item =
            CheckMenuItem::new("System Audio", true, system_audio, None);
        let audio_source_submenu = Submenu::new("Audio Source", true);
        audio_source_submenu.append(&mic_source_item)?;
        audio_source_submenu.append(&system_audio_source_item)?;

        // Input language submenu
        let input_lang_submenu = Submenu::new("Input Language", true);
        let mut input_lang_items = Vec::new();
        let auto_item = CheckMenuItem::new(
            "Auto (detect)",
            true,
            input_language == "auto",
            None,
        );
        input_lang_submenu.append(&auto_item)?;
        input_lang_items.push((auto_item, "auto".to_string()));
        input_lang_submenu.append(&PredefinedMenuItem::separator())?;
        for &(code, name) in WHISPER_LANGUAGES {
            let item = CheckMenuItem::new(name, true, input_language == code, None);
            input_lang_submenu.append(&item)?;
            input_lang_items.push((item, code.to_string()));
        }

        // Target language submenu (only Original + English, since Whisper only translates to English)
        let target_lang_submenu = Submenu::new("Target Language", true);
        let mut target_lang_items = Vec::new();
        let original_item = CheckMenuItem::new(
            "Original Language",
            true,
            target_language == "original",
            None,
        );
        let english_item = CheckMenuItem::new(
            "English (Translation)",
            true,
            target_language == "en",
            None,
        );
        target_lang_submenu.append(&original_item)?;
        target_lang_submenu.append(&english_item)?;
        target_lang_items.push((original_item, "original".to_string()));
        target_lang_items.push((english_item, "en".to_string()));

        let show_overlay_id = show_overlay_item.id().clone();
        let show_subtitle_id = show_subtitle_item.id().clone();
        let type_to_window_id = type_to_window_item.id().clone();
        let settings_id = settings_item.id().clone();
        let exit_id = exit_item.id().clone();
        let mic_source_id = mic_source_item.id().clone();
        let system_audio_source_id = system_audio_source_item.id().clone();

        let menu = Menu::new();
        menu.append(&show_overlay_item)?;
        menu.append(&show_subtitle_item)?;
        menu.append(&type_to_window_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&audio_source_submenu)?;
        menu.append(&input_lang_submenu)?;
        menu.append(&target_lang_submenu)?;
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
            show_subtitle_id,
            type_to_window_id,
            settings_id,
            exit_id,
            mic_source_id,
            system_audio_source_id,
            mic_source_item,
            system_audio_source_item,
            show_subtitle_item,
            type_to_window_item,
            input_language_menu: LanguageMenu {
                items: input_lang_items,
            },
            target_language_menu: LanguageMenu {
                items: target_lang_items,
            },
            icons,
        })
    }

    pub fn set_audio_source_microphone(&self) {
        self.mic_source_item.set_checked(true);
        self.system_audio_source_item.set_checked(false);
    }

    pub fn set_audio_source_system_audio(&self) {
        self.mic_source_item.set_checked(false);
        self.system_audio_source_item.set_checked(true);
    }

    pub fn set_input_language(&self, code: &str) {
        self.input_language_menu.set_selected(code);
    }

    pub fn set_target_language(&self, code: &str) {
        self.target_language_menu.set_selected(code);
    }

    /// Check if a menu ID belongs to the input language menu. Returns the language code if so.
    pub fn input_language_for_menu_id(&self, id: &MenuId) -> Option<String> {
        self.input_language_menu.find_by_menu_id(id)
    }

    /// Check if a menu ID belongs to the target language menu. Returns the language code if so.
    pub fn target_language_for_menu_id(&self, id: &MenuId) -> Option<String> {
        self.target_language_menu.find_by_menu_id(id)
    }

    pub fn set_subtitle_visible(&self, visible: bool) {
        self.show_subtitle_item.set_checked(visible);
    }

    pub fn set_type_to_window(&self, enabled: bool) {
        self.type_to_window_item.set_checked(enabled);
    }

    pub fn set_status(&mut self, status: AppStatus) {
        let (icon, tooltip) = match status {
            AppStatus::Idle => (&self.icons.idle, "Speech to Text - Idle"),
            AppStatus::Recording => (&self.icons.recording, "Speech to Text - Recording..."),
            AppStatus::Processing => {
                (&self.icons.processing, "Speech to Text - Processing...")
            }
            AppStatus::AlwaysListening => {
                (&self.icons.always_listening, "Speech to Text - Listening...")
            }
            AppStatus::AlwaysListeningRecording => {
                (&self.icons.recording, "Speech to Text - Speaking...")
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
