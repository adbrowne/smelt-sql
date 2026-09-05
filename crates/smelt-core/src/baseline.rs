//! Git baseline materialisation for the property diff
//! (`docs/specs/property_diff.md` §"Baseline materialisation").
//!
//! This module owns the *git* half of the feature: resolving a baseline
//! ref (explicit, or the merge-base with `main`/`master`), exporting the
//! project subtree at that commit into a scratch directory via
//! `git archive`, and computing the working-tree-vs-baseline **edited
//! set** by comparing the two already-loaded workspaces (never by asking
//! git for a file list — `docs/specs/property_diff.md` §"Attribution": the
//! edited set is keyed by model/source *names*, and its three predicates
//! are semantic, not path-level).
//!
//! No other `smelt-core` module shells out to git; this is the only place
//! that does. Constraint 8 (`docs/specs/property_diff.md`) — "no repository
//! mutation" — is upheld by using exactly four read-only subcommands
//! (`rev-parse`, `show-ref`, `merge-base`, `cat-file`) plus `archive`, never
//! `checkout`/`worktree`/`stash`/`read-tree`/`update-ref`/`commit`, and by
//! passing `GIT_OPTIONAL_LOCKS=0` to every invocation so nothing can even
//! refresh `.git/index`. [`git_surface_uses_no_mutating_subcommand`] in
//! `tests/baseline.rs` guards this structurally as the module grows.
//!
//! **The cleanup guarantee is honest, not absolute.** [`BaselineCheckout`]'s
//! scratch directory is a [`tempfile::TempDir`], deleted by its own `Drop`.
//! `Drop` does not run under `panic = "abort"`, `std::process::abort`, or
//! SIGKILL — a killed process can leak a scratch directory under the OS
//! temp dir, which is the backstop, not a tested guarantee. What a test
//! *can* honestly prove or checked here: the scratch path does not exist
//! after the value is dropped, on both the happy path and a failing
//! materialisation, and the repository itself
//! (`git status --porcelain`, `git worktree list`, `.git/index`) is
//! byte-unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::discovery::ModelFile;
use crate::sources::SourceInfo;
use crate::workspace::LoadedWorkspace;

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

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return Err(BaselineError::Archive {
                stderr: "git archive produced no stdout pipe".to_string(),
            });
        }
    };
    let unpack_result = tar::Archive::new(stdout).unpack(scratch.path());

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

/// The §"Attribution" edited set: every model or source whose semantic
/// content differs between `base` (the baseline) and `work` (the working
/// tree), keyed by the same names `DiffGraph`'s `upstream`/`edited` use.
#[derive(Debug, Clone, Default)]
pub struct EditedSet {
    pub names: BTreeSet<String>,
    /// Project-relative paths of the files behind an edit, sorted — the
    /// JSON `edited_files` field and the text form's "N files changed"
    /// derive from this, so the two can never disagree with `names`.
    pub files: Vec<String>,
    pub project_config_changed: bool,
}

fn relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Bare dotted source name — `address_segments` with the leading `sources`
/// segment stripped, matching `DiffGraph::from_dependency_graph`'s
/// convention (`crates/smelt-logical/src/analysis/diff.rs`) so `edited` and
/// `upstream` key against each other.
fn source_name(info: &SourceInfo) -> String {
    match info.address_segments.split_first() {
        Some((first, rest)) if first == "sources" => rest.join("."),
        _ => info.address_segments.join("."),
    }
}

/// A `SourceInfo` compared with its (absolute, side-dependent) `path`
/// zeroed, so a field added to `SourceInfo` later is compared automatically
/// (the struct is `PartialEq`) instead of needing a hand-written field list
/// here.
fn source_without_path(info: &SourceInfo) -> SourceInfo {
    let mut cleared = info.clone();
    cleared.path = PathBuf::new();
    cleared
}

/// The §"Attribution" edited-set predicate for one model
/// (`docs/specs/property_diff.md` §"Attribution", Δ2): edited iff its
/// frontmatter-stripped SQL text differs, its parsed frontmatter metadata
/// differs, or its `smelt.yml` model override differs. A model present on
/// only one side is edited (deliberate — `diff_profiles` needs a shifted
/// downstream model to be able to attribute to an added/removed ancestor).
fn model_edited(
    base: Option<&ModelFile>,
    base_config: &crate::config::Config,
    work: Option<&ModelFile>,
    work_config: &crate::config::Config,
    name: &str,
) -> bool {
    let (base, work) = match (base, work) {
        (Some(b), Some(w)) => (b, w),
        _ => return true,
    };
    if smelt_parser::strip_frontmatter(&base.content)
        != smelt_parser::strip_frontmatter(&work.content)
    {
        return true;
    }
    if base.metadata != work.metadata {
        return true;
    }
    let base_override = base_config
        .models
        .get(name)
        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null));
    let work_override = work_config
        .models
        .get(name)
        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null));
    base_override != work_override
}

/// Whether a project-level `smelt.yml` key (any key other than `models`)
/// differs between the two versions.
fn project_config_changed(base: &crate::config::Config, work: &crate::config::Config) -> bool {
    fn without_models(config: &crate::config::Config) -> serde_json::Value {
        let mut value = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = value.as_object_mut() {
            obj.remove("models");
        }
        value
    }
    without_models(base) != without_models(work)
}

/// The §"Attribution" edited set, derived by comparing the two *loaded*
/// workspaces content-first (never `git diff --name-only`): the edited-set
/// predicates are semantic (frontmatter-stripped SQL, parsed metadata, a
/// `smelt.yml` override, a source declaration), not path-level, and
/// `DiffGraph.edited` is keyed by model/source names rather than paths.
///
/// `work` is expected to be a real `load_workspace` of the working
/// directory, so an uncommitted edit is simply content differing from the
/// archived baseline — nothing here compares two commits.
pub fn edited_set(
    base: &LoadedWorkspace,
    base_sources: &[SourceInfo],
    work: &LoadedWorkspace,
    work_sources: &[SourceInfo],
) -> EditedSet {
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();

    let base_models: BTreeMap<String, &ModelFile> = base
        .sql_files
        .iter()
        .map(|m| (m.canonical_path(), m))
        .collect();
    let work_models: BTreeMap<String, &ModelFile> = work
        .sql_files
        .iter()
        .map(|m| (m.canonical_path(), m))
        .collect();

    let all_model_names: BTreeSet<&String> = base_models.keys().chain(work_models.keys()).collect();
    for name in all_model_names {
        let b = base_models.get(name).copied();
        let w = work_models.get(name).copied();
        if model_edited(b, &base.config, w, &work.config, name) {
            names.insert(name.clone());
            if let Some(m) = w.or(b) {
                let root = if w.is_some() {
                    &work.project_root
                } else {
                    &base.project_root
                };
                files.insert(relative_path(root, &m.path));
            }
        }
    }

    let base_sources_by_name: BTreeMap<String, SourceInfo> = base_sources
        .iter()
        .map(|s| (source_name(s), source_without_path(s)))
        .collect();
    let work_sources_by_name: BTreeMap<String, SourceInfo> = work_sources
        .iter()
        .map(|s| (source_name(s), source_without_path(s)))
        .collect();
    let base_sources_raw: BTreeMap<String, &SourceInfo> =
        base_sources.iter().map(|s| (source_name(s), s)).collect();
    let work_sources_raw: BTreeMap<String, &SourceInfo> =
        work_sources.iter().map(|s| (source_name(s), s)).collect();

    let all_source_names: BTreeSet<&String> = base_sources_by_name
        .keys()
        .chain(work_sources_by_name.keys())
        .collect();
    for name in all_source_names {
        let b = base_sources_by_name.get(name);
        let w = work_sources_by_name.get(name);
        let edited = match (b, w) {
            (Some(bi), Some(wi)) => bi != wi,
            _ => true,
        };
        if edited {
            names.insert(name.clone());
            if let Some(info) = work_sources_raw.get(name).or(base_sources_raw.get(name)) {
                let root = if work_sources_raw.contains_key(name) {
                    &work.project_root
                } else {
                    &base.project_root
                };
                files.insert(relative_path(root, &info.path));
            }
        }
    }

    EditedSet {
        names,
        files: files.into_iter().collect(),
        project_config_changed: project_config_changed(&base.config, &work.config),
    }
}
