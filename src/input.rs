use anyhow::{Context, Result};
use enigo::{Enigo, Keyboard, Settings};

pub fn inject_text_with_enigo(text: &str) -> Result<()> {
    let mut enigo =
        Enigo::new(&Settings::default()).context("failed to initialise input simulator")?;
    enigo.text(text).context("failed to inject text")?;
    Ok(())
}
