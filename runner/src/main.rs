use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use runner::Runner;

fn main() -> Result<()> {
    let story_path = story_path().context("could not find games/advent.z5")?;
    let story_path = story_path
        .to_str()
        .context("story path contains invalid UTF-8")?;

    let mut runner = Runner::load_story(story_path)?;
    let commands = ["look", "inventory", "north", "take lamp", "south"];

    print!("{}", runner.run_script(&commands)?);

    Ok(())
}

fn story_path() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join("games").join("advent.z5"),
        manifest_dir.join("games").join("Advent.z5"),
        manifest_dir.join("..").join("games").join("advent.z5"),
        manifest_dir.join("..").join("games").join("Advent.z5"),
    ];

    candidates.into_iter().find(|path| path.exists())
}
