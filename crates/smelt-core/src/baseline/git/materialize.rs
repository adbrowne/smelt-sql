//! Exporting a resolved baseline's commit into a scratch checkout via
//! `git archive`. See `super` for the module-level contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::resolve::ResolvedBaseline;
use super::BaselineError;

/// A materialised baseline checkout: the extracted project subtree, held
/// alive by a scratch [`tempfile::TempDir`] whose `Drop` deletes it (see
/// the module doc comment for the honest limits of that guarantee).
#[derive(Debug)]
pub struct BaselineCheckout {
    // Held only for its `Drop` (deletes the scratch directory) — never read.
    _scratch: tempfile::TempDir,
    project_root: PathBuf,
}

impl BaselineCheckout {
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

/// Export `resolved`'s commit's project subtree into a fresh scratch
/// directory via `git archive`, streamed through `tar` (never
/// `Command::output()`, which would deadlock a pipe on a large project),
/// and scrub any committed `.smelt/` (`docs/specs/property_diff.md`
/// §"Baseline materialisation": "Nothing under `.smelt/` at the baseline is
/// read even if it is committed" — `ingest_loaded_workspace` reads
/// `.smelt/` from disk, so the baseline copy must not have one).
///
/// Delegates to [`materialize_in`] with `std::env::temp_dir()` as the
/// scratch parent.
pub fn materialize(resolved: &ResolvedBaseline) -> Result<BaselineCheckout, BaselineError> {
    materialize_in(resolved, &std::env::temp_dir())
}

/// [`materialize`] with an explicit scratch parent, rather than
/// `std::env::temp_dir()`. Exists because a shared temp dir is not
/// observable in isolation: any concurrent `materialize` call anywhere on
/// the box — same process or another — creates and drops its own
/// `smelt-baseline-*` entries there, so a test asserting scratch hygiene
/// against the system temp dir races every other `materialize` caller in
/// the workspace. Callers that need to *observe* scratch creation/cleanup
/// (rather than just get a checkout) should pass a private directory.
///
/// The scratch [`tempfile::TempDir`] is created **first**, before any
/// fallible step, so every error path below unwinds through its `Drop` and
/// leaves no directory behind (module doc comment; tested by
/// `checkout_scratch_is_deleted_when_materialization_fails`).
pub fn materialize_in(
    resolved: &ResolvedBaseline,
    scratch_parent: &Path,
) -> Result<BaselineCheckout, BaselineError> {
    let scratch = tempfile::Builder::new()
        .prefix("smelt-baseline-")
        .tempdir_in(scratch_parent)
        .map_err(BaselineError::Scratch)?;

    let mut args: Vec<String> = vec![
        "archive".to_string(),
        "--format=tar".to_string(),
        resolved.commit.clone(),
    ];
    if !resolved.rel.is_empty() {
        args.push("--".to_string());
        args.push(resolved.rel.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let mut child = Command::new("git")
        .args(&arg_refs)
        .current_dir(&resolved.repo_root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(BaselineError::GitUnavailable)?;

    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return Err(BaselineError::Archive {
                stderr: "git archive produced no stdout pipe".to_string(),
            });
        }
    };
    let unpack_result = tar::Archive::new(&mut stdout).unpack(scratch.path());

    // Drain whatever `git` has left to write before dropping the read end.
    // `tar` stops at the end-of-archive marker, but `git archive` still emits
    // the trailing block padding after it; closing the pipe first kills git
    // with `SIGPIPE`, which surfaced as an intermittent "`git archive` failed"
    // with an EMPTY stderr whenever the machine was loaded enough for git to
    // still be writing (issue #194).
    let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    drop(stdout);

    let status = child.wait().map_err(BaselineError::GitUnavailable)?;
    if !status.success() {
        let mut stderr_text = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut stderr_text);
        }
        return Err(BaselineError::Archive {
            stderr: stderr_text.trim().to_string(),
        });
    }
    unpack_result.map_err(BaselineError::Unpack)?;

    let project_root = if resolved.rel.is_empty() {
        scratch.path().to_path_buf()
    } else {
        scratch.path().join(&resolved.rel)
    };

    // D6: scrub any committed `.smelt/` — the profile is a function of
    // sources only, and `ingest_loaded_workspace` reads `.smelt/` from disk
    // if present.
    let dot_smelt = project_root.join(".smelt");
    if dot_smelt.exists() {
        std::fs::remove_dir_all(&dot_smelt).map_err(BaselineError::Unpack)?;
    }

    Ok(BaselineCheckout {
        _scratch: scratch,
        project_root,
    })
}
