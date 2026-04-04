use colored::Colorize;

#[derive(Debug)]
struct Frame {
    index: usize,
    name: String,
    file: Option<String>,
}

pub struct BacktraceProcessor {
    in_backtrace: bool,
    current_frame: Option<Frame>,
    frames: Vec<Frame>,
    /// New Rust panic format (>= 1.73) puts the message on the line *after*
    /// "thread '...' panicked at path:line:col:" — track when that's expected.
    next_is_panic_msg: bool,
    /// Buffers a "failures:" line so we can decide whether to suppress it.
    pending_failures_line: bool,
    /// True while we're inside the summary list (indented test paths).
    in_failures_summary: bool,
    /// True when running under `cargo fuzz` — suppress asan/libFuzzer output.
    is_fuzz: bool,
    /// True while we're inside an asan/libFuzzer backtrace block.
    in_asan_backtrace: bool,
    /// True while suppressing the cargo-fuzz "Stack backtrace:" at the end.
    in_fuzz_stack_backtrace: bool,
    /// Used in fuzz mode to collapse consecutive blank lines into one.
    last_was_blank: bool,
}

impl BacktraceProcessor {
    pub fn new(is_fuzz: bool) -> Self {
        Self {
            in_backtrace: false,
            current_frame: None,
            frames: Vec::new(),
            next_is_panic_msg: false,
            pending_failures_line: false,
            in_failures_summary: false,
            is_fuzz,
            in_asan_backtrace: false,
            in_fuzz_stack_backtrace: false,
            last_was_blank: false,
        }
    }

    pub fn process_line(&mut self, line: &str, is_stderr: bool) {
        // ── Asan / libFuzzer noise suppression ───────────────────────────────
        if self.is_fuzz {
            // cargo-fuzz "Stack backtrace:" at the very end — suppress forever.
            if line.trim() == "Stack backtrace:" {
                self.in_fuzz_stack_backtrace = true;
                return;
            }
            if self.in_fuzz_stack_backtrace {
                return;
            }

            // "artifact_prefix='...'; Test unit written to ..."
            if line.starts_with("artifact_prefix=") {
                return;
            }

            // libFuzzer noise lines
            let trimmed = line.trim();
            if trimmed.starts_with("NOTE: libFuzzer")
                || trimmed.starts_with("Combine libFuzzer")
                || trimmed.starts_with("SUMMARY: libFuzzer")
                || trimmed.starts_with("MS: ")
                || trimmed.starts_with("Base64: ")
                || trimmed == "Base64:"
                || trimmed.starts_with("Minimize test case with:")
                || trimmed.starts_with("cargo fuzz tmin")
                || trimmed.chars().all(|c| c == '─')
            {
                return;
            }

            // Collapse consecutive blank lines into one.
            if trimmed.is_empty() {
                if self.last_was_blank {
                    return;
                }
                self.last_was_blank = true;
            } else {
                self.last_was_blank = false;
            }

            // "==NNN== ERROR: libFuzzer: ..." or "==NNN== ERROR: AddressSanitizer: ..."
            if line.starts_with("==") && line.contains("ERROR:") {
                self.in_asan_backtrace = true;
            }
            if self.in_asan_backtrace {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    self.in_asan_backtrace = false;
                    return;
                }
                if trimmed.starts_with('#')
                    || line.starts_with("==")
                    || line.starts_with("  ")
                    || line.starts_with('\t')
                {
                    return;
                }
                self.in_asan_backtrace = false;
                // Fall through and emit this line normally.
            }
        }

        // ── Failures summary suppression ──────────────────────────────────────
        if self.in_failures_summary {
            // Indented test paths → suppress
            if is_indented_test_path(line) {
                return;
            }
            self.in_failures_summary = false;
            // Fall through to emit whatever ended the summary.
        }

        if self.pending_failures_line {
            self.pending_failures_line = false;
            if is_indented_test_path(line) {
                // The buffered "failures:" was the summary header — suppress it
                // and start suppressing the list.
                self.in_failures_summary = true;
                return;
            }
            // It was the section-header "failures:" — emit it then continue.
            self.emit("failures:", is_stderr);
        }

        if !self.in_backtrace && line.trim() == "failures:" {
            self.pending_failures_line = true;
            return;
        }

        // ── Continuation of new-format panic: message is on its own line ──────
        if self.next_is_panic_msg {
            self.next_is_panic_msg = false;
            println!("{}", line.bright_white().bold());
            return;
        }

        if !self.in_backtrace {
            if line.contains("panicked at") {
                self.handle_panic_line(line);
                return;
            }

            if line.trim() == "stack backtrace:" {
                self.in_backtrace = true;
                return;
            }

            self.emit(line, is_stderr);
            return;
        }

        // ── Inside a backtrace ─────────────────────────────────────────────

        // Frame header: "   N: function::name"
        if let Some((idx, name)) = parse_frame_header(line) {
            if let Some(prev) = self.current_frame.take() {
                self.frames.push(prev);
            }
            self.current_frame = Some(Frame { index: idx, name, file: None });
            return;
        }

        // File line: "             at /path/to/file.rs:line:col"
        if let Some(file) = parse_file_line(line) {
            if let Some(ref mut frame) = self.current_frame {
                frame.file = Some(file);
            }
            return;
        }

        // Anything else ends the backtrace section.
        if let Some(frame) = self.current_frame.take() {
            self.frames.push(frame);
        }
        self.flush_backtrace();
        self.in_backtrace = false;

        if !line.trim().starts_with("note:") {
            self.emit(line, is_stderr);
        }
    }

    /// Call after the child process exits to flush any pending backtrace that
    /// wasn't followed by a "note:" line.
    pub fn flush(&mut self) {
        if self.in_backtrace {
            if let Some(frame) = self.current_frame.take() {
                self.frames.push(frame);
            }
            self.flush_backtrace();
            self.in_backtrace = false;
        }
    }

    fn handle_panic_line(&mut self, line: &str) {
        let thread_name = line
            .find('\'')
            .and_then(|s| line[s + 1..].find('\'').map(|e| &line[s + 1..s + 1 + e]))
            .unwrap_or("?");
        let short_name = thread_name.split("::").last().unwrap_or(thread_name);

        let is_new_format = line.trim_end().ends_with(':');

        let location = line
            .find(" panicked at ")
            .map(|p| line[p + " panicked at ".len()..].trim_end_matches(':').trim())
            .unwrap_or("");

        println!(
            "{} {} {} {}",
            "Test".bright_red().bold(),
            short_name.bright_white().bold(),
            "panicked at".bright_red(),
            location.bright_yellow()
        );

        if is_new_format {
            self.next_is_panic_msg = true;
        }
    }

    fn flush_backtrace(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        let frames = std::mem::take(&mut self.frames);
        let user_frames: Vec<&Frame> = frames.iter().filter(|f| is_user_frame(f)).collect();
        let hidden = frames.len() - user_frames.len();
        print_pretty_backtrace(&user_frames, hidden);
    }

    fn emit(&self, line: &str, is_stderr: bool) {
        let out = colorize_test_line(line);
        if is_stderr {
            eprintln!("{}", out);
        } else {
            println!("{}", out);
        }
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parses "   N: function::name" → (N, "function::name")
fn parse_frame_header(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if trimmed.len() == line.len() {
        return None;
    }
    let sep = trimmed.find(": ")?;
    let idx_str = &trimmed[..sep];
    if idx_str.is_empty() || !idx_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let name = trimmed[sep + 2..].trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((idx_str.parse().ok()?, name))
}

/// Parses "             at /path/to/file.rs:line:col" → "/path/to/file.rs:line:col"
fn parse_file_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("at ")?;
    if rest.is_empty() {
        return None;
    }
    Some(rest.to_string())
}

/// Returns true for lines like "    tests::test_foo" — the failures summary list.
fn is_indented_test_path(line: &str) -> bool {
    if !line.starts_with("    ") || line.trim().is_empty() {
        return false;
    }
    let inner = line.trim();
    inner.contains("::") && inner.chars().all(|c| c.is_alphanumeric() || c == ':' || c == '_')
}

// ── Frame classification ──────────────────────────────────────────────────────

fn is_user_frame(frame: &Frame) -> bool {
    if let Some(ref file) = frame.file {
        !file.contains("/rustc/")
            && !file.contains("/.rustup/toolchains/")
            && !file.contains("/.cargo/registry/")
            && !file.contains("/.cargo/git/")
            && !file.contains("\\rustc\\")
            && !file.contains("\\.rustup\\toolchains\\")
            && !file.contains("\\.cargo\\registry\\")
            && !file.contains("\\.cargo\\git\\")
    } else {
        !is_stdlib_fn(&frame.name)
    }
}

fn is_stdlib_fn(name: &str) -> bool {
    name.starts_with("std::")
        || name.starts_with("core::")
        || name.starts_with("alloc::")
        || name.starts_with("test::")
        || name.starts_with("backtrace::")
        || name.starts_with("panic_unwind::")
        || name == "rust_begin_unwind"
        || name.starts_with("__rust")
        || name.starts_with("_ZN")
        || name == "<unknown>"
}

// ── Output formatting ─────────────────────────────────────────────────────────

const ARROW_DOWN: &[&str] = &[
    "   ||",
    "   ||",
    "   ||",
    "\\______/",
    " \\    /",
    "  \\  /",
    "   \\/",
];

const ARROW_UP: &[&str] = &[
    "   /\\",
    "  /  \\",
    " /    \\",
    "/______\\",
    "   ||",
    "   ||",
    "   ||",
];

fn print_pretty_backtrace(user_frames: &[&Frame], hidden: usize) {
    println!();
    println!(
        "{}",
        "━━━ Your Code ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue()
    );
    println!();

    if user_frames.is_empty() {
        println!("  {}", "(no user-code frames found in backtrace)".bright_black());
    } else {
        println!("  {}", "most recent call".bright_black().italic());
        for line in ARROW_DOWN {
            println!("  {}", line.bright_black());
        }
        for frame in user_frames {
            println!(
                "  {} {}",
                format!("#{}", frame.index).bright_black(),
                frame.name.bright_white()
            );
            if let Some(ref file) = frame.file {
                println!("       {} {}", "at".bright_black(), file.bright_yellow());
            }
        }
        for line in ARROW_UP {
            println!("  {}", line.bright_black());
        }
        println!("  {}", "oldest call".bright_black().italic());
    }

    println!();
    if hidden > 0 {
        println!(
            "  {} {} stdlib/dependency frame{} hidden",
            "···".bright_black(),
            hidden.to_string().yellow(),
            if hidden == 1 { "" } else { "s" }
        );
    }
    println!(
        "{}",
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue()
    );
    println!();
}

/// Re-apply colors that libtest strips when its stdout is a pipe.
fn colorize_test_line(line: &str) -> String {
    if let Some(prefix) = line.strip_suffix(" ... ok") {
        return format!("{} ... {}", prefix, "ok".green());
    }
    if let Some(prefix) = line.strip_suffix(" ... FAILED") {
        return format!("{} ... {}", prefix, "FAILED".bright_red().bold());
    }
    if let Some(prefix) = line.strip_suffix(" ... ignored") {
        return format!("{} ... {}", prefix, "ignored".yellow());
    }
    if let Some(rest) = line.strip_prefix("test result: ok") {
        return format!("test result: {}{}", "ok".green(), rest);
    }
    if let Some(rest) = line.strip_prefix("test result: FAILED") {
        return format!("test result: {}{}", "FAILED".bright_red().bold(), rest);
    }
    line.to_string()
}
