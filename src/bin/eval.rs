#[allow(dead_code, unused_imports)]
#[path = "../agent/mod.rs"]
mod agent;
#[allow(dead_code, unused_imports)]
#[path = "../config/mod.rs"]
mod config;
#[allow(dead_code, unused_imports)]
#[path = "../game/mod.rs"]
mod game;
#[allow(dead_code, unused_imports)]
#[path = "../llm/mod.rs"]
mod llm;
#[allow(dead_code, unused_imports)]
#[path = "../logging/mod.rs"]
mod logging;
#[allow(dead_code, unused_imports)]
#[path = "../memory/mod.rs"]
mod memory;

use agent::{AgentEvent, AgentTask, run_single_turn};
use anyhow::{Context, Result, bail};
use config::AppConfig;
use game::GameSession;
use llm::LlmClient;
use logging::SessionLogger;
use memory::WorldModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;

const DEFAULT_WORLD_STATE_PATH: &str = "memory_store/world-state.json";
const DEFAULT_FIRST_NODES_PATH: &str = "eval_data/first_nodes.json";
const DEFAULT_RUNS: usize = 3;
const DEFAULT_COMMAND: &str = "explore to create the full map";

#[derive(Debug)]
struct Args {
    runs: usize,
    calculate_only: bool,
    world_state_path: String,
    first_nodes_path: String,
    command: String,
}

#[derive(Debug, Deserialize)]
struct WorldState {
    locations: HashMap<String, WorldLocation>,
}

#[derive(Debug, Deserialize)]
struct WorldLocation {
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FirstNode {
    title: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct EvalRun {
    run: usize,
    share_of_titles_found: f64,
    share_of_titles_and_descriptions: f64,
}

#[derive(Debug, Serialize)]
struct EvalOutput {
    command: String,
    requested_runs: usize,
    calculation_only: bool,
    runs: Vec<EvalRun>,
    average_share_of_titles_found: f64,
    average_share_of_titles_and_descriptions: f64,
}

fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let args = Args::parse()?;
    let first_nodes = read_json::<HashMap<String, FirstNode>>(&args.first_nodes_path)?;
    let mut runs = Vec::new();

    if args.calculate_only {
        let world_state = read_json::<WorldState>(&args.world_state_path)?;
        let scores = score_world_state(&world_state, &first_nodes);
        runs.push(EvalRun {
            run: 1,
            share_of_titles_found: scores.share_of_titles_found,
            share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
        });
    } else {
        for run in 1..=args.runs {
            eprintln!("Starting eval run {run}/{}...", args.runs);
            let world = run_program(&args.command)
                .with_context(|| format!("failed during eval run {run}"))?;
            let scores = score_world_model(&world, &first_nodes);
            runs.push(EvalRun {
                run,
                share_of_titles_found: scores.share_of_titles_found,
                share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
            });
        }
    }

    let output = EvalOutput {
        command: args.command,
        requested_runs: args.runs,
        calculation_only: args.calculate_only,
        average_share_of_titles_found: average_titles(&runs),
        average_share_of_titles_and_descriptions: average_title_descriptions(&runs),
        runs,
    };

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

impl Args {
    fn parse() -> Result<Self> {
        let mut runs = DEFAULT_RUNS;
        let mut calculate_only = false;
        let mut world_state_path = DEFAULT_WORLD_STATE_PATH.to_string();
        let mut first_nodes_path = DEFAULT_FIRST_NODES_PATH.to_string();
        let mut command = DEFAULT_COMMAND.to_string();
        let mut positional = Vec::new();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    print_usage();
                    std::process::exit(0);
                }
                "-r" | "--runs" => {
                    let value = args.next().context("--runs requires a value")?;
                    runs = value
                        .parse()
                        .with_context(|| format!("invalid --runs value '{value}'"))?;
                }
                "--calculate-only" | "--no-run" => calculate_only = true,
                "--world-state" => {
                    world_state_path = args.next().context("--world-state requires a value")?;
                }
                "--first-nodes" => {
                    first_nodes_path = args.next().context("--first-nodes requires a value")?;
                }
                "--command" | "--prompt" => {
                    command = args.next().context("--command requires a value")?;
                }
                value if value.starts_with('-') => bail!("unknown argument '{value}'"),
                value => positional.push(value.to_string()),
            }
        }

        if runs == 0 {
            bail!("--runs must be at least 1");
        }
        if let Some(path) = positional.first() {
            world_state_path = path.clone();
        }
        if let Some(path) = positional.get(1) {
            first_nodes_path = path.clone();
        }
        if positional.len() > 2 {
            bail!("expected at most two positional paths: <world-state> <first-nodes>");
        }

        Ok(Self {
            runs,
            calculate_only,
            world_state_path,
            first_nodes_path,
            command,
        })
    }
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run --bin eval -- [OPTIONS] [world-state] [first-nodes]\n\
\n\
Options:\n\
  -r, --runs <N>              Number of program runs to execute (default: {DEFAULT_RUNS})\n\
      --calculate-only        Read the world-state file and calculate scores without running\n\
      --world-state <PATH>    World-state path for --calculate-only\n\
      --first-nodes <PATH>    Expected first-nodes path (default: {DEFAULT_FIRST_NODES_PATH})\n\
      --command <TEXT>        Agent command/prompt to run (default: {DEFAULT_COMMAND:?})\n\
  -h, --help                  Show this help"
    );
}

fn run_program(command: &str) -> Result<WorldModel> {
    let config = AppConfig::from_env();
    let logger = SessionLogger::new()?;
    let mut game = GameSession::load(&config.game.story_path)
        .map_err(|e| anyhow::anyhow!("failed to load story '{}': {e}", config.game.story_path))?;
    let mut world = WorldModel::default();
    let llm = LlmClient::new(config.llm.clone());

    let initial_observation = game
        .execute("look")
        .map_err(|e| anyhow::anyhow!("failed to run initial look command: {e}"))?;
    print_game_output(&initial_observation.text);
    world.update_from_observation(&initial_observation.text);

    let mut task = AgentTask::new(command.to_string());
    loop {
        let events = run_single_turn(&mut task, &mut game, &mut world, &llm, &logger);
        for event in &events {
            match event {
                AgentEvent::Action(command) => print_agent_input(command),
                AgentEvent::Observation(text) => print_game_output(text),
                _ => {}
            }
        }
        let is_finished = events
            .iter()
            .any(|event| matches!(event, AgentEvent::Completed(_) | AgentEvent::Failed(_)));

        if is_finished || task.turns >= task.max_turns {
            break;
        }
    }

    Ok(world)
}

fn print_game_output(text: &str) {
    eprintln!("game>{text}");
}

fn print_agent_input(text: &str) {
    eprintln!("agent>{text}");
}

#[derive(Debug)]
struct Scores {
    share_of_titles_found: f64,
    share_of_titles_and_descriptions: f64,
}

fn score_world_state(world_state: &WorldState, first_nodes: &HashMap<String, FirstNode>) -> Scores {
    let world_titles = world_state
        .locations
        .values()
        .filter_map(|location| location.title.clone())
        .collect::<Vec<_>>();
    let world_title_descriptions = world_state
        .locations
        .values()
        .filter_map(|location| Some((location.title.clone()?, location.description.clone()?)))
        .collect::<Vec<_>>();

    score_values(world_titles, world_title_descriptions, first_nodes)
}

fn score_world_model(world: &WorldModel, first_nodes: &HashMap<String, FirstNode>) -> Scores {
    let world_titles = world
        .locations
        .values()
        .map(|location| location.title.clone())
        .collect::<Vec<_>>();
    let world_title_descriptions = world
        .locations
        .values()
        .map(|location| (location.title.clone(), location.description.clone()))
        .collect::<Vec<_>>();

    score_values(world_titles, world_title_descriptions, first_nodes)
}

fn score_values(
    world_titles: Vec<String>,
    world_title_descriptions: Vec<(String, String)>,
    first_nodes: &HashMap<String, FirstNode>,
) -> Scores {
    let expected_titles = first_nodes
        .values()
        .map(|node| node.title.clone())
        .collect::<Vec<_>>();
    let expected_title_descriptions = first_nodes
        .values()
        .map(|node| (node.title.clone(), node.description.clone()))
        .collect::<Vec<_>>();

    Scores {
        share_of_titles_found: multiset_jaccard(&world_titles, &expected_titles),
        share_of_titles_and_descriptions: multiset_jaccard(
            &world_title_descriptions,
            &expected_title_descriptions,
        ),
    }
}

fn average_titles(runs: &[EvalRun]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }

    runs.iter()
        .map(|run| run.share_of_titles_found)
        .sum::<f64>()
        / runs.len() as f64
}

fn average_title_descriptions(runs: &[EvalRun]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }

    runs.iter()
        .map(|run| run.share_of_titles_and_descriptions)
        .sum::<f64>()
        / runs.len() as f64
}

fn read_json<T>(path: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let contents = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&contents).with_context(|| format!("failed to parse {path}"))
}

fn multiset_jaccard<T>(left: &[T], right: &[T]) -> f64
where
    T: Eq + std::hash::Hash + Clone,
{
    let left_counts = counts(left);
    let right_counts = counts(right);

    let mut intersection = 0usize;
    let mut union = 0usize;

    for (item, left_count) in &left_counts {
        let right_count = right_counts.get(item).copied().unwrap_or(0);
        intersection += (*left_count).min(right_count);
        union += (*left_count).max(right_count);
    }

    for (item, right_count) in &right_counts {
        if !left_counts.contains_key(item) {
            union += right_count;
        }
    }

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

fn counts<T>(items: &[T]) -> HashMap<T, usize>
where
    T: Eq + std::hash::Hash + Clone,
{
    let mut counts = HashMap::new();
    for item in items {
        *counts.entry(item.clone()).or_insert(0) += 1;
    }
    counts
}
