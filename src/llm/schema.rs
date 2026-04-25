use serde::{Deserialize, Serialize};

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
