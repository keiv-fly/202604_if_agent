# DFS Blocked Path Handling Design

Date: 2026-04-27

## Goal

Prevent blocked movement attempts from becoming locations or future frontier entries.

The DFS planner should treat a blocked move as evidence about a missing edge from the current real location. A blocked response must not create a new location key, must not update the current location, and must not generate child frontiers.

## Problem

The current DFS frontier model can mistake parser or movement-failure text for a room. For example, a failed movement can produce:

```text
You can't go that way.
```

If this response is classified as a new location, the planner creates frontiers from a fake location. That pollutes the map and wastes the turn budget exploring impossible paths.

The core invariant should be:

```text
frontier = one plausible untested movement command from one real location
```

A blocked response is not a destination. It is evidence that the attempted edge does not exist.

## Non-Goals

This spec does not add parsing for limited-direction room descriptions such as:

```text
The only exit is to the west.
```

It also does not add puzzle solving, non-movement actions, or game-specific route hints. The scope is only blocked-path detection and frontier hygiene.

## Attempt Context

Before executing a movement command, the planner should persist an attempt record:

```rust
PendingMoveAttempt {
    source_location_key: String,
    command: String,
    previous_observation_signature: String,
    frontier_id: Option<String>,
}
```

The command result must be interpreted relative to this attempt. In particular, `source_location_key` remains the authoritative current location until the observation is classified as a successful move.

## Classification Order

Movement result classification must happen before location creation.

Use this order:

1. explicit blocked or failure response
2. same-location response
3. known-location movement
4. new-location movement

This ordering prevents failure text from being passed into location extraction.

## Explicit Blocked Responses

Maintain a small list of blocked-response patterns. Initial examples:

```text
You can't go that way.
You can't go in that direction.
There is no way
You are unable to
You can't
Nothing happens
```

Pattern matching should be conservative enough to avoid treating rich room descriptions as failures. Prefer exact or near-exact normalized response matching for short parser messages, with substring matching only for well-understood failure clauses.

When an explicit blocked response is detected:

```rust
classification = CommandFailedOrBlocked;
current_location_key = pending_attempt.source_location_key;
```

No location should be created from the response text.

## Same-Location Detection

If the command response resolves to the same observation signature as the previous location, classify it as blocked unless there is explicit evidence that the move intentionally returns to the same room.

Example rule:

```rust
if new_observation_signature == pending_attempt.previous_observation_signature {
    classification = CommandFailedOrBlocked;
}
```

This catches blocked moves that reprint the current room description instead of emitting a direct parser error.

## State Updates For Blocked Moves

When a move is blocked, update only the attempted edge:

```rust
frontier.status = Failed;
frontier.expected_destination = None;
frontier.last_attempted_turn = Some(turn);
frontier.failure_reason = Some(blocked_reason);
current_location_key = pending_attempt.source_location_key;
```

The planner must not:

* create a location from the response
* create frontiers from the response
* update `current_location_key` to the response text
* create a known edge to a blocked phrase

## Agent-Session Logging

Every movement response classification should be written to the agent-session log file. The log entry should be emitted after classification and before frontier or world-state mutation, so debugging can compare the raw observation, the chosen classification, and the resulting state update.

The log entry should include:

* attempted source location key
* movement command
* selected frontier id, if any
* previous observation signature
* new observation signature, if available
* classification result
* blocked reason, if applicable
* whether current location will remain unchanged

Example event shape:

```text
planner_observation_classification:
  source=In Forest
  command=south
  frontier=frontier-14
  classification=CommandFailedOrBlocked
  blocked_reason=same_location_response
  current_location_unchanged=true
```

## Blocked Edge Table

Track blocked movement attempts separately from frontier status:

```rust
BlockedEdge {
    source_location_key: String,
    command: String,
    turn: u64,
    reason: String,
    raw_output_hash: String,
}
```

Recommended lookup shape:

```rust
blocked_edges[source_location_key][command] = blocked_edge;
```

The blocked-edge table is the durable record that a command should not be re-added as a pending frontier for the same source location.

## Frontier Creation Filter

Before adding a frontier, check known edges, blocked edges, and existing frontier records:

```rust
for command in candidate_commands {
    if known_edges.contains(source_location_key, command) {
        continue;
    }

    if blocked_edges.contains(source_location_key, command) {
        continue;
    }

    if frontier_exists(source_location_key, command) {
        continue;
    }

    add_frontier(source_location_key, command);
}
```

This makes blocked-path handling idempotent. Once a source-command pair is known to fail, later world updates must not recreate it as pending.

## Frontier Status Semantics

Use these meanings:

* `Pending`: plausible movement command that has not been attempted from this source
* `Explored`: attempted movement command that produced a real location transition or confirmed known transition
* `Failed`: attempted movement command that was blocked or did not change location

If an attempted command returns failure text, the frontier must be `Failed`, not `Explored`.

If `expected_destination` would be a blocked phrase, the destination must instead be `None`.

## Required Invariants

The implementation should preserve these invariants after every command:

* `current_location_key` is always a real known location.
* Every movement response classification is logged in the agent-session file.
* Blocked parser text is never a location key.
* A blocked source-command pair is never pending.
* A blocked source-command pair is never re-added to the frontier.
* A blocked move never creates child frontiers.
* `Failed` frontiers have no expected destination.

## Suggested Tests

Add focused tests for:

1. A direct failure response marks the attempted frontier `Failed`.
2. A direct failure response does not create a location.
3. A direct failure response does not create child frontiers.
4. A same-location response marks the attempted frontier `Failed`.
5. A blocked edge is not re-added during later frontier generation.
6. The current location remains the attempted source after a blocked move.
7. A successful move still creates or resolves the destination location normally.
8. Each movement response classification is emitted to the agent-session log.

## Acceptance Criteria

The blocked-path behavior is complete when:

* failure text such as `You can't go that way.` never appears as a `source_location_key`
* failed movement commands are represented as `Failed` frontiers and blocked edges
* each movement response classification is visible in the agent-session log
* frontier generation skips previously blocked source-command pairs
* blocked responses do not inflate the location count or frontier count
* successful movement behavior is unchanged
