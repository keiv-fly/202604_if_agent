use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldModel {
    pub current_location: String,
    pub locations: HashMap<String, Location>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub location_aliases: HashMap<String, String>,
    #[serde(default)]
    pub next_location_id: usize,
    pub inventory: Vec<String>,
    pub important_objects: Vec<String>,
    pub hypotheses: Vec<String>,
    pub task_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Location {
    #[serde(default)]
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_descriptions: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub provisional: bool,
    pub exits: Vec<Exit>,
    pub objects: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationMatchConfidence {
    Exact,
    Alias,
    Probable,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationMatch {
    pub key: String,
    pub confidence: LocationMatchConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Exit {
    pub direction: String,
    pub destination: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub transition_counts: HashMap<String, usize>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unstable: bool,
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
                    add_unique(&mut loc.observed_descriptions, location_description.clone());
                })
                .or_insert_with(|| {
                    let aliases = self
                        .location_aliases
                        .iter()
                        .filter_map(|(alias, target)| {
                            (target == &self.current_location).then_some(alias.clone())
                        })
                        .collect::<Vec<_>>();
                    Location {
                        id: self.current_location.clone(),
                        title: location_title,
                        description: location_description.clone(),
                        aliases,
                        observed_descriptions: vec![location_description.clone()],
                        provisional: !looks_like_room_description(&location_description),
                        ..Default::default()
                    }
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
        let previous_location = self.canonical_location_key(previous_location);
        if command_failed || self.current_location == previous_location {
            self.apply_command_result_with_destination(&previous_location, command, None);
        } else {
            let destination = self.current_location.clone();
            self.apply_command_result_with_destination(
                &previous_location,
                command,
                Some(&destination),
            );
        }
    }

    pub fn apply_command_result_with_destination(
        &mut self,
        previous_location: &str,
        command: &str,
        destination: Option<&str>,
    ) {
        let Some(direction) = normalize_direction(command) else {
            return;
        };
        if previous_location.trim().is_empty() {
            return;
        }
        let previous_location = self.canonical_location_key(previous_location);

        let destination = destination
            .map(str::trim)
            .filter(|destination| !destination.is_empty())
            .map(|destination| self.canonical_location_key(destination));
        self.set_exit(&previous_location, &direction, destination.clone(), true);

        let Some(dest) = destination else {
            return;
        };
        if dest == previous_location {
            return;
        }
        if let Some(reverse) = reverse_direction(&direction) {
            self.set_exit(&dest, reverse, Some(previous_location.to_string()), false);
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
            self.canonical_location_key(location_hint)
        } else {
            self.current_location.trim().to_string()
        };

        if !target_location.is_empty() {
            let location = self
                .locations
                .entry(target_location.clone())
                .or_insert_with(|| Location {
                    id: target_location.clone(),
                    title: target_location.clone(),
                    provisional: true,
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
                        transition_counts: HashMap::new(),
                        unstable: false,
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
        if let Some(location_match) = self.match_location_snapshot(title, description) {
            if location_match.confidence != LocationMatchConfidence::New {
                return location_match.key;
            }
        }

        if let Some(key) = self.match_single_title_for_context(title, description) {
            return key;
        }

        self.create_location(title, description)
    }

    pub fn match_location_snapshot(&self, title: &str, description: &str) -> Option<LocationMatch> {
        let title = title.trim();
        if let Some(location_match) = self.exact_location_snapshot_match(title, description) {
            return Some(location_match);
        }

        let title_alias = self.location_aliases.get(title).and_then(|key| {
            self.locations.contains_key(key).then_some(LocationMatch {
                key: key.clone(),
                confidence: LocationMatchConfidence::Alias,
            })
        });
        if title_alias.is_some() && !looks_like_room_description(description) {
            return title_alias;
        }

        if looks_like_room_description(description) {
            return None;
        }

        self.probable_location_by_transition_context(title)
            .or_else(|| {
                title_alias.and_then(|location_match| {
                    self.locations
                        .get(&location_match.key)
                        .filter(|location| !location.provisional)
                        .map(|_| location_match)
                })
            })
    }

    pub fn match_location_snapshot_with_context(
        &self,
        title: &str,
        description: &str,
        source_location: &str,
        command: &str,
    ) -> Option<LocationMatch> {
        self.match_location_snapshot(title, description)
            .or_else(|| {
                self.probable_location_by_observed_transition(title, source_location, command)
            })
    }

    pub fn canonical_location_key(&self, key: &str) -> String {
        let key = key.trim();
        if key.is_empty() {
            return String::new();
        }
        if self.locations.contains_key(key) {
            return key.to_string();
        }
        self.location_aliases
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    fn match_single_title_for_context(&self, title: &str, description: &str) -> Option<String> {
        let matching_keys = self
            .locations
            .iter()
            .filter_map(|(key, location)| (location.title == title).then_some(key.clone()))
            .collect::<Vec<_>>();

        (matching_keys.len() == 1 && !looks_like_room_description(description))
            .then(|| matching_keys[0].clone())
    }

    fn probable_location_by_transition_context(&self, title: &str) -> Option<LocationMatch> {
        let candidates = self
            .locations
            .iter()
            .filter_map(|(key, location)| (location.title == title).then_some(key.clone()))
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            return Some(LocationMatch {
                key: candidates[0].clone(),
                confidence: LocationMatchConfidence::Probable,
            });
        }
        None
    }

    fn exact_location_snapshot_match(
        &self,
        title: &str,
        description: &str,
    ) -> Option<LocationMatch> {
        self.locations
            .iter()
            .find(|(_, location)| same_location_snapshot(location, title, description))
            .map(|(key, _)| LocationMatch {
                key: key.clone(),
                confidence: LocationMatchConfidence::Exact,
            })
    }

    fn probable_location_by_observed_transition(
        &self,
        title: &str,
        source_location: &str,
        command: &str,
    ) -> Option<LocationMatch> {
        let direction = normalize_direction(command)?;
        let source_location = self.canonical_location_key(source_location);
        let exit = self
            .locations
            .get(&source_location)?
            .exits
            .iter()
            .find(|exit| {
                normalize_direction(&exit.direction).as_deref() == Some(direction.as_str())
            })?;
        let mut candidates = exit
            .transition_counts
            .keys()
            .chain(exit.destination.iter())
            .map(|destination| self.canonical_location_key(destination))
            .filter(|destination| {
                self.locations
                    .get(destination)
                    .map(|location| location.title == title)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        (candidates.len() == 1).then(|| LocationMatch {
            key: candidates[0].clone(),
            confidence: LocationMatchConfidence::Probable,
        })
    }

    fn create_location(&mut self, title: &str, description: &str) -> String {
        let key = self.next_stable_location_id();
        let aliases = self.aliases_for_new_location(title, description, &key);
        for alias in &aliases {
            self.location_aliases.insert(alias.clone(), key.clone());
        }
        self.locations.insert(
            key.clone(),
            Location {
                id: key.clone(),
                title: title.to_string(),
                description: description.to_string(),
                aliases,
                observed_descriptions: vec![description.to_string()],
                provisional: !looks_like_room_description(description),
                ..Default::default()
            },
        );
        key
    }

    fn next_stable_location_id(&mut self) -> String {
        if self.next_location_id == 0 {
            self.next_location_id = self
                .locations
                .keys()
                .filter_map(|key| key.strip_prefix("loc-"))
                .filter_map(|suffix| suffix.parse::<usize>().ok())
                .max()
                .map(|max_id| max_id + 1)
                .unwrap_or(1);
        }
        let id = format!("loc-{:06}", self.next_location_id);
        self.next_location_id += 1;
        id
    }

    fn aliases_for_new_location(&self, title: &str, description: &str, key: &str) -> Vec<String> {
        let same_title_descriptions = self
            .locations
            .values()
            .filter_map(|location| {
                (location.title == title).then_some(location.description.as_str())
            })
            .chain(std::iter::once(description))
            .collect::<Vec<_>>();
        let readable = if same_title_descriptions.len() == 1 {
            location_key_from_snapshot(title, description)
        } else {
            unique_location_key(
                title,
                description,
                &same_title_descriptions,
                &Vec::new(),
                &self.locations,
                key,
            )
        };
        let mut aliases = vec![readable];
        if !self
            .locations
            .values()
            .any(|location| location.title == title)
            && !self.location_aliases.contains_key(title)
        {
            aliases.push(title.to_string());
        }
        aliases.sort();
        aliases.dedup();
        aliases
    }

    fn set_exit(
        &mut self,
        from: &str,
        direction: &str,
        destination: Option<String>,
        count_transition: bool,
    ) {
        let from = self.canonical_location_key(from);
        let destination = destination.map(|destination| self.canonical_location_key(&destination));
        let location = self
            .locations
            .entry(from.clone())
            .or_insert_with(|| Location {
                id: from.clone(),
                title: from.clone(),
                provisional: true,
                ..Default::default()
            });

        if let Some(existing) = location
            .exits
            .iter_mut()
            .find(|known| known.direction == direction)
        {
            if count_transition {
                if let Some(destination) = &destination {
                    *existing
                        .transition_counts
                        .entry(destination.clone())
                        .or_insert(0) += 1;
                }
            }
            existing.unstable = existing.transition_counts.len() > 1;
            existing.destination = preferred_destination(existing, destination.as_deref());
        } else {
            let mut transition_counts = HashMap::new();
            if count_transition {
                if let Some(destination) = &destination {
                    transition_counts.insert(destination.clone(), 1);
                }
            }
            location.exits.push(Exit {
                direction: direction.to_string(),
                destination,
                transition_counts,
                unstable: false,
            });
        }
    }
}

fn preferred_destination(exit: &Exit, fallback: Option<&str>) -> Option<String> {
    exit.transition_counts
        .iter()
        .max_by(
            |(left_destination, left_count), (right_destination, right_count)| {
                left_count
                    .cmp(right_count)
                    .then_with(|| right_destination.cmp(left_destination))
            },
        )
        .map(|(destination, _)| destination.clone())
        .or_else(|| fallback.map(ToOwned::to_owned))
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn add_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
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

        assert_eq!(
            world
                .locations
                .values()
                .filter(|location| location.title == "In Forest")
                .count(),
            2
        );
        assert!(second_key.starts_with("loc-"));
        assert_eq!(
            world.location_aliases.get("In Forest. near both a valley"),
            Some(&second_key)
        );
        assert!(!world.locations.contains_key("In Forest"));
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

    #[test]
    fn repeated_command_results_mark_probabilistic_exit_unstable() {
        let mut world = WorldModel::default();
        world.current_location = "A".to_string();

        world.apply_command_result_with_destination("A", "east", Some("B"));
        world.apply_command_result_with_destination("A", "east", Some("B"));
        world.apply_command_result_with_destination("A", "east", Some("C"));

        let exit = world
            .locations
            .get("A")
            .expect("source location")
            .exits
            .iter()
            .find(|exit| exit.direction == "east")
            .expect("east exit");

        assert_eq!(exit.transition_counts.get("B"), Some(&2));
        assert_eq!(exit.transition_counts.get("C"), Some(&1));
        assert_eq!(exit.destination.as_deref(), Some("B"));
        assert!(exit.unstable);
    }

    #[test]
    fn runtime_identity_uses_stable_ids_and_readable_aliases() {
        let mut world = WorldModel::default();

        world.update_from_observation("In Forest\nYou are in open forest, with a deep valley.");
        let first_key = world.current_location.clone();
        world.update_from_observation("In Forest\nYou are in open forest near both a valley.");
        let second_key = world.current_location.clone();

        assert!(first_key.starts_with("loc-"));
        assert!(second_key.starts_with("loc-"));
        assert_ne!(first_key, second_key);
        assert_eq!(world.canonical_location_key("In Forest"), first_key);
        assert_eq!(
            world.canonical_location_key("In Forest. near both a valley"),
            second_key
        );
        assert!(!world.locations.contains_key("In Forest"));
    }

    #[test]
    fn transition_context_can_supply_probable_match() {
        let mut world = WorldModel::default();
        world.update_from_observation("A\nYou are in room A.");
        let source = world.current_location.clone();
        world.update_from_observation("B\nYou are in room B.");
        let destination = world.current_location.clone();
        world.apply_command_result_with_destination(&source, "east", Some(&destination));

        let location_match = world
            .match_location_snapshot_with_context(
                "B",
                "You are in room B after a change.",
                &source,
                "east",
            )
            .expect("transition context should identify destination");

        assert_eq!(location_match.key, destination);
        assert_eq!(location_match.confidence, LocationMatchConfidence::Probable);
    }
}
