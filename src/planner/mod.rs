use crate::memory::WorldModel;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfsPlanner {
    visited_location_keys: HashSet<String>,
    dfs_stack: Vec<String>,
    frontier: Vec<FrontierAction>,
    attempted: HashSet<(String, String)>,
    next_frontier_id: usize,
    pub stats: PlannerStats,
}

#[derive(Debug, Clone)]
pub struct ObservationUpdate {
    pub previous_location: String,
    pub current_location: String,
    pub command: String,
    pub command_failed: bool,
    pub action_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationClassification {
    MovedToKnownLocation,
    MovedToNewLocation,
    CommandFailedOrBlocked,
}

impl DfsPlanner {
    pub fn new(world: &WorldModel) -> Self {
        let mut planner = Self {
            visited_location_keys: HashSet::new(),
            dfs_stack: Vec::new(),
            frontier: Vec::new(),
            attempted: HashSet::new(),
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
        let destination_was_known = self
            .visited_location_keys
            .contains(&update.current_location);

        let moved = !update.command_failed
            && !update.previous_location.is_empty()
            && update.previous_location != update.current_location;
        let classification = if update.command_failed || !moved {
            self.stats.failed_moves += 1;
            ObservationClassification::CommandFailedOrBlocked
        } else if destination_was_known {
            ObservationClassification::MovedToKnownLocation
        } else {
            ObservationClassification::MovedToNewLocation
        };
        self.observe_world(world);

        self.attempted
            .insert((update.previous_location.clone(), update.command.clone()));

        if let Some(action_id) = update.action_id {
            if let Some(action) = self
                .frontier
                .iter_mut()
                .find(|action| action.id == action_id)
            {
                action.last_attempted_turn = Some(self.stats.turns);
                action.status = if moved {
                    action.expected_destination = Some(update.current_location);
                    FrontierStatus::Explored
                } else {
                    FrontierStatus::Failed
                };
            }
        }

        classification
    }

    pub fn frontier(&self) -> &[FrontierAction] {
        &self.frontier
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
