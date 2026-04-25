use chrono::Utc;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub turns: Vec<(String, String)>,
}

impl Transcript {
    pub fn add_turn(&mut self, command: String, output: String) {
        self.turns.push((command, output));
    }

    pub fn render(&self) -> String {
        let mut body = String::new();
        for (cmd, out) in &self.turns {
            body.push_str(&format!("> {cmd}\n{out}\n\n"));
        }
        body
    }

    pub fn save_to_disk(&self) -> anyhow::Result<PathBuf> {
        fs::create_dir_all("transcripts")?;
        let stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let path = PathBuf::from(format!("transcripts/session-{stamp}.txt"));
        fs::write(&path, self.render())?;
        Ok(path)
    }
}
