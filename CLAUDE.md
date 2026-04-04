# cargo-prettypanic

A cargo subcommand that makes Rust panic backtraces human-readable by filtering out stdlib/dependency frames and showing only user code.

## Project structure

```
prettypanic/src/main.rs         Entry point, arg parsing, spawns cargo subprocess
prettypanic/src/processor.rs    BacktraceProcessor state machine — parses and reformats output
tester/                         Separate crate with intentional panics for local testing
```

## How to test locally

Build and run against the tester crate directly (no install needed):

```sh
cd prettypanic
cargo build
cd ../tester
../prettypanic/target/debug/cargo-prettypanic prettypanic test
```

Or install globally and use as a real cargo subcommand:

```sh
cd prettypanic
cargo install --path .
cd ../tester
cargo prettypanic test
```

## Key design decisions

**Frame filtering** — A frame is "user code" if its source file does NOT contain
any of these path segments: `/rustc/`, `/.rustup/toolchains/`, `/.cargo/registry/`,
`/.cargo/git/` (and their Windows equivalents). When no file path is present,
function-name heuristics are used (`std::`, `core::`, `alloc::`, `test::`, etc.).
Filtering is always on — there is no option to disable it.

**Panic format handling** — Rust 1.73+ changed the panic format so the message
appears on its own line after `"... panicked at path:line:col:"`. The processor
detects this via the trailing `:` and sets `next_is_panic_msg` to grab the next
line.

**Streaming** — stdout and stderr are read on separate threads and multiplexed
via an `mpsc::channel` so the main thread can process lines in arrival order
without cross-stream races.

## Publishing to crates.io

Before publishing, fill in `prettypanic/Cargo.toml`:

```toml
authors = ["Your Name <email@example.com>"]
repository = "https://github.com/yourname/cargo-prettypanic"
```

Then:

```sh
cd prettypanic
cargo publish
```

Users install with `cargo install cargo-prettypanic` and invoke with
`cargo prettypanic test`, `cargo prettypanic run`, etc.
