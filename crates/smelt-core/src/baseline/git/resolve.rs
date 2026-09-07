//! Resolving a baseline ref: explicit, or the merge-base with
//! `main`/`master`, verified against the project's presence at that
//! commit. See `super` for the module-level contract.

use std::path::{Path, PathBuf};

use super::{run_git, stderr_of, stdout_trimmed, BaselineError};

/// Whether `resolve_baseline`'s default ref was resolved by explicit
/// request or by falling back to the merge-base with `main`/`master`
/// (`docs/specs/property_diff.md` §Surface, the JSON `resolved_as` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedAs {
    Explicit,
    MergeBase,
}

/// A resolved baseline ref: the commit to export, plus enough of the repo
/// layout (`materialize` needs `repo_root`/`rel`) to export it without
/// re-deriving them.
#[derive(Debug, Clone)]
pub struct ResolvedBaseline {
    /// The string the JSON `baseline.ref` field should print — the ref as
    /// given, or `merge-base(<main|master>)` when defaulted.
    pub requested: String,
    pub commit: String,
    pub resolved_as: ResolvedAs,
    pub(super) repo_root: PathBuf,
    /// `project_dir` relative to `repo_root`, forward-slash separated,
    /// empty when the project *is* the repo root.
    pub(super) rel: String,
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
