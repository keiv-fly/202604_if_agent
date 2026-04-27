use crate::llm::schema::AGENT_REPLY_PYDANTIC_MODEL;
use crate::memory::WorldModel;

pub const SYSTEM_PROMPT: &str = r#"You are an autonomous text-adventure agent playing Colossal Cave.

You must solve the given task by reasoning step by step.

Rules:
1. Execute exactly ONE game command per turn.
2. Never output multiple commands.
3. Never invent game results.
4. Use only observations and memory.
5. If uncertain, explore safely.
6. Keep updating the world model.
7. Stop only when the task is clearly complete.
8. Stop immediately if /cancel is requested.
"#;

pub fn build_user_prompt(task: &str, world: &WorldModel) -> String {
    let frontier = world.frontier_summary();
    format!(
        "Task:\n{task}\n\nCompact frontier:\n{frontier}\n\nWorld memory JSON:\n{}\n\nRequired response Pydantic model:\n```python\n{AGENT_REPLY_PYDANTIC_MODEL}\n```\n\nReturn exactly one JSON object that would validate as AgentReply.",
        serde_json::to_string_pretty(world).unwrap_or_else(|_| "{}".to_string())
    )
}
