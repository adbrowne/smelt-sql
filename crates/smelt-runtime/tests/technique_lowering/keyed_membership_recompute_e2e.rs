/// `docs/plans/20260808-membership-sensitivity.md` Phase 2: the keyed run
/// path's own live membership-recompute dispatch — the `plan_is_keyed`
/// branch of `execute.rs` now ALSO consults
/// `resolve_live_membership_recompute_cell` alongside the existing
/// `resolve_live_column_scoped_cell` (W10 Phase 4, `column_scoped_merge_e2e`
/// above uses the latter for a `grain: partition`/`WholeRow`-identity
/// shape).
///
/// **The fixture shape this module is forced into, and why.** A `grain:
/// key` body must satisfy `classify_cumulative`'s aggregate/`GROUP BY`
/// grammar (`incremental_shapes.md` §"Key-grain declaration"): every
/// non-aggregate `SELECT` item must be a literal `GROUP BY` key. That
/// makes a mutable dimension's own attribute column unreachable as a plain
/// enrich-only payload group two ways at once — selecting it forces it
/// into `GROUP BY` (`maintenance::skeleton::skeleton_roles` classifies
/// every `GROUP BY` column `Grouping`-role, and
/// `maintenance::grouping::derive_column_groups` excludes every skeleton
/// column from column-group derivation entirely, so no cell can ever
/// mention it), while wrapping it in an aggregate makes it fold-contributing
/// (`source_contributes_to_fold`), which W10 Phase 3's narrowed
/// `derive_new_data` refuses outright for a mutable source (`both fold and
/// enrich stays refused`, the safety carve-out
/// `crates/smelt-logical/tests/maintenance_new_data_enrich_only_waiver.rs`
/// pins). The one shape that DOES reach a live cell today is a fold
/// aggregate whose own argument does not mention the dimension at all
/// (`COUNT(t.transaction_id)`, reading only the append-only fact) joined
/// against the mutable dimension purely for row admission.
///
/// **Why this cell is `Technique::DeleteInsert`, honestly, not by
/// accident.** Before `docs/plans/20260808-membership-sensitivity.md`, this
/// fixture's `{event_count}` cell reached `ColumnScopedMerge` only through a
/// pre-existing `maintenance::grouping` collector bug (a bare aggregate's
/// own function-name token misresolved as an ambiguous unqualified column
/// reference once 2+ sources are joined, fail-closed-collapsing sensitivity
/// onto every source including the dimension it never actually reads) — see
/// `docs/plans/20260808-membership-sensitivity.md`'s own "Context" section.
/// Phase 1 of that plan derives membership sensitivity directly from the
/// join's `ON t.user_id = u.user_id` predicate — a row-admission read of
/// `raw.users` — independent of that collector bug, so `{event_count}` is
/// now genuinely, legitimately membership-sensitive: deleting `raw.users`'
/// row for a user with staged transactions removes that user's whole group
/// from the join's admitted row set, something no column-scoped `MERGE`
/// (which only ever rewrites columns of rows that already match) can
/// repair. `Technique::DeleteInsert` (`Corner::RecomputeRegion`) is the
/// correct, honest verdict, and the module below now exercises the Phase 2
/// dispatch for it (`resolve_live_membership_recompute_cell` +
/// `execute_staged_membership_recompute`) rather than the retired
/// `ColumnScopedMerge` path.
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::{MutationProfile, SourceFacts};
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::maintenance_driver::{
    resolve_live_membership_recompute_cell, MembershipRecomputeWrite,
};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::{execute_project, NoOpReporter};
use tokio_util::sync::CancellationToken;

/// The keyed model body this whole module exercises: `raw.transactions`
/// (append-only, and — unlike `raw.events` — clocked via its OWN
/// source-YAML `timeseries:` block, the window-forward run shape's
/// admission precondition, `KeyedSnapshotPostureUnsupported` otherwise)
/// folded per `user_id` via `COUNT`, inner-joined to `raw.users`
/// (unclocked `mutation_profile: mutable_snapshot`, `allow_full_scan`
/// declared) purely for row admission — see the module doc comment
/// above for why the dimension's own attribute cannot itself be a
/// selected payload column today.
const MODEL_SQL: &str = "SELECT t.user_id AS user_id, COUNT(t.transaction_id) AS event_count \
     FROM smelt.sources.raw.transactions t \
     JOIN smelt.sources.raw.users u ON t.user_id = u.user_id \
     GROUP BY t.user_id";

const MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: user_id\n\
     maintenance:\n  \
       scan_bounds:\n    \
         per_source:\n      \
           raw.users:\n        \
             allow_full_scan: true\n      \
           raw.transactions:\n        \
             allow_full_scan: true\n\
     ---\n";

fn model_file_text() -> String {
    format!("{MODEL_FILE}{MODEL_SQL}\n")
}

/// Unit-level proof (no backend): `resolve_live_membership_recompute_cell`
/// — the exact resolver `execute.rs`'s `plan_is_keyed` branch now calls
/// alongside `resolve_live_column_scoped_cell` — resolves this model's
/// `raw.users` `UpstreamMutation` cell to `Technique::DeleteInsert` over
/// the proven `RowIdentity::Key(["user_id"])` (the declared
/// `unique_key`), with `WriteSuppression::Suppressed` (P3 comparability
/// holds for `event_count`, an INTEGER column, and this is a
/// steady-state trigger with no ledger catch-up — the suppressed arm is
/// preferred over unconditional per `choice::resolve_write_variant`).
#[test]
fn resolves_suppressed_membership_recompute_for_keyed_dimension_cell() {
    let text = model_file_text();
    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("single-model file");
    };
    let sql_body = &text[sql_offset..];

    let sources = vec![
        SourceFacts {
            name: "raw.transactions".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "raw.users".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
    ];
    let mut explicitly_mutable = HashSet::new();
    explicitly_mutable.insert("raw.users".to_string());

    let (source, cell, _group_columns, write) = resolve_live_membership_recompute_cell(
        sql_body,
        "user_lifetime_status",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolver must not error")
    .expect("a live membership-recompute cell must resolve for raw.users");

    assert_eq!(source, "raw.users");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::DeleteInsert
    );
    assert_eq!(
        cell.row_identity.identity,
        smelt_logical::maintenance::RowIdentity::Key(vec!["user_id".to_string()])
    );
    assert!(
        matches!(write, MembershipRecomputeWrite::StagedRecompute { .. }),
        "expected the change-suppressed matched arm, got {write:?}"
    );
}

/// `BackendFactory` that always opens the same on-disk DuckDB file,
/// mirroring `column_scoped_merge_e2e` above.
struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(Some(std::sync::Arc::from("dev")));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

/// `start`/`end` advance by one day per call — the windowed-keyed-
/// maintenance driver's reconciliation ledger refuses re-folding the
/// SAME partition twice (`docs/specs/incremental_models.md`
/// §"Reprocessing" — never-fold-twice), independent of whether the
/// column-scoped-merge dispatch this module tests fires; each of this
/// test's three runs therefore needs its own fresh day. No transaction
/// rows are staged for days after the first, so the fold contributes
/// nothing new on runs 2/3 — `event_count` stays exactly what run 1
/// computed throughout.
fn request_for_day(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["user_lifetime_status".to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

/// The real-fixture requirement: drive the model above through
/// `execute_project` itself (root `CLAUDE.md` §"Run pipeline parity
/// rule"). First run creates the target via the ordinary `KeyedFold`
/// creation path (the table doesn't exist yet — the creation run must
/// never take the membership-recompute path). A SECOND run over the
/// SAME window, with the table now present, must route through
/// `execute.rs`'s `plan_is_keyed` branch's new live-cell dispatch to
/// `Technique::DeleteInsert` (`RunOutcome.models["user_lifetime_
/// status"].strategy == "delete_insert_suppressed"`) — never the default
/// `cumulative_aggregate` fold label a plain keyed run would otherwise
/// report every time. A THIRD run (no data changes at all since the
/// second) must NOT dispatch the technique at all — mutation-happened
/// discrimination (`docs/specs/incremental_models.md` §"When a mutation
/// cell dispatches") recognizes `raw.users`'s fingerprint is unchanged
/// from the baseline run 2 recorded, so the cell is a no-op and the run
/// falls back to the ordinary `cumulative_aggregate` label — and,
/// independently, the staged-candidate recompute (were it to run) would
/// still find ZERO affected rows, read directly off DuckDB via the SAME
/// staged-candidate shape (`super::basic::staged_candidate_affected_row_counts`).
#[tokio::test]
async fn keyed_run_loop_dispatches_membership_recompute_through_execute_project() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    // `examples/timeseries/smelt.yml` declares no `state:` key, so it
    // defaults to `state.mode: stateless`; this fixture's third run
    // needs the source-mutation baseline run 2 recorded
    // (`docs/specs/state.md` §"`state.mode` and what each posture
    // provides").
    {
        let smelt_yml_path = project_dir.join("smelt.yml");
        let mut smelt_yml = std::fs::read_to_string(&smelt_yml_path).unwrap();
        smelt_yml.push_str("\nstate:\n  mode: intervals\n");
        std::fs::write(&smelt_yml_path, smelt_yml).unwrap();
    }
    std::fs::write(
        project_dir.join("models/user_lifetime_status.sql"),
        model_file_text(),
    )
    .expect("write keyed model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.clone(),
    };

    // Stage the two source tables `execute_project` reads (mirrors
    // `column_scoped_merge_e2e`'s own staging, but over `raw.transactions`
    // — clocked via its OWN source-YAML `timeseries:` block, unlike
    // `raw.events`, whose clock is only ever declared on a downstream
    // MODEL's frontmatter — this fixture declares none) — the CSV seed
    // loader is a separate CLI-level step `execute_project` itself does
    // not perform.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, \
                 user_id INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                 transaction_type VARCHAR)",
            )
            .await
            .expect("create transactions source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_transactions VALUES \
                 (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                 (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase')",
            )
            .await
            .expect("seed transactions");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
            )
            .await
            .expect("seed users");
    }

    // First run: creation. Must not take the membership-recompute path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-1".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_ne!(
            record.strategy, "delete_insert_suppressed",
            "the creation run must not take the membership-recompute path — the target \
             doesn't exist yet"
        );
    }

    // Mutate the dimension in place — mirrors `column_scoped_merge_e2e`'s
    // own narrative even though (per the module doc comment) this
    // fixture's merged column never actually depends on the mutated
    // value.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Second run: the live cell dispatches.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            request_for_day("2025-01-11", "2025-01-12"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(
            record.strategy, "delete_insert_suppressed",
            "a live UpstreamMutation cell must dispatch the keyed run loop through the \
             staged-candidate membership-recompute technique (`docs/plans/\
             20260808-membership-sensitivity.md` Phase 2), not the default cumulative-fold \
             label a plain keyed run would otherwise report"
        );
    }

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let (user1_count, user2_count): (i64, i64) = {
        let c1: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read user 1 event_count");
        let c2: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("read user 2 event_count");
        (c1, c2)
    };
    assert_eq!(
        (user1_count, user2_count),
        (1, 1),
        "the staged-candidate membership recompute must not corrupt the fact-derived \
         event_count — both users still show exactly the one event staged for them"
    );

    // Third run: no data change since run 2 at all. Mutation-happened
    // discrimination (`docs/specs/incremental_models.md` §"When a
    // mutation cell dispatches") now closes the divergence the comment
    // above used to document: the recorded baseline from run 2 matches
    // `raw.users`'s current fingerprint exactly, so the cell is a no-op
    // this run and the run falls back to the ordinary cumulative-fold
    // label — no `DELETE`+`INSERT` for this cell executes at all
    // (strictly stronger than run 2's own change-suppressed zero-row
    // write).
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-3".to_string(),
            request_for_day("2025-01-12", "2025-01-13"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("third run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(
            record.strategy, "cumulative_aggregate",
            "an unchanged source's UpstreamMutation cell must be a no-op, not re-dispatch"
        );
    }

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb for probe");
    let recompute_sql = MODEL_SQL
        .replace(
            "smelt.sources.raw.transactions",
            "main.sources_raw_transactions",
        )
        .replace("smelt.sources.raw.users", "main.sources_raw_users");
    let (deleted, inserted) = super::basic::staged_candidate_affected_row_counts(
        &backend,
        "main.user_lifetime_status",
        "__smelt_probe_zero_write",
        &["user_id"],
        &recompute_sql,
        &["event_count"],
    )
    .await;
    assert_eq!(
        (deleted, inserted),
        (0, 0),
        "an unchanged redelivery must write zero rows — the cell didn't dispatch at all this \
         run, so the staged-candidate recompute itself independently finds nothing to change \
         either"
    );
}

/// `write: diff_patch` pin over the region `DeleteInsert` default
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`,
/// test 8): RED before this phase — the pin was silently ignored
/// (`resolve_live_membership_recompute_cell` `continue`d past a
/// `ChosenTechnique::DiffPatch` choice) and the run fell through to the
/// default `cumulative_aggregate` fold label with no membership-recompute
/// dispatch at all. GREEN after: the run's manifest records
/// `strategy == "diff_patch"`, and the diff-then-patch write leaves an
/// unchanged row's `event_count` untouched (only the genuinely differing
/// rows are ever candidates for the update leg).
const MODEL_FILE_DIFF_PATCH: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: user_id\n\
     maintenance:\n  \
       scan_bounds:\n    \
         per_source:\n      \
           raw.users:\n        \
             allow_full_scan: true\n      \
           raw.transactions:\n        \
             allow_full_scan: true\n  \
       cells:\n    \
         - columns: [event_count]\n      \
           on: raw.users\n      \
           write: diff_patch\n\
     ---\n";

fn model_file_text_diff_patch() -> String {
    format!("{MODEL_FILE_DIFF_PATCH}{MODEL_SQL}\n")
}

#[tokio::test]
async fn diff_patch_pin_over_region_delete_insert_default_writes_only_the_difference() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    // `examples/timeseries/smelt.yml` declares no `state:` key, so it
    // defaults to `state.mode: stateless`; this fixture's third run
    // needs the source-mutation baseline run 2 recorded
    // (`docs/specs/state.md` §"`state.mode` and what each posture
    // provides").
    {
        let smelt_yml_path = project_dir.join("smelt.yml");
        let mut smelt_yml = std::fs::read_to_string(&smelt_yml_path).unwrap();
        smelt_yml.push_str("\nstate:\n  mode: intervals\n");
        std::fs::write(&smelt_yml_path, smelt_yml).unwrap();
    }
    std::fs::write(
        project_dir.join("models/user_lifetime_status.sql"),
        model_file_text_diff_patch(),
    )
    .expect("write diff_patch-pinned keyed model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.clone(),
    };

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, \
                 user_id INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                 transaction_type VARCHAR)",
            )
            .await
            .expect("create transactions source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_transactions VALUES \
                 (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                 (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase')",
            )
            .await
            .expect("seed transactions");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
            )
            .await
            .expect("seed users");
    }

    // First run: creation via the ordinary KeyedFold path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-1".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run must succeed");
    }

    // Mutate the dimension so the `raw.users` `UpstreamMutation` cell is
    // live for run 2 — same narrative as the sibling module test above.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Second run: the diff_patch pin must be enforced, not silently
    // ignored.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            request_for_day("2025-01-11", "2025-01-12"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(
            record.strategy, "diff_patch",
            "a `write: diff_patch` pin over the region DeleteInsert default must be \
             enforced, not silently ignored (RED before this phase: the run fell through \
             to the default `cumulative_aggregate` fold label)"
        );
    }

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let (user1_count, user2_count): (i64, i64) = {
        let c1: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read user 1 event_count");
        let c2: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("read user 2 event_count");
        (c1, c2)
    };
    assert_eq!(
        (user1_count, user2_count),
        (1, 1),
        "the diff_patch write must not corrupt the fact-derived event_count — both users \
         still show exactly the one event staged for them"
    );

    // Third run: no data change at all since run 2. Mutation-happened
    // discrimination (`docs/specs/incremental_models.md` §"When a
    // mutation cell dispatches") now recognizes `raw.users`'s
    // fingerprint is unchanged from the baseline run 2 recorded, so the
    // cell is a no-op and the `diff_patch` write never dispatches at
    // all this run — stronger than the diff-then-patch pattern's own
    // zero-affected-row leg, which never gets a chance to run.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-3".to_string(),
            request_for_day("2025-01-12", "2025-01-13"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("third run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(
            record.strategy, "cumulative_aggregate",
            "an unchanged source's UpstreamMutation cell must be a no-op, not re-dispatch"
        );
    }

    let conn = duckdb::Connection::open(&db_path).expect("reconnect after run 3");
    let (user1_count_after, user2_count_after): (i64, i64) = {
        let c1: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read user 1 event_count");
        let c2: i64 = conn
            .query_row(
                "SELECT event_count FROM main.user_lifetime_status WHERE user_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("read user 2 event_count");
        (c1, c2)
    };
    assert_eq!(
        (user1_count_after, user2_count_after),
        (1, 1),
        "an unchanged redelivery through the diff_patch write must leave both rows exactly \
         as they were — only a genuine difference is ever written"
    );
}

/// Genuine membership change (`docs/plans/20260808-membership-
/// sensitivity.md` Phases 2-3): deleting a dimension row that ALREADY
/// has no staged facts is a no-op repair (the membership-sensitive
/// `{event_count}` cell has nothing to remove); adding a dimension row
/// that matches EXISTING, previously-unadmitted facts is a genuine
/// repair — the inner join now admits a user_id it did not before, and
/// only the staged-candidate recompute (never a column-scoped `MERGE`,
/// which cannot create rows) can pick it up; a THIRD run then deletes a
/// dimension row that DOES have currently-admitted facts (user 2) — a
/// genuine departure, repaired by `emit_staged_candidate_conditional_
/// recompute`'s own anti-join `DELETE` (`docs/plans/
/// 20260808-membership-sensitivity.md` Phase 3; the region-scoped
/// `emit_staged_candidate_conditional` this driver used before Phase 3
/// would have left a departed row stale — see that emitter's own
/// "user 3 … must be left untouched entirely" contract in
/// `crates/smelt-runtime/tests/statement_parity.rs`, which is a
/// DIFFERENT, still-correct semantics for its own region-scoped
/// callers).
#[tokio::test]
async fn genuine_membership_change_repairs_to_full_refresh_state() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/user_lifetime_status.sql"),
        model_file_text(),
    )
    .expect("write keyed model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.clone(),
    };

    // Three users: 1 and 2 have staged transactions AND a matching dim
    // row (both admitted from the start). User 3 has staged
    // transactions but NO matching dim row yet — the inner join
    // excludes user 3 entirely, so `user_lifetime_status` never gets a
    // row for them on the first run. User 4's dim row has no staged
    // transactions at all — deleting it is the "no currently-admitted
    // facts" no-op leg.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, \
                 user_id INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                 transaction_type VARCHAR)",
            )
            .await
            .expect("create transactions source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_transactions VALUES \
                 (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                 (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase'), \
                 (3, 3, 30.00, TIMESTAMP '2025-01-10 10:00:00', 'purchase')",
            )
            .await
            .expect("seed transactions");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02'), \
                 (4, 'Dana', DATE '2025-01-03')",
            )
            .await
            .expect("seed users — user 3 deliberately has no matching dim row yet");
    }

    // First run: creation. Only users 1 and 2 are admitted (user 3's
    // dim row does not exist yet; user 4 has no staged transactions so
    // contributes no row either way).
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-1".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run must succeed");
    }
    {
        let conn = duckdb::Connection::open(&db_path).expect("reconnect after run 1");
        let mut stmt = conn
            .prepare("SELECT user_id FROM main.user_lifetime_status ORDER BY user_id")
            .expect("prepare");
        let admitted: Vec<i64> = stmt
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(
            admitted,
            vec![1, 2],
            "user 3 must not be admitted before its dim row exists"
        );
    }

    // Genuine membership change: delete user 4's dim row (no currently-
    // admitted facts — a no-op repair) AND add user 3's dim row (matches
    // EXISTING staged facts — a genuine new admission).
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("DELETE FROM main.sources_raw_users WHERE user_id = 4")
            .await
            .expect("delete dim row with no admitted facts");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES (3, 'Carol', DATE '2025-01-04')",
            )
            .await
            .expect("add dim row matching existing facts");
    }

    // Second run: the membership recompute must pick up user 3's newly
    // admitted row.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            request_for_day("2025-01-11", "2025-01-12"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(record.strategy, "delete_insert_suppressed");
    }

    // Post-repair state must equal a full recompute of the model SQL
    // over the CURRENT (post-mutation) source state.
    let conn = duckdb::Connection::open(&db_path).expect("reconnect after run 2");
    let mut stmt = conn
        .prepare("SELECT user_id, event_count FROM main.user_lifetime_status ORDER BY user_id")
        .expect("prepare");
    let actual: Vec<(i64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(
        actual,
        vec![(1, 1), (2, 1), (3, 1)],
        "post-repair state must equal a full recompute of the model SQL: user 3 admitted \
         (its dim row now exists), users 1/2 unchanged, user 4 never appears (no staged \
         facts either way)"
    );

    // Third run: genuine departure. Delete user 2's dim row — user 2
    // DOES have currently-admitted facts (a staged transaction and a
    // row in `user_lifetime_status` right now), so the inner join can
    // no longer admit it at all: the recomputed candidate has no row
    // for user 2 whatsoever, not merely a changed one.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("DELETE FROM main.sources_raw_users WHERE user_id = 2")
            .await
            .expect("delete dim row with currently-admitted facts");
    }
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-3".to_string(),
            request_for_day("2025-01-12", "2025-01-13"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("third run must succeed");
        let record = outcome
            .models
            .get("user_lifetime_status")
            .expect("user_lifetime_status ran");
        assert_eq!(record.strategy, "delete_insert_suppressed");
    }

    let conn = duckdb::Connection::open(&db_path).expect("reconnect after run 3");
    let mut stmt = conn
        .prepare("SELECT user_id, event_count FROM main.user_lifetime_status ORDER BY user_id")
        .expect("prepare");
    let actual: Vec<(i64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(
        actual,
        vec![(1, 1), (3, 1)],
        "user 2 must be genuinely DELETED once its dim row departs — the recompute's own \
         anti-join DELETE (`emit_staged_candidate_conditional_recompute`, Phase 3) removes \
         it rather than leaving a stale row behind"
    );
}
