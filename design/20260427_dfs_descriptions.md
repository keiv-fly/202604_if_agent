# DFS location descriptions

## Problem

The DFS eval can find every expected room title while still scoring poorly on `share_of_titles_and_descriptions`.

In the observed run, title coverage was complete:

```text
share_of_titles_found_info: 8/8, 8, 8
```

but title-description coverage was low:

```text
share_of_titles_and_descriptions_info: 2/14, 8, 8
```

The traversal is therefore not the primary issue. The issue is how room descriptions are captured and retained.

Adventure often prints a full room description on the first visit, but later revisits may print only the title plus visible objects. For example, `Inside Building` is first seen with:

```text
Inside Building
You are inside a building, a well house for a large spring.
```

Later, the same location can be printed as:

```text
Inside Building

There are some keys on the ground here.

There is tasty food here.
```

If the world model overwrites the original description with the later object-only output, the saved `(title, description)` pair no longer matches the expected eval node.

## Decision

When recording a location description for DFS world-state and eval scoring, use the first non-empty line after the title from a `look` command.

That line is the canonical location description for the current location. Object listings, blank lines, parser messages, and later abbreviated revisit output should not replace it.

Examples:

```text
At End Of Road
You are standing at the end of a road before a small brick building. Around you is a forest. A small stream flows out of the building and down a gully.
```

Canonical description:

```text
You are standing at the end of a road before a small brick building. Around you is a forest. A small stream flows out of the building and down a gully.
```

```text
Inside Building
You are inside a building, a well house for a large spring.

There are some keys on the ground here.
```

Canonical description:

```text
You are inside a building, a well house for a large spring.
```

## Implementation outline

1. After DFS moves to a newly discovered location, issue `look` to get a stable room rendering.
2. Parse the `look` output as:
   - title: first recognized location-title line
   - description: first non-empty line after the title
3. Store that description only if it is non-empty.
4. Do not overwrite an existing non-empty description with output that lacks a room-description line.
5. Keep object listings separate from `Location.description`.

This means DFS still only uses movement commands for exploration decisions, but it may use `look` as a read-only observation step for canonicalizing the map state.

## Expected effect

The run already discovers all expected titles. Stabilizing descriptions from `look` should make discovered `(title, description)` pairs match the expected first-node descriptions much more reliably and should raise `share_of_titles_and_descriptions` without changing the core DFS frontier traversal.
