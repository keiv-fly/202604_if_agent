use anyhow::{Result, anyhow};

pub fn validate_game_command(command: &str) -> Result<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("game command cannot be empty"));
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(anyhow!("only one command line is allowed"));
    }
    if trimmed.contains(';') {
        return Err(anyhow!("command batching is not allowed"));
    }
    Ok(trimmed.to_string())
}
