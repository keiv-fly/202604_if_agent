use crate::config::app::LlmConfig;
use crate::llm::prompt::SYSTEM_PROMPT;
use crate::llm::schema::{AgentReply, agent_reply_response_format};
use crate::logging::SessionLogger;
use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde_json::json;
use std::error::Error;
use std::fmt;

#[derive(Clone)]
pub struct LlmClient {
    config: LlmConfig,
    client: Client,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn disabled(&self) -> bool {
        self.config.api_key.trim().is_empty()
    }

    pub fn next_action(&self, user_prompt: &str, logger: &SessionLogger) -> Result<AgentReply> {
        if self.disabled() {
            return Err(anyhow!("OPENROUTER_API_KEY is not set"));
        }

        let call_number = logger.next_llm_call_number();
        let payload = json!({
            "model": self.config.model,
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": user_prompt}
            ],
            "response_format": agent_reply_response_format()
        });
        logger.write_llm_artifact(call_number, "prompt.txt", user_prompt);
        logger.write_llm_json_artifact(call_number, "request.json", &payload);

        let response_result: Result<serde_json::Value> = (|| {
            let response = self
                .client
                .post(&self.config.base_url)
                .bearer_auth(&self.config.api_key)
                .header("HTTP-Referer", "https://local.if-agent")
                .header("X-Title", "if-agent")
                .json(&payload)
                .send()
                .context("failed to call OpenRouter")?;
            let response = response
                .error_for_status()
                .context("OpenRouter returned error status")?;

            response
                .json()
                .context("failed to decode OpenRouter response")
        })();
        let value: serde_json::Value = match response_result {
            Ok(value) => value,
            Err(err) => {
                logger.write_llm_artifact(call_number, "error.txt", &err.to_string());
                logger.log(
                    "llm_call",
                    &compact_error_summary(call_number, user_prompt, logger, &err.to_string()),
                );
                return Err(err);
            }
        };
        logger.write_llm_json_artifact(call_number, "response.json", &value);

        let content = match value["choices"][0]["message"]["content"].as_str() {
            Some(content) => content,
            None => {
                let err = anyhow!("missing content in OpenRouter response");
                logger.write_llm_artifact(call_number, "error.txt", &err.to_string());
                logger.log(
                    "llm_call",
                    &compact_error_summary(call_number, user_prompt, logger, &err.to_string()),
                );
                return Err(err);
            }
        };
        if let Ok(answer_json) = extract_reply_json(content) {
            logger.write_llm_json_artifact(call_number, "answer.json", &answer_json);
        } else {
            logger.write_llm_artifact(call_number, "answer.txt", content);
        }

        let reply = match parse_reply(content) {
            Ok(reply) => reply,
            Err(err) => {
                logger.write_llm_artifact(call_number, "error.txt", &err.to_string());
                logger.log(
                    "llm_call",
                    &compact_error_summary(call_number, user_prompt, logger, &err.to_string()),
                );
                return Err(err);
            }
        };
        logger.log(
            "llm_call",
            &compact_call_summary(call_number, user_prompt, content, &value, &reply, logger),
        );
        Ok(reply)
    }
}

#[derive(Debug)]
pub struct LlmResponseParseError {
    raw_response: String,
    source: Option<serde_json::Error>,
}

impl LlmResponseParseError {
    fn new(raw_response: &str, source: Option<serde_json::Error>) -> Self {
        Self {
            raw_response: raw_response.to_string(),
            source,
        }
    }

    pub fn raw_response(&self) -> &str {
        &self.raw_response
    }
}

impl fmt::Display for LlmResponseParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to parse agent JSON response")
    }
}

impl Error for LlmResponseParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

fn parse_reply(content: &str) -> Result<AgentReply> {
    let value = extract_reply_json(content)?;

    serde_json::from_value(value)
        .map_err(|source| LlmResponseParseError::new(content, Some(source)).into())
}

fn extract_reply_json(content: &str) -> Result<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        return Ok(value);
    }

    let start = content
        .find('{')
        .ok_or_else(|| LlmResponseParseError::new(content, None))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| LlmResponseParseError::new(content, None))?;
    let snippet = &content[start..=end];

    serde_json::from_str(snippet)
        .map_err(|source| LlmResponseParseError::new(content, Some(source)).into())
}

fn compact_call_summary(
    call_number: usize,
    user_prompt: &str,
    content: &str,
    response: &serde_json::Value,
    reply: &AgentReply,
    logger: &SessionLogger,
) -> String {
    let total_tokens = response["usage"]["total_tokens"]
        .as_u64()
        .map(|tokens| tokens.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let command = reply.action.command.replace('\n', " ");

    format!(
        "#{call_number:03} status=ok prompt_chars={} answer_chars={} tokens={} complete={} action={} command={:?} files={}/{call_number:03}-*",
        user_prompt.chars().count(),
        content.chars().count(),
        total_tokens,
        reply.task_status.complete,
        reply.action.action_type,
        command,
        logger.llm_dir().display(),
    )
}

fn compact_error_summary(
    call_number: usize,
    user_prompt: &str,
    logger: &SessionLogger,
    error: &str,
) -> String {
    format!(
        "#{call_number:03} status=error prompt_chars={} error={:?} files={}/{call_number:03}-*",
        user_prompt.chars().count(),
        error,
        logger.llm_dir().display(),
    )
}
