use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldModel {
    pub current_location: String,
    pub locations: HashMap<String, Location>,
    pub inventory: Vec<String>,
    pub important_objects: Vec<String>,
    pub hypotheses: Vec<String>,
    pub task_notes: Vec<String>,
    #[serde(skip)]
    current_path: Vec<String>,
    #[serde(skip)]
    location_paths: HashMap<String, String>,
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
    pub fn update_from_observation_with_command(&mut self, text: &str, command: Option<&str>) {
        if let Some((location_title, location_description)) = extract_location_snapshot(text) {
            let next_path = next_path(&self.current_path, command);
            let location_id = self.resolve_location_id(
                &location_title,
                &location_description,
                next_path.as_deref(),
            );
            self.current_location = location_id.clone();
            self.current_path = next_path
                .map(|path| split_path(&path))
                .unwrap_or_else(|| self.current_path.clone());
            self.locations
                .entry(location_id.clone())
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

    fn resolve_location_id(
        &mut self,
        location_title: &str,
        location_description: &str,
        path: Option<&str>,
    ) -> String {
        let canonical_description = canonicalize_description(location_description);
        let matching_ids = self
            .locations
            .iter()
            .filter_map(|(id, loc)| {
                (loc.title == location_title
                    && canonicalize_description(&loc.description) == canonical_description)
                    .then_some(id.clone())
            })
            .collect::<Vec<_>>();

        if matching_ids.is_empty() {
            return self.make_location_id(location_title, &canonical_description, path);
        }

        if matching_ids.len() == 1 {
            if let Some(path) = path {
                self.location_paths
                    .entry(matching_ids[0].clone())
                    .or_insert_with(|| path.to_string());
            }
            return matching_ids[0].clone();
        }

        if let Some(path) = path {
            if let Some(matching_path_id) = matching_ids
                .iter()
                .find(|id| self.location_paths.get(*id).map(String::as_str) == Some(path))
            {
                return matching_path_id.clone();
            }
        }

        self.make_location_id(location_title, &canonical_description, path)
    }

    fn make_location_id(
        &mut self,
        location_title: &str,
        canonical_description: &str,
        path: Option<&str>,
    ) -> String {
        let base = if canonical_description.is_empty() {
            format!("{location_title}::path:{}", path.unwrap_or("origin"))
        } else {
            format!(
                "{location_title}::desc:{}",
                short_hash(canonical_description)
            )
        };

        let mut candidate = base;
        let mut suffix = 2;
        while self.locations.contains_key(&candidate) {
            candidate = format!("{location_title}::variant:{suffix}");
            suffix += 1;
        }

        if let Some(path) = path {
            self.location_paths
                .insert(candidate.clone(), path.to_string());
        }

        candidate
    }

    pub fn save_to_disk(&self) -> anyhow::Result<PathBuf> {
        fs::create_dir_all("memory_store")?;
        let _stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let path = PathBuf::from("memory_store/world-state.json");
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }
}

fn canonicalize_description(description: &str) -> String {
    description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn short_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn split_path(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|step| !step.is_empty())
        .map(str::to_string)
        .collect()
}

fn next_path(current_path: &[String], command: Option<&str>) -> Option<String> {
    let movement = command.and_then(normalize_move_command)?;
    let mut updated = current_path.to_vec();
    updated.push(movement.to_string());
    Some(updated.join("/"))
}

fn normalize_move_command(command: &str) -> Option<&'static str> {
    let mut normalized = command.trim().to_lowercase();
    if let Some(stripped) = normalized.strip_prefix("go ") {
        normalized = stripped.trim().to_string();
    }
    match normalized.as_str() {
        "north" | "n" => Some("north"),
        "south" | "s" => Some("south"),
        "east" | "e" => Some("east"),
        "west" | "w" => Some("west"),
        "northeast" | "ne" => Some("northeast"),
        "northwest" | "nw" => Some("northwest"),
        "southeast" | "se" => Some("southeast"),
        "southwest" | "sw" => Some("southwest"),
        "up" | "u" => Some("up"),
        "down" | "d" => Some("down"),
        "in" | "inside" | "enter" | "enter building" | "enter cave" => Some("in"),
        "out" | "outside" | "exit" => Some("out"),
        _ => None,
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
