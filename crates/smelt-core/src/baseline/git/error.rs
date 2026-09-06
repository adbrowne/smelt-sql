//! Errors the baseline-git surface can produce.

use std::path::PathBuf;

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
