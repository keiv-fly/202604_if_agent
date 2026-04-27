use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;

const DEFAULT_WORLD_STATE_PATH: &str = "memory_store/world-state.json";
const DEFAULT_FIRST_NODES_PATH: &str = "eval_data/first_nodes.json";

#[derive(Debug, Deserialize)]
struct WorldState {
    locations: HashMap<String, WorldLocation>,
}

#[derive(Debug, Deserialize)]
struct WorldLocation {
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FirstNode {
    title: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct EvalOutput {
    share_of_titles_found: f64,
    share_of_titles_and_descriptions: f64,
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let world_state_path = args
        .next()
        .unwrap_or_else(|| DEFAULT_WORLD_STATE_PATH.to_string());
    let first_nodes_path = args
        .next()
        .unwrap_or_else(|| DEFAULT_FIRST_NODES_PATH.to_string());

    let world_state = read_json::<WorldState>(&world_state_path)?;
    let first_nodes = read_json::<HashMap<String, FirstNode>>(&first_nodes_path)?;

    let world_titles = world_state
        .locations
        .values()
        .filter_map(|location| location.title.clone())
        .collect::<Vec<_>>();
    let expected_titles = first_nodes
        .values()
        .map(|node| node.title.clone())
        .collect::<Vec<_>>();

    let world_title_descriptions = world_state
        .locations
        .values()
        .filter_map(|location| Some((location.title.clone()?, location.description.clone()?)))
        .collect::<Vec<_>>();
    let expected_title_descriptions = first_nodes
        .values()
        .map(|node| (node.title.clone(), node.description.clone()))
        .collect::<Vec<_>>();

    let output = EvalOutput {
        share_of_titles_found: multiset_jaccard(&world_titles, &expected_titles),
        share_of_titles_and_descriptions: multiset_jaccard(
            &world_title_descriptions,
            &expected_title_descriptions,
        ),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

fn read_json<T>(path: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let contents = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&contents).with_context(|| format!("failed to parse {path}"))
}

fn multiset_jaccard<T>(left: &[T], right: &[T]) -> f64
where
    T: Eq + std::hash::Hash + Clone,
{
    let left_counts = counts(left);
    let right_counts = counts(right);

    let mut intersection = 0usize;
    let mut union = 0usize;

    for (item, left_count) in &left_counts {
        let right_count = right_counts.get(item).copied().unwrap_or(0);
        intersection += (*left_count).min(right_count);
        union += (*left_count).max(right_count);
    }

    for (item, right_count) in &right_counts {
        if !left_counts.contains_key(item) {
            union += right_count;
        }
    }

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

fn counts<T>(items: &[T]) -> HashMap<T, usize>
where
    T: Eq + std::hash::Hash + Clone,
{
    let mut counts = HashMap::new();
    for item in items {
        *counts.entry(item.clone()).or_insert(0) += 1;
    }
    counts
}
