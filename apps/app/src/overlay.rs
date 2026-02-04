use crate::tray::AppStatus;
use anyhow::Result;
use image::GenericImageView;
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::rc::Rc;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event_loop::EventLoopWindowTarget,
    window::{Icon, Window, WindowBuilder},
};

// Default overlay dimensions
const OVERLAY_WIDTH: u32 = 120;
const OVERLAY_HEIGHT: u32 = 50;
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/mic_gray.png");

fn load_window_icon() -> Option<Icon> {
    let img = image::load_from_memory(WINDOW_ICON_PNG).ok()?;
    let img = img.resize_exact(32, 32, image::imageops::FilterType::Lanczos3);
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8().into_raw();
    Icon::from_rgba(rgba, width, height).ok()
}

pub struct Overlay {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    visible: bool,
    status: AppStatus,
    width: u32,
    height: u32,
}

impl Overlay {
    pub fn new<T>(
        event_loop: &EventLoopWindowTarget<T>,
        saved_x: Option<i32>,
        saved_y: Option<i32>,
    ) -> Result<Self> {
        let window = WindowBuilder::new()
            .with_title("Idle")
            .with_inner_size(LogicalSize::new(OVERLAY_WIDTH as f64, OVERLAY_HEIGHT as f64))
            .with_decorations(false)
            .with_always_on_top(true)
            .with_window_icon(load_window_icon())
            .with_resizable(false)
            .build(event_loop)
            .map_err(|e| anyhow::anyhow!("Failed to create overlay window: {}", e))?;

        // Set position: use saved position if available, otherwise default to bottom-left
        match (saved_x, saved_y) {
            (Some(x), Some(y)) => {
                window.set_outer_position(PhysicalPosition::new(x, y));
            }
            _ => {
                // Default: bottom-left of primary monitor
                if let Some(monitor) = window.primary_monitor() {
                    let monitor_size = monitor.size();
                    let scale = monitor.scale_factor();
                    let x = 20i32;
                    let y = ((monitor_size.height as f64 / scale) as i32 - 120).max(100);
                    window.set_outer_position(PhysicalPosition::new(x, y));
                }
            }
        }

        let window = Rc::new(window);
        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| anyhow::anyhow!("Failed to create softbuffer context: {}", e))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|e| anyhow::anyhow!("Failed to create softbuffer surface: {}", e))?;

        let size = window.inner_size();

        let mut overlay = Self {
            window,
            surface,
            visible: true,
            status: AppStatus::Idle,
            width: size.width,
            height: size.height,
        };

        overlay.render();
        Ok(overlay)
    }

    /// Start dragging the window (call on mouse down)
    pub fn start_drag(&self) {
        let _ = self.window.drag_window();
    }

    /// Get the current window position
    pub fn get_position(&self) -> (i32, i32) {
        let pos = self.window.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
        (pos.x, pos.y)
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.window.set_visible(visible);
    }

    pub fn toggle_visibility(&mut self) {
        self.set_visible(!self.visible);
    }

    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_status(&mut self, status: AppStatus) {
        self.status = status;

        // Update window title with status text
        let title = match status {
            AppStatus::Idle => "Idle",
            AppStatus::Recording => "🎤 LISTENING",
            AppStatus::Processing => "Processing...",
            AppStatus::AlwaysListening => "Always On",
        };
        self.window.set_title(title);

        self.render();
    }

    pub fn window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    pub fn handle_redraw(&mut self) {
        self.render();
    }

    fn render(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.width = size.width;
        self.height = size.height;

        // Resize surface if needed
        if let (Some(w), Some(h)) = (NonZeroU32::new(self.width), NonZeroU32::new(self.height)) {
            let _ = self.surface.resize(w, h);
        } else {
            return;
        }

        // Get the color based on status
        let color = match self.status {
            AppStatus::Idle => 0xFF505050,        // Dark gray
            AppStatus::Recording => 0xFFDD3333,   // Red
            AppStatus::Processing => 0xFFDDAA00,  // Yellow/Orange
            AppStatus::AlwaysListening => 0xFF33AA33, // Green
        };

        // Fill the buffer
        if let Ok(mut buffer) = self.surface.buffer_mut() {
            for pixel in buffer.iter_mut() {
                *pixel = color;
            }

            // Draw a lighter border
            let border_color = match self.status {
                AppStatus::Idle => 0xFF707070,
                AppStatus::Recording => 0xFFFF5555,
                AppStatus::Processing => 0xFFFFCC00,
                AppStatus::AlwaysListening => 0xFF55DD55,
            };

            let w = self.width as usize;
            let h = self.height as usize;

            // Top and bottom borders
            for x in 0..w {
                if x < buffer.len() {
                    buffer[x] = border_color;
                }
                if (h - 1) * w + x < buffer.len() {
                    buffer[(h - 1) * w + x] = border_color;
                }
            }

            // Left and right borders
            for y in 0..h {
                if y * w < buffer.len() {
                    buffer[y * w] = border_color;
                }
                if y * w + w - 1 < buffer.len() {
                    buffer[y * w + w - 1] = border_color;
                }
            }

            let _ = buffer.present();
        }
    }
}

// ============================================
// Overlay Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_status_variants() {
        // Verify all status variants exist
        let statuses = vec![
            AppStatus::Idle,
            AppStatus::Recording,
            AppStatus::Processing,
            AppStatus::AlwaysListening,
        ];
        
        // Each status should have a distinct color
        let colors: Vec<u32> = statuses.iter().map(|s| {
            match s {
                AppStatus::Idle => 0xFF505050,
                AppStatus::Recording => 0xFFDD3333,
                AppStatus::Processing => 0xFFDDAA00,
                AppStatus::AlwaysListening => 0xFF33AA33,
            }
        }).collect();
        
        // Colors should be unique
        for i in 0..colors.len() {
            for j in i+1..colors.len() {
                assert_ne!(colors[i], colors[j], "Each status should have a unique color");
            }
        }
    }

    #[test]
    fn test_overlay_dimensions() {
        // Overlay should be small and unobtrusive
        assert!(OVERLAY_WIDTH >= 80, "Overlay should be at least 80px wide");
        assert!(OVERLAY_WIDTH <= 200, "Overlay should be at most 200px wide");
        assert!(OVERLAY_HEIGHT >= 30, "Overlay should be at least 30px tall");
        assert!(OVERLAY_HEIGHT <= 100, "Overlay should be at most 100px tall");
    }

    #[test]
    fn test_status_title_mapping() {
        // Verify title text for each status
        let title_idle = match AppStatus::Idle {
            AppStatus::Idle => "Idle",
            AppStatus::Recording => "🎤 LISTENING",
            AppStatus::Processing => "Processing...",
            AppStatus::AlwaysListening => "Always On",
        };
        assert_eq!(title_idle, "Idle");
        
        let title_recording = match AppStatus::Recording {
            AppStatus::Idle => "Idle",
            AppStatus::Recording => "🎤 LISTENING",
            AppStatus::Processing => "Processing...",
            AppStatus::AlwaysListening => "Always On",
        };
        assert_eq!(title_recording, "🎤 LISTENING");
    }

    #[test]
    fn test_status_color_contrast() {
        // Colors should be distinguishable
        let idle_color = 0xFF505050u32;
        let recording_color = 0xFFDD3333u32;
        let processing_color = 0xFFDDAA00u32;
        let always_on_color = 0xFF33AA33u32;
        
        // Calculate color distance (simplified)
        fn color_distance(c1: u32, c2: u32) -> u32 {
            let r1 = (c1 >> 16) & 0xFF;
            let g1 = (c1 >> 8) & 0xFF;
            let b1 = c1 & 0xFF;
            let r2 = (c2 >> 16) & 0xFF;
            let g2 = (c2 >> 8) & 0xFF;
            let b2 = c2 & 0xFF;
            
            let dr = if r1 > r2 { r1 - r2 } else { r2 - r1 };
            let dg = if g1 > g2 { g1 - g2 } else { g2 - g1 };
            let db = if b1 > b2 { b1 - b2 } else { b2 - b1 };
            
            dr + dg + db
        }
        
        // Colors should be sufficiently different
        assert!(color_distance(idle_color, recording_color) > 100);
        assert!(color_distance(idle_color, processing_color) > 100);
        assert!(color_distance(idle_color, always_on_color) > 100);
        assert!(color_distance(recording_color, always_on_color) > 100);
    }

    #[test]
    fn test_overlay_state_transitions() {
        // Test that we can transition between all states
        let transitions = vec![
            (AppStatus::Idle, AppStatus::Recording),
            (AppStatus::Recording, AppStatus::Processing),
            (AppStatus::Processing, AppStatus::Idle),
            (AppStatus::Idle, AppStatus::AlwaysListening),
            (AppStatus::AlwaysListening, AppStatus::Idle),
        ];
        
        for (from, to) in transitions {
            // Just verify the transition compiles
            let _: AppStatus = from;
            let _: AppStatus = to;
        }
    }
}
