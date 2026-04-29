use crate::memory::WorldModel;
use crate::memory::world::{location_key_from_snapshot, location_snapshot_from_observation};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub const CANONICAL_MOVES: [&str; 12] = [
    "north",
    "south",
    "east",
    "west",
    "northeast",
    "northwest",
    "southeast",
    "southwest",
    "up",
    "down",
    "in",
    "out",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerDecisionKind {
    CommandPlan,
    Complete,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPlan {
    pub commands: Vec<String>,
    pub route_commands: Vec<String>,
    pub frontier_action_command: String,
    pub start_location_key: String,
    pub frontier_source_location_key: String,
    pub selected_frontier_action_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PlannerDecision {
    pub kind: PlannerDecisionKind,
    pub plan: Option<CommandPlan>,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PlannerStats {
    pub turns: usize,
    pub generated_command_plans: usize,
    pub executed_movement_commands: usize,
    pub failed_moves: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontierStatus {
    Pending,
    Routing,
    Executing,
    Explored,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontierAction {
    pub id: String,
    pub source_location_key: String,
    pub command: String,
    pub expected_destination: Option<String>,
    pub status: FrontierStatus,
    pub priority: usize,
    pub reason: String,
    pub discovery_turn: usize,
    pub last_attempted_turn: Option<usize>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedEdge {
    pub source_location_key: String,
    pub command: String,
    pub turn: u64,
    pub reason: String,
    pub raw_output_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMoveAttempt {
    pub source_location_key: String,
    pub command: String,
    pub previous_observation_signature: String,
    pub frontier_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfsPlanner {
    visited_location_keys: HashSet<String>,
    dfs_stack: Vec<String>,
    frontier: Vec<FrontierAction>,
    attempted: HashSet<(String, String)>,
    blocked_edges: HashMap<String, HashMap<String, BlockedEdge>>,
    next_frontier_id: usize,
    pub stats: PlannerStats,
}

#[derive(Debug, Clone)]
pub struct ObservationUpdate {
    pub attempt: PendingMoveAttempt,
    pub current_location: String,
    pub classification: ObservationClassification,
    pub blocked_reason: Option<String>,
    pub raw_output_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationClassification {
    MovedToKnownLocation,
    MovedToNewLocation,
    MovedToSameLocation,
    CommandFailedOrBlocked,
}

#[derive(Debug, Clone)]
pub struct ClassifiedObservation {
    pub classification: ObservationClassification,
    pub blocked_reason: Option<String>,
    pub new_observation_signature: Option<String>,
    pub raw_output_hash: String,
    pub current_location_unchanged: bool,
}

impl DfsPlanner {
    pub fn new(world: &WorldModel) -> Self {
        let mut planner = Self {
            visited_location_keys: HashSet::new(),
            dfs_stack: Vec::new(),
            frontier: Vec::new(),
            attempted: HashSet::new(),
            blocked_edges: HashMap::new(),
            next_frontier_id: 1,
            stats: PlannerStats::default(),
        };
        planner.observe_world(world);
        planner
    }

    pub fn observe_world(&mut self, world: &WorldModel) {
        let current = location_key(world);
        if current.is_empty() {
            return;
        }

        if self.visited_location_keys.insert(current.clone()) {
            self.dfs_stack.push(current.clone());
        }
        self.add_default_frontier(&current);
    }

    pub fn decide(&mut self, world: &WorldModel) -> PlannerDecision {
        self.observe_world(world);
        let current = location_key(world);
        if current.is_empty() {
            return PlannerDecision {
                kind: PlannerDecisionKind::Blocked,
                plan: None,
                reason: "current location is unknown".to_string(),
            };
        }

        let mut selected: Option<(usize, Vec<String>)> =
            self.frontier
                .iter()
                .enumerate()
                .find_map(|(index, action)| {
                    if action.status == FrontierStatus::Pending
                        && action.source_location_key == current
                    {
                        Some((index, Vec::new()))
                    } else {
                        None
                    }
                });

        let graph = known_route_graph(world);
        for (index, action) in self.frontier.iter().enumerate() {
            if selected.is_some() {
                break;
            }
            if action.status != FrontierStatus::Pending {
                continue;
            }
            if let Some(route) = route_between(&graph, &current, &action.source_location_key) {
                selected = Some((index, route));
                break;
            }
        }

        let Some((index, route_commands)) = selected else {
            if self
                .frontier
                .iter()
                .any(|action| action.status == FrontierStatus::Pending)
            {
                return PlannerDecision {
                    kind: PlannerDecisionKind::Blocked,
                    plan: None,
                    reason: "pending frontier exists, but no known route reaches it".to_string(),
                };
            }
            return PlannerDecision {
                kind: PlannerDecisionKind::Complete,
                plan: None,
                reason: "frontier exhausted".to_string(),
            };
        };

        let action = self.frontier[index].clone();
        let mut commands = route_commands.clone();
        commands.push(action.command.clone());
        self.stats.generated_command_plans += 1;

        PlannerDecision {
            kind: PlannerDecisionKind::CommandPlan,
            reason: format!("selected pending frontier {}", action.id),
            plan: Some(CommandPlan {
                commands,
                route_commands,
                frontier_action_command: action.command,
                start_location_key: current,
                frontier_source_location_key: action.source_location_key,
                selected_frontier_action_id: action.id,
                reason: "deterministic DFS frontier order".to_string(),
            }),
        }
    }

    pub fn apply_observation(
        &mut self,
        update: ObservationUpdate,
        world: &WorldModel,
    ) -> ObservationClassification {
        self.stats.turns += 1;
        self.stats.executed_movement_commands += 1;
        let classification = update.classification;
        let moved = classification != ObservationClassification::CommandFailedOrBlocked;
        if classification == ObservationClassification::CommandFailedOrBlocked {
            self.stats.failed_moves += 1;
        } else {
            self.observe_world(world);
        }

        self.attempted.insert((
            update.attempt.source_location_key.clone(),
            update.attempt.command.clone(),
        ));

        if let Some(reason) = update.blocked_reason.clone() {
            self.blocked_edges
                .entry(update.attempt.source_location_key.clone())
                .or_default()
                .insert(
                    update.attempt.command.clone(),
                    BlockedEdge {
                        source_location_key: update.attempt.source_location_key.clone(),
                        command: update.attempt.command.clone(),
                        turn: self.stats.turns as u64,
                        reason,
                        raw_output_hash: update.raw_output_hash.clone(),
                    },
                );
        }

        if let Some(action_id) = update.attempt.frontier_id {
            if let Some(action) = self
                .frontier
                .iter_mut()
                .find(|action| action.id == action_id)
            {
                action.last_attempted_turn = Some(self.stats.turns);
                action.status = if moved {
                    action.expected_destination = Some(update.current_location);
                    action.failure_reason = None;
                    FrontierStatus::Explored
                } else {
                    action.expected_destination = None;
                    action.failure_reason = update.blocked_reason;
                    FrontierStatus::Failed
                };
            }
        }

        classification
    }

    pub fn pending_move_attempt(
        &self,
        source_location_key: &str,
        command: &str,
        frontier_id: Option<String>,
        world: &WorldModel,
    ) -> PendingMoveAttempt {
        PendingMoveAttempt {
            source_location_key: source_location_key.to_string(),
            command: command.to_string(),
            previous_observation_signature: world_observation_signature(world),
            frontier_id,
        }
    }

    pub fn classify_observation(
        &self,
        attempt: &PendingMoveAttempt,
        raw_observation: &str,
        command_failed: bool,
    ) -> ClassifiedObservation {
        let raw_output_hash = raw_output_hash(raw_observation);
        if command_failed {
            return ClassifiedObservation {
                classification: ObservationClassification::CommandFailedOrBlocked,
                blocked_reason: Some("command_execution_failed".to_string()),
                new_observation_signature: None,
                raw_output_hash,
                current_location_unchanged: true,
            };
        }

        if let Some(reason) = explicit_blocked_reason(raw_observation) {
            return ClassifiedObservation {
                classification: ObservationClassification::CommandFailedOrBlocked,
                blocked_reason: Some(reason),
                new_observation_signature: Some(observation_signature(raw_observation)),
                raw_output_hash,
                current_location_unchanged: true,
            };
        }

        let new_observation_signature = observation_signature(raw_observation);
        if !attempt.previous_observation_signature.is_empty()
            && new_observation_signature == attempt.previous_observation_signature
        {
            return ClassifiedObservation {
                classification: ObservationClassification::MovedToSameLocation,
                blocked_reason: None,
                new_observation_signature: Some(new_observation_signature),
                raw_output_hash,
                current_location_unchanged: true,
            };
        }

        let observed_location = location_snapshot_from_observation(raw_observation)
            .map(|(location, description)| {
                location_key_for_snapshot(&self.visited_location_keys, &location, &description)
            })
            .unwrap_or_default();
        if observed_location == attempt.source_location_key {
            return ClassifiedObservation {
                classification: ObservationClassification::MovedToSameLocation,
                blocked_reason: None,
                new_observation_signature: Some(new_observation_signature),
                raw_output_hash,
                current_location_unchanged: true,
            };
        }

        let classification = if self.visited_location_keys.contains(&observed_location) {
            ObservationClassification::MovedToKnownLocation
        } else {
            ObservationClassification::MovedToNewLocation
        };
        ClassifiedObservation {
            classification,
            blocked_reason: None,
            new_observation_signature: Some(new_observation_signature),
            raw_output_hash,
            current_location_unchanged: false,
        }
    }

    pub fn frontier(&self) -> &[FrontierAction] {
        &self.frontier
    }

    pub fn blocked_edges(&self) -> &HashMap<String, HashMap<String, BlockedEdge>> {
        &self.blocked_edges
    }

    pub fn frontier_counts(&self) -> HashMap<&'static str, usize> {
        let mut counts = HashMap::new();
        for action in &self.frontier {
            let key = match action.status {
                FrontierStatus::Pending => "pending",
                FrontierStatus::Routing => "routing",
                FrontierStatus::Executing => "executing",
                FrontierStatus::Explored => "explored",
                FrontierStatus::Failed => "failed",
                FrontierStatus::Blocked => "blocked",
            };
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
    }

    fn add_default_frontier(&mut self, location: &str) {
        for command in CANONICAL_MOVES {
            if self
                .frontier
                .iter()
                .any(|action| action.source_location_key == location && action.command == command)
            {
                continue;
            }
            if self
                .attempted
                .contains(&(location.to_string(), command.to_string()))
            {
                continue;
            }
            if self
                .blocked_edges
                .get(location)
                .and_then(|commands| commands.get(command))
                .is_some()
            {
                continue;
            }
            let id = format!("frontier-{}", self.next_frontier_id);
            self.next_frontier_id += 1;
            self.frontier.push(FrontierAction {
                id,
                source_location_key: location.to_string(),
                command: command.to_string(),
                expected_destination: None,
                status: FrontierStatus::Pending,
                priority: self.next_frontier_id,
                reason: "canonical movement frontier".to_string(),
                discovery_turn: self.stats.turns,
                last_attempted_turn: None,
                failure_reason: None,
            });
        }
    }
}

pub fn normalize_move_command(command: &str) -> Option<String> {
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
        "in" | "inside" | "enter" | "enter building" | "enter cave" => Some("in".to_string()),
        "out" | "outside" | "exit" => Some("out".to_string()),
        _ => None,
    }
}

pub fn location_key(world: &WorldModel) -> String {
    world.current_location.trim().to_string()
}

pub fn world_observation_signature(world: &WorldModel) -> String {
    let key = location_key(world);
    if key.is_empty() {
        return String::new();
    }
    let description = world
        .locations
        .get(&key)
        .map(|location| location.description.as_str())
        .unwrap_or_default();
    normalized_signature(&format!("{key}\n{description}"))
}

fn location_key_for_snapshot(
    known_location_keys: &HashSet<String>,
    title: &str,
    description: &str,
) -> String {
    let base_key = location_key_from_snapshot(title, description);
    let title_prefix = format!("{}. ", title.trim());
    let matching_keys = known_location_keys
        .iter()
        .filter(|key| {
            *key == &base_key
                || key.strip_prefix(&format!("{base_key}#")).is_some()
                || key.starts_with(&title_prefix)
        })
        .collect::<Vec<_>>();

    if matching_keys.len() == 1 {
        return matching_keys[0].clone();
    }

    base_key
}

fn looks_like_room_description(description: &str) -> bool {
    let normalized = normalized_signature(description);
    normalized.starts_with("you are ")
        || normalized.starts_with("you have ")
        || normalized.starts_with("at your feet ")
}

fn observation_signature(text: &str) -> String {
    if let Some((location, description)) = location_snapshot_from_observation(text) {
        return normalized_signature(&format!("{location}\n{description}"));
    }
    normalized_signature(text)
}

fn normalized_signature(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
}

fn explicit_blocked_reason(text: &str) -> Option<String> {
    let normalized = normalized_signature(text)
        .trim_matches(|ch: char| matches!(ch, '.' | '!' | '?'))
        .to_string();
    if normalized.is_empty() {
        return Some("empty_response".to_string());
    }

    let exact_failures = [
        "you can't go that way",
        "you can't go in that direction",
        "nothing happens",
    ];
    if exact_failures.contains(&normalized.as_str()) {
        return Some("explicit_blocked_response".to_string());
    }

    let short_failure = normalized.len() <= 96
        && (normalized.starts_with("there is no way")
            || normalized.starts_with("you are unable to")
            || normalized.starts_with("you can't")
            || normalized.starts_with("but you aren't in anything")
            || normalized.starts_with("the pipes are too small")
            || normalized.starts_with("you don't fit through"));
    if short_failure {
        return Some("explicit_blocked_response".to_string());
    }

    if normalized.len() <= 160 && normalized.contains("the only exit is to") {
        return Some("only_exit_response".to_string());
    }

    None
}

fn raw_output_hash(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn known_route_graph(world: &WorldModel) -> HashMap<String, Vec<(String, String)>> {
    let mut graph: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (source, location) in &world.locations {
        for exit in &location.exits {
            let Some(destination) = &exit.destination else {
                continue;
            };
            let Some(command) = normalize_move_command(&exit.direction) else {
                continue;
            };
            graph
                .entry(source.clone())
                .or_default()
                .push((destination.clone(), command));
        }
    }
    graph
}

fn route_between(
    graph: &HashMap<String, Vec<(String, String)>>,
    start: &str,
    goal: &str,
) -> Option<Vec<String>> {
    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();
    queue.push_back((start.to_string(), Vec::<String>::new()));
    seen.insert(start.to_string());

    while let Some((location, path)) = queue.pop_front() {
        if location == goal {
            return Some(path);
        }
        for (next, command) in graph.get(&location).into_iter().flatten() {
            if !seen.insert(next.clone()) {
                continue;
            }
            let mut next_path = path.clone();
            next_path.push(command.clone());
            queue.push_back((next.clone(), next_path));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::world::Location;

    fn world_at(location: &str, description: &str) -> WorldModel {
        let mut world = WorldModel::default();
        world.current_location = location.to_string();
        world.locations.insert(
            location.to_string(),
            Location {
                title: location.to_string(),
                description: description.to_string(),
                ..Default::default()
            },
        );
        world
    }

    fn selected_attempt(planner: &mut DfsPlanner, world: &WorldModel) -> PendingMoveAttempt {
        let decision = planner.decide(world);
        let plan = decision.plan.expect("expected a command plan");
        planner.pending_move_attempt(
            &plan.frontier_source_location_key,
            &plan.frontier_action_command,
            Some(plan.selected_frontier_action_id),
            world,
        )
    }

    #[test]
    fn direct_failure_marks_frontier_failed_without_destination() {
        let world = world_at("Forest", "You are standing in a forest.");
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);

        let classified = planner.classify_observation(&attempt, "You can't go that way.", false);
        let classification = planner.apply_observation(
            ObservationUpdate {
                attempt: attempt.clone(),
                current_location: attempt.source_location_key.clone(),
                classification: classified.classification,
                blocked_reason: classified.blocked_reason,
                raw_output_hash: classified.raw_output_hash,
            },
            &world,
        );

        assert_eq!(
            classification,
            ObservationClassification::CommandFailedOrBlocked
        );
        let frontier = planner
            .frontier()
            .iter()
            .find(|action| action.id == attempt.frontier_id.as_deref().unwrap())
            .expect("frontier should exist");
        assert_eq!(frontier.status, FrontierStatus::Failed);
        assert_eq!(frontier.expected_destination, None);
        assert_eq!(
            planner
                .blocked_edges()
                .get("Forest")
                .and_then(|commands| commands.get(&attempt.command))
                .map(|edge| edge.reason.as_str()),
            Some("explicit_blocked_response")
        );
    }

    #[test]
    fn blocked_response_does_not_create_location_or_child_frontiers() {
        let mut world = world_at("Forest", "You are standing in a forest.");
        let initial_location_count = world.locations.len();
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);
        let initial_frontier_count = planner.frontier().len();

        let classified = planner.classify_observation(&attempt, "You can't go that way.", false);
        if classified.classification != ObservationClassification::CommandFailedOrBlocked {
            world.update_from_observation("You can't go that way.");
        }
        planner.apply_observation(
            ObservationUpdate {
                attempt: attempt.clone(),
                current_location: attempt.source_location_key.clone(),
                classification: classified.classification,
                blocked_reason: classified.blocked_reason,
                raw_output_hash: classified.raw_output_hash,
            },
            &world,
        );

        assert_eq!(world.locations.len(), initial_location_count);
        assert!(!world.locations.contains_key("You can't go that way."));
        assert_eq!(planner.frontier().len(), initial_frontier_count);
    }

    #[test]
    fn same_location_response_is_moved_to_same_location() {
        let world = world_at("Forest", "You are standing in a forest.");
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);
        let response = "Forest\nYou are standing in a forest.";

        let classified = planner.classify_observation(&attempt, response, false);

        assert_eq!(
            classified.classification,
            ObservationClassification::MovedToSameLocation
        );
        assert_eq!(classified.blocked_reason, None);
        assert!(classified.current_location_unchanged);
    }

    #[test]
    fn same_location_move_marks_frontier_explored_with_source_destination() {
        let mut world = world_at("Forest", "You are standing in a forest.");
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);
        let response = "Forest\nYou are standing in a forest.";

        let classified = planner.classify_observation(&attempt, response, false);
        world.update_from_observation(response);
        world.apply_command_result_with_destination(
            &attempt.source_location_key,
            &attempt.command,
            Some(&attempt.source_location_key),
        );
        planner.apply_observation(
            ObservationUpdate {
                attempt: attempt.clone(),
                current_location: location_key(&world),
                classification: classified.classification,
                blocked_reason: classified.blocked_reason,
                raw_output_hash: classified.raw_output_hash,
            },
            &world,
        );

        let frontier = planner
            .frontier()
            .iter()
            .find(|action| action.id == attempt.frontier_id.as_deref().unwrap())
            .expect("frontier should exist");
        assert_eq!(frontier.status, FrontierStatus::Explored);
        assert_eq!(frontier.expected_destination.as_deref(), Some("Forest"));
        assert_eq!(
            world
                .locations
                .get("Forest")
                .and_then(|location| location
                    .exits
                    .iter()
                    .find(|exit| exit.direction == attempt.command))
                .and_then(|exit| exit.destination.as_deref()),
            Some("Forest")
        );
        assert!(
            planner
                .blocked_edges()
                .get("Forest")
                .and_then(|commands| commands.get(&attempt.command))
                .is_none()
        );
    }

    #[test]
    fn blocked_edge_is_not_readded_during_later_frontier_generation() {
        let world = world_at("Forest", "You are standing in a forest.");
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);
        let classified = planner.classify_observation(&attempt, "Nothing happens.", false);
        planner.apply_observation(
            ObservationUpdate {
                attempt: attempt.clone(),
                current_location: attempt.source_location_key.clone(),
                classification: classified.classification,
                blocked_reason: classified.blocked_reason,
                raw_output_hash: classified.raw_output_hash,
            },
            &world,
        );

        planner.observe_world(&world);

        let matching = planner
            .frontier()
            .iter()
            .filter(|action| {
                action.source_location_key == "Forest" && action.command == attempt.command
            })
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].status, FrontierStatus::Failed);
    }

    #[test]
    fn successful_move_still_creates_destination_frontiers() {
        let mut world = world_at("Forest", "You are standing in a forest.");
        let mut planner = DfsPlanner::new(&world);
        let attempt = selected_attempt(&mut planner, &world);
        let response = "Clearing\nYou are in a sunny clearing.";

        let classified = planner.classify_observation(&attempt, response, false);
        assert_eq!(
            classified.classification,
            ObservationClassification::MovedToNewLocation
        );
        world.update_from_observation(response);
        planner.apply_observation(
            ObservationUpdate {
                attempt,
                current_location: location_key(&world),
                classification: classified.classification,
                blocked_reason: classified.blocked_reason,
                raw_output_hash: classified.raw_output_hash,
            },
            &world,
        );

        assert!(
            world
                .locations
                .values()
                .any(|location| location.title == "Clearing")
        );
        assert!(
            planner
                .frontier()
                .iter()
                .any(|action| { action.source_location_key.starts_with("Clearing") })
        );
    }
}
