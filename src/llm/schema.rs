use serde::{Deserialize, Serialize};
use serde_json::json;

pub const AGENT_REPLY_PYDANTIC_MODEL: &str = r#"
class AgentAction(BaseModel):
    type: Literal["game_command"] = Field(description="The kind of action to perform.")
    command: str = Field(description="Exactly one text-adventure command to send to the game.")


class MemoryUpdate(BaseModel):
    location: str = Field(description="Current inferred location, or an empty string if unknown.")
    new_exits: list[str] = Field(description="New exits discovered from the latest observation.")
    new_objects: list[str] = Field(description="New visible or relevant objects discovered.")
    notes: list[str] = Field(description="Concise task or world-model notes to remember.")


class TaskStatus(BaseModel):
    complete: bool = Field(description="Whether the requested task is complete.")
    summary: str = Field(description="Completion summary, or an empty string while incomplete.")


class AgentReply(BaseModel):
    thought: str = Field(description="Brief reasoning for the next action.")
    action: AgentAction
    memory_update: MemoryUpdate
    task_status: TaskStatus"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReply {
    pub thought: String,
    pub action: AgentAction,
    pub memory_update: MemoryUpdate,
    pub task_status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryUpdate {
    pub location: String,
    pub new_exits: Vec<String>,
    pub new_objects: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    pub complete: bool,
    pub summary: String,
}

pub fn agent_reply_json_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "thought": {
                "type": "string",
                "description": "Brief reasoning for the next action."
            },
            "action": {
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "enum": ["game_command"],
                        "description": "The kind of action to perform."
                    },
                    "command": {
                        "type": "string",
                        "description": "Exactly one text-adventure command to send to the game."
                    }
                },
                "required": ["type", "command"],
                "additionalProperties": false
            },
            "memory_update": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "Current inferred location, or an empty string if unknown."
                    },
                    "new_exits": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New exits discovered from the latest observation."
                    },
                    "new_objects": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New visible or relevant objects discovered."
                    },
                    "notes": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Concise task or world-model notes to remember."
                    }
                },
                "required": ["location", "new_exits", "new_objects", "notes"],
                "additionalProperties": false
            },
            "task_status": {
                "type": "object",
                "properties": {
                    "complete": {
                        "type": "boolean",
                        "description": "Whether the requested task is complete."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Completion summary, or an empty string while incomplete."
                    }
                },
                "required": ["complete", "summary"],
                "additionalProperties": false
            }
        },
        "required": ["thought", "action", "memory_update", "task_status"],
        "additionalProperties": false
    })
}

pub fn agent_reply_response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "agent_reply",
            "strict": true,
            "schema": agent_reply_json_schema()
        }
    })
}
