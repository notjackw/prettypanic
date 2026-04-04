mod processor;

use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use colored::Colorize;
use processor::BacktraceProcessor;

fn main() {
    let args: Vec<String> = env::args().collect();

    // When invoked as `cargo prettypanic <cmd>`, cargo passes:
    //   args[0] = "cargo-prettypanic"
    //   args[1] = "prettypanic"
    //   args[2..] = user's args
    let user_args: &[String] = if args.get(1).map(String::as_str) == Some("prettypanic") {
        &args[2..]
    } else {
        &args[1..]
    };

    let mut cargo_args: Vec<String> = Vec::new();

    for arg in user_args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => cargo_args.push(arg.clone()),
        }
    }

    if cargo_args.is_empty() {
        print_help();
        std::process::exit(1);
    }

    std::process::exit(run_cargo(cargo_args));
}

fn print_help() {
    println!("{}", "cargo-prettypanic".bright_cyan().bold());
    println!("Makes panic backtraces readable by showing only your code.\n");
    println!(
        "{}  cargo prettypanic <subcommand> [cargo-args...]",
        "USAGE:".bold()
    );
    println!();
    println!("{}  cargo prettypanic test", "EXAMPLES:".bold());
    println!("    cargo prettypanic test my_test_fn");
    println!("    cargo prettypanic run");
}

fn run_cargo(cargo_args: Vec<String>) -> i32 {
    // `+nightly` / `+stable` / `+<toolchain>` must be the very first argument
    // passed to cargo; rustup intercepts it before cargo sees anything else.
    // Strip it from cargo_args and prepend it directly to the command.
    let (toolchain, cargo_args): (Option<String>, Vec<String>) =
        if cargo_args.first().map(|s| s.starts_with('+')).unwrap_or(false) {
            let mut it = cargo_args.into_iter();
            (it.next(), it.collect())
        } else {
            (None, cargo_args)
        };

    let mut cmd = Command::new("cargo");
    if let Some(tc) = toolchain {
        cmd.arg(tc);
    }
    cmd.arg("--color").arg("always");
    cmd.env("RUST_BACKTRACE", "1");

    // Detect the subcommand (skipping any leading flags like --color).
    let subcommand = cargo_args.iter().find(|a| !a.starts_with('-')).map(String::as_str);
    let is_fuzz = subcommand == Some("fuzz");

    // For `cargo test`, also pass --color always to the test harness (the
    // binary that actually prints "ok" / "FAILED"). It goes after `--`.
    let is_test = subcommand == Some("test");
    if is_test {
        if let Some(dash_pos) = cargo_args.iter().position(|a| a == "--") {
            // User already has `--`; inject our flag right after it.
            cmd.args(&cargo_args[..=dash_pos]);
            cmd.arg("--color").arg("always");
            cmd.args(&cargo_args[dash_pos + 1..]);
        } else {
            cmd.args(&cargo_args);
            cmd.arg("--").arg("--color").arg("always");
        }
    } else {
        cmd.args(&cargo_args);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: failed to spawn cargo: {}", "error".red().bold(), e);
            return 1;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Multiplex stdout and stderr through a single channel so we can process
    // them in order on the main thread without races between the two streams.
    let (tx, rx) = mpsc::channel::<(bool, String)>();

    let tx_out = tx.clone();
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            if tx_out.send((false, line)).is_err() {
                break;
            }
        }
    });

    let tx_err = tx;
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().flatten() {
            if tx_err.send((true, line)).is_err() {
                break;
            }
        }
    });

    let mut out_proc = BacktraceProcessor::new(is_fuzz);
    let mut err_proc = BacktraceProcessor::new(is_fuzz);

    for (is_stderr, line) in rx {
        if is_stderr {
            err_proc.process_line(&line, true);
        } else {
            out_proc.process_line(&line, false);
        }
    }

    // Flush any backtrace that ended without a trailing "note:" line
    out_proc.flush();
    err_proc.flush();

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    child.wait().ok().and_then(|s| s.code()).unwrap_or(1)
}
