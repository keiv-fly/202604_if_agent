use crate::game::clean::clean_output;
use crate::game::transcript::Transcript;
use crate::game::validation::validate_game_command;
use anyhow::Result;
use runner::{Runner, RunnerError};

#[derive(Debug, Clone)]
pub struct Observation {
    pub text: String,
}

#[derive(Debug)]
pub enum GameError {
    Runner(RunnerError),
    InvalidCommand(String),
}

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runner(err) => write!(f, "runner error: {err}"),
            Self::InvalidCommand(msg) => write!(f, "invalid command: {msg}"),
        }
    }
}

impl std::error::Error for GameError {}

impl From<RunnerError> for GameError {
    fn from(value: RunnerError) -> Self {
        Self::Runner(value)
    }
}

pub struct GameSession {
    runner: Runner,
    transcript: Transcript,
}

impl GameSession {
    pub fn load(story_path: &str) -> Result<Self, GameError> {
        let runner = Runner::load_story(story_path)?;
        Ok(Self {
            runner,
            transcript: Transcript::default(),
        })
    }

    pub fn execute(&mut self, command: &str) -> Result<Observation, GameError> {
        let command =
            validate_game_command(command).map_err(|e| GameError::InvalidCommand(e.to_string()))?;
        let raw = self.runner.send_command(&command)?;
        let mut cleaned = clean_output(&raw);
        if should_describe_after_move(&cleaned, &command) {
            let look_raw = self.runner.send_command("look")?;
            cleaned = clean_output(&look_raw);
        }
        self.transcript.add_turn(command.clone(), cleaned.clone());
        Ok(Observation { text: cleaned })
    }

    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }
}

fn is_movement_command(command: &str) -> bool {
    matches!(
        command.trim().to_lowercase().as_str(),
        "n" | "s"
            | "e"
            | "w"
            | "ne"
            | "nw"
            | "se"
            | "sw"
            | "u"
            | "d"
            | "in"
            | "out"
            | "north"
            | "south"
            | "east"
            | "west"
            | "northeast"
            | "northwest"
            | "southeast"
            | "southwest"
            | "up"
            | "down"
    )
}

fn should_describe_after_move(observation: &str, command: &str) -> bool {
    if !is_movement_command(command) {
        return false;
    }
    if observation.trim().is_empty() {
        return true;
    }

    let mut non_empty_lines = observation.lines().filter(|line| !line.trim().is_empty());
    let Some(line) = non_empty_lines.next() else {
        return true;
    };
    if non_empty_lines.next().is_some() {
        return false;
    }

    let line = line.trim();
    !line.ends_with('.') && !line.ends_with('!') && !line.ends_with('?')
}
