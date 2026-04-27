# Eval Binary

The `eval` binary runs the agent against the game and scores the discovered world map against `eval_data/first_nodes.json`.

By default it runs the agent three times with this command:

```text
Explore to create a full map. Only use the actions to move. Do not do any other actions that are not moves.
```

Each run prints a live transcript:

```text
game><z-machine output>
agent><agent input>
```

At the end, it prints JSON with the per-run scores and averages:

```json
{
  "command": "Explore to create a full map. Only use the actions to move. Do not do any other actions that are not moves.",
  "requested_runs": 3,
  "calculation_only": false,
  "runs": [
    {
      "run": 1,
      "share_of_titles_found": 0.5,
      "share_of_titles_found_info": "6/12, 10, 8",
      "share_of_titles_and_descriptions": 0.0,
      "share_of_titles_and_descriptions_info": "0/18, 10, 8",
      "share_of_exits": 0.0,
      "share_of_exits_info": "0/28, 0, 28"
    }
  ],
  "average_share_of_titles_found": 0.5,
  "average_share_of_titles_and_descriptions": 0.0,
  "average_share_of_exits": 0.0
}
```

For `share_of_titles_found`, duplicate titles are treated as separate node occurrences. For example, two nodes named `In Forest` are scored as two distinct title entries.

For `share_of_exits`, each item is one source location, one normalized direction, and one destination location from that direction. Multiple possible destinations under one direction count as separate items, so `forest_deep_valley` contributes 4 exit items and `hill_in_road` contributes 5.

The `*_info` fields are formatted as `<jaccard numerator>/<jaccard denominator>, <actual discovered item count>, <ground truth item count from first_nodes.json>`.

## Run Eval

Run with the defaults:

```powershell
cargo run --bin eval
```

Run a specific number of times:

```powershell
cargo run --bin eval -- --runs 5
```

or:

```powershell
cargo run --bin eval -- -r 5
```

Use a different agent command:

```powershell
cargo run --bin eval -- --command "explore the cave thoroughly"
```

## Calculate Only

Use `--calculate-only` to score an existing world-state file without running the agent or z-machine:

```powershell
cargo run --bin eval -- --calculate-only
```

By default this reads:

```text
memory_store/world-state.json
```

Override the world-state path:

```powershell
cargo run --bin eval -- --calculate-only --world-state memory_store/world-state.json
```

## Options

```text
Usage: cargo run --bin eval -- [OPTIONS] [world-state] [first-nodes]

Options:
  -r, --runs <N>              Number of program runs to execute. Default: 3.
      --calculate-only        Read the world-state file and calculate scores without running.
      --no-run                Alias for --calculate-only.
      --world-state <PATH>    World-state path for --calculate-only. Default: memory_store/world-state.json.
      --first-nodes <PATH>    Expected first-nodes path. Default: eval_data/first_nodes.json.
      --command <TEXT>        Agent command/prompt to run. Default: "Explore to create a full map. Only use the actions to move. Do not do any other actions that are not moves.".
      --prompt <TEXT>         Alias for --command.
  -h, --help                  Show help.
```

You can also provide the world-state and first-nodes paths as positional arguments:

```powershell
cargo run --bin eval -- memory_store/world-state.json eval_data/first_nodes.json
```

The positional world-state path is only used when `--calculate-only` is enabled.
