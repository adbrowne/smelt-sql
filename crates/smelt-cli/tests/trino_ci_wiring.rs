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

/// A test binary this repo's live-Trino census can name: `(crate, binary,
/// own source text, combined source text)`. `own` is exactly the text cargo
/// compiles into this one binary directly: the root file, or `<dir>/main.rs`
/// plus every sibling `.rs` file beside it (this repo's `tests/<dir>/main.rs`
/// plus submodules convention, e.g. `statement_parity`). `combined`
/// additionally folds in the crate's shared `tests/common/mod.rs` helper
/// when `own` declares `mod common;` — needed to check a delegated skip line
/// (some binaries call a shared helper that prints it, rather than printing
/// their own), but never to DECIDE census membership: `common/mod.rs` is a
/// grab-bag serving many unrelated multi-backend suites (Spark/BigQuery
/// included), and its own `pub fn trino_env(` definition would otherwise
/// make every file with a bare `mod common;` look live-gated whether or not
/// it ever calls it.
struct TestBinary {
    krate: &'static str,
    binary: String,
    own: String,
    combined: String,
}

/// Every call-site string a genuine live Trino leg contains — checked
/// against a binary's OWN text only (never `combined`; see [`TestBinary`]'s
/// doc comment for why). Not a mere textual mention (a doc comment, an
/// assertion checking someone else's file, an `unreachable!` arm proving a
/// match is never taken): a raw env lookup, the shared `trino_env()` gate, a
/// call into `smelt-backend-trino`'s equivalent `live_env_or_skip()`, or
/// `targets_to_run_with_trino()`. `TargetKind::Trino { .. }` alone does NOT
/// qualify — several multi-backend parity suites carry an exhaustive
/// `unreachable!()` arm for it because `targets_to_run` (without
/// `_with_trino`) never yields that variant.
const LIVE_LEG_MARKERS: [&str; 4] = [
    "env::var(\"SMELT_TRINO_URL\")",
    "trino_env(",
    "targets_to_run_with_trino(",
    "live_env_or_skip(",
];

fn has_live_leg_marker(text: &str) -> bool {
    LIVE_LEG_MARKERS.iter().any(|m| text.contains(m))
}

/// Narrower than [`has_live_leg_marker`]: true only for a **single-purpose**
/// Trino test — one gated on `trino_env()`/`live_env_or_skip()`/a raw
/// `SMELT_TRINO_URL` lookup, which skips (all or part of) its own body,
/// printing "Skipping …", when the tier is absent. A multi-backend `_parity`
/// suite driven by `targets_to_run_with_trino()` alone (e.g.
/// `materialization_parity.rs`) does not qualify: it has no per-Trino
/// "Skipping" line of its own to check — Trino is simply absent from the
/// loop it still runs for every other configured backend, a different (and
/// equally legitimate) skip shape that
/// [`the_trino_job_fails_if_a_leg_skips_with_the_tier_up`]'s CI-side grep
/// still catches if it ever silently no-ops with the tier up. Checked
/// against OWN text, same reason as [`LIVE_LEG_MARKERS`].
fn is_single_purpose_skip_gated(text: &str) -> bool {
    text.contains("trino_env(")
        || text.contains("env::var(\"SMELT_TRINO_URL\")")
        || text.contains("live_env_or_skip(")
}

/// Walks `crates/<krate>/tests/`, grouping files into cargo's own test-binary
/// units: every root-level `<name>.rs` is binary `<name>`; every `<dir>/
/// main.rs` is binary `<dir>`, combined with every other `.rs` file directly
/// inside `<dir>`. A subdirectory with no `main.rs` (e.g. `common/`) is not a
/// binary — cargo never compiles it standalone; it is folded into whichever
/// binary's own text declares `mod common;`.
fn discover_test_binaries(krate: &'static str) -> Vec<TestBinary> {
    let tests_dir = repo_root().join("crates").join(krate).join("tests");
    let common_text = {
        let common_mod = tests_dir.join("common").join("mod.rs");
        fs::read_to_string(&common_mod).ok()
    };
    let fold_common = |own: &str| -> String {
        let mut combined = own.to_string();
        if own.contains("mod common;") {
            if let Some(c) = &common_text {
                combined.push('\n');
                combined.push_str(c);
            }
        }
        combined
    };

    let mut binaries = Vec::new();
    let entries = fs::read_dir(&tests_dir)
        .unwrap_or_else(|e| panic!("read_dir {tests_dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();

    for entry in &entries {
        let path = entry.path();
        if path.is_file() {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let own = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
            let combined = fold_common(&own);
            binaries.push(TestBinary {
                krate,
                binary: name.to_string(),
                own,
                combined,
            });
        } else if path.is_dir() {
            let dir_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap()
                .to_string();
            let main_rs = path.join("main.rs");
            if !main_rs.exists() {
                // Not a cargo test binary on its own (e.g. `common/`,
                // `oracle/`) — folded into whichever binary's own text
                // declares `mod <dir_name>;`/reads it directly, handled via
                // the root-file `mod common;` branch above for `common`
                // specifically. Other helper-only subdirs carry no live leg
                // of their own to derive.
                continue;
            }
            let mut own = String::new();
            let mut sub_entries = fs::read_dir(&path)
                .unwrap_or_else(|e| panic!("read_dir {path:?}: {e}"))
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
                .collect::<Vec<_>>();
            sub_entries.sort();
            for sub in &sub_entries {
                own.push_str(
                    &fs::read_to_string(sub).unwrap_or_else(|e| panic!("read {sub:?}: {e}")),
                );
                own.push('\n');
            }
            let combined = fold_common(&own);
            binaries.push(TestBinary {
                krate,
                binary: dir_name,
                own,
                combined,
            });
        }
    }
    binaries
}

/// The derived live-gated census across every crate this repo's Trino tier
/// touches from test code: `smelt-cli` and `smelt-backend-trino`.
/// `smelt-runtime` carries no live Trino leg today (`docs/outcomes/
/// 20260913-trino-incremental/phases/03d-summary.md` — the keyed-fold
/// `statement_parity` leg was attempted and found blocked by gaps 4/5, not
/// landed), so it is not scanned here; add it back the day a live leg lands
/// there.
fn live_gated_census() -> Vec<TestBinary> {
    ["smelt-cli", "smelt-backend-trino"]
        .into_iter()
        .flat_map(discover_test_binaries)
        // This very file (`trino_ci_wiring`) matches the markers because it
        // spells them out as string literals to scan FOR them — a
        // self-referential false positive, not a live leg of its own. It
        // needs no Docker tier: every test in it is a pure file assertion.
        .filter(|b| b.binary != "trino_ci_wiring")
        .filter(|b| has_live_leg_marker(&b.own))
        .collect()
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

/// Phase 3d (`docs/outcomes/20260913-trino-incremental/phases/03d-plan.md`)
/// — the live-gated census named in the two tests below is now **derived**
/// (`live_gated_census`, a directory scan of `crates/{smelt-cli,
/// smelt-backend-trino}/tests`), not a hardcoded list — so a new Trino live
/// leg lands in this gate the moment its file exists, with no second commit
/// to teach this test its name.
///
/// Test: every derived live-gated `(crate, binary)` pair must appear in the
/// `trino-integration` job — either covered by a whole-crate step (`-p
/// <crate>` with no narrowing `--test` flag anywhere in that step's `run:`
/// block) or named explicitly (`--test <binary>` in a step that also
/// carries `-p <crate>`). Fails today for any binary the job's steps don't
/// yet run — `trino_incremental_families`, `trino_state_residency`,
/// `trino_ddl_live`, `trino_lock_versioning`, … are unrun in CI until the
/// smelt-cli step's `--test` list is widened to cover them.
#[test]
fn the_trino_job_runs_every_live_gated_trino_test_binary() {
    let wf = workflow();
    let body = job_body(&wf, "trino-integration");
    // Every `run:` block's raw text, so a binary named in one step and a
    // crate named in another don't falsely combine into "covered".
    let run_blocks: Vec<&str> = body
        .split("- name:")
        .filter(|block| block.contains("run:") || block.contains("run: |"))
        .collect();

    for bin in live_gated_census() {
        let crate_flag = format!("-p {}", bin.krate);
        let test_flag = format!("--test {}", bin.binary);
        let covered = run_blocks.iter().any(|block| {
            if !block.contains(&crate_flag) {
                return false;
            }
            let whole_crate_step = !block.contains("--test ");
            whole_crate_step || block.contains(&test_flag)
        });
        assert!(
            covered,
            "trino-integration job has no step running `-p {} --test {}` (nor an unnarrowed \
             `-p {}` step) — every live-gated Trino test binary must run in CI:\n{body}",
            bin.krate, bin.binary, bin.krate
        );
    }
}

/// Test 5: every Trino-gated test file reads `SMELT_TRINO_URL` through a
/// helper that yields `Option`/`Result` and prints a `Skipping` line on the
/// unset path, with no fabricated default supplied to the lookup itself.
/// `statement_client.rs` is the one file in this census with no live legs at
/// all (it talks only to a local stub coordinator) — its bar is simply that
/// it never reads `SMELT_TRINO_URL`, so there is no gate to fake.
#[test]
fn every_trino_gated_test_file_skips_through_the_shared_env_gate() {
    for bin in live_gated_census()
        .into_iter()
        .filter(|b| is_single_purpose_skip_gated(&b.own))
    {
        let text = &bin.combined;
        let rel = format!("crates/{}/tests/{}", bin.krate, bin.binary);
        assert!(
            text.contains("SMELT_TRINO_URL"),
            "{rel} is expected to have a live Trino leg gated on SMELT_TRINO_URL: {rel}"
        );
        assert!(
            text.to_lowercase().contains("skipping"),
            "{rel} must print a 'skipping ...' line when SMELT_TRINO_URL is unset"
        );
        for line in text.lines() {
            // The lookup itself, not any line merely mentioning the name —
            // e.g. `trino_env()`'s own port-parse panic message embeds the
            // string "SMELT_TRINO_URL" in prose, which is not a fabricated
            // default for the lookup.
            if line.contains("env::var(\"SMELT_TRINO_URL\")") {
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
