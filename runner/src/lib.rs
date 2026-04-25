mod ffi;

use std::ffi::{CStr, CString, NulError};
use std::fmt;
use std::os::raw::c_char;

use ffi::BocfelHandle;

const INITIAL_OUTPUT_BUFFER_LEN: usize = 16 * 1024;

pub struct Runner {
    raw: *mut BocfelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    NullHandle,
    StoryLoadFailed(String),
    CommandFailed(String),
    OutputTooLarge,
    InvalidUtf8,
    InteriorNul(String),
}

impl Runner {
    pub fn load_story(path: &str) -> Result<Self, RunnerError> {
        let story_path = cstring(path)?;
        let raw = unsafe { ffi::bocfel_create(story_path.as_ptr()) };

        if raw.is_null() {
            return Err(RunnerError::StoryLoadFailed(format!(
                "failed to load story file: {path}"
            )));
        }

        Ok(Self { raw })
    }

    pub fn send_command(&mut self, command: &str) -> Result<String, RunnerError> {
        if self.raw.is_null() {
            return Err(RunnerError::NullHandle);
        }

        let command = cstring(command)?;
        let mut output_buffer = vec![0_u8; INITIAL_OUTPUT_BUFFER_LEN];

        let status = unsafe {
            ffi::bocfel_send_command(
                self.raw,
                command.as_ptr(),
                output_buffer.as_mut_ptr().cast::<c_char>(),
                output_buffer.len() as u32,
            )
        };

        if status == 1 {
            return Err(RunnerError::OutputTooLarge);
        }

        if status != 0 {
            return Err(RunnerError::CommandFailed(self.last_error()));
        }

        c_string_from_buffer(&output_buffer)
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

    fn last_error(&mut self) -> String {
        let error = unsafe { ffi::bocfel_last_error(self.raw) };
        if error.is_null() {
            return "unknown Bocfel error".to_owned();
        }

        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { ffi::bocfel_destroy(self.raw) };
            self.raw = std::ptr::null_mut();
        }
    }
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullHandle => write!(formatter, "Bocfel handle is null"),
            Self::StoryLoadFailed(message) => write!(formatter, "story load failed: {message}"),
            Self::CommandFailed(message) => write!(formatter, "command failed: {message}"),
            Self::OutputTooLarge => write!(formatter, "Bocfel output exceeded the output buffer"),
            Self::InvalidUtf8 => write!(formatter, "Bocfel output was not valid UTF-8"),
            Self::InteriorNul(message) => write!(formatter, "{message}"),
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

fn c_string_from_buffer(buffer: &[u8]) -> Result<String, RunnerError> {
    let len = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());

    String::from_utf8(buffer[..len].to_vec()).map_err(|_| RunnerError::InvalidUtf8)
}
