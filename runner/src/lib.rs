mod ffi;

use std::env;
use std::ffi::{CString, NulError};
use std::fmt;
use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const CHILD_MODE_ARG: &str = "--runner-persistent-child";
const CHILD_EXE_ENV: &str = "RUNNER_CHILD_EXE";

pub struct Runner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pending_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    StoryLoadFailed(String),
    CommandFailed(String),
    ChildExited(String),
    Io(String),
    InvalidUtf8,
    InteriorNul(String),
    MissingChildPipe(&'static str),
    Protocol(String),
}

impl Runner {
    pub fn load_story(path: &str) -> Result<Self, RunnerError> {
        cstring(path)?;

        let child_exe = match env::var_os(CHILD_EXE_ENV) {
            Some(path) => path.into(),
            None => env::current_exe()
                .map_err(|err| RunnerError::Io(format!("failed to locate current exe: {err}")))?,
        };

        let mut child = Command::new(child_exe)
            .arg(CHILD_MODE_ARG)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| RunnerError::StoryLoadFailed(format!("failed to start Bocfel: {err}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or(RunnerError::MissingChildPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(RunnerError::MissingChildPipe("stdout"))?;

        let mut runner = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            pending_output: String::new(),
        };
        runner.pending_output = runner.read_until_prompt()?;
        Ok(runner)
    }

    pub fn send_command(&mut self, command: &str) -> Result<String, RunnerError> {
        cstring(command)?;
        self.stdin
            .write_all(command.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|err| RunnerError::Io(format!("failed to send command to Bocfel: {err}")))?;

        let mut output = String::new();
        if !self.pending_output.is_empty() {
            output.push_str(&std::mem::take(&mut self.pending_output));
        }
        output.push_str(&self.read_until_prompt()?);
        Ok(output)
    }

    pub fn run_commands(&mut self, commands: &[&str]) -> Result<Vec<CommandResult>, RunnerError> {
        commands
            .iter()
            .map(|command| {
                let output = self.send_command(command)?;
                Ok(CommandResult {
                    command: (*command).to_owned(),
                    output,
                })
            })
            .collect()
    }

    pub fn run_script(&mut self, commands: &[&str]) -> Result<String, RunnerError> {
        let mut output = String::new();
        for command in commands {
            output.push_str(&self.send_command(command)?);
        }

        Ok(output)
    }

    fn read_until_prompt(&mut self) -> Result<String, RunnerError> {
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];

        loop {
            let count = self
                .stdout
                .read(&mut byte)
                .map_err(|err| RunnerError::Io(format!("failed to read Bocfel output: {err}")))?;
            if count == 0 {
                if bytes.is_empty() {
                    return Err(RunnerError::ChildExited("Bocfel exited without output".to_string()));
                }
                break;
            }

            bytes.push(byte[0]);
            if ends_with_prompt(&bytes) {
                break;
            }
        }

        String::from_utf8(bytes).map_err(|_| RunnerError::InvalidUtf8)
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoryLoadFailed(message) => write!(formatter, "story load failed: {message}"),
            Self::CommandFailed(message) => write!(formatter, "command failed: {message}"),
            Self::ChildExited(message) => write!(formatter, "Bocfel child exited: {message}"),
            Self::Io(message) => write!(formatter, "{message}"),
            Self::InvalidUtf8 => write!(formatter, "Bocfel output was not valid UTF-8"),
            Self::InteriorNul(message) => write!(formatter, "{message}"),
            Self::MissingChildPipe(pipe) => write!(formatter, "Bocfel child missing {pipe} pipe"),
            Self::Protocol(message) => write!(formatter, "Bocfel protocol error: {message}"),
        }
    }
}

impl std::error::Error for RunnerError {}

fn cstring(value: &str) -> Result<CString, RunnerError> {
    CString::new(value).map_err(interior_nul_error)
}

fn interior_nul_error(error: NulError) -> RunnerError {
    RunnerError::InteriorNul(format!(
        "string contains an interior NUL byte at position {}",
        error.nul_position()
    ))
}

fn ends_with_prompt(bytes: &[u8]) -> bool {
    let mut index = bytes.len();
    while index > 0 && matches!(bytes[index - 1], b' ' | b'\t') {
        index -= 1;
    }

    if index == 0 || bytes[index - 1] != b'>' {
        return false;
    }

    index == 1 || matches!(bytes[index - 2], b'\n' | b'\r')
}

pub fn run_persistent_child_from_args() -> Result<bool, RunnerError> {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        return Ok(false);
    };
    if first != CHILD_MODE_ARG {
        return Ok(false);
    }

    let story_path = args
        .next()
        .ok_or_else(|| RunnerError::Protocol(format!("{CHILD_MODE_ARG} requires a story path")))?;
    run_embedded_interactive(&story_path)?;
    Ok(true)
}

fn run_embedded_interactive(story_path: &str) -> Result<(), RunnerError> {
    let story_path = cstring(story_path)?;
    let status = unsafe { ffi::bocfel_run_interactive(story_path.as_ptr()) };
    if status != 0 {
        return Err(RunnerError::CommandFailed(format!(
            "Bocfel exited with status {status}"
        )));
    }

    Ok(())
}
