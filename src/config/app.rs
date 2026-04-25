use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub game: GameConfig,
    pub llm: LlmConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone)]
pub struct GameConfig {
    pub story_path: String,
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct UiConfig {
    pub max_history_lines: usize,
    pub input_history_limit: usize,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let story_path =
            env::var("IF_AGENT_STORY_PATH").unwrap_or_else(|_| "games/Advent.z5".to_string());
        let api_key = env::var("OPENROUTER_API_KEY").unwrap_or_default();
        let base_url = env::var("OPENROUTER_BASE_URL")
            .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
        let model = env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "openai/gpt-5.4".to_string());
        let temperature = env::var("OPENROUTER_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.2);
        let max_tokens = env::var("OPENROUTER_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200);

        Self {
            game: GameConfig { story_path },
            llm: LlmConfig {
                api_key,
                base_url,
                model,
                temperature,
                max_tokens,
            },
            ui: UiConfig {
                max_history_lines: 2_000,
                input_history_limit: 200,
            },
        }
    }
}
