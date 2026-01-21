//! TUI editor handling - external editor integration

use std::io::Write;
use std::process::Command;

/// Opens content in $EDITOR and returns the edited content
pub fn open_in_editor(content: &str) -> std::io::Result<String> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());

    // Create temp file
    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!("kto_edit_{}.txt", std::process::id()));

    // Write content to temp file
    {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(content.as_bytes())?;
    }

    // Open editor
    let status = Command::new(&editor)
        .arg(&temp_path)
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Editor exited with error"
        ));
    }

    // Read back content
    let result = std::fs::read_to_string(&temp_path)?;

    // Cleanup
    let _ = std::fs::remove_file(&temp_path);

    Ok(result.trim().to_string())
}
