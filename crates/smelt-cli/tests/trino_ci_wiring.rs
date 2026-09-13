//! Standing gate for the Trino CI wiring
//! (`docs/outcomes/20260913-trino-target-spine/phases/10-plan.md`): the
//! `trino-integration` job in `.github/workflows/compat.yml` is gated the way
//! `spark-parity` is, its `changes` filter actually covers every path the
//! tier lives at, it runs the tier and every live Trino suite with a teardown
//! that always fires, it fails loudly if a leg silently skips with the tier
//! up, and every Trino-gated test file uses the shared env gate rather than
//! an ad-hoc one. Pure file assertions — no Docker, no network.

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

fn workflow() -> String {
    read(".github/workflows/compat.yml")
}

/// Extracts the named top-level job's body (from its `  <name>:` header up to
/// the next line at the same two-space indent, or EOF). Good enough for a
/// GitHub Actions workflow, whose jobs are always two-space-indented map keys.
fn job_body<'a>(workflow: &'a str, job_name: &str) -> &'a str {
    let header = format!("\n  {job_name}:");
    let start = workflow
        .find(&header)
        .unwrap_or_else(|| panic!("no `{job_name}:` job found in compat.yml"));
    let after_header = start + header.len();
    let rest = &workflow[after_header..];
    let end = rest
        .lines()
        .scan(0usize, |offset, line| {
            let this = *offset;
            *offset += line.len() + 1;
            Some((this, line))
        })
        .find(|(_, line)| !line.is_empty() && !line.starts_with(' '))
        .map(|(offset, _)| offset)
        .unwrap_or(rest.len());
    &workflow[after_header..after_header + end]
}

/// Test 1: a `trino-integration:` job exists, depends on `changes`, and its
/// `if:` carries all four gating clauses. Fails against a job that silently
/// runs on every PR (no `if:` at all) or one that a failing `changes` job
/// would skip outright (no `!cancelled()`).
#[test]
fn compat_workflow_has_a_trino_job_gated_like_spark() {
    let wf = workflow();
    let body = job_body(&wf, "trino-integration");

    assert!(
        body.contains("needs: changes"),
        "trino-integration job must declare `needs: changes`:\n{body}"
    );
    assert!(
        body.contains("!cancelled()"),
        "trino-integration job's `if:` must guard against a `changes`-job \
         failure silently skipping it, like spark-parity's `!cancelled()`:\n{body}"
    );
    assert!(
        body.contains("github.event_name == 'schedule'"),
        "trino-integration job must run on schedule:\n{body}"
    );
    assert!(
        body.contains("'run-docker-tests'"),
        "trino-integration job must run when explicitly labeled:\n{body}"
    );
    assert!(
        body.contains("needs.changes.outputs.trino == 'true'"),
        "trino-integration job must run when the `changes` job flags Trino-relevant paths:\n{body}"
    );
}

/// Test 2: the `changes` job outputs `trino`, has a `trino:` filter block,
/// and every path the Trino tier actually lives at is matched by at least
/// one glob in it. Paths are read off the filesystem, not restated here, so
/// a moved file fails this test rather than rotting silently.
#[test]
fn changes_job_declares_a_trino_filter_covering_every_trino_path() {
    let wf = workflow();
    let changes_body = job_body(&wf, "changes");

    assert!(
        changes_body.contains("trino: ${{ steps.filter.outputs.trino }}"),
        "the `changes` job must output `trino`:\n{changes_body}"
    );

    let filter_start = changes_body
        .find("\n            trino:")
        .unwrap_or_else(|| {
            panic!("no `trino:` filter block in the `changes` job:\n{changes_body}")
        });
    let filter_rest = &changes_body[filter_start + 1..];
    let filter_block: String = filter_rest
        .lines()
        .skip(1)
        .take_while(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && line.starts_with("              - ")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let globs: Vec<String> = filter_block
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- "))
        .map(|g| g.trim().trim_matches('\'').to_string())
        .collect();
    assert!(
        !globs.is_empty(),
        "the `trino:` filter block must list at least one glob:\n{filter_block}"
    );

    let live_paths = [
        "scripts/trino-up.sh",
        "scripts/trino-compose.yml",
        "scripts/trino-catalog/iceberg.properties",
        "crates/smelt-backend-trino/src/lib.rs",
        "examples/trino_spine/smelt.yml",
        "crates/smelt-cli/tests/trino_smoke.rs",
    ];
    for path in live_paths {
        assert!(
            repo_root().join(path).exists(),
            "expected {path} to exist — update this test's live_paths list if it moved"
        );
        assert!(
            globs.iter().any(|g| path_matches_glob(g, path)),
            "{path} is not matched by any glob in the `trino:` filter block: {globs:?}"
        );
    }
}

/// Minimal glob matcher covering the two shapes `dorny/paths-filter` patterns
/// take in this workflow: a `dir/**` recursive prefix, and a `dir/name*`
/// single-segment filename glob. No `regex` dependency needed for these two
/// shapes.
fn path_matches_glob(glob: &str, path: &str) -> bool {
    if let Some(prefix) = glob.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    match glob.rfind('/') {
        Some(idx) => {
            let (dir, name_pat) = (&glob[..idx], &glob[idx + 1..]);
            match path.rfind('/') {
                Some(path_idx) => {
                    let (path_dir, path_name) = (&path[..path_idx], &path[path_idx + 1..]);
                    path_dir == dir && filename_glob_match(name_pat, path_name)
                }
                None => false,
            }
        }
        None => !path.contains('/') && filename_glob_match(glob, path),
    }
}

fn filename_glob_match(pat: &str, name: &str) -> bool {
    match pat.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pat == name,
    }
}

/// Test 3: the job's steps bring the tier up, export the `SMELT_TRINO_*`
/// vars, run the backend crate's tests plus each live-gated CLI leg, and
/// carry an `if: always()` teardown.
#[test]
fn the_trino_job_brings_the_tier_up_runs_the_live_suites_and_tears_it_down() {
    let wf = workflow();
    let body = job_body(&wf, "trino-integration");

    assert!(
        body.contains("scripts/trino-up.sh"),
        "trino-integration job must start the tier:\n{body}"
    );
    assert!(
        body.contains("SMELT_TRINO_URL"),
        "trino-integration job must export SMELT_TRINO_URL for the tests it runs:\n{body}"
    );
    for leg in [
        "-p smelt-backend-trino",
        "--test trino_smoke",
        "--test seed_parity",
        "--test materialization_parity",
    ] {
        assert!(
            body.contains(leg),
            "trino-integration job must run `{leg}`:\n{body}"
        );
    }
    assert!(
        body.contains("scripts/trino-down.sh"),
        "trino-integration job must tear the tier down:\n{body}"
    );

    let down_pos = body.find("scripts/trino-down.sh").expect("checked above");
    let preceding = &body[..down_pos];
    let last_if = preceding
        .rfind("if:")
        .unwrap_or_else(|| panic!("teardown step must carry an `if:` guard:\n{body}"));
    let if_line = &preceding[last_if..];
    let if_line = if_line.lines().next().unwrap_or(if_line);
    assert!(
        if_line.contains("always()"),
        "the teardown step's `if:` must be `always()` so it runs even when a prior step fails:\n{if_line}"
    );
}

/// Test 4: the job carries a no-skip guard — a grep-based check under
/// `set -o pipefail` that fails the job if `Skipping` appears anywhere in a
/// live leg's captured output. This is the anti-vacuous-pass half that
/// matters in CI (where `SMELT_TRINO_URL` *is* set): a green job in which
/// every leg skipped is indistinguishable from one that actually ran.
#[test]
fn the_trino_job_fails_if_a_leg_skips_with_the_tier_up() {
    let wf = workflow();
    let body = job_body(&wf, "trino-integration");

    assert!(
        body.contains("set -o pipefail"),
        "trino-integration job must run its test steps under `set -o pipefail` so a `grep` \
         failure after a pipe is not masked by the pipe's own exit code:\n{body}"
    );
    assert!(
        body.to_lowercase().contains("skipping") && body.contains("grep"),
        "trino-integration job must grep its captured test output for 'skipping' and fail the \
         step if found:\n{body}"
    );
}

/// Test 5: every Trino-gated test file reads `SMELT_TRINO_URL` through a
/// helper that yields `Option`/`Result` and prints a `Skipping` line on the
/// unset path, with no fabricated default supplied to the lookup itself.
/// `statement_client.rs` is the one file in this census with no live legs at
/// all (it talks only to a local stub coordinator) — its bar is simply that
/// it never reads `SMELT_TRINO_URL`, so there is no gate to fake.
#[test]
fn every_trino_gated_test_file_skips_through_the_shared_env_gate() {
    let live_gated_files = [
        "crates/smelt-backend-trino/tests/backend_live.rs",
        "crates/smelt-backend-trino/tests/capability_probes.rs",
        "crates/smelt-cli/tests/trino_smoke.rs",
        "crates/smelt-cli/tests/seed_parity.rs",
    ];
    for rel in live_gated_files {
        let text = read(rel);
        assert!(
            text.contains("SMELT_TRINO_URL"),
            "{rel} is expected to have a live Trino leg gated on SMELT_TRINO_URL: {rel}"
        );
        assert!(
            text.to_lowercase().contains("skipping"),
            "{rel} must print a 'skipping ...' line when SMELT_TRINO_URL is unset"
        );
        for line in text.lines() {
            if line.contains("SMELT_TRINO_URL") {
                assert!(
                    !line.contains("unwrap_or"),
                    "{rel} must not fabricate a default for the SMELT_TRINO_URL lookup itself \
                     (found: {line}) — the whole gate exists so an unset URL skips rather than \
                     silently pointing at a wrong default"
                );
            }
        }
    }

    let stub_only = "crates/smelt-backend-trino/tests/statement_client.rs";
    let text = read(stub_only);
    assert!(
        !text.contains("env::var(\"SMELT_TRINO_URL\")"),
        "{stub_only} is documented as talking only to a local stub coordinator; if it now \
         reads SMELT_TRINO_URL it has grown a live leg and belongs in live_gated_files above \
         with the shared-gate checks applied to it"
    );
}
