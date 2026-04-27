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
        if let Some((location_title, location_description)) =
            location_snapshot_from_observation(text)
        {
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

    pub fn apply_command_result(
        &mut self,
        previous_location: &str,
        command: &str,
        command_failed: bool,
    ) {
        let Some(direction) = normalize_direction(command) else {
            return;
        };
        if previous_location.trim().is_empty() {
            return;
        }

        let destination = if command_failed || self.current_location == previous_location {
            None
        } else {
            Some(self.current_location.clone())
        };
        self.set_exit(previous_location, &direction, destination.clone());

        if let (Some(dest), Some(reverse)) = (destination, reverse_direction(&direction)) {
            self.set_exit(&dest, reverse, Some(previous_location.to_string()));
        }
    }

    pub fn apply_llm_memory(
        &mut self,
        location_hint: &str,
        new_exits: &[String],
        new_objects: &[String],
        notes: &[String],
    ) {
        let target_location = if !location_hint.trim().is_empty() {
            location_hint.trim()
        } else {
            self.current_location.trim()
        };

        if !target_location.is_empty() {
            let location = self
                .locations
                .entry(target_location.to_string())
                .or_insert_with(|| Location {
                    title: target_location.to_string(),
                    ..Default::default()
                });

            for exit in new_exits {
                let direction =
                    normalize_direction(exit).unwrap_or_else(|| exit.trim().to_lowercase());
                if direction.is_empty() {
                    continue;
                }
                if !location
                    .exits
                    .iter()
                    .any(|known| known.direction == direction)
                {
                    location.exits.push(Exit {
                        direction,
                        destination: None,
                    });
                }
            }

            for object in new_objects {
                let item = object.trim();
                if item.is_empty() {
                    continue;
                }
                if !location.objects.iter().any(|known| known == item) {
                    location.objects.push(item.to_string());
                }
            }
        }

        for note in notes {
            let note = note.trim();
            if note.is_empty() {
                continue;
            }
            if !self.task_notes.iter().any(|existing| existing == note) {
                self.task_notes.push(note.to_string());
            }
        }
    }

    pub fn frontier_summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Current location: {}", self.current_location));

        for (title, loc) in &self.locations {
            let unexplored = loc
                .exits
                .iter()
                .filter(|exit| exit.destination.is_none())
                .map(|exit| exit.direction.as_str())
                .collect::<Vec<_>>();
            if unexplored.is_empty() {
                continue;
            }
            lines.push(format!(
                "- {title}: unexplored exits [{}]",
                unexplored.join(", ")
            ));
        }

        if lines.len() == 1 {
            lines.push("- No known frontier exits yet.".to_string());
        }
        lines.join("\n")
    }

    fn set_exit(&mut self, from: &str, direction: &str, destination: Option<String>) {
        let location = self
            .locations
            .entry(from.to_string())
            .or_insert_with(|| Location {
                title: from.to_string(),
                ..Default::default()
            });

        if let Some(existing) = location
            .exits
            .iter_mut()
            .find(|known| known.direction == direction)
        {
            existing.destination = destination;
        } else {
            location.exits.push(Exit {
                direction: direction.to_string(),
                destination,
            });
        }
    }
}

fn normalize_direction(command: &str) -> Option<String> {
    let mut normalized = command.trim().to_lowercase();
    if let Some(stripped) = normalized.strip_prefix("go ") {
        normalized = stripped.trim().to_string();
    }
    match normalized.as_str() {
        "north" | "n" => Some("north".to_string()),
        "south" | "s" => Some("south".to_string()),
        "east" | "e" => Some("east".to_string()),
        "west" | "w" => Some("west".to_string()),
        "northeast" | "ne" => Some("northeast".to_string()),
        "northwest" | "nw" => Some("northwest".to_string()),
        "southeast" | "se" => Some("southeast".to_string()),
        "southwest" | "sw" => Some("southwest".to_string()),
        "up" | "u" => Some("up".to_string()),
        "down" | "d" => Some("down".to_string()),
        "in" | "inside" | "enter" => Some("in".to_string()),
        "out" | "outside" | "exit" => Some("out".to_string()),
        _ => None,
    }
}

fn reverse_direction(direction: &str) -> Option<&'static str> {
    match direction {
        "north" => Some("south"),
        "south" => Some("north"),
        "east" => Some("west"),
        "west" => Some("east"),
        "northeast" => Some("southwest"),
        "northwest" => Some("southeast"),
        "southeast" => Some("northwest"),
        "southwest" => Some("northeast"),
        "up" => Some("down"),
        "down" => Some("up"),
        "in" => Some("out"),
        "out" => Some("in"),
        _ => None,
    }
}

pub fn location_snapshot_from_observation(text: &str) -> Option<(String, String)> {
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
