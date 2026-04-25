## Specs: Rust + embedded Bocfel runner for `advent.z5`

### Goal

Create a Rust program that embeds Bocfel source code and runs a Z-machine story file such as:

```text
games/advent.z5
```

The Rust program must send predefined commands to the game and capture text output.

Example commands:

```text
look
inventory
north
take lamp
south
```

Bocfel is a Z-machine interpreter supporting versions 1–5, 7, and 8, with limited version 6 support. `advent.z5` is a version 5 story file, so it is in Bocfel’s supported range. ([cspiegel.github.io][1])

---

# 1. Project purpose

The program should:

1. Compile Bocfel C/C++ code together with Rust.
2. Load a `.z5` Z-machine story file.
3. Run the interpreter programmatically.
4. Send text commands from Rust into the Bocfel VM.
5. Capture game output text.
6. Print command/output pairs to the terminal.

This is not a full interactive UI. It is a minimal embedded runner.

---

# 2. Important note about Bocfel

Modern Bocfel is C++, not plain C. Bocfel 2.0 was ported from C to C++. ([cspiegel.github.io][2])

So the Rust project should embed Bocfel as C++ code using:

```toml
cc = "1"
bindgen = "0.69"
```

or manually written `extern "C"` FFI bindings.

---

# 3. Suggested directory structure

```text
bocfel-rust-runner/
├── Cargo.toml
├── build.rs
├── games/
│   └── advent.z5
├── bocfel/
│   ├── src/
│   │   └── ... Bocfel source files ...
│   └── include/
│       └── bocfel_embed.h
└── src/
    ├── main.rs
    └── ffi.rs
```

---

# 4. Rust-facing API

Rust should expose a safe wrapper like:

```rust
pub struct BocfelRunner {
    raw: *mut BocfelHandle,
}

impl BocfelRunner {
    pub fn load_story(path: &str) -> Result<Self, BocfelError>;

    pub fn send_command(&mut self, command: &str) -> Result<String, BocfelError>;

    pub fn run_commands(&mut self, commands: &[&str]) -> Result<Vec<CommandResult>, BocfelError>;
}

pub struct CommandResult {
    pub command: String,
    pub output: String,
}
```

Example usage:

```rust
fn main() -> anyhow::Result<()> {
    let mut runner = BocfelRunner::load_story("games/advent.z5")?;

    let commands = [
        "look",
        "inventory",
        "north",
        "take lamp",
        "south",
    ];

    for result in runner.run_commands(&commands)? {
        println!("> {}", result.command);
        println!("{}", result.output);
    }

    Ok(())
}
```

---

# 5. C/C++ embedding layer

Create a small C-compatible wrapper around Bocfel.

File:

```text
bocfel/include/bocfel_embed.h
```

API:

```c
#pragma once

#ifdef __cplusplus
extern "C" {
#endif

typedef struct BocfelHandle BocfelHandle;

BocfelHandle* bocfel_create(const char* story_path);

void bocfel_destroy(BocfelHandle* handle);

int bocfel_send_command(
    BocfelHandle* handle,
    const char* command,
    char* output_buffer,
    unsigned int output_buffer_len
);

const char* bocfel_last_error(BocfelHandle* handle);

#ifdef __cplusplus
}
#endif
```

Implementation file:

```text
bocfel/src/bocfel_embed.cpp
```

Responsibilities:

1. Initialize Bocfel.
2. Load the story file.
3. Provide input lines from Rust.
4. Capture screen/output text into an internal buffer.
5. Return captured text after each command.

---

# 6. Input model

Rust sends a command as a full line:

```text
take lamp
```

The embedding layer should append a newline before passing it to Bocfel:

```text
take lamp\n
```

The C++ side should provide this line whenever Bocfel asks for keyboard input.

---

# 7. Output model

Bocfel normally writes to a Glk or terminal interface.

For embedding, replace or adapt the output backend so that text goes into a buffer:

```cpp
std::string output;
```

Whenever Bocfel prints text, append it:

```cpp
handle->output += text;
```

After each command:

1. Run Bocfel until it requests the next input.
2. Copy the new output into `output_buffer`.
3. Clear or mark the consumed output.

---

# 8. Execution lifecycle

Expected flow:

```text
Rust starts
  ↓
bocfel_create("games/advent.z5")
  ↓
Bocfel loads story
  ↓
Bocfel runs until first input prompt
  ↓
Rust sends "look"
  ↓
Bocfel processes command
  ↓
Bocfel runs until next input prompt
  ↓
Rust receives output
  ↓
Repeat for remaining commands
  ↓
bocfel_destroy()
```

---

# 9. build.rs

`build.rs` should compile the Bocfel wrapper and selected Bocfel source files:

```rust
fn main() {
    cc::Build::new()
        .cpp(true)
        .include("bocfel/include")
        .include("bocfel/src")
        .file("bocfel/src/bocfel_embed.cpp")
        // Add required Bocfel .cpp files here.
        .flag_if_supported("-std=c++17")
        .compile("bocfel_embedded");

    println!("cargo:rerun-if-changed=bocfel/src/bocfel_embed.cpp");
    println!("cargo:rerun-if-changed=bocfel/include/bocfel_embed.h");
}
```

---

# 10. Cargo.toml

```toml
[package]
name = "bocfel-rust-runner"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"

[build-dependencies]
cc = "1"
```

---

# 11. FFI declarations in Rust

```rust
use std::os::raw::{c_char, c_uint};

#[repr(C)]
pub struct BocfelHandle {
    _private: [u8; 0],
}

extern "C" {
    pub fn bocfel_create(story_path: *const c_char) -> *mut BocfelHandle;

    pub fn bocfel_destroy(handle: *mut BocfelHandle);

    pub fn bocfel_send_command(
        handle: *mut BocfelHandle,
        command: *const c_char,
        output_buffer: *mut c_char,
        output_buffer_len: c_uint,
    ) -> i32;

    pub fn bocfel_last_error(handle: *mut BocfelHandle) -> *const c_char;
}
```

---

# 12. Rust safety requirements

The safe Rust wrapper must:

1. Use `CString` for all strings passed into C++.
2. Check for null pointers.
3. Free the Bocfel handle in `Drop`.
4. Convert output buffer bytes back into UTF-8 safely.
5. Return `Result<T, BocfelError>` instead of panicking.

---

# 13. Error handling

Errors should include:

```rust
pub enum BocfelError {
    NullHandle,
    StoryLoadFailed(String),
    CommandFailed(String),
    OutputTooLarge,
    InvalidUtf8,
}
```

The C++ wrapper should store the last error inside `BocfelHandle`.

---

# 14. Initial command test

The first test should run:

```text
look
inventory
north
take lamp
south
```

Expected terminal format:

```text
> look
You are standing at the end of a road before a small brick building...

> inventory
You are empty-handed.

> north
...
```

Exact output depends on the `advent.z5` build.

---

# 15. Non-goals

Do not implement:

1. Full interactive terminal UI.
2. Save/restore files.
3. Sound.
4. Graphics.
5. Blorb resource handling.
6. Multiple simultaneous game sessions.
7. A fallback external `bocfel` process.

Only embedded Bocfel should run the game.

---

# 16. Minimal success criteria

The project is successful when:

```bash
cargo run
```

does all of this:

1. Compiles Rust and embedded Bocfel code.
2. Opens:

```text
games/advent.z5
```

3. Runs the predefined command list.
4. Captures Bocfel output.
5. Prints each command and its resulting game text.

[1]: https://cspiegel.github.io/bocfel/index.html?utm_source=chatgpt.com "Overview - Bocfel"
[2]: https://cspiegel.github.io/bocfel/downloads.html?utm_source=chatgpt.com "Bocfel Downloads"
