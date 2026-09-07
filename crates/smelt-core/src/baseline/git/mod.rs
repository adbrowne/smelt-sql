//! The git-surface half of baseline materialisation: resolving a baseline
//! ref and exporting the project subtree at that commit into a scratch
//! directory. See `super` (`baseline/mod.rs`) for the module-level contract
//! this half upholds (Constraint 8 "no repository mutation", the cleanup
//! guarantee).
//!
//! Split into three parts: [`error`] holds [`BaselineError`]; [`resolve`]
//! resolves a baseline ref (`resolve_baseline`, `discover_repo_root`,
//! `git_watch_paths`); [`materialize`] exports the resolved commit into a
//! scratch checkout (`materialize`, `materialize_in`). The `run_git` helper
//! below is shared by `resolve`'s subcommands; `materialize` streams `git
//! archive` directly since it needs the raw child process, not captured
//! output.

use std::path::Path;
use std::process::{Command, Output};

mod error;
mod materialize;
mod resolve;

pub use error::BaselineError;
pub use materialize::{materialize, materialize_in, BaselineCheckout};
pub use resolve::{
    discover_repo_root, git_watch_paths, resolve_baseline, ResolvedAs, ResolvedBaseline,
};

/// Run one git subcommand rooted at `repo_root`, capturing stdout/stderr.
/// `GIT_OPTIONAL_LOCKS=0` means no invocation can refresh or write
/// `.git/index` — half of Constraint 8 ("no repository mutation") for free.
fn run_git(repo_root: &Path, args: &[&str]) -> Result<Output, BaselineError> {
    Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(BaselineError::GitUnavailable)
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn stdout_trimmed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
