use crate::game::GameSession;
use crate::game::validation::validate_game_command;
use crate::llm::prompt::build_user_prompt;
use crate::llm::{AgentReply, LlmClient, LlmResponseParseError};
use crate::logging::SessionLogger;
use crate::memory::WorldModel;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thought(String),
    Action(String),
    Observation(String),
    Completed(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct AgentTask {
    pub prompt: String,
    pub max_turns: usize,
    pub turns: usize,
    pub repeated_guard: usize,
    pub last_command: Option<String>,
}

impl AgentTask {
    pub fn new(prompt: String) -> Self {
        Self {
            prompt,
            max_turns: 30,
            turns: 0,
            repeated_guard: 0,
            last_command: None,
        }
    }
}

pub fn run_single_turn(
    task: &mut AgentTask,
    game: &mut GameSession,
    world: &mut WorldModel,
    llm: &LlmClient,
    logger: &SessionLogger,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();

    if task.turns >= task.max_turns {
        events.push(AgentEvent::Completed(
            "Stopping task due to max-turn safety guard.".to_string(),
        ));
        return events;
    }

    let transcript_tail = game.transcript().render();
    let prompt = build_user_prompt(&task.prompt, &transcript_tail, world);

    let reply = match llm.next_action(&prompt, logger) {
        Ok(v) => v,
        Err(err) => {
            let message = format!("LLM error: {err}");
            logger.log("llm_error", &message);
            if let Some(parse_error) = err.downcast_ref::<LlmResponseParseError>() {
                logger.log("llm_unparsed_response", parse_error.raw_response());
            }
            events.push(AgentEvent::Failed(message));
            return events;
        }
    };

    apply_reply(task, reply, game, world, logger, &mut events);
    task.turns += 1;
    events
}

fn apply_reply(
    task: &mut AgentTask,
    reply: AgentReply,
    game: &mut GameSession,
    world: &mut WorldModel,
    logger: &SessionLogger,
    events: &mut Vec<AgentEvent>,
) {
    events.push(AgentEvent::Thought(reply.thought.clone()));
    logger.log("agent_thought", &reply.thought);

    if reply.task_status.complete {
        events.push(AgentEvent::Completed(reply.task_status.summary));
        return;
    }

    if reply.action.action_type != "game_command" {
        events.push(AgentEvent::Failed(
            "invalid action type from LLM".to_string(),
        ));
        return;
    }

    let command = match validate_game_command(&reply.action.command) {
        Ok(c) => c,
        Err(e) => {
            events.push(AgentEvent::Failed(format!("invalid command from LLM: {e}")));
            return;
        }
    };

    if task.last_command.as_deref() == Some(command.as_str()) {
        task.repeated_guard += 1;
    } else {
        task.repeated_guard = 0;
    }
    task.last_command = Some(command.clone());

    if task.repeated_guard >= 2 {
        events.push(AgentEvent::Failed(
            "loop guard: same command repeated too many times".to_string(),
        ));
        return;
    }

    events.push(AgentEvent::Action(command.clone()));
    logger.log("agent_action", &command);

    let obs = match game.execute(&command) {
        Ok(obs) => obs.text,
        Err(e) => {
            events.push(AgentEvent::Failed(format!("game command failed: {e}")));
            return;
        }
    };

    world.update_from_observation_with_command(&obs, Some(&command));
    world.task_notes.extend(reply.memory_update.notes);
    logger.log("game_output", &obs);
    events.push(AgentEvent::Observation(obs));
}
