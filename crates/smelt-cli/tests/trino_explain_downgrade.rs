//! `docs/outcomes/20260913-trino-ledger/outcome.md` phase 4: `smelt explain`
//! must not abort on a `trino` target the way it did before this phase
//! (`smelt_backend::maintenance_dialect(SqlDialect::Trino)` returning `Err`
//! used to `?`-propagate straight out of the command). Availability
//! resolution and the maintenance-plan report are independent of the
//! maintenance-statement dialect; only the per-cell statement text
//! (`--show-sql`) is unavailable, and it must name the gap rather than
//! silently print nothing or fall back to another dialect's spelling.
//!
//! Offline only — `smelt explain` never opens a live connection, so a
//! placeholder `trino` target (no real coordinator) is enough.

use std::process::Command;

/// Phase 5 test 7 (`docs/outcomes/20260913-trino-ledger/outcome.md`): the
/// per-model `merge_key:` override `cs_merged`'s column-scoped-merge cell
/// needs (`smelt_maintenance_testkit::recipe::ValueEnrichedRecipe::model_file`'s
/// own doc comment: the column-scoped `MERGE`'s `ON`-predicate key is not
/// declarable in `.sql` frontmatter under `grain: partition`).
const FOUR_SHAPES_SMELT_YML: &str = "name: trino_four_shapes_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: trino\n    host: trino.internal\n    port: 8080\n    \
    user: smelt\n    catalog: iceberg\n    schema: main\n\
    default_materialization: table\n\
    models:\n  cs_merged:\n    merge_key: [id]\n";

const SMELT_YML: &str = "name: trino_explain_downgrade_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: trino\n    host: trino.internal\n    port: 8080\n    \
    user: smelt\n    catalog: iceberg\n    schema: main\n\
    default_materialization: table\n";

const PAYMENTS_SOURCE: &str = "description: payments\n\
    mutation_profile: append_only\n\
    timeseries:\n  event_time_column: pay_date\n  partition_column: pay_date\n  granularity: day\n\
    columns:\n\
    - name: user_id\n  type: INTEGER\n\
    - name: pay_date\n  type: DATE\n\
    - name: amount\n  type: DOUBLE\n";

const KEYED_FOLD_MODEL_SQL: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
     SELECT user_id, SUM(amount) AS lifetime_spend\n\
     FROM smelt.sources.payments\nGROUP BY user_id\n";

const REGION_MODEL_SQL: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
     timeseries:\n  partition_column: pay_date\n  event_time_column: pay_date\n  \
     granularity: day\n---\n\
     SELECT user_id, pay_date, amount FROM smelt.sources.payments\n";

/// A `refresh: incremental` / `grain: key` model (`Technique::KeyedFold`,
/// which needs the reconciliation ledger) on a `trino` target — Trino
/// realises no engine-resident state structure, so this cell must downgrade
/// to `PerGroupRecompute` (`docs/specs/state.md` §"The degradation
/// contract").
fn stage_trino_keyed_fold_project() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        KEYED_FOLD_MODEL_SQL,
    )
    .unwrap();
    tmp
}

/// `smelt explain <model> --json` on a `trino` target exits 0 (never
/// aborts on the missing `MaintenanceDialect`) and its JSON carries a
/// `state_downgrade` whose `original` names the ideal (never-run)
/// technique — the ideal plan stays derived and visible even though it
/// cannot run.
#[test]
fn explain_on_a_trino_target_reports_instead_of_aborting() {
    let tmp = stage_trino_keyed_fold_project();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend --json");

    assert!(
        output.status.success(),
        "smelt explain must exit 0 on a trino target, not abort: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("explain --json output must parse: {e}\n{stdout}"));

    let cells = json["cells"].as_array().expect("cells array");
    let downgraded = cells
        .iter()
        .find(|c| c.get("state_downgrade").is_some())
        .unwrap_or_else(|| panic!("expected a cell carrying state_downgrade: {stdout}"));
    let downgrade = &downgraded["state_downgrade"];
    let original = downgrade["original"]
        .as_str()
        .expect("state_downgrade.original must be a string");
    let executed = downgraded["technique"]
        .as_str()
        .expect("cell must carry the executed technique");
    assert_ne!(
        original, executed,
        "original must name the ideal technique, not the one actually executed: {stdout}"
    );
    assert_eq!(original, "KeyedFold");
    assert_eq!(executed, "PerGroupRecompute");
}

/// `smelt explain <model> --show-sql` on a `trino` target must not abort —
/// `20260913-trino-incremental` phase 3 landed `MaintenanceDialect::Trino`,
/// so the old "no maintenance-statement dialect for 'trino'" abort this
/// pinned no longer happens on any cell. `lifetime_spend`'s cells (`KeyedFold`
/// downgraded to `PerGroupRecompute`, and a `Backfill`-trigger `DeleteInsert`)
/// still print "no statements" here — a pre-existing, dialect-independent gap
/// in the technique-preview builder for this cell shape (it needs facts this
/// fixture's plan cell does not carry), not a Trino refusal — so the
/// assertion is narrowed to "no `Unsupported*Dialect` text", not "prints
/// real SQL".
#[test]
fn explain_show_sql_on_trino_does_not_abort() {
    let tmp = stage_trino_keyed_fold_project();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend --show-sql");

    assert!(
        output.status.success(),
        "smelt explain --show-sql must exit 0 on a trino target, not abort: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Unsupported"),
        "no maintenance-dialect refusal should appear now that Trino has one: {stdout}"
    );
}

/// `smelt rebuild <model> --dry-run` on a `trino` target: a `DeleteInsert`
/// (region-recompute) cell is dialect-invariant text over the shared
/// `emit_delete_insert` emitter, so once `MaintenanceDialect::Trino` exists
/// (`20260913-trino-incremental` phase 3) it renders real statements the
/// same as on any other dialect — no named gap, no silent `continue` past a
/// model whose maintenance statements could not be rendered
/// (`execute/project/dry_run.rs`'s old `let Ok(dialect) = ... else {
/// continue };`, which used to fire here before this phase).
#[test]
fn dry_run_on_trino_renders_delete_insert() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        REGION_MODEL_SQL,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("rebuild")
        .arg("lifetime_spend")
        .arg("--start")
        .arg("2026-01-01")
        .arg("--end")
        .arg("2026-01-02")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt rebuild lifetime_spend --dry-run");

    assert!(
        output.status.success(),
        "rebuild --dry-run must exit 0 on a trino target, not abort: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Would run: lifetime_spend"),
        "the model's compiled-SQL line must still print: {stdout}"
    );
    assert!(
        stdout.contains("DELETE FROM") && stdout.contains("INSERT INTO"),
        "the region DELETE+INSERT statements must actually render now that Trino has a \
         MaintenanceDialect: {stdout}"
    );
}

/// Phase 6 test 8 (`docs/outcomes/20260913-trino-ledger/phases/06-plan.md`):
/// `contract.deferral`'s refusal (`DeclaredContractRequiresState`) is a
/// `smelt-db` diagnostic, not a `smelt-logical` maintenance-plan refusal —
/// `smelt explain` builds the plan directly and never runs the diagnostic
/// gate, so it stays silent on a `contract.deferral`-declaring model even
/// on a `trino` target (compare `explain_on_a_trino_target_reports_
/// instead_of_aborting` above, whose `--json` cells carry no refusal at
/// all). `smelt rebuild --dry-run` DOES run the diagnostic-parity gate
/// (`docs/specs/architecture.md` §"Diagnostic range encoding" and
/// `execute/project/mod.rs`'s own "Diagnostic-parity gate (analysis ↔
/// build) — runs for dry_run too" comment), proving the refusal is not
/// diagnostics-only — it reaches the CLI boundary, non-zero exit, naming
/// the declaration and the backend.
#[test]
fn explain_on_trino_reports_the_deferral_refusal() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/region_deferred.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: pay_date\n  event_time_column: pay_date\n  \
         granularity: day\ncontract:\n  deferral: 1 day\n---\n\
         SELECT user_id, pay_date, amount FROM smelt.sources.payments\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("rebuild")
        .arg("region_deferred")
        .arg("--start")
        .arg("2026-01-01")
        .arg("--end")
        .arg("2026-01-02")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt rebuild region_deferred --dry-run");

    assert!(
        !output.status.success(),
        "a declared contract.deferral on a trino target (no reconciliation ledger) must refuse \
         the build, not silently proceed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("DeclaredContractRequiresState") && stderr.contains("contract.deferral"),
        "the refusal must name its diagnostic code and the declaration: {stderr}"
    );
    assert!(
        stderr.contains("trino"),
        "the refusal must name the backend that cannot supply the ledger: {stderr}"
    );
}

// =============================================================================
// Phase 5 (`docs/outcomes/20260913-trino-ledger/outcome.md`) test 7: one
// model per structure-bearing shape, staged into a single Trino-target
// project. Success criterion 4's "every cell that would have used a Trino
// correctness structure resolves to its recompute-family equivalent" checked
// at the CLI boundary across all four shapes, not just `lifetime_spend`'s
// keyed fold.
// =============================================================================

/// Stage a project with one model per structure-bearing shape, all reading
/// a `trino` target:
///
/// - `lifetime_spend` — `Technique::KeyedFold` (reconciliation ledger),
///   [`stage_trino_keyed_fold_project`]'s own fixture.
/// - `cs_merged` — `Technique::ColumnScopedMerge` (transactional merge
///   ledger): an append-only fact `LEFT JOIN`-enriched by a
///   `mutable_snapshot` dimension's payload column
///   (`smelt_maintenance_testkit::recipe::ValueEnrichedRecipe`'s shape).
/// - `dag_kchain_b` — a key-addressed `Technique::PerGroupRecompute`
///   (fingerprint sidecar): reads `dag_kchain_a`, a clockless keyed model,
///   as a model edge (`smelt_maintenance_testkit::dag::keyed_chain_dag`'s
///   shape).
/// - `customer_history` — `Technique::SuccessionPatch` (tombstone ledger):
///   `explain_maintenance::support::stage_succession_project`'s shape.
fn stage_trino_four_shapes_project() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), FOUR_SHAPES_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();

    // Keyed fold.
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        KEYED_FOLD_MODEL_SQL,
    )
    .unwrap();

    // Column-scoped merge.
    std::fs::write(
        tmp.path().join("models/sources/cs_fact.yml"),
        "description: column-scoped-merge fact source.\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/sources/cs_dim.yml"),
        "description: column-scoped-merge mutable dimension.\n\
         mutation_profile: mutable_snapshot\nunique_key: [id]\n\
         columns:\n\
         - name: id\n  type: INTEGER\n\
         - name: attr\n  type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/cs_merged.sql"),
        "---\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         refresh: incremental\ngrain: partition\n\
         maintenance:\n  scan_bounds:\n    per_source:\n      cs_dim:\n        \
         allow_full_scan: true\n---\n\
         SELECT f.d AS d, f.id AS id, f.val AS val, dim.attr AS attr\n\
         FROM smelt.sources.cs_fact f LEFT JOIN smelt.sources.cs_dim dim ON f.id = dim.id\n",
    )
    .unwrap();

    // Key-addressed per-group recompute.
    std::fs::write(
        tmp.path().join("models/sources/ka_events.yml"),
        "description: key-addressed chain source.\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/dag_kchain_a.sql"),
        "---\nrefresh: incremental\ngrain: key\n\
         maintenance:\n  scan_bounds:\n    per_source:\n      ka_events:\n        \
         allow_full_scan: true\n---\n\
         SELECT id, SUM(val) AS total FROM smelt.sources.ka_events GROUP BY id\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/dag_kchain_b.sql"),
        "---\nrefresh: incremental\ngrain: key\n---\n\
         SELECT id, ANY_VALUE(total) AS total FROM smelt.dag_kchain_a GROUP BY id\n",
    )
    .unwrap();

    // Succession.
    std::fs::write(
        tmp.path().join("models/sources/customer_changes.yml"),
        "description: customer change-event stream.\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: effective_ts\n  partition_column: ingested_date\n  \
         granularity: day\n\
         columns:\n\
         - name: customer_id\n  type: INTEGER\n\
         - name: effective_ts\n  type: TIMESTAMP\n\
         - name: region\n  type: VARCHAR\n\
         - name: ingested_date\n  type: DATE\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/customer_history.sql"),
        "---\nrefresh: incremental\n---\n\
         SELECT customer_id, effective_ts, region, \
         LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS valid_to\n\
         FROM smelt.sources.customer_changes\n",
    )
    .unwrap();

    tmp
}

/// `smelt explain <model> --json` on a `trino` target, for one model per
/// structure-bearing shape: exit 0, a `state_downgrade` naming the shape's
/// own ideal technique, and no `Unsupported*Dialect` refusal text anywhere
/// in the report.
#[test]
fn explain_on_trino_downgrades_every_structure_bearing_shape() {
    let tmp = stage_trino_four_shapes_project();

    for (model, expected_original) in [
        ("lifetime_spend", "KeyedFold"),
        ("cs_merged", "ColumnScopedMerge"),
        ("dag_kchain_b", "PerGroupRecompute"),
        ("customer_history", "SuccessionPatch"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
            .arg("explain")
            .arg(model)
            .arg("--json")
            .arg("--project-dir")
            .arg(tmp.path())
            .output()
            .unwrap_or_else(|e| panic!("spawn smelt explain {model} --json: {e}"));

        assert!(
            output.status.success(),
            "smelt explain {model} --json must exit 0 on a trino target, not abort: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("Unsupported"),
            "{model}: expected no Unsupported*Dialect refusal text in the JSON report: {stdout}"
        );
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{model}: explain --json output must parse: {e}\n{stdout}"));

        let cells = json["cells"]
            .as_array()
            .unwrap_or_else(|| panic!("{model}: expected a cells array: {stdout}"));
        let downgraded = cells
            .iter()
            .find(|c| c.get("state_downgrade").is_some())
            .unwrap_or_else(|| {
                panic!("{model}: expected a cell carrying state_downgrade: {stdout}")
            });
        let downgrade = &downgraded["state_downgrade"];
        let original = downgrade["original"]
            .as_str()
            .unwrap_or_else(|| panic!("{model}: state_downgrade.original must be a string"));
        assert_eq!(
            original, expected_original,
            "{model}: unexpected ideal technique: {stdout}"
        );
        let missing = downgrade["missing"]
            .as_str()
            .unwrap_or_else(|| panic!("{model}: state_downgrade.missing must be a string"));
        assert!(
            !missing.is_empty(),
            "{model}: state_downgrade.missing must name the missing structure: {stdout}"
        );
    }
}
