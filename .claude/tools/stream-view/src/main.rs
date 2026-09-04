//! Renders a `claude --print --output-format stream-json --verbose` JSONL
//! stream as readable text on stdout, for outcome-loop.sh (docs/outcome_loop.md).
//!
//! Tool-call detail (args + results) is gated on a ctrl+o toggle, read from
//! `/dev/tty` directly since stdin here is the piped JSONL stream, not free
//! for interactive reads. State optionally persists to `--state <path>` so it
//! survives outcome-loop.sh spawning a fresh process each iteration.
//!
//! Unrecognized/non-JSON lines pass through unchanged, so stray stderr text
//! (crashes, 429s) stays visible instead of being silently swallowed.

use nix::sys::termios::{tcgetattr, tcsetattr, LocalFlags, SetArg, SpecialCharacterIndices, Termios};
use nix::unistd::{getpgrp, tcgetpgrp};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

const CTRL_O: u8 = 0x0f;

fn main() {
    let state_path = parse_state_arg(std::env::args().skip(1));

    let initial_full = state_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim() == "full")
        .unwrap_or(false); // default: compact, matching Claude Code's own collapsed default

    if let Some(p) = &state_path {
        if !std::path::Path::new(p).exists() {
            let _ = std::fs::write(p, "compact");
        }
    }

    let full = Arc::new(AtomicBool::new(initial_full));
    let restore = setup_tty_toggle(full.clone(), state_path);
    run_formatter(&full);
    // Restore explicitly here rather than relying on the listener thread's
    // own teardown: that thread stays blocked in a 1-byte read for the
    // process's whole life (it only ever sees EOF/error, never on the happy
    // path), so main() returning is the one point guaranteed to run this.
    if let Some(restore) = restore {
        restore.restore();
    }
}

fn parse_state_arg(mut args: impl Iterator<Item = String>) -> Option<String> {
    while let Some(arg) = args.next() {
        if arg == "--state" {
            return args.next();
        }
    }
    None
}

struct TtyRestore {
    tty: File,
    orig: Termios,
}

impl TtyRestore {
    fn restore(&self) {
        let _ = tcsetattr(&self.tty, SetArg::TCSANOW, &self.orig);
    }
}

/// Best-effort: if there's no controlling terminal, we're not in its
/// foreground process group (tcsetattr/read would raise SIGTTOU/SIGTTIN and
/// stop this process — e.g. a plain `nohup ... &` from an interactive
/// shell), or termios setup fails, the toggle is simply unavailable for this
/// run — formatting still works.
fn setup_tty_toggle(full: Arc<AtomicBool>, state_path: Option<String>) -> Option<TtyRestore> {
    let tty = OpenOptions::new().read(true).open("/dev/tty").ok()?;

    match tcgetpgrp(&tty) {
        Ok(pgrp) if pgrp == getpgrp() => {}
        _ => return None,
    }

    let orig = tcgetattr(&tty).ok()?;
    let mut raw = orig.clone();
    raw.local_flags &= !(LocalFlags::ICANON | LocalFlags::ECHO);
    raw.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
    raw.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
    tcsetattr(&tty, SetArg::TCSANOW, &raw).ok()?;

    let listener_tty = tty.try_clone().ok()?;
    thread::spawn(move || listen_for_toggle(listener_tty, full, state_path));

    Some(TtyRestore { tty, orig })
}

fn listen_for_toggle(mut tty: File, full: Arc<AtomicBool>, state_path: Option<String>) {
    let mut buf = [0u8; 1];
    loop {
        match tty.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) if buf[0] == CTRL_O => {
                let now_full = !full.load(Ordering::Relaxed);
                full.store(now_full, Ordering::Relaxed);
                if let Some(p) = &state_path {
                    let _ = std::fs::write(p, if now_full { "full" } else { "compact" });
                }
                let (label, hint) = if now_full {
                    ("shown", "hide")
                } else {
                    ("hidden", "show")
                };
                let mut out = io::stdout();
                let _ = writeln!(out, "── tool-call detail {label} (ctrl+o to {hint}) ──");
                let _ = out.flush();
            }
            Ok(_) => {}
        }
    }
}

fn run_formatter(full: &AtomicBool) {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut out = io::stdout();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        // Byte-level read + lossy UTF-8 conversion (rather than
        // BufRead::lines(), which errors the whole line out on invalid
        // UTF-8) so a stray non-UTF-8 byte can't blank the live display for
        // the rest of the iteration — the stray-text passthrough contract
        // below needs to actually hold.
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
            buf.pop();
        }
        let line = String::from_utf8_lossy(&buf);
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let _ = writeln!(out, "{line}");
                continue;
            }
        };
        render_event(&mut out, &value, full.load(Ordering::Relaxed));
    }
}

fn render_event(out: &mut impl Write, value: &Value, is_full: bool) {
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "system" => {
            if value.get("subtype").and_then(Value::as_str) == Some("init") {
                let model = value.get("model").and_then(Value::as_str).unwrap_or("?");
                let _ = writeln!(out, "── session start (model={model}) ──");
            }
        }
        "assistant" => render_assistant(out, value, is_full),
        "user" if is_full => render_user(out, value),
        "result" => render_result(out, value),
        _ => {}
    }
}

fn render_assistant(out: &mut impl Write, value: &Value, is_full: bool) {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    if !t.trim().is_empty() {
                        let _ = writeln!(out, "{t}");
                    }
                }
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("?");
                if is_full {
                    let input = block.get("input").map(Value::to_string).unwrap_or_default();
                    let _ = writeln!(out, "→ {name} {}", truncate(&input, 300));
                } else {
                    let _ = writeln!(out, "→ {name}");
                }
            }
            _ => {}
        }
    }
}

fn render_user(out: &mut impl Write, value: &Value) {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let text = tool_result_text(block.get("content").unwrap_or(&Value::Null));
        let flat = text.replace('\n', " ");
        let flat = flat.trim();
        if !flat.is_empty() {
            let _ = writeln!(out, "  ⇢ {}", truncate(flat, 300));
        }
    }
}

fn render_result(out: &mut impl Write, value: &Value) {
    let cost = value
        .get("total_cost_usd")
        .map(Value::to_string)
        .unwrap_or_else(|| "?".into());
    let turns = value
        .get("num_turns")
        .map(Value::to_string)
        .unwrap_or_else(|| "?".into());
    let dur = value
        .get("duration_ms")
        .map(Value::to_string)
        .unwrap_or_else(|| "?".into());
    let _ = writeln!(out, "── result: cost=${cost} turns={turns} duration={dur}ms ──");
}

fn tool_result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|it| it.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    } else {
        s.to_string()
    }
}
