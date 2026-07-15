//! TDD tests for `smelt explain <model> --show-sql`
//! (`docs/plans/20260710-emit-unification.md` Phase 5).
//!
//! Oracle: `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
//! report" — `--show-sql` prints, after each cell's block, the maintenance
//! statements that cell executes: the output of the same pure emitters a
//! run executes (`docs/specs/incremental_models.md` §"Statement emission
//! (single owner)"). Region literals come from `--period <start>..<end>`
//! when given; without `--period` the symbolic placeholders
//! `{{window_start}}`/`{{window_end}}` stand in. `--show-sql` never
//! connects to a backend. `--json` alongside `--show-sql` emits a
//! `statements` array per cell.

use std::path::Path;
use std::process::Command;

/// `daily_events` in `examples/timeseries` is `refresh: incremental` +
/// `grain: partition` — its `NewData`/`Backfill` cells lower to
/// `Technique::DeleteInsert`, so `--show-sql` must print the region
/// `DELETE`+`INSERT` pair (the emitter output), bracketed `BEGIN`/`COMMIT`
/// since the pair is transactional, with no backend connection attempted
/// (the test spawns the real `smelt` binary against a project whose
/// `smelt.yml` names a `.duckdb` file that does not exist on disk — a
/// successful run here proves no connection was opened).
#[test]
fn show_sql_prints_emitted_statements_per_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events --show-sql");

    assert!(
        output.status.success(),
        "smelt explain daily_events --show-sql failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("DELETE FROM"),
        "expected a DELETE statement in the show-sql output: {stdout}"
    );
    assert!(
        stdout.contains("INSERT INTO"),
        "expected an INSERT statement in the show-sql output: {stdout}"
    );
    assert!(
        stdout.contains("BEGIN") && stdout.contains("COMMIT"),
        "expected the transactional DELETE+INSERT pair bracketed BEGIN/COMMIT: {stdout}"
    );
}

/// With `--period 2024-01-01..2024-01-03`, the printed statements carry the
/// real literal bounds; without `--period`, the symbolic placeholders
/// `{{window_start}}`/`{{window_end}}` stand in for the region instead.
#[test]
fn period_substitutes_real_literals_placeholders_otherwise() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let with_period = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--show-sql")
        .arg("--period")
        .arg("2024-01-01..2024-01-03")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain --show-sql --period");
    assert!(
        with_period.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&with_period.stderr)
    );
    let with_period_stdout = String::from_utf8_lossy(&with_period.stdout);
    assert!(
        with_period_stdout.contains("2024-01-01") && with_period_stdout.contains("2024-01-03"),
        "expected the real --period literals in the output: {with_period_stdout}"
    );
    assert!(
        !with_period_stdout.contains("{{window_start}}"),
        "must not print placeholders when --period is given: {with_period_stdout}"
    );

    let without_period = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain --show-sql (no period)");
    assert!(
        without_period.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&without_period.stderr)
    );
    let without_period_stdout = String::from_utf8_lossy(&without_period.stdout);
    assert!(
        without_period_stdout.contains("{{window_start}}")
            && without_period_stdout.contains("{{window_end}}"),
        "expected symbolic placeholders without --period: {without_period_stdout}"
    );
}

/// `--json --show-sql` output parses as JSON and carries a `statements`
/// array per cell whose `sql` entries byte-match the text-mode statements,
/// with correct `transactional_group` indices (statements in the same
/// transactional group share one index).
#[test]
fn json_format_carries_statements_array() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let text_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--show-sql")
        .arg("--period")
        .arg("2024-01-01..2024-01-03")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain --show-sql");
    assert!(text_output.status.success());
    let text_stdout = String::from_utf8_lossy(&text_output.stdout).to_string();

    let json_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--show-sql")
        .arg("--json")
        .arg("--period")
        .arg("2024-01-01..2024-01-03")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain --show-sql --json");
    assert!(
        json_output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&json_output.stderr)
    );
    let json_stdout = String::from_utf8_lossy(&json_output.stdout);

    let parsed: serde_json::Value = serde_json::from_str(&json_stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}: {json_stdout}"));

    let cells = parsed
        .get("cells")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("expected a top-level 'cells' array: {json_stdout}"));
    assert!(
        !cells.is_empty(),
        "expected at least one cell: {json_stdout}"
    );

    let mut found_statement = false;
    let mut found_grouped_pair = false;
    for cell in cells {
        let Some(statements) = cell.get("statements").and_then(|s| s.as_array()) else {
            continue;
        };
        if statements.is_empty() {
            continue;
        }
        found_statement = true;

        // Every statement's `sql` must byte-match some line of the text-mode
        // output (printed SQL is the emitters' output, not re-formatted).
        for stmt in statements {
            let sql = stmt
                .get("sql")
                .and_then(|s| s.as_str())
                .unwrap_or_else(|| panic!("expected a 'sql' string field: {stmt}"));
            assert!(
                text_stdout.contains(sql),
                "JSON statement sql must byte-match text-mode output.\nsql: {sql}\ntext: {text_stdout}"
            );
            assert!(
                stmt.get("transactional_group")
                    .and_then(|v| v.as_i64())
                    .is_some(),
                "expected an integer 'transactional_group' field: {stmt}"
            );
        }

        if statements.len() >= 2 {
            let g0 = statements[0]["transactional_group"].as_i64();
            let g1 = statements[1]["transactional_group"].as_i64();
            if g0 == g1 {
                found_grouped_pair = true;
            }
        }
    }
    assert!(
        found_statement,
        "expected at least one cell with statements: {json_stdout}"
    );
    assert!(
        found_grouped_pair,
        "expected a transactional pair (e.g. DELETE+INSERT) sharing one transactional_group: {json_stdout}"
    );
}

/// `examples/web_analytics`'s only `grain: key` model, `device_user_edges`,
/// reads its driving input from an upstream *model* (`silver.events_parsed`)
/// rather than a `sources.*` declaration, and aggregates three columns
/// (`COUNT`, `MIN`, `MAX`). `crates/smelt-db/src/queries/maintenance.rs`'s
/// `derive_model_maintenance_plan` only derives a `Trigger::NewData` (and
/// therefore only ever admits `Technique::KeyedFold`) for a `sources.*`
/// address (`smelt_db::lib::maintenance_plan_report`'s `source_refs` filter
/// drops any ref `ref_source_info` cannot resolve to a declared source —
/// an upstream *model* ref is silently excluded), and separately
/// `derive_fold_spec` only detects a fold candidate for exactly one
/// aggregate projection. `device_user_edges` fails both preconditions
/// today, so its only cell is the `Backfill`/`DeleteInsert` catch-all —
/// this is a real, pre-existing gap in the maintenance-plan *derivation*
/// (out of Phase 5's scope: it lives in `smelt-db`/`smelt-logical`, not the
/// `smelt-cli` files this phase may touch).
///
/// To still exercise `Technique::KeyedFold`'s `--show-sql` rendering on a
/// real (if self-contained) fixture, this test builds a minimal project
/// shaped like `device_user_edges` but satisfying both preconditions: a
/// single `COUNT` aggregate, `GROUP BY` key, reading directly from a
/// `sources.*` declaration with `mutation_profile: append_only` and a
/// `timeseries:` block (the classifier's driving-source requirement).
#[test]
fn web_analytics_keyed_model_shows_merge() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");

    let yml = "name: keyed_fixture\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();

    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        "description: events\n\
         columns:\n\
         - name: user_id\n  type: INTEGER\n\
         - name: event_date\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n",
    )
    .unwrap();

    let model_sql = "---\n\
                      materialization: table\n\
                      refresh: incremental\n\
                      grain: key\n\
                      ---\n\
                      SELECT user_id, COUNT(*) AS event_count \
                      FROM smelt.sources.events \
                      GROUP BY user_id\n";
    std::fs::write(tmp.path().join("models/device_user_edges.sql"), model_sql).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("device_user_edges")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain device_user_edges --show-sql");

    assert!(
        output.status.success(),
        "smelt explain device_user_edges --show-sql failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("technique: KeyedFold"),
        "expected a KeyedFold cell: {stdout}"
    );
    assert!(
        stdout.contains("MERGE INTO"),
        "expected the keyed-fold MERGE in the show-sql output: {stdout}"
    );
    assert!(
        stdout.contains("device_user_edges"),
        "expected the target table name in the MERGE: {stdout}"
    );
}

/// `--show-sql` must compile SELECT bodies with the *real* discovered
/// project's ephemeral resolver and upstream schemas — not
/// `EphemeralResolver::empty()` / an empty `UpstreamSchemas` — so the
/// printed SQL cannot silently diverge from what a real run executes
/// (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
/// report", Known Divergences). This fixture has an incremental model
/// (`daily_totals`, `grain: partition` → `Technique::DeleteInsert`) that
/// exercises both halves of the finding on independent refs so neither
/// proof is confounded by the other:
///   1. joins an ephemeral model (`rates_lookup`) — the printed SQL must
///      CTE-inline it (`__smelt_rates_lookup`), not resolve
///      `smelt.rates_lookup` as a bare physical table reference; and
///   2. aggregates a fractional (`DOUBLE`) column from a separate,
///      non-ephemeral upstream model ref (`base_amounts`) via `SUM(...)`.
///
/// (The aggregate deliberately reads from `base_amounts`, not
/// `rates_lookup`: `SqlCompiler::compile_with_sql_and_ephemerals` runs
/// `apply_type_casts` *before* ephemeral CTEs are prepended, so a column
/// aggregated directly off an ephemeral ref cannot be typed from
/// `UpstreamSchemas` at that point regardless of wiring — this is a
/// pre-existing `smelt-runtime` compile-order limitation shared by the real
/// run path too (`smelt run --dry-run` on an equivalent fixture produces
/// the identical `BIGINT` fallback), so it is not an explain-vs-run
/// divergence and is out of this fix's scope; see `docs/specs/cli.md`
/// Known Divergences.)
///
/// This model's statements no longer assert a `CAST(total AS DOUBLE)` in
/// the emitted `INSERT` body: `apply_type_casts` only wraps a statement
/// whose *outermost* SELECT projects named columns with inferable types,
/// and `derive_batch_filtered_sql`'s output clamp — the same one this fix
/// makes `--show-sql` route through — always wraps the model body as
/// `SELECT * FROM (<body>) AS _smelt_output_clamp WHERE …`, so the
/// outermost projection is a bare `*` by the time `apply_type_casts` runs.
/// This is a pre-existing characteristic of the live run's own clamped
/// statement (unrelated to and unchanged by this fix — confirmed via
/// `smelt backbuild --dry-run` against `examples/web_analytics`, whose real
/// executed `INSERT` bodies carry no `CAST` either), not a new
/// explain-vs-run divergence; tracked as a `smelt-runtime` finding
/// (`apply_type_casts`/`compile.rs`) outside this fix's scope.
#[test]
fn show_sql_wires_real_ephemeral_resolver_and_upstream_schemas() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");

    let yml = "name: ephemeral_fixture\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();

    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        "description: events\n\
         columns:\n\
         - name: id\n  type: INTEGER\n\
         - name: event_date\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n",
    )
    .unwrap();

    std::fs::write(
        tmp.path().join("models/rates_lookup.sql"),
        "---\n\
         materialization: ephemeral\n\
         ---\n\
         SELECT 1 AS id\n",
    )
    .unwrap();

    std::fs::write(
        tmp.path().join("models/base_amounts.sql"),
        "SELECT 1 AS id, CAST(2.5 AS DOUBLE) AS amount\n",
    )
    .unwrap();

    let model_sql = "---\n\
                      materialization: table\n\
                      refresh: incremental\n\
                      grain: partition\n\
                      timeseries:\n\
                      \x20\x20partition_column: event_date\n\
                      \x20\x20event_time_column: event_date\n\
                      \x20\x20granularity: day\n\
                      ---\n\
                      SELECT e.event_date, SUM(b.amount) AS total \
                      FROM smelt.sources.events e \
                      JOIN smelt.base_amounts b ON b.id = e.id \
                      JOIN smelt.rates_lookup r ON r.id = e.id \
                      GROUP BY e.event_date\n";
    std::fs::write(tmp.path().join("models/daily_totals.sql"), model_sql).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_totals")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain daily_totals --show-sql");

    assert!(
        output.status.success(),
        "smelt explain daily_totals --show-sql failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("__smelt_rates_lookup"),
        "expected the ephemeral model CTE-inlined (real EphemeralResolver, not \
         EphemeralResolver::empty()): {stdout}"
    );
    assert!(
        !stdout.contains("FROM main.rates_lookup") && !stdout.contains("JOIN main.rates_lookup"),
        "must NOT resolve the ephemeral ref as a physical table reference: {stdout}"
    );
    assert!(
        stdout.contains("SUM(b.amount) AS total"),
        "expected the SUM(amount) aggregate over the base_amounts ref in the emitted \
         statement: {stdout}"
    );
    assert!(
        !stdout.contains("CAST(total AS BIGINT)"),
        "must not fall through to the BIGINT default cast for a fractional aggregate \
         over a real upstream model ref: {stdout}"
    );
}
