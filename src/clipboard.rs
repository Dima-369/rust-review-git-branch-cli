use anyhow::Result;

/// Copy text to system clipboard
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    use arboard::Clipboard;

    let mut clipboard =
        Clipboard::new().map_err(|e| anyhow::anyhow!("Failed to access clipboard: {e}"))?;
    clipboard
        .set_text(text)
        .map_err(|e| anyhow::anyhow!("Failed to set clipboard text: {e}"))?;
    Ok(())
}
