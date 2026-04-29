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
#[allow(dead_code, unused_imports)]
#[path = "../planner/mod.rs"]
mod planner;

use agent::AgentTask;
use anyhow::{Context, Result, bail};
use config::AppConfig;
use game::GameSession;
use game::validation::validate_game_command;
use llm::prompt::build_user_prompt;
use llm::{LlmClient, LlmResponseParseError};
use logging::SessionLogger;
use memory::WorldModel;
use planner::{
    DfsPlanner, ObservationClassification, ObservationUpdate, PlannerDecisionKind, location_key,
    normalize_move_command,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Write};

const DEFAULT_WORLD_STATE_PATH: &str = "memory_store/world-state.json";
const DEFAULT_FIRST_NODES_PATH: &str = "eval_gt/map_gt_01.json";
const DEFAULT_RUNS: usize = 3;
const DEFAULT_STRATEGY: EvalStrategy = EvalStrategy::Dfs;
const DEFAULT_COMMAND: &str = "Explore to create a full map. Only use the actions to move. Do not do any other actions that are not moves.";

#[derive(Debug)]
struct Args {
    runs: usize,
    calculate_only: bool,
    world_state_path: String,
    first_nodes_path: String,
    command: String,
    strategy: EvalStrategy,
    dfs_max_turns: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum EvalStrategy {
    Dfs,
    LlmAgent,
}

#[derive(Debug, Deserialize)]
struct WorldState {
    locations: HashMap<String, WorldLocation>,
}

#[derive(Debug, Deserialize)]
struct WorldLocation {
    title: Option<String>,
    description: Option<String>,
    #[serde(default)]
    exits: Vec<WorldStateExit>,
}

#[derive(Debug, Deserialize)]
struct WorldStateExit {
    direction: String,
    destination: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FirstNode {
    title: String,
    description: String,
    exits: HashMap<String, HashMap<String, f64>>,
    #[serde(default)]
    exits_to: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct EvalRun {
    run: usize,
    strategy: EvalStrategy,
    planner_turns: usize,
    generated_command_plans: usize,
    executed_movement_commands: usize,
    failed_moves: usize,
    frontier_counts: HashMap<String, usize>,
    share_of_titles_found: f64,
    share_of_titles_found_info: String,
    share_of_titles_and_descriptions: f64,
    share_of_titles_and_descriptions_info: String,
    share_of_exits: f64,
    share_of_exits_info: String,
    share_exit_to: f64,
    share_exit_to_info: String,
}

#[derive(Debug, Serialize)]
struct EvalOutput {
    command: String,
    strategy: EvalStrategy,
    requested_runs: usize,
    calculation_only: bool,
    runs: Vec<EvalRun>,
    average_share_of_titles_found: f64,
    average_share_of_titles_and_descriptions: f64,
    average_share_of_exits: f64,
    average_share_exit_to: f64,
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
        write_world_state_exit_to_artifacts(&logger, 1, &world_state, &first_nodes)?;
        let scores = score_world_state(&world_state, &first_nodes);
        logger.log(
            "eval_run_score",
            &format!(
                "run=1 titles={} title_descriptions={} exits={} exit_to={}",
                scores.share_of_titles_found,
                scores.share_of_titles_and_descriptions,
                scores.share_of_exits,
                scores.share_exit_to
            ),
        );
        runs.push(EvalRun {
            run: 1,
            strategy: args.strategy,
            planner_turns: 0,
            generated_command_plans: 0,
            executed_movement_commands: 0,
            failed_moves: 0,
            frontier_counts: HashMap::new(),
            share_of_titles_found: scores.share_of_titles_found,
            share_of_titles_found_info: scores.share_of_titles_found_info,
            share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
            share_of_titles_and_descriptions_info: scores.share_of_titles_and_descriptions_info,
            share_of_exits: scores.share_of_exits,
            share_of_exits_info: scores.share_of_exits_info,
            share_exit_to: scores.share_exit_to,
            share_exit_to_info: scores.share_exit_to_info,
        });
    } else {
        for run in 1..=args.runs {
            print_eval_message(
                &logger,
                &format!("Starting eval run {run}/{}...", args.runs),
            );
            logger.log("eval_run_start", &format!("run={run}/{}", args.runs));
            let run_result = run_program(&args.command, args.strategy, args.dfs_max_turns, &logger)
                .with_context(|| format!("failed during eval run {run}"))?;
            write_final_map_artifact(&logger, run, &run_result.world, &first_nodes)
                .with_context(|| format!("failed to write final map for eval run {run}"))?;
            write_world_model_exit_to_artifacts(&logger, run, &run_result.world, &first_nodes)
                .with_context(|| format!("failed to write exit_to artifacts for eval run {run}"))?;
            let scores = score_world_model(&run_result.world, &first_nodes);
            logger.log(
                "eval_run_score",
                &format!(
                    "run={run} titles={} title_descriptions={} exits={} exit_to={}",
                    scores.share_of_titles_found,
                    scores.share_of_titles_and_descriptions,
                    scores.share_of_exits,
                    scores.share_exit_to
                ),
            );
            runs.push(EvalRun {
                run,
                strategy: args.strategy,
                planner_turns: run_result.planner_turns,
                generated_command_plans: run_result.generated_command_plans,
                executed_movement_commands: run_result.executed_movement_commands,
                failed_moves: run_result.failed_moves,
                frontier_counts: run_result.frontier_counts,
                share_of_titles_found: scores.share_of_titles_found,
                share_of_titles_found_info: scores.share_of_titles_found_info,
                share_of_titles_and_descriptions: scores.share_of_titles_and_descriptions,
                share_of_titles_and_descriptions_info: scores.share_of_titles_and_descriptions_info,
                share_of_exits: scores.share_of_exits,
                share_of_exits_info: scores.share_of_exits_info,
                share_exit_to: scores.share_exit_to,
                share_exit_to_info: scores.share_exit_to_info,
            });
        }
    }

    let output = EvalOutput {
        command: args.command,
        strategy: args.strategy,
        requested_runs: args.runs,
        calculation_only: args.calculate_only,
        average_share_of_titles_found: average_titles(&runs),
        average_share_of_titles_and_descriptions: average_title_descriptions(&runs),
        average_share_of_exits: average_exits(&runs),
        average_share_exit_to: average_exit_to(&runs),
        runs,
    };

    io::stdout().flush()?;
    let output_json = serde_json::to_string_pretty(&output)?;
    logger.log("eval_result", &output_json);
    println!("{output_json}");

    Ok(())
}

#[derive(Debug, Serialize)]
struct FinalMapLocation {
    title: String,
    description: String,
    exits: BTreeMap<String, BTreeMap<String, usize>>,
}

fn write_final_map_artifact(
    logger: &SessionLogger,
    run: usize,
    world: &WorldModel,
    first_nodes: &HashMap<String, FirstNode>,
) -> Result<()> {
    let path = logger
        .llm_dir()
        .join(format!("run-{run:03}-final-map.json"));
    let json = serde_json::to_string_pretty(&final_map_artifact(world, first_nodes))
        .context("serialize final map to JSON")?;
    fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    logger.log(
        "eval_final_map",
        &format!("run={run} path={}", path.display()),
    );
    Ok(())
}

fn final_map_artifact(
    world: &WorldModel,
    first_nodes: &HashMap<String, FirstNode>,
) -> BTreeMap<String, FinalMapLocation> {
    let node_ids = first_node_ids_by_location(first_nodes);
    let mut locations = BTreeMap::new();

    for (source_key, location) in &world.locations {
        let source = node_id_for_world_model_location(source_key, location, &node_ids);
        let mut exits = BTreeMap::<String, BTreeMap<String, usize>>::new();

        for exit in &location.exits {
            let Some(command) = final_map_move_command(&exit.direction) else {
                continue;
            };
            let destinations = exits.entry(command).or_default();

            if exit.transition_counts.is_empty() {
                if let Some(destination) = exit.destination.as_ref() {
                    destinations
                        .entry(node_id_for_world_model_destination(
                            destination,
                            world,
                            first_nodes,
                            &node_ids,
                        ))
                        .or_insert(0);
                }
                continue;
            }

            for (destination, count) in &exit.transition_counts {
                *destinations
                    .entry(node_id_for_world_model_destination(
                        destination,
                        world,
                        first_nodes,
                        &node_ids,
                    ))
                    .or_insert(0) += *count;
            }
        }

        locations.insert(
            source,
            FinalMapLocation {
                title: location.title.clone(),
                description: location.description.clone(),
                exits,
            },
        );
    }

    locations
}

fn write_world_state_exit_to_artifacts(
    logger: &SessionLogger,
    run: usize,
    world_state: &WorldState,
    first_nodes: &HashMap<String, FirstNode>,
) -> Result<()> {
    let actual = exit_to_items_from_exit_items(&world_state_exit_items(world_state, first_nodes));
    write_exit_to_artifacts(logger, run, actual, first_nodes)
}

fn write_world_model_exit_to_artifacts(
    logger: &SessionLogger,
    run: usize,
    world: &WorldModel,
    first_nodes: &HashMap<String, FirstNode>,
) -> Result<()> {
    let actual = exit_to_items_from_exit_items(&world_model_exit_items(world, first_nodes));
    write_exit_to_artifacts(logger, run, actual, first_nodes)
}

fn write_exit_to_artifacts(
    logger: &SessionLogger,
    run: usize,
    actual: Vec<ExitToItem>,
    first_nodes: &HashMap<String, FirstNode>,
) -> Result<()> {
    let expected = first_node_exit_to_items(first_nodes);
    let actual_path = logger.llm_dir().join(format!("run-{run:03}-exit-to.json"));
    let diff_path = logger
        .llm_dir()
        .join(format!("run-{run:03}-exit-to-diff.json"));
    let actual_json = serde_json::to_string_pretty(&exit_to_destination_map(&actual))
        .context("serialize exit_to")?;
    let diff_json = serde_json::to_string_pretty(&exit_to_diff(&expected, &actual))
        .context("serialize exit_to diff")?;

    fs::write(&actual_path, actual_json)
        .with_context(|| format!("write {}", actual_path.display()))?;
    fs::write(&diff_path, diff_json).with_context(|| format!("write {}", diff_path.display()))?;
    logger.log(
        "eval_exit_to_artifacts",
        &format!(
            "run={run} actual={} diff={}",
            actual_path.display(),
            diff_path.display()
        ),
    );
    Ok(())
}

impl Args {
    fn parse() -> Result<Self> {
        let mut runs = DEFAULT_RUNS;
        let mut calculate_only = false;
        let mut world_state_path = DEFAULT_WORLD_STATE_PATH.to_string();
        let mut first_nodes_path = DEFAULT_FIRST_NODES_PATH.to_string();
        let mut command = DEFAULT_COMMAND.to_string();
        let mut strategy = DEFAULT_STRATEGY;
        let mut dfs_max_turns = AgentTask::new(String::new()).max_turns;
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
                "--strategy" => {
                    let value = args.next().context("--strategy requires a value")?;
                    strategy = EvalStrategy::parse(&value)?;
                }
                "--dfs-max-turns" => {
                    let value = args.next().context("--dfs-max-turns requires a value")?;
                    dfs_max_turns = value
                        .parse()
                        .with_context(|| format!("invalid --dfs-max-turns value '{value}'"))?;
                }
                value if value.starts_with('-') => bail!("unknown argument '{value}'"),
                value => positional.push(value.to_string()),
            }
        }

        if runs == 0 {
            bail!("--runs must be at least 1");
        }
        if dfs_max_turns == 0 {
            bail!("--dfs-max-turns must be at least 1");
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
            strategy,
            dfs_max_turns,
        })
    }
}

impl EvalStrategy {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "dfs" => Ok(Self::Dfs),
            "llm-agent" => Ok(Self::LlmAgent),
            _ => bail!("unknown --strategy '{value}', expected 'dfs' or 'llm-agent'"),
        }
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
      --strategy <STRATEGY>   Eval strategy: dfs or llm-agent (default: dfs)\n\
      --dfs-max-turns <N>     DFS strategy turn cap (default: 30)\n\
      --command <TEXT>        Agent command/prompt to run (default: {DEFAULT_COMMAND:?})\n\
  -h, --help                  Show this help"
    );
}

#[derive(Debug)]
struct RunResult {
    world: WorldModel,
    planner_turns: usize,
    generated_command_plans: usize,
    executed_movement_commands: usize,
    failed_moves: usize,
    frontier_counts: HashMap<String, usize>,
}

fn run_program(
    command: &str,
    strategy: EvalStrategy,
    dfs_max_turns: usize,
    logger: &SessionLogger,
) -> Result<RunResult> {
    let config = AppConfig::from_env();
    let mut game = GameSession::load(&config.game.story_path)
        .map_err(|e| anyhow::anyhow!("failed to load story '{}': {e}", config.game.story_path))?;
    let mut world = WorldModel::default();

    let initial_observation = game
        .execute("look")
        .map_err(|e| anyhow::anyhow!("failed to run initial look command: {e}"))?;
    logger.log("game_output", &initial_observation.text);
    print_game_output(&initial_observation.text);
    world.update_from_observation(&initial_observation.text);

    match strategy {
        EvalStrategy::Dfs => run_dfs_strategy(command, dfs_max_turns, &mut game, world, logger),
        EvalStrategy::LlmAgent => {
            let llm = LlmClient::new(config.llm.clone());
            run_llm_agent_strategy(command, &mut game, world, &llm, logger)
        }
    }
}

fn run_llm_agent_strategy(
    command: &str,
    game: &mut GameSession,
    mut world: WorldModel,
    llm: &LlmClient,
    logger: &SessionLogger,
) -> Result<RunResult> {
    let mut task = AgentTask::new(command.to_string());
    loop {
        let is_finished = run_eval_turn(&mut task, game, &mut world, llm, logger);

        if is_finished || task.turns >= task.max_turns {
            break;
        }
    }

    Ok(RunResult {
        world,
        planner_turns: 0,
        generated_command_plans: 0,
        executed_movement_commands: 0,
        failed_moves: 0,
        frontier_counts: HashMap::new(),
    })
}

fn run_dfs_strategy(
    _command: &str,
    max_turns: usize,
    game: &mut GameSession,
    mut world: WorldModel,
    logger: &SessionLogger,
) -> Result<RunResult> {
    let mut planner = DfsPlanner::new(&world);

    loop {
        if planner.stats.turns >= max_turns {
            print_eval_message(
                logger,
                "Stopping DFS strategy due to max-turn safety guard.",
            );
            break;
        }

        let decision = planner.decide(&world);
        logger.log("planner_decision", &format!("{decision:?}"));

        match decision.kind {
            PlannerDecisionKind::CommandPlan => {
                let plan = decision
                    .plan
                    .context("planner returned CommandPlan without a plan")?;
                for command in &plan.commands {
                    validate_game_command(command)
                        .with_context(|| format!("invalid DFS command '{command}'"))?;
                    if normalize_move_command(command).as_deref() != Some(command.as_str()) {
                        bail!("DFS command '{command}' is not canonical movement");
                    }
                }

                let command = plan
                    .commands
                    .first()
                    .context("planner returned empty command plan")?
                    .clone();
                if let Some(step) = plan.route_steps.first() {
                    if step.command != command {
                        bail!(
                            "DFS route step command '{}' did not match first planned command '{}'",
                            step.command,
                            command
                        );
                    }
                }
                let expected_route_destination = plan
                    .route_steps
                    .first()
                    .map(|step| step.expected_destination.clone());
                let action_id = if plan.route_commands.is_empty() && !plan.is_recovery {
                    Some(plan.selected_frontier_action_id.clone())
                } else {
                    None
                };

                print_agent_input(&command);
                logger.log(
                    "planner_command",
                    &format!(
                        "command={command} selected={} route_len={} recovery={} reason={}",
                        plan.selected_frontier_action_id,
                        plan.route_commands.len(),
                        plan.is_recovery,
                        plan.reason
                    ),
                );

                let previous_location = location_key(&world);
                let attempt =
                    planner.pending_move_attempt(&previous_location, &command, action_id, &world);
                let command_result = game.execute(&command);
                let command_failed = command_result.is_err();
                let observation = match command_result {
                    Ok(observation) => observation.text,
                    Err(err) => {
                        print_eval_message(logger, &format!("game command failed: {err}"));
                        String::new()
                    }
                };
                let classified =
                    planner.classify_observation(&attempt, &observation, command_failed);
                logger.log(
                    "planner_observation_classification",
                    &format!(
                        "source={} command={} frontier={} previous_observation_signature={} new_observation_signature={} classification={:?} blocked_reason={} current_location_unchanged={}",
                        attempt.source_location_key,
                        attempt.command,
                        attempt.frontier_id.as_deref().unwrap_or(""),
                        attempt.previous_observation_signature,
                        classified.new_observation_signature.as_deref().unwrap_or(""),
                        classified.classification,
                        classified.blocked_reason.as_deref().unwrap_or(""),
                        classified.current_location_unchanged,
                    ),
                );

                if classified.classification == ObservationClassification::CommandFailedOrBlocked {
                    world.current_location = attempt.source_location_key.clone();
                    world.apply_command_result(&previous_location, &command, true);
                } else if !observation.is_empty() {
                    if classified.classification == ObservationClassification::MovedToNewLocation {
                        let canonical_observation = match game.execute("look") {
                            Ok(look_observation) => {
                                logger.log("game_output", &look_observation.text);
                                print_game_output(&look_observation.text);
                                look_observation.text
                            }
                            Err(err) => {
                                print_eval_message(
                                    logger,
                                    &format!("look after newly discovered location failed: {err}"),
                                );
                                observation.clone()
                            }
                        };
                        world.update_from_observation(&canonical_observation);
                    } else {
                        world.update_from_observation(&observation);
                    }
                    world.apply_command_result_with_destination(
                        &previous_location,
                        &command,
                        Some(&location_key(&world)),
                    );
                }
                logger.log("game_output", &observation);
                print_game_output(&observation);

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

                if let Some(expected) = expected_route_destination {
                    let actual = location_key(&world);
                    if actual != expected {
                        logger.log(
                            "planner_route_mismatch",
                            &format!(
                                "command={command} expected_location={expected} actual_location={actual} action={} route_len_remaining={} repair=replan_from_actual_location",
                                plan.selected_frontier_action_id,
                                plan.route_commands.len().saturating_sub(1),
                            ),
                        );
                        print_eval_message(
                            logger,
                            &format!(
                                "Known DFS transition landed at {actual:?}, expected {expected:?}; replanning from current location."
                            ),
                        );
                    }
                }
            }
            PlannerDecisionKind::Complete => {
                print_eval_message(logger, &format!("DFS complete: {}", decision.reason));
                break;
            }
            PlannerDecisionKind::Blocked => {
                print_eval_message(logger, &format!("DFS blocked: {}", decision.reason));
                break;
            }
        }
    }

    let frontier_counts = planner
        .frontier_counts()
        .into_iter()
        .map(|(status, count)| (status.to_string(), count))
        .collect();
    let frontier_path = logger.llm_dir().join("dfs_frontier.json");
    let frontier_json = serde_json::to_string_pretty(planner.frontier())
        .context("serialize dfs frontier to JSON")?;
    fs::write(&frontier_path, &frontier_json)
        .with_context(|| format!("write {}", frontier_path.display()))?;
    Ok(RunResult {
        world,
        planner_turns: planner.stats.turns,
        generated_command_plans: planner.stats.generated_command_plans,
        executed_movement_commands: planner.stats.executed_movement_commands,
        failed_moves: planner.stats.failed_moves,
        frontier_counts,
    })
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

    let prompt = build_user_prompt(&task.prompt, world);
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

    let previous_location = world.current_location.clone();
    let observation = match game.execute(&command) {
        Ok(observation) => observation.text,
        Err(err) => {
            world.apply_command_result(&previous_location, &command, true);
            print_eval_message(logger, &format!("game command failed: {err}"));
            return true;
        }
    };

    world.update_from_observation(&observation);
    world.apply_command_result(&previous_location, &command, false);
    world.apply_llm_memory(
        &reply.memory_update.location,
        &reply.memory_update.new_exits,
        &reply.memory_update.new_objects,
        &reply.memory_update.notes,
    );
    logger.log("game_output", &observation);
    print_game_output(&observation);
    task.turns += 1;

    false
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
    share_of_exits: f64,
    share_of_exits_info: String,
    share_exit_to: f64,
    share_exit_to_info: String,
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
    let world_exits = world_state_exit_items(world_state, first_nodes);

    score_values(
        world_titles,
        world_title_descriptions,
        world_exits,
        first_nodes,
    )
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
    let world_exits = world_model_exit_items(world, first_nodes);

    score_values(
        world_titles,
        world_title_descriptions,
        world_exits,
        first_nodes,
    )
}

fn score_values(
    world_titles: Vec<String>,
    world_title_descriptions: Vec<(String, String)>,
    world_exits: Vec<ExitItem>,
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
    let expected_exits = first_node_exit_items(first_nodes);
    let expected_exit_to = first_node_exit_to_items(first_nodes);
    let actual_exit_to = exit_to_items_from_exit_items(&world_exits);
    let actual_exit_count = world_exits.len();
    let ground_truth_exit_count = expected_exits.len();
    let actual_exit_to_count = actual_exit_to.len();
    let ground_truth_exit_to_count = expected_exit_to.len();
    let world_title_nodes = disambiguate_duplicate_titles(world_titles);
    let expected_title_nodes = disambiguate_duplicate_titles(expected_titles);
    let title_score = multiset_jaccard(&world_title_nodes, &expected_title_nodes);
    let title_description_score =
        multiset_jaccard(&world_title_descriptions, &expected_title_descriptions);
    let exit_score = multiset_jaccard(&world_exits, &expected_exits);
    let exit_to_score = multiset_jaccard(&actual_exit_to, &expected_exit_to);

    Scores {
        share_of_titles_found: title_score.value,
        share_of_titles_found_info: title_score.info(actual_title_count, ground_truth_title_count),
        share_of_titles_and_descriptions: title_description_score.value,
        share_of_titles_and_descriptions_info: title_description_score.info(
            actual_title_description_count,
            ground_truth_title_description_count,
        ),
        share_of_exits: exit_score.value,
        share_of_exits_info: exit_score.info(actual_exit_count, ground_truth_exit_count),
        share_exit_to: exit_to_score.value,
        share_exit_to_info: exit_to_score.info(actual_exit_to_count, ground_truth_exit_to_count),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExitItem {
    source: String,
    direction: String,
    destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExitToItem {
    source: String,
    destination: String,
}

#[derive(Debug, Serialize)]
struct ExitToDiff {
    shared_by_both: Vec<ExitToPair>,
    missing_in_actual: Vec<ExitToPair>,
    extra_in_actual: Vec<ExitToPair>,
}

#[derive(Debug, Serialize)]
struct ExitToPair {
    source: String,
    destination: String,
}

fn first_node_exit_items(first_nodes: &HashMap<String, FirstNode>) -> Vec<ExitItem> {
    first_nodes
        .iter()
        .flat_map(|(source, node)| {
            node.exits
                .iter()
                .flat_map(move |(direction, destinations)| {
                    destinations.keys().filter_map(move |destination| {
                        Some(ExitItem {
                            source: source.clone(),
                            direction: normalize_move_command(direction)?,
                            destination: destination.clone(),
                        })
                    })
                })
        })
        .collect()
}

fn first_node_exit_to_items(first_nodes: &HashMap<String, FirstNode>) -> Vec<ExitToItem> {
    unique_exit_to_items(
        first_nodes
            .iter()
            .flat_map(|(source, node)| {
                if node.exits_to.is_empty() {
                    derived_first_node_exit_to_items(source, node)
                } else {
                    node.exits_to
                        .keys()
                        .map(|destination| ExitToItem {
                            source: source.clone(),
                            destination: destination.clone(),
                        })
                        .collect()
                }
            })
            .collect(),
    )
}

fn derived_first_node_exit_to_items(source: &str, node: &FirstNode) -> Vec<ExitToItem> {
    unique_exit_to_items(
        node.exits
            .values()
            .flat_map(|destinations| destinations.keys())
            .map(|destination| ExitToItem {
                source: source.to_string(),
                destination: destination.clone(),
            })
            .collect(),
    )
}

fn exit_to_items_from_exit_items(exits: &[ExitItem]) -> Vec<ExitToItem> {
    unique_exit_to_items(
        exits
            .iter()
            .map(|exit| ExitToItem {
                source: exit.source.clone(),
                destination: exit.destination.clone(),
            })
            .collect(),
    )
}

fn unique_exit_to_items(items: Vec<ExitToItem>) -> Vec<ExitToItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

fn exit_to_destination_map(items: &[ExitToItem]) -> BTreeMap<String, Vec<String>> {
    let mut destinations_by_source = BTreeMap::<String, BTreeSet<String>>::new();
    for item in items {
        destinations_by_source
            .entry(item.source.clone())
            .or_default()
            .insert(item.destination.clone());
    }

    destinations_by_source
        .into_iter()
        .map(|(source, destinations)| (source, destinations.into_iter().collect()))
        .collect()
}

fn exit_to_diff(expected: &[ExitToItem], actual: &[ExitToItem]) -> ExitToDiff {
    let expected_pairs = exit_to_pair_set(expected);
    let actual_pairs = exit_to_pair_set(actual);
    ExitToDiff {
        shared_by_both: expected_pairs
            .intersection(&actual_pairs)
            .map(exit_to_pair_from_tuple)
            .collect(),
        missing_in_actual: expected_pairs
            .difference(&actual_pairs)
            .map(exit_to_pair_from_tuple)
            .collect(),
        extra_in_actual: actual_pairs
            .difference(&expected_pairs)
            .map(exit_to_pair_from_tuple)
            .collect(),
    }
}

fn exit_to_pair_set(items: &[ExitToItem]) -> BTreeSet<(String, String)> {
    items
        .iter()
        .map(|item| (item.source.clone(), item.destination.clone()))
        .collect()
}

fn exit_to_pair_from_tuple(pair: &(String, String)) -> ExitToPair {
    ExitToPair {
        source: pair.0.clone(),
        destination: pair.1.clone(),
    }
}

fn world_state_exit_items(
    world_state: &WorldState,
    first_nodes: &HashMap<String, FirstNode>,
) -> Vec<ExitItem> {
    let node_ids = first_node_ids_by_location(first_nodes);
    let mut exit_items = Vec::new();

    for (source_key, location) in &world_state.locations {
        let source = node_id_for_world_state_location(source_key, location, &node_ids);
        for exit in &location.exits {
            let Some(direction) = normalize_move_command(&exit.direction) else {
                continue;
            };
            let Some(destination) = exit.destination.as_ref() else {
                continue;
            };

            exit_items.push(ExitItem {
                source: source.clone(),
                direction,
                destination: node_id_for_world_state_destination(
                    destination,
                    world_state,
                    first_nodes,
                    &node_ids,
                ),
            });
        }
    }

    exit_items
}

fn world_model_exit_items(
    world: &WorldModel,
    first_nodes: &HashMap<String, FirstNode>,
) -> Vec<ExitItem> {
    let node_ids = first_node_ids_by_location(first_nodes);
    let mut exit_items = Vec::new();

    for (source_key, location) in &world.locations {
        let source = node_id_for_world_model_location(source_key, location, &node_ids);
        for exit in &location.exits {
            let Some(direction) = normalize_move_command(&exit.direction) else {
                continue;
            };

            if !exit.transition_counts.is_empty() {
                for destination in exit.transition_counts.keys() {
                    exit_items.push(ExitItem {
                        source: source.clone(),
                        direction: direction.clone(),
                        destination: node_id_for_world_model_destination(
                            destination,
                            world,
                            first_nodes,
                            &node_ids,
                        ),
                    });
                }
                continue;
            }

            let Some(destination) = exit.destination.as_ref() else {
                continue;
            };

            exit_items.push(ExitItem {
                source: source.clone(),
                direction,
                destination: node_id_for_world_model_destination(
                    destination,
                    world,
                    first_nodes,
                    &node_ids,
                ),
            });
        }
    }

    exit_items
}

fn final_map_move_command(direction: &str) -> Option<String> {
    let command = normalize_move_command(direction)?;
    Some(
        match command.as_str() {
            "north" => "n",
            "south" => "s",
            "east" => "e",
            "west" => "w",
            "northeast" => "ne",
            "northwest" => "nw",
            "southeast" => "se",
            "southwest" => "sw",
            "up" => "u",
            "down" => "d",
            "in" => "in",
            "out" => "out",
            _ => return None,
        }
        .to_string(),
    )
}

fn first_node_ids_by_location(
    first_nodes: &HashMap<String, FirstNode>,
) -> HashMap<(String, String), String> {
    first_nodes
        .iter()
        .map(|(key, node)| {
            (
                (
                    normalize_score_text(&node.title),
                    normalize_score_text(&node.description),
                ),
                key.clone(),
            )
        })
        .collect()
}

fn node_id_for_world_state_destination(
    destination: &str,
    world_state: &WorldState,
    first_nodes: &HashMap<String, FirstNode>,
    node_ids: &HashMap<(String, String), String>,
) -> String {
    if first_nodes.contains_key(destination) {
        return destination.to_string();
    }

    world_state
        .locations
        .get(destination)
        .map(|location| node_id_for_world_state_location(destination, location, node_ids))
        .unwrap_or_else(|| destination.to_string())
}

fn node_id_for_world_model_destination(
    destination: &str,
    world: &WorldModel,
    first_nodes: &HashMap<String, FirstNode>,
    node_ids: &HashMap<(String, String), String>,
) -> String {
    if first_nodes.contains_key(destination) {
        return destination.to_string();
    }

    world
        .locations
        .get(destination)
        .map(|location| node_id_for_world_model_location(destination, location, node_ids))
        .unwrap_or_else(|| destination.to_string())
}

fn node_id_for_world_state_location(
    key: &str,
    location: &WorldLocation,
    node_ids: &HashMap<(String, String), String>,
) -> String {
    match (&location.title, &location.description) {
        (Some(title), Some(description)) => node_ids
            .get(&(
                normalize_score_text(title),
                normalize_score_text(description),
            ))
            .cloned()
            .unwrap_or_else(|| key.to_string()),
        _ => key.to_string(),
    }
}

fn node_id_for_world_model_location(
    key: &str,
    location: &memory::world::Location,
    node_ids: &HashMap<(String, String), String>,
) -> String {
    node_ids
        .get(&(
            normalize_score_text(&location.title),
            normalize_score_text(&location.description),
        ))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn normalize_score_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
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

fn average_exits(runs: &[EvalRun]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }

    runs.iter().map(|run| run.share_of_exits).sum::<f64>() / runs.len() as f64
}

fn average_exit_to(runs: &[EvalRun]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }

    runs.iter().map(|run| run.share_exit_to).sum::<f64>() / runs.len() as f64
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

#[cfg(test)]
mod tests {
    use super::*;
    use memory::world::Location;

    #[test]
    fn final_map_exits_use_command_abbreviations_and_transition_counts() {
        let mut world = WorldModel::default();
        world.current_location = "Start".to_string();
        world.locations.insert(
            "Start".to_string(),
            Location {
                title: "Start".to_string(),
                description: "You are at the start.".to_string(),
                ..Default::default()
            },
        );
        world.locations.insert(
            "Forest".to_string(),
            Location {
                title: "Forest".to_string(),
                description: "You are in the forest.".to_string(),
                ..Default::default()
            },
        );

        world.apply_command_result_with_destination("Start", "north", Some("Forest"));
        world.apply_command_result_with_destination("Start", "n", Some("Forest"));

        let first_nodes = HashMap::from([
            (
                "start".to_string(),
                FirstNode {
                    title: "Start".to_string(),
                    description: "You are at the start.".to_string(),
                    exits: HashMap::new(),
                    exits_to: HashMap::new(),
                },
            ),
            (
                "forest".to_string(),
                FirstNode {
                    title: "Forest".to_string(),
                    description: "You are in the forest.".to_string(),
                    exits: HashMap::new(),
                    exits_to: HashMap::new(),
                },
            ),
        ]);

        let final_map = final_map_artifact(&world, &first_nodes);

        assert_eq!(
            final_map
                .get("start")
                .and_then(|location| location.exits.get("n"))
                .and_then(|destinations| destinations.get("forest")),
            Some(&2)
        );
    }
}
