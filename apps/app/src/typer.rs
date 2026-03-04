use anyhow::Result;
use enigo::{Enigo, Keyboard, Settings};

pub struct Typer {
    enigo: Enigo,
}

impl Typer {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| anyhow::anyhow!("Failed to initialize Enigo: {:?}", e))?;

        Ok(Self { enigo })
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        // Small delay to ensure the target window is ready
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Use clipboard paste (Ctrl+V) — much more reliable than keystroke simulation
        // which drops characters on many Windows apps
        if let Err(e) = self.type_via_clipboard(text) {
            tracing::warn!("Clipboard paste failed ({e}), falling back to enigo");
            self.enigo
                .text(text)
                .map_err(|e| anyhow::anyhow!("Failed to type text: {:?}", e))?;
        }

        Ok(())
    }

    fn type_via_clipboard(&mut self, text: &str) -> Result<()> {
        use enigo::Key;

        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| anyhow::anyhow!("Failed to open clipboard: {e}"))?;

        // Save current clipboard content
        let old_text = clipboard.get_text().ok();

        // Set our text
        clipboard.set_text(text)
            .map_err(|e| anyhow::anyhow!("Failed to set clipboard: {e}"))?;

        // Paste with Ctrl+V
        std::thread::sleep(std::time::Duration::from_millis(10));
        self.enigo.key(Key::Control, enigo::Direction::Press)
            .map_err(|e| anyhow::anyhow!("key press: {:?}", e))?;
        self.enigo.key(Key::Unicode('v'), enigo::Direction::Click)
            .map_err(|e| anyhow::anyhow!("key click: {:?}", e))?;
        self.enigo.key(Key::Control, enigo::Direction::Release)
            .map_err(|e| anyhow::anyhow!("key release: {:?}", e))?;

        // Restore old clipboard after a brief delay
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Some(old) = old_text {
            let _ = clipboard.set_text(&old);
        }

        Ok(())
    }
}
