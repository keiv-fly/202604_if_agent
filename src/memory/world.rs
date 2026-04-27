use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldModel {
    pub current_location: String,
    pub locations: HashMap<String, Location>,
    pub inventory: Vec<String>,
    pub important_objects: Vec<String>,
    pub hypotheses: Vec<String>,
    pub task_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Location {
    #[serde(alias = "id")]
    pub title: String,
    pub description: String,
    pub exits: Vec<Exit>,
    pub objects: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Exit {
    pub direction: String,
    pub destination: Option<String>,
}

impl WorldModel {
    pub fn update_from_observation(&mut self, text: &str) {
        if let Some((location_title, location_description)) = extract_location_snapshot(text) {
            self.current_location = location_title.clone();
            self.locations
                .entry(location_title.clone())
                .and_modify(|loc| loc.description = location_description.clone())
                .or_insert(Location {
                    title: location_title,
                    description: location_description,
                    ..Default::default()
                });
        }

        if text.to_lowercase().contains("you are currently holding") {
            self.inventory = text
                .lines()
                .filter_map(|line| line.trim().strip_prefix("- "))
                .map(|s| s.trim().to_string())
                .collect();
        }
    }

    pub fn save_to_disk(&self) -> anyhow::Result<PathBuf> {
        fs::create_dir_all("memory_store")?;
        let _stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let path = PathBuf::from("memory_store/world-state.json");
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }
}

fn extract_location_snapshot(text: &str) -> Option<(String, String)> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    let mut latest_snapshot: Option<(String, String)> = None;
    for window in lines.windows(2) {
        let candidate = window[0];
        let detail = window[1];
        if detail.to_lowercase().starts_with("you are") && !candidate.ends_with('!') {
            latest_snapshot = Some((candidate.to_string(), detail.to_string()));
        }
    }

    if let Some(title_only) = lines.iter().rev().find(|line| is_location_title_line(line)) {
        if latest_snapshot
            .as_ref()
            .map(|(title, _)| title != title_only)
            .unwrap_or(true)
        {
            return Some(((*title_only).to_string(), String::new()));
        }
    }

    latest_snapshot.or_else(|| {
        lines
            .first()
            .map(|line| ((*line).to_string(), text.to_string()))
    })
}

fn is_location_title_line(line: &str) -> bool {
    if line.is_empty() || line.ends_with('!') {
        return false;
    }
    if line.contains('>') {
        return false;
    }

    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() || words.len() > 8 {
        return false;
    }

    words.iter().all(|word| {
        word.chars()
            .next()
            .map(|ch| ch.is_ascii_uppercase())
            .unwrap_or(false)
    })
}
