use crate::config::app::LlmConfig;
use crate::llm::prompt::SYSTEM_PROMPT;
use crate::llm::schema::{AgentReply, agent_reply_response_format};
use crate::logging::SessionLogger;
use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde_json::json;
use std::error::Error;
use std::fmt;
use std::thread;
use std::time::Duration;

const LLM_MAX_RETRIES: usize = 3;

#[derive(Clone)]
pub struct LlmClient {
    config: LlmConfig,
    client: Client,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        let client = Client::builder()
            .user_agent("if-agent/0.2.1")
            .timeout(Duration::from_secs(150))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { config, client }
    }

    pub fn disabled(&self) -> bool {
        self.config.api_key.trim().is_empty()
    }

    pub fn next_action(&self, user_prompt: &str, logger: &SessionLogger) -> Result<AgentReply> {
        if self.disabled() {
            return Err(anyhow!("OPENROUTER_API_KEY is not set"));
        }

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

        for attempt in 1..=LLM_MAX_RETRIES + 1 {
            let call_number = logger.next_llm_call_number();
            logger.write_llm_artifact(call_number, "prompt.txt", user_prompt);
            logger.write_llm_json_artifact(call_number, "request.json", &payload);

            match self.next_action_attempt(user_prompt, logger, call_number, &payload) {
                Ok(reply) => return Ok(reply),
                Err(err) => {
                    let err_with_chain = format_error_chain(&err);
                    logger.write_llm_artifact(call_number, "error.txt", &err_with_chain);
                    logger.log(
                        "llm_call",
                        &compact_error_summary(call_number, user_prompt, logger, &err_with_chain),
                    );

                    if attempt <= LLM_MAX_RETRIES {
                        logger.log(
                            "llm_retry",
                            &compact_retry_summary(call_number, attempt, &err_with_chain),
                        );
                        thread::sleep(Duration::from_secs(attempt as u64));
                        continue;
                    }

                    return Err(err);
                }
            }
        }

        unreachable!("retry loop should return after success or final error")
    }

    fn next_action_attempt(
        &self,
        user_prompt: &str,
        logger: &SessionLogger,
        call_number: usize,
        payload: &serde_json::Value,
    ) -> Result<AgentReply> {
        let response_result: Result<serde_json::Value> = (|| {
            let response = self
                .client
                .post(&self.config.base_url)
                .bearer_auth(self.config.api_key.trim())
                .header("HTTP-Referer", "https://local.if-agent")
                .header("X-Title", "if-agent")
                .json(&payload)
                .send()
                .context("failed to call OpenRouter")?;
            let status = response.status();
            let response_text = response
                .text()
                .context("failed to read OpenRouter response body")?;

            if !status.is_success() {
                logger.write_llm_artifact(call_number, "response.txt", &response_text);
                return Err(anyhow!("OpenRouter returned error status {status}"));
            }

            let value = match serde_json::from_str::<serde_json::Value>(&response_text) {
                Ok(value) => value,
                Err(err) => {
                    logger.write_llm_artifact(call_number, "response.txt", &response_text);
                    return Err(err).context("failed to decode OpenRouter response");
                }
            };
            logger.write_llm_json_artifact(call_number, "response.json", &value);
            Ok(value)
        })();
        let value: serde_json::Value = response_result?;
        let content = match value["choices"][0]["message"]["content"].as_str() {
            Some(content) => content,
            None => {
                return Err(anyhow!("missing content in OpenRouter response"));
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

fn compact_retry_summary(call_number: usize, attempt: usize, error: &str) -> String {
    format!("#{call_number:03} retry={attempt}/{LLM_MAX_RETRIES} retrying_after_error={error:?}")
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut output = String::new();

    for (index, cause) in err.chain().enumerate() {
        if index > 0 {
            output.push_str(" | caused by: ");
        }
        output.push_str(&cause.to_string());
    }

    output
}
