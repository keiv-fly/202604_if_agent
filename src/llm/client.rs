use crate::config::app::LlmConfig;
use crate::llm::prompt::SYSTEM_PROMPT;
use crate::llm::schema::AgentReply;
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::json;

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

    pub fn next_action(&self, user_prompt: &str) -> Result<AgentReply> {
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
            "response_format": { "type": "json_object" }
        });

        let value: serde_json::Value = self
            .client
            .post(&self.config.base_url)
            .bearer_auth(&self.config.api_key)
            .header("HTTP-Referer", "https://local.if-agent")
            .header("X-Title", "if-agent")
            .json(&payload)
            .send()
            .context("failed to call OpenRouter")?
            .error_for_status()
            .context("OpenRouter returned error status")?
            .json()
            .context("failed to decode OpenRouter response")?;

        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("missing content in OpenRouter response"))?;

        parse_reply(content)
    }
}

fn parse_reply(content: &str) -> Result<AgentReply> {
    if let Ok(reply) = serde_json::from_str::<AgentReply>(content) {
        return Ok(reply);
    }

    let start = content
        .find('{')
        .ok_or_else(|| anyhow!("missing JSON object start"))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| anyhow!("missing JSON object end"))?;
    let snippet = &content[start..=end];

    serde_json::from_str(snippet).context("failed to parse agent JSON response")
}
