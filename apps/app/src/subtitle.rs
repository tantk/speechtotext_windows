//! Subtitle overlay bar for displaying transcribed/translated text
//!
//! Renders text using GDI (Windows) for proper font support including CJK,
//! displayed in a semi-transparent always-on-top window.

use anyhow::Result;
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;
use tao::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::EventLoopWindowTarget,
    platform::windows::WindowExtWindows,
    window::{Window, WindowBuilder},
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    GetDC, GetDIBits, ReleaseDC, SelectObject, SetBkColor, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HFONT, OPAQUE,
};

const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 44;
const BG_COLOR: u32 = 0xFF_1A1A2E;
const TEXT_COLOR_RGB: u32 = 0x00_F0F0F0;
const FADE_SECONDS: f64 = 5.0;

pub const AUTO_FONT: &str = "Auto";

pub const FONTS: &[&str] = &[
    "Auto",
    "Segoe UI",
    "Arial",
    "Calibri",
    "Verdana",
    "Tahoma",
    "Consolas",
    "Comic Sans MS",
];

pub const FONT_SIZES: &[u32] = &[16, 20, 24, 28, 32, 36, 40, 48];

pub struct SubtitleBar {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    width: u32,
    height: u32,
    text: String,
    last_text_time: Option<Instant>,
    visible: bool,
    font_name: String,
    font_size: u32,
}

impl SubtitleBar {
    pub fn new<T>(
        event_loop: &EventLoopWindowTarget<T>,
        saved_x: Option<i32>,
        saved_y: Option<i32>,
        saved_w: Option<u32>,
        saved_h: Option<u32>,
        font_name: &str,
        font_size: u32,
    ) -> Result<Self> {
        let w = saved_w.unwrap_or(DEFAULT_WIDTH);
        let h = saved_h.unwrap_or(DEFAULT_HEIGHT);

        let window = WindowBuilder::new()
            .with_title("Subtitles")
            .with_inner_size(LogicalSize::new(w as f64, h as f64))
            .with_decorations(false)
            .with_always_on_top(true)
            .with_resizable(true)
            .with_visible(false)
            .build(event_loop)
            .map_err(|e| anyhow::anyhow!("Failed to create subtitle window: {}", e))?;

        match (saved_x, saved_y) {
            (Some(x), Some(y)) if x > -30_000 && y > -30_000 => {
                window.set_outer_position(PhysicalPosition::new(x, y));
            }
            _ => {
                // Default: centered near bottom of screen
                if let Some(monitor) = window.primary_monitor() {
                    let size = monitor.size();
                    let x = ((size.width - w) / 2) as i32;
                    let y = (size.height - h - 60) as i32;
                    window.set_outer_position(PhysicalPosition::new(x, y));
                }
            }
        }

        // Semi-transparent, no taskbar entry
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
            };
            let hwnd = HWND(window.hwnd() as *mut _);
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            SetWindowLongW(
                hwnd,
                GWL_EXSTYLE,
                style | WS_EX_LAYERED.0 as i32 | WS_EX_TOOLWINDOW.0 as i32,
            );
            use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA};
            SetLayeredWindowAttributes(hwnd, None, 220, LWA_ALPHA)?;
        }

        let window = Rc::new(window);
        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| anyhow::anyhow!("softbuffer context: {}", e))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|e| anyhow::anyhow!("softbuffer surface: {}", e))?;

        Ok(Self {
            window,
            surface,
            width: w,
            height: h,
            text: String::new(),
            last_text_time: None,
            visible: false,
            font_name: font_name.to_string(),
            font_size,
        })
    }

    pub fn start_drag(&self) {
        let _ = self.window.drag_window();
    }

    pub fn get_position(&self) -> (i32, i32) {
        let pos = self
            .window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0));
        (pos.x, pos.y)
    }

    pub fn get_size(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width, size.height)
    }

    pub fn hwnd(&self) -> isize {
        self.window.hwnd()
    }

    pub fn window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    pub fn show(&mut self) {
        if !self.visible {
            self.visible = true;
            self.window.set_visible(true);
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            self.visible = false;
            self.window.set_visible(false);
        }
    }

    pub fn toggle_visibility(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.trim_to_fit();
        self.last_text_time = Some(Instant::now());
        if self.visible {
            self.render();
        }
    }

    /// Append text for rolling subtitle display. Trims old text to keep it fitting.
    pub fn append_text(&mut self, text: &str) {
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(text);
        self.trim_to_fit();
        self.last_text_time = Some(Instant::now());
        if self.visible {
            self.render();
        }
    }

    /// Trim text from the front until it fits within the subtitle bar dimensions.
    fn trim_to_fit(&mut self) {
        if self.text.is_empty() || self.width == 0 || self.height == 0 {
            return;
        }
        let effective_font = if self.font_name == AUTO_FONT {
            detect_script_font(&self.text).unwrap_or("Segoe UI")
        } else {
            &self.font_name
        };
        while !self.text.is_empty() {
            let text_h =
                measure_text_height(&self.text, self.width, self.height, effective_font, self.font_size);
            if text_h <= self.height {
                break;
            }
            let drop = (self.width / self.font_size.max(8)) as usize;
            let drop = drop.max(1);
            let char_count = self.text.chars().count();
            if char_count <= drop {
                break;
            }
            if let Some((byte_offset, _)) = self.text.char_indices().nth(drop) {
                if let Some(pos) = self.text[byte_offset..]
                    .find(|c: char| c == ' ' || c == '\u{3000}')
                {
                    self.text = self.text[byte_offset + pos + 1..].to_string();
                } else {
                    self.text = self.text[byte_offset..].to_string();
                }
            } else {
                break;
            }
        }
    }

    pub fn set_font(&mut self, name: &str) {
        self.font_name = name.to_string();
        if self.visible {
            self.render();
        }
    }

    pub fn set_font_size(&mut self, size: u32) {
        self.font_size = size;
        let new_h = (size as f64 * 1.8).max(DEFAULT_HEIGHT as f64) as u32;
        self.window
            .set_inner_size(PhysicalSize::new(self.width, new_h));
        if self.visible {
            self.render();
        }
    }

    pub fn handle_redraw(&mut self) {
        self.render();
    }

    /// Check if text should fade out. Returns true if text was cleared.
    pub fn check_fade(&mut self) -> bool {
        if let Some(t) = self.last_text_time {
            if t.elapsed().as_secs_f64() > FADE_SECONDS && !self.text.is_empty() {
                self.text.clear();
                self.last_text_time = None;
                if self.visible {
                    self.render();
                }
                return true;
            }
        }
        false
    }

    fn render(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.width = size.width;
        self.height = size.height;

        if let (Some(w), Some(h)) = (NonZeroU32::new(self.width), NonZeroU32::new(self.height)) {
            let _ = self.surface.resize(w, h);
        } else {
            return;
        }

        let pixels = if !self.text.is_empty() {
            render_text_gdi(
                &self.text,
                self.width,
                self.height,
                &self.font_name,
                self.font_size,
            )
        } else {
            None
        };

        if let Ok(mut buffer) = self.surface.buffer_mut() {
            if let Some(px) = pixels {
                for (i, pixel) in buffer.iter_mut().enumerate() {
                    *pixel = if i < px.len() { px[i] } else { BG_COLOR };
                }
            } else {
                for pixel in buffer.iter_mut() {
                    *pixel = BG_COLOR;
                }
            }
            let _ = buffer.present();
        }
    }
}

/// Detect the dominant script and return an appropriate system font.
fn detect_script_font(text: &str) -> Option<&'static str> {
    let mut cjk = 0u32;
    let mut japanese = 0u32;
    let mut korean = 0u32;
    let mut arabic = 0u32;
    let mut thai = 0u32;
    let mut devanagari = 0u32;
    let mut total = 0u32;

    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        total += 1;
        let cp = ch as u32;
        match cp {
            0x3040..=0x309F | 0x30A0..=0x30FF | 0x31F0..=0x31FF => japanese += 1,
            0xAC00..=0xD7AF | 0x1100..=0x11FF => korean += 1,
            0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF => cjk += 1,
            0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF => arabic += 1,
            0x0E00..=0x0E7F => thai += 1,
            0x0900..=0x097F => devanagari += 1,
            _ => {}
        }
    }

    if total == 0 {
        return None;
    }

    if japanese > 0 && (japanese + cjk) * 2 > total {
        return Some("Yu Gothic UI");
    }
    if korean > 0 && (korean + cjk) * 2 > total {
        return Some("Malgun Gothic");
    }
    if cjk * 2 > total {
        return Some("Microsoft YaHei");
    }
    if arabic * 2 > total {
        return Some("Segoe UI");
    }
    if thai * 2 > total {
        return Some("Leelawadee UI");
    }
    if devanagari * 2 > total {
        return Some("Nirmala UI");
    }

    None
}

/// Measure the height that text would occupy when rendered with word-wrap.
fn measure_text_height(text: &str, width: u32, _height: u32, font_name: &str, font_size: u32) -> u32 {
    unsafe {
        let hdc_screen = GetDC(HWND::default());
        let hdc_mem = CreateCompatibleDC(hdc_screen);

        let font_wide: Vec<u16> = font_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut face_buf = [0u16; 32];
        for (i, &ch) in font_wide.iter().enumerate() {
            if i >= 32 {
                break;
            }
            face_buf[i] = ch;
        }

        let hfont: HFONT = CreateFontW(
            -(font_size as i32),
            0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 0, 0,
            windows::core::PCWSTR(face_buf.as_ptr()),
        );
        let old_font = SelectObject(hdc_mem, hfont);

        let mut rect = RECT {
            left: 4,
            top: 2,
            right: width as i32 - 4,
            bottom: 0,
        };

        let mut wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            hdc_mem,
            &mut wide,
            &mut rect,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
        );

        let measured_height = (rect.bottom - rect.top) as u32 + 4;

        SelectObject(hdc_mem, old_font);
        let _ = DeleteObject(hfont);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(HWND::default(), hdc_screen);

        measured_height
    }
}

/// Render text to a pixel buffer using Windows GDI for proper font support.
fn render_text_gdi(
    text: &str,
    width: u32,
    height: u32,
    font_name: &str,
    font_size: u32,
) -> Option<Vec<u32>> {
    let effective_font = if font_name == AUTO_FONT {
        detect_script_font(text).unwrap_or("Segoe UI")
    } else {
        font_name
    };

    unsafe {
        let hdc_screen = GetDC(HWND::default());
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbmp = CreateCompatibleBitmap(hdc_screen, width as i32, height as i32);
        let old_bmp = SelectObject(hdc_mem, hbmp);

        let bg_r = (BG_COLOR >> 16) & 0xFF;
        let bg_g = (BG_COLOR >> 8) & 0xFF;
        let bg_b = BG_COLOR & 0xFF;
        let bg_colorref = bg_b | (bg_g << 8) | (bg_r << 16);
        SetBkMode(hdc_mem, OPAQUE);
        SetBkColor(
            hdc_mem,
            windows::Win32::Foundation::COLORREF(bg_colorref),
        );
        SetTextColor(
            hdc_mem,
            windows::Win32::Foundation::COLORREF(TEXT_COLOR_RGB),
        );

        let font_wide: Vec<u16> = effective_font
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut face_buf = [0u16; 32];
        for (i, &ch) in font_wide.iter().enumerate() {
            if i >= 32 {
                break;
            }
            face_buf[i] = ch;
        }

        let hfont: HFONT = CreateFontW(
            -(font_size as i32),
            0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 0, 0,
            windows::core::PCWSTR(face_buf.as_ptr()),
        );
        let old_font = SelectObject(hdc_mem, hfont);

        let mut wide: Vec<u16> = text.encode_utf16().collect();

        // Measure text height first to decide layout
        let mut measure_rect = RECT {
            left: 4,
            top: 0,
            right: width as i32 - 4,
            bottom: 0,
        };
        let mut measure_wide = wide.clone();
        DrawTextW(
            hdc_mem,
            &mut measure_wide,
            &mut measure_rect,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
        );
        let text_height = measure_rect.bottom - measure_rect.top;
        let bar_height = height as i32;

        let line_height = font_size as i32 + 4;
        if bar_height <= line_height * 2 && text_height <= bar_height {
            // Single line: vertically centered
            let mut rect = RECT {
                left: 4,
                top: 2,
                right: width as i32 - 4,
                bottom: bar_height,
            };
            DrawTextW(
                hdc_mem,
                &mut wide,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        } else {
            // Multi-line: align to bottom so latest text is always visible
            let top = if text_height > bar_height {
                bar_height - text_height // negative offset scrolls to bottom
            } else {
                2
            };
            let mut rect = RECT {
                left: 4,
                top,
                right: width as i32 - 4,
                bottom: bar_height,
            };
            DrawTextW(
                hdc_mem,
                &mut wide,
                &mut rect,
                DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
            );
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let pixel_count = (width * height) as usize;
        let mut bgra: Vec<u8> = vec![0u8; pixel_count * 4];
        GetDIBits(
            hdc_mem,
            hbmp,
            0,
            height,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old_font);
        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hfont);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(HWND::default(), hdc_screen);

        let mut pixels = Vec::with_capacity(pixel_count);
        for i in 0..pixel_count {
            let off = i * 4;
            let b = bgra[off] as u32;
            let g = bgra[off + 1] as u32;
            let r = bgra[off + 2] as u32;
            pixels.push(0xFF_000000 | (r << 16) | (g << 8) | b);
        }

        Some(pixels)
    }
}
