# runner

Rust crate for a minimal embedded Bocfel runner. The crate exposes a library API
and a small binary that loads the Adventure story file and sends a fixed command
list to the runner.

## Layout

```text
runner/
├── Cargo.toml
├── build.rs
├── bocfel/
│   ├── include/bocfel_embed.h
│   └── src/bocfel_embed.cpp
└── src/
    ├── lib.rs
    ├── ffi.rs
    └── main.rs
```

`src/lib.rs` provides the `Runner`, `RunnerError`, and `CommandResult` types.
`src/main.rs` is the executable entry point.

## Build

From this directory:

```powershell
cargo check
```

During compilation, `build.rs` downloads Bocfel 2.5 from the official Bocfel
site, extracts it under Cargo's output directory, and compiles Bocfel's C++
sources together with the local C-compatible embedding wrapper.

For offline or pinned local builds, set one of these environment variables:

```powershell
$env:RUNNER_BOCFEL_TARBALL = "C:\path\to\bocfel-2.5.tar.gz"
cargo check
```

or:

```powershell
$env:RUNNER_BOCFEL_SOURCE_DIR = "C:\path\to\bocfel-2.5"
cargo check
```

## Run

From this directory:

```powershell
cargo run
```

The binary looks for the story file in these locations:

```text
runner/games/advent.z5
runner/games/Advent.z5
../games/advent.z5
../games/Advent.z5
```

It then sends these commands:

```text
look
inventory
north
take lamp
south
```

Current status: `cargo run` downloads and compiles Bocfel, loads the Adventure
story file, feeds the predefined command list into the embedded Bocfel CLI loop,
and prints the captured transcript.

## Test

Run the Rust test suite:

```powershell
cargo test
```

Run formatting verification:

```powershell
cargo fmt --check
```

Run a compile-only verification:

```powershell
cargo check
```

