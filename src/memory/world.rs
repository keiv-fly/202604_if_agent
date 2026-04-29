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
            let location_key =
                self.location_key_for_snapshot(&location_title, &location_description);
            self.current_location = location_key.clone();
            self.locations
                .entry(location_key)
                .and_modify(|loc| {
                    if should_update_location_description(&loc.description, &location_description) {
                        loc.description = location_description.clone();
                    }
                })
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

    pub fn location_key_for_snapshot(&mut self, title: &str, description: &str) -> String {
        let title = title.trim();
        let matching_keys = self
            .locations
            .iter()
            .filter_map(|(key, location)| (location.title == title).then_some(key))
            .collect::<Vec<_>>();

        if matching_keys.len() == 1 && !looks_like_room_description(description) {
            return matching_keys[0].clone();
        }

        if let Some(key) = self.locations.iter().find_map(|(key, location)| {
            same_location_snapshot(location, title, description).then_some(key.clone())
        }) {
            return key;
        }

        let same_title_keys = self
            .locations
            .iter()
            .filter_map(|(key, location)| (location.title == title).then_some(key.clone()))
            .collect::<Vec<_>>();

        if same_title_keys.is_empty() {
            return location_key_from_snapshot(title, description);
        }

        self.rekey_same_title_locations(title, description)
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

    fn rekey_same_title_locations(&mut self, title: &str, new_description: &str) -> String {
        let mut entries = self
            .locations
            .iter()
            .filter_map(|(key, location)| {
                (location.title == title).then_some((key.clone(), location.clone()))
            })
            .collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        entries.push((
            String::new(),
            Location {
                title: title.to_string(),
                description: new_description.to_string(),
                ..Default::default()
            },
        ));

        let descriptions = entries
            .iter()
            .map(|(_, location)| location.description.as_str())
            .collect::<Vec<_>>();
        let mut old_to_new = HashMap::new();
        let mut planned_keys = Vec::new();
        for (old_key, location) in &entries {
            let planned = unique_location_key(
                title,
                &location.description,
                &descriptions,
                &planned_keys,
                &self.locations,
                old_key,
            );
            if !old_key.is_empty() {
                old_to_new.insert(old_key.clone(), planned.clone());
            }
            planned_keys.push(planned);
        }

        let mut moved_locations = Vec::new();
        for (old_key, new_key) in &old_to_new {
            if old_key == new_key {
                continue;
            }
            if let Some(location) = self.locations.remove(old_key) {
                moved_locations.push((new_key.clone(), location));
            }
        }
        for (new_key, location) in moved_locations {
            self.locations.insert(new_key, location);
        }
        self.rewrite_location_references(&old_to_new);

        planned_keys
            .last()
            .cloned()
            .expect("new location key should be planned")
    }

    fn rewrite_location_references(&mut self, old_to_new: &HashMap<String, String>) {
        if let Some(new_current) = old_to_new.get(&self.current_location) {
            self.current_location = new_current.clone();
        }

        for location in self.locations.values_mut() {
            for exit in &mut location.exits {
                if let Some(destination) = &mut exit.destination {
                    if let Some(new_destination) = old_to_new.get(destination) {
                        *destination = new_destination.clone();
                    }
                }
            }
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

    if let Some((index, title)) = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| is_location_title_line(line))
    {
        let description = lines
            .iter()
            .skip(index + 1)
            .copied()
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_string();
        return Some(((*title).to_string(), description));
    }

    lines
        .first()
        .map(|line| ((*line).to_string(), text.to_string()))
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

pub fn location_key_from_snapshot(title: &str, description: &str) -> String {
    let _ = description;
    title.trim().to_string()
}

fn same_location_snapshot(location: &Location, title: &str, description: &str) -> bool {
    location.title == title.trim()
        && normalized_key_material(&location.description) == normalized_key_material(description)
}

fn unique_location_key(
    title: &str,
    description: &str,
    descriptions: &[&str],
    planned_keys: &[String],
    locations: &HashMap<String, Location>,
    current_key: &str,
) -> String {
    let base_key = location_key_with_distinguishing_words(title, description, descriptions);
    let normalized = normalized_key_material(description);
    let descriptions_are_same = descriptions
        .iter()
        .all(|candidate| normalized_key_material(candidate) == normalized);
    if !descriptions_are_same && key_is_available(&base_key, planned_keys, locations, current_key) {
        return base_key;
    }

    let hash = description_hash(description);
    for hash_len in 7..=hash.len() {
        let candidate = format!("{base_key}#{}", &hash[..hash_len]);
        if key_is_available(&candidate, planned_keys, locations, current_key) {
            return candidate;
        }
    }

    base_key
}

fn location_key_with_distinguishing_words(
    title: &str,
    description: &str,
    descriptions: &[&str],
) -> String {
    let normalized_description = normalized_key_material(description);
    if normalized_description.is_empty() {
        return title.trim().to_string();
    }
    if descriptions
        .iter()
        .all(|candidate| normalized_key_material(candidate) == normalized_description)
    {
        return format!("{}.", title.trim());
    }

    let start = common_prefix_char_len(descriptions)
        .min(normalized_description.chars().count().saturating_sub(1));
    let suffix = normalized_description
        .chars()
        .skip(start)
        .take(20)
        .collect::<String>()
        .trim()
        .to_string();

    format!("{}. {}", title.trim(), suffix)
}

fn common_prefix_char_len(descriptions: &[&str]) -> usize {
    let Some(first) = descriptions.first() else {
        return 0;
    };
    let first_chars = normalized_key_material(first).chars().collect::<Vec<_>>();
    let mut len = first_chars.len();
    for description in descriptions.iter().skip(1) {
        let chars = normalized_key_material(description)
            .chars()
            .collect::<Vec<_>>();
        len = len.min(
            first_chars
                .iter()
                .zip(chars.iter())
                .take_while(|(left, right)| left == right)
                .count(),
        );
    }
    len
}

fn key_is_available(
    key: &str,
    planned_keys: &[String],
    locations: &HashMap<String, Location>,
    current_key: &str,
) -> bool {
    !planned_keys.iter().any(|planned| planned == key)
        && (!locations.contains_key(key) || key == current_key)
}

fn description_hash(description: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in normalized_key_material(description).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn looks_like_room_description(description: &str) -> bool {
    let normalized = normalized_key_material(description);
    normalized.starts_with("you are ")
        || normalized.starts_with("you have ")
        || normalized.starts_with("at your feet ")
}

fn should_update_location_description(existing: &str, next: &str) -> bool {
    let next = next.trim();
    if next.is_empty() {
        return false;
    }
    if existing.trim().is_empty() {
        return true;
    }
    looks_like_room_description(next)
}

fn normalized_key_material(text: &str) -> String {
    normalized_words(text).join(" ")
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_first_non_empty_line_after_title() {
        let observation = "Inside Building\n\
            You are inside a building, a well house for a large spring.\n\n\
            There are some keys on the ground here.\n\n\
            There is tasty food here.";

        let (title, description) =
            location_snapshot_from_observation(observation).expect("location snapshot");

        assert_eq!(title, "Inside Building");
        assert_eq!(
            description,
            "You are inside a building, a well house for a large spring."
        );
    }

    #[test]
    fn object_listing_does_not_replace_existing_description() {
        let mut world = WorldModel::default();
        world.update_from_observation(
            "Inside Building\nYou are inside a building, a well house for a large spring.",
        );
        let key = world.current_location.clone();

        world.update_from_observation(
            "Inside Building\n\nThere are some keys on the ground here.\n\nThere is tasty food here.",
        );

        assert_eq!(world.current_location, key);
        assert_eq!(
            world
                .locations
                .get(&key)
                .expect("location should remain known")
                .description,
            "You are inside a building, a well house for a large spring."
        );
    }

    #[test]
    fn location_key_uses_only_title_without_duplicates() {
        assert_eq!(
            location_key_from_snapshot("In Forest", "You are in open forest, with a deep valley."),
            "In Forest"
        );
    }

    #[test]
    fn repeated_location_titles_use_first_twenty_different_description_characters() {
        let mut world = WorldModel::default();
        world.update_from_observation("In Forest\nYou are in open forest, with a deep valley.");

        world.update_from_observation("In Forest\nYou are in open forest near both a valley.");
        let second_key = world.current_location.clone();

        assert!(
            world
                .locations
                .contains_key("In Forest. with a deep valley")
        );
        assert_eq!(second_key, "In Forest. near both a valley");
    }

    #[test]
    fn location_key_adds_hash_when_different_words_are_not_enough() {
        let locations = HashMap::new();
        let planned = vec!["In Forest. you are in".to_string()];
        let key = unique_location_key(
            "In Forest",
            "You are in open forest.",
            &["You are in open forest.", "You are in open forest."],
            &planned,
            &locations,
            "",
        );

        assert!(key.starts_with("In Forest.#"));
        assert_eq!(
            key.strip_prefix("In Forest.#").expect("hash suffix").len(),
            7
        );
    }
}
