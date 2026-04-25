use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SessionLogger {
    path: PathBuf,
}

impl SessionLogger {
    pub fn new() -> anyhow::Result<Self> {
        fs::create_dir_all("logs")?;
        let stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let path = PathBuf::from(format!("logs/agent-session-{stamp}.txt"));
        Ok(Self { path })
    }

    pub fn log(&self, event: &str, text: &str) {
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => f,
            Err(_) => return,
        };
        let ts = Utc::now().to_rfc3339();
        let _ = writeln!(file, "[{ts}] {event}: {text}");
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}
