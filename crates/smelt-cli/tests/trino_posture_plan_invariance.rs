//! `docs/outcomes/20260913-trino-ledger/outcome.md` phase 9: the
//! optionality rule (`docs/specs/state.md` §"The optionality rule") says
//! `state.mode` changes what a posture can *tell you*, never what the
//! maintenance plan *computes*. This proves it for the incremental shapes
//! that cannot yet complete a live run on Trino (phase 6's summary — no
//! `MaintenanceDialect` variant): `smelt explain <model> --json`'s cells
//! are byte-identical under `state.mode: intervals` and `state.mode:
//! stateless` for both a `grain: key` and a `grain: partition` model.
//!
//! Offline — `smelt explain` never opens a live connection, so a
//! placeholder `trino` target (no real coordinator) is enough. Model
//! bodies reused verbatim from `trino_explain_downgrade.rs`'s
//! `stage_trino_four_shapes_project` fixture: `lifetime_spend` (`grain:
//! key`, `Technique::KeyedFold`) and `cs_merged` (`grain: partition`,
//! `Technique::ColumnScopedMerge`) are the two shapes whose ideal
//! technique actually requires a state structure Trino cannot supply — a
//! plain `grain: partition` cell with no key-addressed edge needs nothing
//! and so carries no `state_downgrade` at all (`docs/specs/state.md`
//! §"The degradation contract"), which would make test 6 vacuous.

use std::process::Command;

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

const CS_FACT_SOURCE: &str = "description: column-scoped-merge fact source.\n\
    mutation_profile: append_only\n\
    timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
    columns:\n\
    - name: d\n  type: DATE\n\
    - name: id\n  type: INTEGER\n\
    - name: val\n  type: INTEGER\n";

const CS_DIM_SOURCE: &str = "description: column-scoped-merge mutable dimension.\n\
    mutation_profile: mutable_snapshot\nunique_key: [id]\n\
    columns:\n\
    - name: id\n  type: INTEGER\n\
    - name: attr\n  type: INTEGER\n";

const CS_MERGED_MODEL_SQL: &str =
    "---\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
     refresh: incremental\ngrain: partition\n\
     maintenance:\n  scan_bounds:\n    per_source:\n      cs_dim:\n        \
     allow_full_scan: true\n---\n\
     SELECT f.d AS d, f.id AS id, f.val AS val, dim.attr AS attr\n\
     FROM smelt.sources.cs_fact f LEFT JOIN smelt.sources.cs_dim dim ON f.id = dim.id\n";

fn smelt_yml(state_mode: &str) -> String {
    format!(
        "name: trino_posture_plan_invariance_fixture\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: trino\n    host: trino.internal\n    port: 8080\n    \
         user: smelt\n    catalog: iceberg\n    schema: main\n\
         default_materialization: table\n\
         models:\n  cs_merged:\n    merge_key: [id]\n\
         state:\n  mode: {state_mode}\n"
    )
}

/// Stage a project carrying both incremental shapes whose ideal technique
/// requires an unrealisable Trino state structure (`lifetime_spend` —
/// `grain: key`, `Technique::KeyedFold`; `cs_merged` — `grain: partition`,
/// `Technique::ColumnScopedMerge`), under the given `state.mode`.
fn stage_project(state_mode: &str) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), smelt_yml(state_mode)).unwrap();
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
    std::fs::write(
        tmp.path().join("models/sources/cs_fact.yml"),
        CS_FACT_SOURCE,
    )
    .unwrap();
    std::fs::write(tmp.path().join("models/sources/cs_dim.yml"), CS_DIM_SOURCE).unwrap();
    std::fs::write(tmp.path().join("models/cs_merged.sql"), CS_MERGED_MODEL_SQL).unwrap();
    tmp
}

fn explain_json(project_dir: &std::path::Path, model: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg(model)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("spawn smelt explain {model} --json: {e}"));
    assert!(
        output.status.success(),
        "smelt explain {model} --json must exit 0: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("{model}: explain --json output must parse: {e}\n{stdout}"))
}

/// Test 5: `smelt explain --json`'s cells are byte-identical across
/// `state.mode: intervals` and `state.mode: stateless`, for both
/// incremental shapes.
#[test]
fn maintenance_plan_is_byte_identical_across_state_modes_on_trino() {
    let intervals_tmp = stage_project("intervals");
    let stateless_tmp = stage_project("stateless");

    for model in ["lifetime_spend", "cs_merged"] {
        let intervals_json = explain_json(intervals_tmp.path(), model);
        let stateless_json = explain_json(stateless_tmp.path(), model);
        assert_eq!(
            intervals_json["cells"], stateless_json["cells"],
            "{model}: maintenance plan cells must be identical under state.mode: intervals \
             and state.mode: stateless"
        );
    }
}

/// Test 6: the fixture actually carries at least one downgraded cell in
/// each shape, so test 5 cannot pass vacuously over a plan with no
/// state-dependent cells in it.
#[test]
fn the_fixture_actually_carries_downgraded_cells() {
    let tmp = stage_project("intervals");

    for (model, expected_original) in [
        ("lifetime_spend", "KeyedFold"),
        ("cs_merged", "ColumnScopedMerge"),
    ] {
        let json = explain_json(tmp.path(), model);
        let cells = json["cells"]
            .as_array()
            .unwrap_or_else(|| panic!("{model}: expected a cells array: {json}"));
        let downgraded = cells
            .iter()
            .find(|c| c.get("state_downgrade").is_some())
            .unwrap_or_else(|| panic!("{model}: expected a cell carrying state_downgrade: {json}"));
        let original = downgraded["state_downgrade"]["original"]
            .as_str()
            .unwrap_or_else(|| panic!("{model}: state_downgrade.original must be a string"));
        assert_eq!(
            original, expected_original,
            "{model}: unexpected ideal technique: {json}"
        );
    }
}
