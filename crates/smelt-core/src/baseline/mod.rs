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
//!
//! Split into two submodules: [`git`] holds the git-subcommand surface
//! (ref resolution, `git archive` materialisation); [`edited_set`] holds
//! the pure workspace-comparison half (no git invocation).

mod edited_set;
mod git;

pub use edited_set::{edited_set, EditedSet};
pub use git::{
    discover_repo_root, git_watch_paths, materialize, materialize_in, resolve_baseline,
    BaselineCheckout, BaselineError, ResolvedAs, ResolvedBaseline,
};
