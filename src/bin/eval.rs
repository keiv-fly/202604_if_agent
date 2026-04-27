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

use agent::AgentTask;
use anyhow::{Context, Result, bail};
use config::AppConfig;
use game::GameSession;
use game::validation::validate_game_command;
use llm::prompt::build_user_prompt;
use llm::{LlmClient, LlmResponseParseError};
use logging::SessionLogger;
use memory::WorldModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};

const DEFAULT_WORLD_STATE_PATH: &str = "memory_store/world-state.json";
const DEFAULT_FIRST_NODES_PATH: &str = "eval_data/first_nodes.json";
const DEFAULT_RUNS: usize = 3;
const DEFAULT_COMMAND: &str = "Explore to create a full map. Only use the actions to move. Do not do any other actions that are not moves.";

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
    share_of_titles_found_info: String,
    share_of_titles_and_descriptions: f64,
    share_of_titles_and_descriptions_info: String,
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
    if runner::run_persistent_child_from_args()? {
        return Ok(());
    }

    dotenvy::dotenv().ok();

    let args = Args::parse()?;
    let first_nodes = read_json::<HashMap<String, FirstNode>>(&args.first_nodes_path)?;
    let logger = SessionLogger::new_in("logs_eval")?;
    logger.log(
        "eval_start",
        &format!(
            "runs={} calculate_only={} world_state={} first_nodes={} command={:?}",
            args.runs,
            args.calculate_only,
            args.world_state_path,
            args.first_nodes_path,
            args.command
        ),
    );
    let mut runs = Vec::new();

    if args.calculate_only {
        let world_state = read_json::<WorldState>(&args.world_state_path)?;
        let scores = score_world_state(&world_state, &first_nodes);
        logger.log(
            "eval_run_score",
            &format!(
                "run=1 titles={} title_descriptions={}",
                scores.share_of_titles_found, scores.share_of_titles_and_descriptions
            ),
        );
        runs.push(EvalRun {
            run: 1,
            share_of_titles_found: scores.share_of_titles_found,
            share_of_titles_found_info: scores.share_of_titles_found_info,
            share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
            share_of_titles_and_descriptions_info: scores.share_of_titles_and_descriptions_info,
        });
    } else {
        for run in 1..=args.runs {
            print_eval_message(
                &logger,
                &format!("Starting eval run {run}/{}...", args.runs),
            );
            logger.log("eval_run_start", &format!("run={run}/{}", args.runs));
            let world = run_program(&args.command, &logger)
                .with_context(|| format!("failed during eval run {run}"))?;
            let scores = score_world_model(&world, &first_nodes);
            logger.log(
                "eval_run_score",
                &format!(
                    "run={run} titles={} title_descriptions={}",
                    scores.share_of_titles_found, scores.share_of_titles_and_descriptions
                ),
            );
            runs.push(EvalRun {
                run,
                share_of_titles_found: scores.share_of_titles_found,
                share_of_titles_found_info: scores.share_of_titles_found_info,
                share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
                share_of_titles_and_descriptions_info: scores.share_of_titles_and_descriptions_info,
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

    io::stdout().flush()?;
    let output_json = serde_json::to_string_pretty(&output)?;
    logger.log("eval_result", &output_json);
    println!("{output_json}");

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

fn run_program(command: &str, logger: &SessionLogger) -> Result<WorldModel> {
    let config = AppConfig::from_env();
    let mut game = GameSession::load(&config.game.story_path)
        .map_err(|e| anyhow::anyhow!("failed to load story '{}': {e}", config.game.story_path))?;
    let mut world = WorldModel::default();
    let llm = LlmClient::new(config.llm.clone());

    let initial_observation = game
        .execute("look")
        .map_err(|e| anyhow::anyhow!("failed to run initial look command: {e}"))?;
    logger.log("game_output", &initial_observation.text);
    print_game_output(&initial_observation.text);
    world.update_from_observation_with_command(&initial_observation.text, None);

    let mut task = AgentTask::new(command.to_string());
    loop {
        let is_finished = run_eval_turn(&mut task, &mut game, &mut world, &llm, &logger);

        if is_finished || task.turns >= task.max_turns {
            break;
        }
    }

    Ok(world)
}

fn run_eval_turn(
    task: &mut AgentTask,
    game: &mut GameSession,
    world: &mut WorldModel,
    llm: &LlmClient,
    logger: &SessionLogger,
) -> bool {
    let movement_only_mode = task
        .prompt
        .to_lowercase()
        .contains("only use the actions to move");

    if task.turns >= task.max_turns {
        print_eval_message(logger, "Stopping task due to max-turn safety guard.");
        return true;
    }

    let transcript_tail = game.transcript().render();
    let prompt = build_user_prompt(&task.prompt, &transcript_tail, world);
    let reply = match llm.next_action(&prompt, logger) {
        Ok(value) => value,
        Err(err) => {
            let message = format!("LLM error: {err}");
            logger.log("llm_error", &message);
            if let Some(parse_error) = err.downcast_ref::<LlmResponseParseError>() {
                logger.log("llm_unparsed_response", parse_error.raw_response());
            }
            print_eval_message(logger, &message);
            return true;
        }
    };

    logger.log("agent_thought", &reply.thought);
    if reply.task_status.complete {
        print_eval_message(logger, &reply.task_status.summary);
        return true;
    }

    if reply.action.action_type != "game_command" {
        print_eval_message(logger, "invalid action type from LLM");
        return true;
    }

    let mut command = match validate_game_command(&reply.action.command) {
        Ok(command) => command,
        Err(err) => {
            logger.log("agent_action", &reply.action.command);
            print_agent_input(&reply.action.command);
            print_eval_message(logger, &format!("invalid command from LLM: {err}"));
            return true;
        }
    };
    if movement_only_mode {
        let Some(normalized) = normalize_move_command(&command) else {
            logger.log("agent_action", &command);
            print_agent_input(&command);
            print_eval_message(logger, "ignored non-movement command in movement-only mode");
            task.turns += 1;
            return false;
        };
        command = normalized;
    }

    if task.last_command.as_deref() == Some(command.as_str()) {
        task.repeated_guard += 1;
    } else {
        task.repeated_guard = 0;
    }
    task.last_command = Some(command.clone());

    if task.repeated_guard >= 2 {
        logger.log("agent_action", &command);
        print_agent_input(&command);
        print_eval_message(logger, "loop guard: same command repeated too many times");
        return true;
    }

    print_agent_input(&command);
    logger.log("agent_action", &command);

    let observation = match game.execute(&command) {
        Ok(observation) => observation.text,
        Err(err) => {
            print_eval_message(logger, &format!("game command failed: {err}"));
            return true;
        }
    };

    world.update_from_observation_with_command(&observation, Some(&command));
    world.task_notes.extend(reply.memory_update.notes);
    logger.log("game_output", &observation);
    print_game_output(&observation);
    task.turns += 1;

    false
}

fn normalize_move_command(command: &str) -> Option<String> {
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

fn print_game_output(text: &str) {
    print_tagged_block("game", text);
}

fn print_agent_input(text: &str) {
    print_prefixed_lines("agent> ", text);
}

fn print_eval_message(logger: &SessionLogger, text: &str) {
    logger.log("eval_output", text);
    println!("eval>{text}");
    let _ = io::stdout().flush();
}

fn print_tagged_block(tag: &str, text: &str) {
    println!("<{tag}>");
    if !text.is_empty() {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
    }
    println!("</{tag}>");
    let _ = io::stdout().flush();
}

fn print_prefixed_lines(prefix: &str, text: &str) {
    if text.is_empty() {
        println!("{prefix}");
        let _ = io::stdout().flush();
        return;
    }

    for line in text.lines() {
        println!("{prefix}{line}");
    }
    let _ = io::stdout().flush();
}

#[derive(Debug)]
struct Scores {
    share_of_titles_found: f64,
    share_of_titles_found_info: String,
    share_of_titles_and_descriptions: f64,
    share_of_titles_and_descriptions_info: String,
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
    let actual_title_count = world_titles.len();
    let ground_truth_title_count = expected_titles.len();
    let actual_title_description_count = world_title_descriptions.len();
    let ground_truth_title_description_count = expected_title_descriptions.len();
    let world_title_nodes = disambiguate_duplicate_titles(world_titles);
    let expected_title_nodes = disambiguate_duplicate_titles(expected_titles);
    let title_score = multiset_jaccard(&world_title_nodes, &expected_title_nodes);
    let title_description_score =
        multiset_jaccard(&world_title_descriptions, &expected_title_descriptions);

    Scores {
        share_of_titles_found: title_score.value,
        share_of_titles_found_info: title_score.info(actual_title_count, ground_truth_title_count),
        share_of_titles_and_descriptions: title_description_score.value,
        share_of_titles_and_descriptions_info: title_description_score.info(
            actual_title_description_count,
            ground_truth_title_description_count,
        ),
    }
}

fn disambiguate_duplicate_titles(titles: Vec<String>) -> Vec<String> {
    let title_counts = counts(&titles);
    let mut seen = HashMap::<String, usize>::new();

    titles
        .into_iter()
        .map(|title| {
            if title_counts.get(&title).copied().unwrap_or(0) <= 1 {
                return title;
            }

            let occurrence = seen.entry(title.clone()).or_insert(0);
            *occurrence += 1;
            format!("{title}#{}", *occurrence)
        })
        .collect()
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

struct JaccardScore {
    value: f64,
    numerator: usize,
    denominator: usize,
}

impl JaccardScore {
    fn info(&self, actual_count: usize, ground_truth_count: usize) -> String {
        format!(
            "{}/{}, {}, {}",
            self.numerator, self.denominator, actual_count, ground_truth_count
        )
    }
}

fn multiset_jaccard<T>(left: &[T], right: &[T]) -> JaccardScore
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

    let value = if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    };

    JaccardScore {
        value,
        numerator: intersection,
        denominator: union,
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
