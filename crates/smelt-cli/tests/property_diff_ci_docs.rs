//! Text gates over the CI guide and the dogfood workflow
//! (`docs/outcomes/20260905-property-diff/phases/06-plan.md` tasks 5–7).
//! These check the *documents*, not runtime behaviour — the marker
//! byte-identity check (R4) is the most valuable test in this phase: it is
//! what stops a workflow silently turning "update the previous comment"
//! into "stack a new one on every push."

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// R4: `diff_render::MARKER`, `docs-site/docs/guide/ci.md`, and
/// `.github/workflows/property-diff.yml` must all quote the marker literal
/// byte-for-byte. Fails against a marker renamed or reformatted in any one
/// of the three places — the failure mode that silently turns "update"
/// back into "stack", after which every push adds a new comment and
/// reviewers stop reading them.
#[test]
fn the_marker_literal_is_identical_in_code_docs_and_workflow() {
    let marker = smelt_logical::analysis::diff_render::MARKER;

    let ci_guide = read("docs-site/docs/guide/ci.md");
    let workflow = read(".github/workflows/property-diff.yml");

    assert!(
        ci_guide.contains(marker),
        "docs-site/docs/guide/ci.md must quote the marker byte-for-byte: {marker:?}"
    );
    assert!(
        workflow.contains(marker),
        ".github/workflows/property-diff.yml must quote the marker byte-for-byte: {marker:?}"
    );
}

/// The CI guide documents the find-and-update mechanism, not just the
/// create path. Fails against a guide that shows only `gh pr comment`
/// (creating a new comment every time) with no PATCH step — the spec's
/// explicit "update not stack" clause (§"Pull-request comment").
#[test]
fn the_ci_guide_documents_the_update_not_stack_mechanism() {
    let ci_guide = read("docs-site/docs/guide/ci.md");
    assert!(ci_guide.contains("smelt explain --diff"), "{ci_guide}");
    assert!(ci_guide.contains("--markdown"), "{ci_guide}");
    assert!(ci_guide.contains("gh pr comment"), "{ci_guide}");
    assert!(
        ci_guide.contains("issues/comments/"),
        "expected a PATCH .../issues/comments/<id> line documenting the update path: {ci_guide}"
    );
}

/// The dogfood workflow requests `pull-requests: write` (needed to post or
/// update a comment) and guards that step against forked PRs, which get a
/// read-only token. Fails against a workflow missing the permission (every
/// run 403s) or missing the fork guard (every fork PR run fails loudly
/// instead of degrading to the job-summary-only path).
#[test]
fn the_dogfood_workflow_requests_pull_requests_write() {
    let workflow = read(".github/workflows/property-diff.yml");
    assert!(
        workflow.contains("pull-requests: write"),
        "expected `pull-requests: write` in the permissions block: {workflow}"
    );
    assert!(
        workflow.contains("github.event.pull_request.head.repo.full_name == github.repository"),
        "expected the same fork guard docs-pr-preview.yml uses: {workflow}"
    );
}
