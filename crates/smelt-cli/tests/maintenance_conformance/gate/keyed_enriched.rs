//! The change-suppressed column-scoped `MERGE` pool (`KeyedEnrichedRecipe`): a `grain: key` fold over an append-only fact source inner-joined to a `mutable_snapshot` dimension admitted purely for row membership.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::support::snapshot_table_rows;
use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_keyed_schedule, KeyShape, SourcePosture, SourceRecipe,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{read_source_snapshot, GenRow};
use smelt_runtime::maintenance_driver::resolve_live_membership_recompute_cell;

// ---------------------------------------------------------------------
// W10 Phase 5 (`docs/plans/20260720-prod-w10-keyed-mutable-admission.md`):
// the change-suppressed column-scoped `MERGE`'s generative conformance leg.
//
// `KeyedRecipe` has no dimension/enrichment support (its model reads exactly
// one source) and `MutableEnrichedRecipe` is `grain: partition` and SELECTS
// its dimension's own attribute column directly — a shape `derive_new_data`
// cannot admit at `grain: key` today (selecting the attribute forces it into
// the fold's column-group, tripping the both-fold-and-enrich refusal Phase 3
// keeps in place). [`KeyedEnrichedRecipe`] is the one reachable shape: a
// `grain: key` fold over an append-only fact source, inner-joined to a
// `mutable_snapshot` dimension declared `allow_full_scan` PURELY for row
// admission — the dimension's own payload column is never selected or
// aggregated, so Phase 2's fold-contribution classifier returns `false` for
// it and Phase 3's waiver admits the source instead of refusing the whole
// plan. This is a fixed pool of one model shape (like `MutableEnrichedRecipe`);
// the generative axis here is the WINDOW SCHEDULE, not the model shape
// (plan Phase 5 "Implementation shape").
// ---------------------------------------------------------------------

/// The fixed fact+dimension `grain: key` shape Phase 4's runtime dispatch
/// reaches: `SELECT <key>, COUNT(<fact>.val) AS event_count FROM
/// smelt.sources.<fact> f JOIN smelt.sources.<dim> dim ON f.<key> = dim.<key>
/// GROUP BY <key>`. Declared inside `gate.rs` rather than added to
/// `smelt-maintenance-testkit` — this phase's Critical files list is
/// `crates/smelt-cli/tests/maintenance_conformance/**` only.
#[derive(Debug, Clone)]
pub(crate) struct KeyedEnrichedRecipe {
    model_name: String,
    fact: SourceRecipe,
    dimension: SourceRecipe,
}

impl KeyedEnrichedRecipe {
    /// The pool's one fixed shape — mirrors [`MutableEnrichedRecipe::new`]'s
    /// own doc comment: exactly one mutable-dimension-enriched keyed shape
    /// needs to be reachable, not a generated construct family.
    fn new() -> Self {
        Self {
            model_name: "recipe_keyed_enriched".to_string(),
            fact: SourceRecipe {
                name: "keyed_enrich_fact".to_string(),
                clock_column: "d".to_string(),
                key_column: "id".to_string(),
                payload_column: "val".to_string(),
                key_shape: KeyShape::Single,
                posture: SourcePosture::AppendOnly,
                key_recurrence: None,
            },
            dimension: SourceRecipe::mutable_dimension("keyed_enrich_dim"),
        }
    }

    /// The model's `SELECT` body: the fact source folded via `COUNT`,
    /// inner-joined to the dimension purely for row admission — the
    /// dimension's own `attr` column is never read, so it never contributes
    /// to the fold (Phase 2's classifier) and stays outside the output's
    /// own column groups.
    fn model_body(&self) -> String {
        let fact_src = format!("smelt.sources.{}", self.fact.name);
        let dim_src = format!("smelt.sources.{}", self.dimension.name);
        let id = &self.fact.key_column;
        let val = &self.fact.payload_column;
        let dim_id = &self.dimension.key_column;
        format!(
            "SELECT f.{id} AS {id}, COUNT(f.{val}) AS event_count FROM {fact_src} f JOIN \
             {dim_src} dim ON f.{id} = dim.{dim_id} GROUP BY f.{id}"
        )
    }

    /// The full model file: `grain: key` frontmatter with a top-level
    /// `unique_key:` (the `RowIdentity::Key` precondition,
    /// `incremental_models.md` §"Per-cell write addressing") and the
    /// dimension declared `allow_full_scan` (its `ColumnScopedMerge` cell's
    /// admission precondition — `incremental_shapes.md` §"Admission matrix"),
    /// mirroring `crates/smelt-runtime/tests/technique_lowering.rs`'s
    /// `keyed_column_scoped_merge_e2e::MODEL_FILE`.
    fn model_file(&self) -> String {
        // `keyed_enrich_fact` is this model's own fold-driving source (a
        // `COUNT` aggregate) — phase 19 (`docs/outcomes/
        // 20260815-definition-delta-migrate`) now derives an
        // `UpstreamMutation` cell for it too (an `AppendOnly` source named
        // in a value-sensitive column group), with no statically derivable
        // scan bound, so it needs the same `allow_full_scan` escape hatch
        // the dimension already declares.
        format!(
            "---\nrefresh: incremental\ngrain: key\nunique_key: {id}\nmaintenance:\n  \
             scan_bounds:\n    per_source:\n      {dim}:\n        allow_full_scan: true\n      \
             {fact}:\n        allow_full_scan: true\n---\n\
             {body}\n",
            id = self.fact.key_column,
            dim = self.dimension.name,
            fact = self.fact.name,
            body = self.model_body(),
        )
    }

    /// The oracle query for this recipe: [`Self::model_body`] with the fact
    /// source reference swapped for `fact_table_ref` (a full-refresh oracle
    /// or an `STracker`-materialized `S_k` temp table) and the dimension's
    /// reference swapped for its physical table — mirrors
    /// [`MutableEnrichedRecipe::oracle_body_over`], except this recipe never
    /// mutates its dimension, so "current physical state" and "state at
    /// staging time" always coincide.
    fn oracle_body_over(&self, fact_table_ref: &str) -> String {
        self.model_body()
            .replace(&format!("smelt.sources.{}", self.fact.name), fact_table_ref)
            .replace(
                &format!("smelt.sources.{}", self.dimension.name),
                &format!("main.sources_{}", self.dimension.name),
            )
    }
}

/// Ids seeded into the staged dimension table, wide enough to cover every id
/// [`arb_keyed_schedule`] can generate (2-3 windows, up to 2 fresh ids per
/// window on top of the one shared re-touched key) plus this test's own
/// hand-appended zero-change redelivery window.
pub(crate) const KEYED_ENRICHED_DIM_SEED_MAX_ID: i64 = 150;

/// Stage a [`KeyedEnrichedRecipe`] into a fresh temp project + DuckDB file —
/// the keyed-enriched-pool counterpart of [`stage_mixed_recipe`]/
/// [`stage_keyed_recipe`]: writes both source YAMLs + the model file,
/// creates both physical source tables, and pre-seeds the dimension with one
/// row per id in `1..=KEYED_ENRICHED_DIM_SEED_MAX_ID` (`attr = id * 100`) so
/// every fact row a generated schedule inserts already has a matching
/// dimension row to join against.
pub(crate) fn stage_keyed_enriched_recipe(
    recipe: &KeyedEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact.source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension.source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=KEYED_ENRICHED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`KeyedEnrichedRecipe`]'s staged fact source table.
pub(crate) fn insert_fact_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ),
        [],
    )?;
    Ok(())
}

/// Insert one row into a [`KeyedEnrichedRecipe`]'s staged dimension source
/// table — the "add a dim row matching existing facts" genuine-membership-
/// change window (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
pub(crate) fn insert_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES ({id}, {attr})",
            recipe.dimension.name,
        ),
        [],
    )?;
    Ok(())
}

/// Update a [`KeyedEnrichedRecipe`]'s staged dimension row's `attr` column —
/// the "change a joined attribute" window. The recipe's own model body never
/// selects `attr` (module doc comment on [`KeyedEnrichedRecipe::model_body`]),
/// so this mutation is deliberately invisible in the maintained output; it
/// exercises the membership-recompute dispatch firing and reproducing the
/// oracle without corruption, not a value change.
pub(crate) fn update_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a [`KeyedEnrichedRecipe`]'s staged dimension row — the genuine-
/// departure window: a fact row keyed on `id` may already be admitted
/// (joined to this dim row), so removing it must make that fact disappear
/// from the maintained output entirely, not merely go stale.
pub(crate) fn delete_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`KeyedEnrichedRecipe`] through the real maintenance
/// derivation — the keyed-enriched-pool counterpart of
/// [`classify_keyed_full`]/[`classify_mixed`]. Unlike the resolver-level
/// proof in [`keyed_enriched_recipe_admits_membership_recompute`]
/// (which calls `resolve_live_membership_recompute_cell` directly and never
/// consults the model's OTHER triggers), this goes through
/// `smelt_db::maintenance_plan_report`/`file_diagnostics` — the SAME
/// multi-trigger derivation `derive_model_maintenance_plan_impl` runs for
/// every trigger the model has (including the `NewData` trigger Phase 3's
/// waiver governs) — so a regression in the waiver surfaces here even
/// though it would NOT surface in the resolver-only proof (the resolver
/// only ever looks up the `UpstreamMutation` cell by trigger, independent
/// of whether a sibling `NewData` trigger was refused).
pub(crate) fn classify_keyed_enriched_full(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged keyed-enriched-pool model {:?} (expected at {}) not found among discovered \
             models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// `keyed_enriched_recipe_admits_membership_recompute` (rewrite of
/// `keyed_enriched_recipe_admits_suppressed_column_scoped_merge`,
/// `docs/plans/20260808-membership-sensitivity.md` Phase 3): the recipe's
/// dimension is read purely in the `JOIN`'s `ON` predicate — a row-admission
/// read — so per `incremental_models.md` §"The plan matrix" its derived plan
/// now carries a membership-sensitive `UpstreamMutation(dim)` cell assigned
/// `Technique::DeleteInsert` (the recompute family), never
/// `Technique::ColumnScopedMerge` (Phase 1's review checklist: "membership
/// cells cannot receive `ColumnScopedMerge`"), WITHOUT any diagnostic
/// refusing the model overall, AND for which
/// `resolve_live_membership_recompute_cell` — the exact resolver
/// `execute.rs`'s `plan_is_keyed` branch calls alongside
/// `resolve_live_column_scoped_cell` — resolves
/// `WriteSuppression::Suppressed`
/// (`crates/smelt-runtime/tests/technique_lowering.rs`'s
/// `keyed_membership_recompute_e2e::resolves_suppressed_membership_recompute_for_keyed_dimension_cell`
/// unit-level proof, generalized to this pool's own recipe). Guards against
/// silent degradation back to `Unconditional`-only, outright refusal of the
/// `UpstreamMutation` cell, or the whole model dying at `execute_project`'s
/// pre-execution diagnostic gate with `MaintenanceNoAdmissibleTechnique`
/// even though the `UpstreamMutation` cell itself resolves fine in
/// isolation.
#[test]
fn keyed_enriched_recipe_admits_membership_recompute() {
    let recipe = KeyedEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_enriched_recipe(&recipe, &tmp).expect("stage keyed-enriched recipe");

    let (plan, diagnostics) =
        classify_keyed_enriched_full(&project, &recipe).expect("classify keyed-enriched recipe");
    let plan = plan.expect("maintenance_plan_report must return a plan for the staged recipe");

    let dim_source = recipe.dimension.name.clone();
    assert!(
        plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::DeleteInsert),
        "expected an UpstreamMutation({dim_source}) cell with Technique::DeleteInsert (the \
         membership-sensitive recompute family) in the derived plan, got: {plan:#?}"
    );
    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::ColumnScopedMerge),
        "a membership-sensitive cell must never receive Technique::ColumnScopedMerge — it \
         cannot fix which rows exist, only rewrite already-admitted rows' columns"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.severity == smelt_db::DiagnosticSeverity::Error),
        "the staged keyed-enriched recipe must produce zero Error diagnostics: {diagnostics:#?}"
    );

    let text = recipe.model_file();
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
            name: recipe.fact.name.clone(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: recipe.dimension.name.clone(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
    ];
    let mut explicitly_mutable = std::collections::HashSet::new();
    explicitly_mutable.insert(recipe.dimension.name.clone());

    let (source, cell, _group_columns, write) = resolve_live_membership_recompute_cell(
        sql_body,
        &recipe.model_name,
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolver must not error")
    .expect(
        "a live DeleteInsert membership-recompute cell must resolve for the enrich-only \
         mutable dimension — if this fails, admission has regressed back to refusing the \
         whole plan or to only an Unconditional write (choice::resolve_write_variant)",
    );

    assert_eq!(source, recipe.dimension.name);
    assert_eq!(cell.technique, Technique::DeleteInsert);
    assert!(
        matches!(
            write,
            smelt_runtime::maintenance_driver::MembershipRecomputeWrite::StagedRecompute { .. }
        ),
        "expected the change-suppressed matched arm, got {write:?}"
    );
}

/// Default deterministic case count for
/// `keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery` —
/// small, since every case drives several real `execute_project` windows
/// plus one appended redelivery window.
pub(crate) const KEYED_ENRICHED_DEFAULT_CASES: usize = 4;

pub(crate) fn keyed_enriched_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_KEYED_ENRICHED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KEYED_ENRICHED_DEFAULT_CASES)
}

/// The end-state equivalence assertion for a [`KeyedEnrichedRecipe`] — the
/// keyed-enriched-pool counterpart of [`assert_keyed_equivalence`]. Unlike
/// [`assert_mixed_settled`]'s `OracleMode` gating (needed because a
/// `grain: partition` model's column-scoped merge only ever settles its
/// window on the NEXT catch-up run), the membership-recompute technique
/// this recipe now dispatches through recomputes the model's FULL current
/// state every run (`resolve_live_membership_recompute_cell`'s own
/// `candidate_select`), so equivalence holds after every window
/// unconditionally, even once the dimension itself starts being mutated
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
pub(crate) async fn assert_keyed_enriched_equivalence(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "keyed-enriched end-state equivalence violated for model {:?} at run {k}: \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// The fixed dimension key every generated [`KeyedEnrichedRecipe`] schedule
/// case's hand-built dim-mutation windows exercise, chosen well outside both
/// [`KEYED_ENRICHED_DIM_SEED_MAX_ID`]'s pre-seeded range and
/// `arb_keyed_schedule`'s own generated id space (`KEYED_SHARED_KEY_ID = 1`
/// plus a `next_id` counter starting at 100, incrementing by at most 6 per
/// case) — so it never collides with a generated fact row's own id.
pub(crate) const DIM_MUTATION_TEST_ID: i64 = 9001;

/// `keyed_enriched_pool_upholds_equivalence_under_dim_mutation` (rewrite of
/// `keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery`,
/// `docs/plans/20260808-membership-sensitivity.md` Phase 3): drives a
/// generated [`KeyedSchedule`] against [`KeyedEnrichedRecipe`] through the
/// real `execute_project` pipeline (`stage_keyed_enriched_recipe` +
/// `LinkCProject::run_quiet`), asserting end-state equivalence against the
/// full-refresh oracle after every window, THEN appends four hand-built
/// windows that genuinely mutate the dimension — the point of this rewrite,
/// since the generated schedule alone never un-admits or newly admits a
/// fact (the dimension is pre-seeded wide enough to already cover every
/// generated id):
///
/// 1. a fresh fact row keyed on [`DIM_MUTATION_TEST_ID`], with no matching
///    dim row yet — must stay un-admitted, same as the full-refresh oracle's
///    own inner join.
/// 2. a dim row added matching that now-unmatched fact — a genuine new
///    admission only the recompute family (never `ColumnScopedMerge`,
///    which cannot create rows) can pick up.
/// 3. the dim row's `attr` mutated — invisible in the output (the recipe
///    never selects `attr`), proving the dispatch fires and reproduces the
///    oracle without corruption on a mutation that changes nothing
///    observable.
/// 4. the dim row deleted — a genuine departure: `DIM_MUTATION_TEST_ID` DOES
///    have a currently-admitted fact, so this is exactly the scenario
///    `emit_staged_candidate_conditional`'s (pre-Phase-3) `DELETE` left
///    stale — the row must now disappear from the maintained output.
///
/// Finally, one hand-built zero-change window (no new fact rows, no
/// dimension mutation) proves mutation-happened discrimination
/// (`docs/specs/incremental_models.md` §"When a mutation cell dispatches")
/// skips the cell entirely once the dimension's fingerprint is unchanged
/// since the prior run's recorded baseline: the maintained table's full
/// contents are asserted byte-identical before and after.
#[test]
fn keyed_enriched_pool_upholds_equivalence_under_dim_mutation() {
    let n = keyed_enriched_case_count();
    let mut runner = TestRunner::deterministic();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedEnrichedRecipe::new();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_enriched_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: keyed-enriched recipe failed to stage: {e}"));

        let mut tracker = STracker::new(&recipe.fact);
        let mut last_window_end: Option<chrono::NaiveDate> = None;

        rt.block_on(async {
            for (w, window) in schedule.0.iter().enumerate() {
                for row in &window.rows {
                    insert_fact_row_keyed_enriched(&project, &recipe, row)
                        .unwrap_or_else(|e| panic!("case {i}: insert fact row failed: {e}"));
                }

                let snapshot = {
                    let conn = project.connect().expect("connect");
                    read_source_snapshot(&conn, &recipe.fact)
                };

                let mut request = base_request("dev");
                request.start = Some(window.start.format("%Y-%m-%d").to_string());
                request.end = Some(window.end.format("%Y-%m-%d").to_string());
                let outcome = project
                    .run_quiet(&format!("keyed-enriched-run-{i}-{w}"), request)
                    .await
                    .unwrap_or_else(|e| panic!("case {i}: window {w} run failed: {e}"));

                let record = outcome
                    .models
                    .get(&recipe.model_name)
                    .unwrap_or_else(|| panic!("case {i}: model did not run in window {w}"));
                if w == 0 {
                    assert_ne!(
                        record.strategy, "delete_insert_suppressed",
                        "case {i}: the creation run must not take the membership-recompute \
                         path — the target doesn't exist yet"
                    );
                } else if w == 1 {
                    // The first post-creation window has no recorded
                    // mutation-fingerprint baseline yet for the dimension
                    // (`docs/specs/incremental_models.md` §"When a mutation
                    // cell dispatches"), so it always dispatches once —
                    // regardless of whether the schedule's OWN fact-only
                    // windows ever touch the dimension.
                    assert_eq!(
                        record.strategy, "delete_insert_suppressed",
                        "case {i}: window {w} must dispatch the keyed run loop through the \
                         staged-candidate membership-recompute technique once the target \
                         exists — no baseline is recorded yet"
                    );
                } else {
                    // Every window in `schedule.0` only inserts fact rows —
                    // it never mutates the dimension — so once a baseline
                    // is recorded (window 1 above), mutation-happened
                    // discrimination correctly finds nothing changed and
                    // the cell is a no-op.
                    assert_eq!(
                        record.strategy, "cumulative_aggregate",
                        "case {i}: window {w} must NOT dispatch the membership-recompute \
                         technique — the dimension is unchanged since the last recorded \
                         baseline"
                    );
                }

                let k = tracker.record_run(window.start, window.end, snapshot);
                assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                    .await
                    .unwrap_or_else(|e| {
                        panic!("case {i}: window {w} equivalence check failed: {e}")
                    });
                last_window_end = Some(window.end);
            }

            let mut next_start = last_window_end.expect("schedule generated at least one window");
            let mut run_dim_mutation_window = |label: &'static str| {
                let start = next_start;
                let end = start + chrono::Duration::days(1);
                next_start = end;
                (label, start, end)
            };

            // Window 1: a fresh fact row with no matching dim row yet — must
            // stay un-admitted.
            let (label, start, end) = run_dim_mutation_window("unmatched-fact");
            insert_fact_row_keyed_enriched(
                &project,
                &recipe,
                &GenRow {
                    d: start,
                    id: DIM_MUTATION_TEST_ID,
                    val: Some(42),
                },
            )
            .unwrap_or_else(|e| panic!("case {i}: {label}: insert fact row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let admitted: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("count admitted rows");
                assert_eq!(
                    admitted, 0,
                    "case {i}: {label}: a fact with no matching dim row must not be admitted"
                );
            }

            // Window 2: add the matching dim row — a genuine new admission.
            let (label, start, end) = run_dim_mutation_window("dim-add-admits");
            insert_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID, 900_100)
                .unwrap_or_else(|e| panic!("case {i}: {label}: insert dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let event_count: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT event_count FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("newly admitted row must exist");
                assert_eq!(
                    event_count, 1,
                    "case {i}: {label}: the newly admitted fact must be folded correctly"
                );
            }

            // Window 3: change the dim row's `attr` — never selected by the
            // model body, so invisible in the output; only proves the
            // dispatch fires without corruption.
            let (label, start, end) = run_dim_mutation_window("dim-attr-change-invisible");
            update_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID, 900_199)
                .unwrap_or_else(|e| panic!("case {i}: {label}: update dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));

            // Window 4: delete the dim row — a genuine departure.
            // `DIM_MUTATION_TEST_ID` DOES have a currently-admitted fact, so
            // this is exactly the scenario the pre-Phase-3 region-scoped
            // emitter left stale.
            let (label, start, end) = run_dim_mutation_window("dim-delete-departs");
            delete_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID)
                .unwrap_or_else(|e| panic!("case {i}: {label}: delete dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let survives: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("count surviving rows");
                assert_eq!(
                    survives, 0,
                    "case {i}: {label}: a genuinely departed dim row's fact must be DELETED \
                     from the maintained output, not left stale"
                );
            }

            // Zero-change redelivery: a fresh, never-processed window with
            // no new fact rows and no dimension mutation. Mutation-happened
            // discrimination (`docs/specs/incremental_models.md` §"When a
            // mutation cell dispatches") now recognizes the dimension's
            // fingerprint is unchanged since window 4's recorded baseline,
            // so the `UpstreamMutation` cell is a no-op this run and the
            // run falls back to the ordinary cumulative-fold label — no
            // staged-candidate `DELETE`+`INSERT` executes at all, which is
            // strictly stronger than the change-suppressed arm's own
            // zero-affected-row no-op path this window used to exercise.
            let (label, redelivery_start, redelivery_end) = run_dim_mutation_window("redelivery");

            let maintained_before = {
                let backend = project.backend().await.expect("backend");
                snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                    .await
                    .expect("snapshot before redelivery")
            };

            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(redelivery_start.format("%Y-%m-%d").to_string());
            request.end = Some(redelivery_end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: model did not run on redelivery"));
            assert_eq!(
                record.strategy, "cumulative_aggregate",
                "case {i}: the zero-change redelivery window must NOT dispatch the \
                 staged-candidate membership-recompute technique — the dimension is unchanged \
                 since the last recorded baseline"
            );

            let k = tracker.record_run(redelivery_start, redelivery_end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: redelivery equivalence check failed: {e}"));

            let maintained_after = {
                let backend = project.backend().await.expect("backend");
                snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                    .await
                    .expect("snapshot after redelivery")
            };
            assert_eq!(
                maintained_before, maintained_after,
                "case {i}: the change-suppressed arm (and its departed-key DELETE predicate) \
                 must write nothing observable when nothing changed — the maintained table's \
                 contents must be byte-identical before and after the zero-change redelivery \
                 run"
            );
        });
    }
}
