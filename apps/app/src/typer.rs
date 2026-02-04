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

        self.enigo
            .text(text)
            .map_err(|e| anyhow::anyhow!("Failed to type text: {:?}", e))?;

        Ok(())
    }
}
