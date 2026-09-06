//! The git-surface half of baseline materialisation: resolving a baseline
//! ref and exporting the project subtree at that commit into a scratch
//! directory. See `super` for the module-level contract this half upholds
//! (Constraint 8 "no repository mutation", the cleanup guarantee).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Errors the baseline-git surface can produce
/// (`docs/specs/property_diff.md` §"Baseline materialisation",
/// §Constraints item 6 "Fail-loud"). Every variant names what the user
/// must do; none is recoverable into an empty diff.
#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    #[error("{} is not inside a git work tree", .dir.display())]
    NotAGitWorkTree { dir: PathBuf },
    #[error("unknown git ref '{git_ref}': {stderr}")]
    UnknownRef { git_ref: String, stderr: String },
    #[error(
        "no `main` or `master` branch found to resolve the default baseline against; pass an explicit ref"
    )]
    NoBaseBranch,
    #[error("could not compute the merge-base with '{base}': {stderr}")]
    MergeBaseFailed { base: String, stderr: String },
    #[error(
        "baseline commit {commit} has no smelt.yml or smelt.yaml at project path '{rel}'; the project did not exist there at that ref"
    )]
    NoProjectAtRef { commit: String, rel: String },
    #[error("git is not available or could not be run: {0}")]
    GitUnavailable(std::io::Error),
    #[error("could not resolve the real path of '{}': {source}", .path.display())]
    PathResolutionFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("`git archive` failed: {stderr}")]
    Archive { stderr: String },
    #[error("failed to extract the baseline archive: {0}")]
    Unpack(std::io::Error),
    #[error("failed to create a scratch directory for the baseline checkout: {0}")]
    Scratch(std::io::Error),
}

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

/// Whether `resolve_baseline`'s default ref was resolved by explicit
/// request or by falling back to the merge-base with `main`/`master`
/// (`docs/specs/property_diff.md` §Surface, the JSON `resolved_as` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedAs {
    Explicit,
    MergeBase,
}

/// A resolved baseline ref: the commit to export, plus enough of the repo
/// layout ([`materialize`] needs `repo_root`/`rel`) to export it without
/// re-deriving them.
#[derive(Debug, Clone)]
pub struct ResolvedBaseline {
    /// The string the JSON `baseline.ref` field should print — the ref as
    /// given, or `merge-base(<main|master>)` when defaulted.
    pub requested: String,
    pub commit: String,
    pub resolved_as: ResolvedAs,
    repo_root: PathBuf,
    /// `project_dir` relative to `repo_root`, forward-slash separated,
    /// empty when the project *is* the repo root.
    rel: String,
}

/// Resolve `project_dir`'s git work-tree root and its path relative to
/// that root. Not itself part of `resolve_baseline`'s public surface
/// (private), but the two calls (`show_toplevel` + `strip_prefix`) that
/// implement §"Baseline materialisation"'s use of `git archive <ref> --
/// <project-relative path>`.
fn show_toplevel_and_rel(project_dir: &Path) -> Result<(PathBuf, String), BaselineError> {
    let output = run_git(project_dir, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Err(BaselineError::NotAGitWorkTree {
            dir: project_dir.to_path_buf(),
        });
    }
    let repo_root_raw = PathBuf::from(stdout_trimmed(&output));
    let repo_root =
        repo_root_raw
            .canonicalize()
            .map_err(|source| BaselineError::PathResolutionFailed {
                path: repo_root_raw.clone(),
                source,
            })?;
    let project_dir_canon =
        project_dir
            .canonicalize()
            .map_err(|source| BaselineError::PathResolutionFailed {
                path: project_dir.to_path_buf(),
                source,
            })?;

    let rel = if project_dir_canon == repo_root {
        String::new()
    } else {
        match project_dir_canon.strip_prefix(&repo_root) {
            Ok(rel_path) => rel_path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => {
                // Should be unreachable given `--show-toplevel` returned an
                // ancestor of `project_dir`, but fail loud rather than
                // silently exporting the wrong subtree if it ever happens.
                return Err(BaselineError::NotAGitWorkTree {
                    dir: project_dir.to_path_buf(),
                });
            }
        }
    };
    Ok((repo_root, rel))
}

/// Cheaply discover `project_dir`'s git work-tree root, without resolving
/// or materialising a baseline (`docs/outcomes/20260905-property-diff/
/// phases/07-plan.md` D2): the editor needs this just to derive `.git`
/// watch globs at startup, and computing a full baseline for that would
/// pay for a `merge-base` lookup no watcher registration needs. `None`
/// when `project_dir` is not inside a git work tree — the caller (the LSP)
/// registers no `.git` watcher for that project, exactly as a workspace
/// with no resolvable baseline shows no lens (D8, non-git silence).
pub fn discover_repo_root(project_dir: &Path) -> Option<PathBuf> {
    show_toplevel_and_rel(project_dir)
        .ok()
        .map(|(root, _)| root)
}

fn branch_exists(repo_root: &Path, name: &str) -> Result<bool, BaselineError> {
    let output = run_git(
        repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )?;
    Ok(output.status.success())
}

fn verify_commit(repo_root: &Path, git_ref: &str) -> Result<String, BaselineError> {
    let output = run_git(
        repo_root,
        &["rev-parse", "--verify", &format!("{git_ref}^{{commit}}")],
    )?;
    if !output.status.success() {
        return Err(BaselineError::UnknownRef {
            git_ref: git_ref.to_string(),
            stderr: stderr_of(&output),
        });
    }
    Ok(stdout_trimmed(&output))
}

fn merge_base_with(repo_root: &Path, base: &str) -> Result<String, BaselineError> {
    let output = run_git(repo_root, &["merge-base", "HEAD", base])?;
    if !output.status.success() {
        return Err(BaselineError::MergeBaseFailed {
            base: base.to_string(),
            stderr: stderr_of(&output),
        });
    }
    Ok(stdout_trimmed(&output))
}

fn project_exists_at_ref(repo_root: &Path, commit: &str, rel: &str) -> Result<(), BaselineError> {
    for name in ["smelt.yml", "smelt.yaml"] {
        let pathspec = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        let output = run_git(
            repo_root,
            &["cat-file", "-e", &format!("{commit}:{pathspec}")],
        )?;
        if output.status.success() {
            return Ok(());
        }
    }
    Err(BaselineError::NoProjectAtRef {
        commit: commit.to_string(),
        rel: rel.to_string(),
    })
}

/// Resolve the baseline ref for `project_dir` (`docs/specs/property_diff.md`
/// §Surface `--diff [<ref>]`): `explicit` when given, else the merge-base of
/// `HEAD` with `main` (falling back to `master`). Either way, verifies the
/// project actually existed at that commit (`smelt.yml`/`smelt.yaml` at the
/// project's relative path) before returning success — a baseline that
/// resolves to a commit with no project is `NoProjectAtRef`, never treated
/// as an empty diff.
pub fn resolve_baseline(
    project_dir: &Path,
    explicit: Option<&str>,
) -> Result<ResolvedBaseline, BaselineError> {
    let (repo_root, rel) = show_toplevel_and_rel(project_dir)?;

    let (requested, commit, resolved_as) = match explicit {
        Some(git_ref) => {
            let commit = verify_commit(&repo_root, git_ref)?;
            (git_ref.to_string(), commit, ResolvedAs::Explicit)
        }
        None => {
            let base = if branch_exists(&repo_root, "main")? {
                "main"
            } else if branch_exists(&repo_root, "master")? {
                "master"
            } else {
                return Err(BaselineError::NoBaseBranch);
            };
            let commit = merge_base_with(&repo_root, base)?;
            (format!("merge-base({base})"), commit, ResolvedAs::MergeBase)
        }
    };

    project_exists_at_ref(&repo_root, &commit, &rel)?;

    Ok(ResolvedBaseline {
        requested,
        commit,
        resolved_as,
        repo_root,
        rel,
    })
}

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

impl ResolvedBaseline {
    /// The git work-tree root this baseline was resolved in
    /// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D2). Used
    /// by editor callers to derive the `.git` watch globs that trigger a
    /// prompt re-check of the resolved commit — re-resolution, not the
    /// watch, is the correctness mechanism (see [`git_watch_paths`]).
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

/// The `.git` paths whose change should prompt an editor to re-resolve
/// `resolve_baseline` promptly (`docs/specs/property_diff.md` §Surface
/// "Editor"; `docs/outcomes/20260905-property-diff/phases/07-plan.md` D2).
///
/// This is a **trigger**, not the correctness mechanism: several clients
/// (and some VS Code configurations) never report changes under `.git`, so
/// a design that relied on this watch firing would silently serve a stale
/// baseline after e.g. a `git checkout`. The re-resolve-and-compare-commit
/// step in the caller is what actually decides whether the cached baseline
/// is still valid; this list only makes that check happen sooner.
pub fn git_watch_paths(resolved: &ResolvedBaseline) -> Vec<PathBuf> {
    let git_dir = resolved.repo_root.join(".git");
    vec![
        git_dir.join("HEAD"),
        git_dir.join("refs"),
        git_dir.join("packed-refs"),
    ]
}

/// Export `resolved`'s commit's project subtree into a fresh scratch
/// directory via `git archive`, streamed through `tar` (never
/// `Command::output()`, which would deadlock a pipe on a large project),
/// and scrub any committed `.smelt/` (`docs/specs/property_diff.md`
/// §"Baseline materialisation": "Nothing under `.smelt/` at the baseline is
/// read even if it is committed" — `ingest_loaded_workspace` reads
/// `.smelt/` from disk, so the baseline copy must not have one).
///
/// The scratch [`tempfile::TempDir`] is created **first**, before any
/// fallible step, so every error path below unwinds through its `Drop` and
/// leaves no directory behind (module doc comment; tested by
/// `checkout_scratch_is_deleted_when_materialization_fails`).
pub fn materialize(resolved: &ResolvedBaseline) -> Result<BaselineCheckout, BaselineError> {
    let scratch = tempfile::Builder::new()
        .prefix("smelt-baseline-")
        .tempdir()
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
