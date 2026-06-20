//! Integration tests for selector `+` graph operators (D-38): strip before
//! entity resolution, re-attach to the resolved full path.
//!
//! Spec: `docs/specs/cli.md` §"Argument resolution algorithm" (graph operators)
//! and `docs/specs/model_selection.md` §"Selection methods".

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path, name: &str) {
    let yml = format!(
        "name: {name}\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n"
    );
    fs::write(dir.join("smelt.yml"), yml).unwrap();
}

/// Stage a two-model workspace: `base` (leaf) and `derived` (depends on base).
/// Returns the project root path.
fn stage_base_derived(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, name);
    fs::write(root.join("models").join("base.sql"), "SELECT 1 AS x\n").unwrap();
    fs::write(
        root.join("models").join("derived.sql"),
        "SELECT x + 1 AS y FROM smelt.base\n",
    )
    .unwrap();
    root
}

fn run_dry(project_dir: &Path, select: &str) -> std::process::Output {
    Command::new(smelt_bin())
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(["--select", select, "--dry-run"])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt binary should be runnable")
}

// ── D-38: `+` graph operators stripped before resolution, re-attached after ──

/// `+derived` strips the `+` before entity resolution → resolves `derived` →
/// re-attaches `+` → upstream operator preserved → `base` (the upstream) is
/// included in the dry-run set (D-38).
#[test]
fn plus_prefix_resolves_then_reattaches() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_prefix");

    let output = run_dry(&root, "+derived");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select +derived should succeed (+ is stripped before resolution)\nstderr: {stderr}\nstdout: {stdout}"
    );
    // Upstream `base` is included because the `+` upstream flag is preserved.
    assert!(
        stdout.contains("Would run: base"),
        "--select +derived should include upstream 'base' (operator preserved)\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select +derived should include 'derived' itself\nstdout: {stdout}"
    );
}

/// `base+` preserves the trailing `+` downstream operator through entity
/// resolution → downstream `derived` is included (D-38).
#[test]
fn plus_suffix_resolves_then_reattaches() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_suffix");

    let output = run_dry(&root, "base+");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select base+ should succeed\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: base"),
        "--select base+ should include 'base'\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select base+ (downstream) should include downstream 'derived'\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// `+base+` — both upstream and downstream operators are preserved through
/// resolution: the whole graph is selected (D-38).
#[test]
fn plus_both_resolves_and_traverses_all() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "plus_both");

    let output = run_dry(&root, "+base+");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--select +base+ should succeed\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: base"),
        "--select +base+ should include 'base'\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: derived"),
        "--select +base+ should include downstream 'derived'\nstdout: {stdout}\nstderr: {stderr}"
    );
}

// ── D-37: entity-name selector not found → hard error; method selectors
//          that match nothing → exit 0 no-op ──────────────────────────────────

/// `--select typo_name` where `typo_name` resolves to no entity → non-zero
/// exit with "not found" in stderr.  A typo'd model name must fail loudly.
/// (`cli.md` §"No-op vs unresolvable selector"; `model_selection.md` Constraint 4)
#[test]
fn unresolvable_entity_select_is_hard_error() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "entity_notfound");

    let output = run_dry(&root, "definitely_does_not_exist_typo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "--select <unknown entity> should exit non-zero\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stderr.contains("not found") || stderr.contains("definitely_does_not_exist_typo"),
        "stderr should contain 'not found' or the unknown name; got:\n{stderr}"
    );
}

/// `--select tag:nonexistent` — a `tag:` selector that matches no models is a
/// *valid empty selection* (Constraint 4 of `model_selection.md`): exit 0 with
/// a "no models matched" message to stderr.
#[test]
fn empty_tag_selection_is_noop_exit_0() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "tag_noop");

    let output = run_dry(&root, "tag:definitely_nonexistent_tag");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "--select tag:<nonexistent> should exit 0 (no-op)\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stderr.contains("no models matched"),
        "stderr should contain 'no models matched'; got:\n{stderr}"
    );
}

/// `--select generator_file:<missing path>` — a `generator_file:` selector
/// pointing at a non-existent file is a valid empty selection: exit 0, no
/// error diagnostic.
#[test]
fn generator_file_no_match_is_noop() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "gen_file_noop");

    let output = run_dry(&root, "generator_file:/nonexistent/path/generator.sql");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "--select generator_file:<missing> should exit 0 (no-op)\nstderr: {stderr}\nstdout: {stdout}"
    );
    // No hard error expected — the selection is valid but empty.
    assert!(
        !stderr.contains("error") && !stderr.contains("Error"),
        "stderr should not contain an error message; got:\n{stderr}"
    );
}

/// Bare leaf `events_parsed` with no scope, where two models in different
/// directories share that leaf name → non-zero ambiguity diagnostic listing
/// both full paths.
#[test]
fn bare_leaf_ambiguous_is_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("ambiguous_leaf");
    fs::create_dir_all(root.join("models/alpha")).unwrap();
    fs::create_dir_all(root.join("models/beta")).unwrap();
    write_smelt_yml(&root, "ambiguous_leaf");
    // Two models with the same leaf name in different sub-directories.
    fs::write(
        root.join("models/alpha/events_parsed.sql"),
        "SELECT 1 AS id\n",
    )
    .unwrap();
    fs::write(
        root.join("models/beta/events_parsed.sql"),
        "SELECT 2 AS id\n",
    )
    .unwrap();

    let output = run_dry(&root, "events_parsed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "bare ambiguous leaf 'events_parsed' should exit non-zero\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stderr.contains("events_parsed"),
        "stderr should mention the ambiguous name; got:\n{stderr}"
    );
    // Both full paths should appear in the error.
    assert!(
        stderr.contains("alpha.events_parsed") && stderr.contains("beta.events_parsed"),
        "stderr should list ALL matching candidates; got:\n{stderr}"
    );
}

/// Bare leaf `events_parsed` with a single match `silver.events_parsed` →
/// non-zero exit with a "did you mean" hint naming the full path.
/// (`cli.md` §"Argument resolution algorithm" step 4)
#[test]
fn unresolvable_entity_select_did_you_mean_hint() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("did_you_mean");
    fs::create_dir_all(root.join("models/silver")).unwrap();
    write_smelt_yml(&root, "did_you_mean");
    // One model inside a subdirectory; full path is `silver.events_parsed`.
    fs::write(
        root.join("models/silver/events_parsed.sql"),
        "SELECT 1 AS id\n",
    )
    .unwrap();

    // `events_parsed` (bare leaf) has no exact match; the only entity that
    // matches by leaf is `silver.events_parsed` → NotFound with a single hint.
    let output = run_dry(&root, "events_parsed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "--select events_parsed (bare leaf, one subdirectory match) should exit non-zero\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stderr.to_lowercase().contains("did you mean"),
        "stderr should contain a 'did you mean' hint; got:\n{stderr}"
    );
    assert!(
        stderr.contains("silver.events_parsed"),
        "stderr 'did you mean' hint should include the full path 'silver.events_parsed'; got:\n{stderr}"
    );
}

// ── D-39: `--exclude +model` inconsistent-set refusal ────────────────────────

fn run_dry_exclude(project_dir: &Path, exclude: &str) -> std::process::Output {
    Command::new(smelt_bin())
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(["--exclude", exclude, "--dry-run"])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt binary should be runnable")
}

/// Workspace: `shared_upstream` (leaf), `needs_upstream` (→ shared_upstream),
/// `also_needs` (→ shared_upstream).
/// `--exclude +needs_upstream` expands to {needs_upstream, shared_upstream}.
/// `also_needs` is retained but requires `shared_upstream` (excluded) → error.
#[test]
fn exclude_upstream_needed_by_retained_is_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("excl_inconsistent");
    fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, "excl_inconsistent");
    fs::write(
        root.join("models").join("shared_upstream.sql"),
        "SELECT 1 AS id\n",
    )
    .unwrap();
    fs::write(
        root.join("models").join("needs_upstream.sql"),
        "SELECT id FROM smelt.shared_upstream\n",
    )
    .unwrap();
    fs::write(
        root.join("models").join("also_needs.sql"),
        "SELECT id FROM smelt.shared_upstream\n",
    )
    .unwrap();

    // +needs_upstream expands to {needs_upstream, shared_upstream}; also_needs
    // is retained but needs shared_upstream which is now excluded → error.
    let output = run_dry_exclude(&root, "+needs_upstream");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "--exclude +needs_upstream with a retained model that needs the excluded upstream \
         should exit non-zero\nstderr: {stderr}\nstdout: {stdout}"
    );
    // The error must name the retained model and the missing upstream.
    assert!(
        stderr.contains("also_needs") || stderr.contains("Inconsistent"),
        "stderr should mention the retained model 'also_needs' or 'Inconsistent'; got:\n{stderr}"
    );
    assert!(
        stderr.contains("shared_upstream"),
        "stderr should mention the missing upstream 'shared_upstream'; got:\n{stderr}"
    );
}

/// Workspace: `shared_upstream` (leaf), `needs_upstream` (→ shared_upstream),
/// `independent` (no deps).
/// `--exclude +needs_upstream` expands to {needs_upstream, shared_upstream};
/// `independent` is retained and has no upstream deps → consistent → OK.
#[test]
fn exclude_upstream_not_needed_ok() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("excl_consistent");
    fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, "excl_consistent");
    fs::write(
        root.join("models").join("shared_upstream.sql"),
        "SELECT 1 AS id\n",
    )
    .unwrap();
    fs::write(
        root.join("models").join("needs_upstream.sql"),
        "SELECT id FROM smelt.shared_upstream\n",
    )
    .unwrap();
    fs::write(
        root.join("models").join("independent.sql"),
        "SELECT 42 AS x\n",
    )
    .unwrap();

    // +needs_upstream expands to {needs_upstream, shared_upstream}; independent
    // remains and has no deps that were excluded → consistent → exit 0.
    let output = run_dry_exclude(&root, "+needs_upstream");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "--exclude +needs_upstream with only independent models retained should succeed\n\
         stderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: independent"),
        "independent model should still be in the dry-run output\nstdout: {stdout}"
    );
}

/// Workspace: `leaf` (no deps), `downstream` (→ leaf), `unrelated` (no deps).
/// `--exclude downstream` (bare, no `+`) removes only `downstream`, not `leaf`.
/// `leaf` and `unrelated` are retained and have no unmet deps → consistent.
#[test]
fn exclude_bare_model_only() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("excl_bare");
    fs::create_dir_all(root.join("models")).unwrap();
    write_smelt_yml(&root, "excl_bare");
    fs::write(root.join("models").join("leaf.sql"), "SELECT 1 AS id\n").unwrap();
    fs::write(
        root.join("models").join("downstream.sql"),
        "SELECT id FROM smelt.leaf\n",
    )
    .unwrap();
    fs::write(
        root.join("models").join("unrelated.sql"),
        "SELECT 99 AS z\n",
    )
    .unwrap();

    // Bare --exclude downstream: only downstream is removed, leaf survives.
    // No retained model has a missing upstream → consistent → exit 0.
    let output = run_dry_exclude(&root, "downstream");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "--exclude downstream (bare, no +) should succeed: leaf and unrelated survive \
         with no unmet deps\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: leaf"),
        "leaf should still be in the dry-run output\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("Would run: unrelated"),
        "unrelated should still be in the dry-run output\nstdout: {stdout}"
    );
    assert!(
        !stdout.contains("Would run: downstream"),
        "downstream was excluded and must not appear\nstdout: {stdout}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────

/// `path:` is NOT a recognised selection method — a `path:models/silver`
/// selector is treated as a model-name reference that fails to resolve,
/// confirming no `path:` method was added (D-38).
#[test]
fn no_path_method_path_colon_errors_on_resolution() {
    let tmp = TempDir::new().unwrap();
    let root = stage_base_derived(&tmp, "no_path_method");

    let output = run_dry(&root, "path:models/silver");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "path:models/silver should fail — not a recognised method (treated as model name which does not exist)\nstderr: {stderr}\nstdout: {stdout}"
    );
    // The error should be a resolution failure, not a silent "0 models matched".
    assert!(
        stderr.contains("path:models/silver") || stderr.contains("not found"),
        "stderr should indicate the unresolvable selector; got:\n{stderr}"
    );
}
