mod agent;
mod config;
mod game;
mod llm;
mod logging;
mod memory;
mod tui;

use crate::config::AppConfig;
use crate::game::GameSession;
use crate::llm::LlmClient;
use crate::logging::SessionLogger;
use crate::memory::WorldModel;

fn main() -> anyhow::Result<()> {
    if runner::run_persistent_child_from_args()? {
        return Ok(());
    }

    dotenvy::dotenv().ok();

    let config = AppConfig::from_env();
    let logger = SessionLogger::new()?;

    let game = GameSession::load(&config.game.story_path)
        .map_err(|e| anyhow::anyhow!("Failed to load story '{}': {e}", config.game.story_path))?;

    let world = WorldModel::default();
    let llm = LlmClient::new(config.llm.clone());

    let (transcript, memory) = tui::run_tui(config, game, world, llm, logger.clone())?;

    let transcript_path = transcript.save_to_disk()?;
    let memory_path = memory.save_to_disk()?;

    logger.log(
        "shutdown",
        &format!("transcript={:?}, memory={:?}", transcript_path, memory_path),
    );
    println!("Session saved: {:?}", transcript_path);
    println!("World saved: {:?}", memory_path);
    println!("Log saved: {:?}", logger.path());

    Ok(())
}
