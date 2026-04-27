use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone)]
pub struct SessionLogger {
    path: PathBuf,
    llm_dir: PathBuf,
    llm_call_counter: Arc<AtomicUsize>,
}

impl SessionLogger {
    pub fn new() -> anyhow::Result<Self> {
        Self::new_in("logs")
    }

    pub fn new_in(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        fs::create_dir_all(root)?;
        let base_stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let (path, llm_dir) = unique_session_paths(root, &base_stamp);
        fs::create_dir_all(&llm_dir)?;

        Ok(Self {
            path,
            llm_dir,
            llm_call_counter: Arc::new(AtomicUsize::new(0)),
        })
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

    pub fn llm_dir(&self) -> &PathBuf {
        &self.llm_dir
    }

    pub fn next_llm_call_number(&self) -> usize {
        self.llm_call_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn write_llm_artifact(&self, call_number: usize, name: &str, contents: &str) {
        let path = self.llm_dir.join(format!("{call_number:03}-{name}"));
        let _ = fs::write(path, contents);
    }

    pub fn write_llm_json_artifact(
        &self,
        call_number: usize,
        name: &str,
        value: &serde_json::Value,
    ) {
        let contents = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
        self.write_llm_artifact(call_number, name, &contents);
    }
}

fn unique_session_paths(root: &Path, base_stamp: &str) -> (PathBuf, PathBuf) {
    for suffix in 0.. {
        let stamp = if suffix == 0 {
            base_stamp.to_string()
        } else {
            format!("{base_stamp}-{suffix}")
        };
        let path = root.join(format!("agent-session-{stamp}.txt"));
        let llm_dir = root.join(&stamp);

        if !path.exists() && !llm_dir.exists() {
            return (path, llm_dir);
        }
    }

    unreachable!("unbounded session suffix search should always return")
}
