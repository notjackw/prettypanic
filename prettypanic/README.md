# cargo-prettypanic

A cargo subcommand that makes Rust panic backtraces human-readable by showing only **your code**.

Instead of this:

```
thread 'tests::test_divide' panicked at 'division by zero!', src/main.rs:7:9
stack backtrace:
   0: rust_begin_unwind
             at /rustc/.../library/std/src/panicking.rs:608:5
   1: core::panicking::panic_fmt
             at /rustc/.../library/core/src/panicking.rs:67:14
   2: tester::divide
             at ./src/main.rs:7:9
   3: tester::tests::test_divide
             at ./src/main.rs:27:17
   4: tester::tests::test_divide::{{closure}}
             at ./src/main.rs:26:29
   5: core::ops::function::FnOnce::call_once
   ...16 more stdlib frames
```

You get this:

```
Test test_divide panicked at src/main.rs:7:9
division by zero!

━━━ Your Code ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  most recent call
     ||
     ||
     ||
  \______/
   \    /
    \  /
     \/
  #2 tester::divide
       at ./src/main.rs:7:9
  #3 tester::tests::test_divide
       at ./src/main.rs:27:17
  #4 tester::tests::test_divide::{{closure}}
       at ./src/main.rs:26:29
     /\
    /  \
   /    \
  /______\
     ||
     ||
     ||
  oldest call

  ··· 14 stdlib/dependency frames hidden
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Local testing

Clone the repo and build it, then invoke the binary directly with `prettypanic` as the first argument (this is what cargo normally passes when using it as a subcommand):

```sh
git clone https://github.com/YOUR_USERNAME/cargo-prettypanic
cd cargo-prettypanic
cargo build

# In your project:
/path/to/cargo-prettypanic/target/debug/cargo-prettypanic prettypanic test
```

Or point it at any cargo project you like — just replace the path:

```sh
cd /your/rust/project
/path/to/cargo-prettypanic/target/debug/cargo-prettypanic prettypanic test
```

## Installation

```sh
cargo install cargo-prettypanic
```

## Usage

Use it exactly like `cargo`, just replace `cargo` with `cargo prettypanic`:

```sh
cargo prettypanic test
cargo prettypanic test my_test_name
cargo prettypanic run
cargo prettypanic +nightly fuzz run fuzz_target_1
```

## How it works

- Runs your cargo command with `RUST_BACKTRACE=1` automatically
- Filters out frames from `/rustc/`, `~/.rustup/toolchains/`, `~/.cargo/registry/`, and `~/.cargo/git/`
- Falls back to function-name heuristics (`std::`, `core::`, `alloc::`, etc.) for frames without file paths
- Re-applies colors that cargo strips when output is piped
- Suppresses asan/libFuzzer noise when running `cargo fuzz`

## License

MIT
