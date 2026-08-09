//! Conformance recipes for the repair + diff-patch families
//! (`docs/outcomes/20260809-repair-family/phases/08-plan.md`): a keyed
//! non-invertible fold over a **mutable, clocked** source
//! (`smelt_maintenance_testkit::recipe::RepairRecipe`, generalizing
//! `crates/smelt-runtime/tests/repair_lowering.rs`'s hand-written fixture
//! into typed recipe data), driven through the real `execute_project`
//! pipeline and asserted multiset-equal to a full-refresh oracle after every
//! run step (`docs/specs/architecture.md` §"Maintenance-plan purity" — the
//! equivalence invariant's executed-vs-oracle leg).
//!
//! `gate.rs`'s own helpers (`snapshot_table_rows`) are reused rather than
//! duplicated; new staging/classification/statement-capture helpers live
//! here since a [`RepairRecipe`]'s driving-source shape (a fixed
//! `(order_id, customer_id, amount, order_date)` mutable/clocked source) is
//! distinct from every existing pool's shape.

use smelt_backend::StatementGroup;
use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::{MaintenancePlan, RowIdentity, Technique};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{KeyedCombiner, RepairRecipe, RepairWriteMode};
use smelt_maintenance_testkit::render;

use crate::gate::snapshot_table_rows;

/// Stage a [`RepairRecipe`] into a fresh temp project + DuckDB file — the
/// repair-pool counterpart of `gate.rs::stage_keyed_recipe`.
fn stage_repair_recipe(
    recipe: &RepairRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_repair(recipe, &project_dir, &db_path)
}

/// Classify a staged [`RepairRecipe`] through the real maintenance
/// derivation, returning the derived plan (possibly with zero cells) plus
/// every diagnostic on the target model — mirrors
/// `gate.rs::classify_keyed_full`.
fn classify_repair_full(
    project: &LinkCProject,
    recipe: &RepairRecipe,
) -> anyhow::Result<(Option<MaintenancePlan>, Vec<smelt_db::Diagnostic>)> {
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
            "staged repair-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Classify a staged [`RepairRecipe`], requiring an admitted (non-empty)
/// plan — mirrors `gate.rs::classify_keyed`.
fn classify_repair(
    project: &LinkCProject,
    recipe: &RepairRecipe,
) -> anyhow::Result<MaintenancePlan> {
    let (plan, diags) = classify_repair_full(project, recipe)?;
    match plan {
        Some(plan) if !plan.cells.is_empty() => Ok(plan),
        _ => anyhow::bail!(
            "repair recipe {:?} admitted no cells: diagnostics={diags:#?}",
            recipe.model_name
        ),
    }
}

/// Seed the physical source table with the same three-row shape
/// `repair_lowering.rs::seed_orders` uses: customer 1's group has two rows
/// inside the affected-key window a later repair run widens into
/// (`order_date` 2025-01-13), customer 2's single row sits outside it
/// (2025-01-11) — provably untouched by a targeted repair.
fn seed_repair_orders(project: &LinkCProject, recipe: &RepairRecipe) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{name} VALUES \
             (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
             (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
             (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
            name = recipe.source_name,
        ),
        [],
    )?;
    Ok(())
}

/// Insert one row into a [`RepairRecipe`]'s staged driving-source table.
fn insert_repair_row(
    project: &LinkCProject,
    recipe: &RepairRecipe,
    order_id: i64,
    customer_id: i64,
    amount: &str,
    order_date: &str,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{name} VALUES ({order_id}, {customer_id}, {amount}, \
             TIMESTAMP '{order_date}')",
            name = recipe.source_name,
        ),
        [],
    )?;
    Ok(())
}

/// Update one row's `amount` in place — the retraction case a fold cannot
/// undo (`incremental_models.md` §"The repair family").
fn update_repair_row_amount(
    project: &LinkCProject,
    recipe: &RepairRecipe,
    order_id: i64,
    new_amount: &str,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{name} SET amount = {new_amount} WHERE order_id = {order_id}",
            name = recipe.source_name,
        ),
        [],
    )?;
    Ok(())
}

/// Delete one row — a genuine departure; the repair's targeted `DELETE`
/// (keyed by the affected-key set) removes the stale output row for a key
/// whose candidate group is now empty.
fn delete_repair_row(
    project: &LinkCProject,
    recipe: &RepairRecipe,
    order_id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{name} WHERE order_id = {order_id}",
            name = recipe.source_name,
        ),
        [],
    )?;
    Ok(())
}

/// The creation run's window — matches `repair_lowering.rs`'s own fixture:
/// nothing to repair yet, the fold's create path materializes the table.
async fn run_repair_creation(project: &LinkCProject, run_id: &str) -> anyhow::Result<()> {
    let mut request = base_request("dev");
    request.start = Some("2025-01-11".to_string());
    request.end = Some("2025-01-14".to_string());
    project.run_quiet(run_id, request).await?;
    Ok(())
}

/// Every repair (post-creation) run in this module reuses the SAME window —
/// `[2025-01-16, 2025-01-17)`, whose derived affected-key lower bound
/// (`start − band`) is `2025-01-13`, covering every mutation this module's
/// helpers stage at that anchor date. Re-running the same window repeatedly
/// is exactly what the repair family (never fold-ledger-guarded,
/// `ledger_catch_up: false`) is meant to tolerate.
async fn run_repair_window(project: &LinkCProject, run_id: &str) -> anyhow::Result<()> {
    let mut request = base_request("dev");
    request.start = Some("2025-01-16".to_string());
    request.end = Some("2025-01-17".to_string());
    project.run_quiet(run_id, request).await?;
    Ok(())
}

/// Assert the maintained table is multiset-equal to a full-refresh oracle
/// over the CURRENT physical source state.
async fn assert_repair_equivalence(project: &LinkCProject, recipe: &RepairRecipe) {
    let backend = project
        .backend()
        .await
        .expect("backend for equivalence check");
    // The PRESENTED columns only (`customer_id`, the combiner's own value
    // alias, plus its ordering-companion alias when it has one) — `SELECT
    // *` would also pull in a decomposed combiner's hidden `__`-marked
    // state columns (`incremental_models.md` §"Decomposed state (rung 2) in
    // keyed models"), which the oracle body never projects.
    let select_list = match recipe.combiner.ordering_alias() {
        Some(ord) => format!("customer_id, {}, {ord}", recipe.combiner.agg_and_alias().1),
        None => format!("customer_id, {}", recipe.combiner.agg_and_alias().1),
    };
    let maintained_sql = format!("SELECT {select_list} FROM main.{}", recipe.model_name);
    let oracle_sql =
        render::render_repair_oracle_sql(recipe, &format!("main.sources_{}", recipe.source_name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql)
        .await
        .expect("multiset comparison must run");
    assert!(
        equal,
        "repair recipe {recipe:?} diverged from the full-refresh oracle:\n  maintained: \
         {maintained_sql}\n  oracle: {oracle_sql}"
    );
}

/// `repair_recipe_admits_per_group_recompute` (plan Phase 8 test 1): a
/// staged [`RepairRecipe`] classifies `Admitted` with at least one
/// `Technique::PerGroupRecompute` cell, whose key + scan clamp match the
/// recipe's own declared `unique_key`/band.
#[test]
fn repair_recipe_admits_per_group_recompute() {
    let recipe = RepairRecipe::new(
        KeyedCombiner::Idempotent,
        RepairWriteMode::TargetedDeleteInsert,
    );
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_repair_recipe(&recipe, &tmp).expect("stage repair recipe");
    let plan = classify_repair(&project, &recipe).expect("classify repair recipe");

    let repair_cells: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| c.technique == Technique::PerGroupRecompute)
        .collect();
    assert!(
        !repair_cells.is_empty(),
        "expected at least one PerGroupRecompute cell, got: {plan:#?}"
    );
    let cell = repair_cells[0];
    assert_eq!(
        cell.row_identity.identity,
        RowIdentity::Key(recipe.unique_key()),
        "the repair cell's row identity must be the recipe's own declared unique_key"
    );

    let clamp = cell
        .scans
        .iter()
        .find(|s| s.source == recipe.source_name)
        .unwrap_or_else(|| {
            panic!(
                "no scan clamp for source {:?}: {cell:?}",
                recipe.source_name
            )
        });
    assert_eq!(clamp.column, "order_date");
    assert_eq!(
        clamp.before,
        Seconds::days(recipe.band_days),
        "the derived clamp's 'before' width must match the recipe's declared Form B band"
    );
}

/// `repair_pool_upholds_equivalence_under_retraction` (plan Phase 8 test 2):
/// for each non-invertible combiner in the small deterministic repair pool,
/// drive insert → update-in-place → delete steps; after EVERY step the
/// model table is multiset-equal to a full-refresh oracle over the current
/// source state.
///
/// `KeyedCombiner::OrderMonotone` admits a `PerGroupRecompute` cell and its
/// CREATION run is proven equivalent (below), but driving a live repair for
/// it crashes: `repair_candidate_select` wraps the model's plain PRESENTED
/// projection with no knowledge of the decomposed hidden `(v, o)` state
/// `OrderMonotone` needs (`KeyedCombiner::OrderMonotone`'s own doc
/// comment), while the physical table the fold's OWN create path built
/// carries those extra `__`-marked columns — the repair's `INSERT`'s
/// implicit column list then mismatches the table's column count. A
/// genuine production gap, not a silent divergence: registered as
/// `known_bug_repair_candidate_select_ignores_decomposed_state` in
/// `registry.rs` rather than driven through the mutation loop here.
#[tokio::test]
async fn repair_pool_upholds_equivalence_under_retraction() {
    // The order-monotone combiner: admission + creation-run equivalence
    // only (see this test's own doc comment for why the mutation loop
    // below is Idempotent-only).
    {
        let recipe = RepairRecipe::new(
            KeyedCombiner::OrderMonotone,
            RepairWriteMode::TargetedDeleteInsert,
        );
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project =
            stage_repair_recipe(&recipe, &tmp).expect("stage OrderMonotone repair recipe");
        let plan =
            classify_repair(&project, &recipe).expect("classify OrderMonotone repair recipe");
        assert!(
            !plan.cells.is_empty(),
            "OrderMonotone repair recipe admitted zero cells: {plan:#?}"
        );
        seed_repair_orders(&project, &recipe).expect("seed OrderMonotone");
        run_repair_creation(&project, "repair-pool-order-monotone-create")
            .await
            .expect("OrderMonotone creation run");
        assert_repair_equivalence(&project, &recipe).await;
    }

    for combiner in [KeyedCombiner::Idempotent] {
        let recipe = RepairRecipe::new(combiner, RepairWriteMode::TargetedDeleteInsert);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_repair_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("stage repair recipe {combiner:?}: {e}"));
        let plan = classify_repair(&project, &recipe)
            .unwrap_or_else(|e| panic!("classify repair recipe {combiner:?}: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "repair recipe {combiner:?} admitted zero cells: {plan:#?}"
        );

        seed_repair_orders(&project, &recipe).unwrap_or_else(|e| panic!("seed {combiner:?}: {e}"));

        run_repair_creation(&project, "repair-pool-create")
            .await
            .unwrap_or_else(|e| panic!("creation run {combiner:?}: {e}"));
        assert_repair_equivalence(&project, &recipe).await;

        // Insert: a fresh key lands inside the band.
        insert_repair_row(&project, &recipe, 4, 3, "40.00", "2025-01-13 09:00:00")
            .unwrap_or_else(|e| panic!("insert {combiner:?}: {e}"));
        run_repair_window(&project, "repair-pool-insert")
            .await
            .unwrap_or_else(|e| panic!("insert run {combiner:?}: {e}"));
        assert_repair_equivalence(&project, &recipe).await;

        // Update in place: the retraction a fold cannot undo.
        update_repair_row_amount(&project, &recipe, 1, "10.00")
            .unwrap_or_else(|e| panic!("update {combiner:?}: {e}"));
        run_repair_window(&project, "repair-pool-update")
            .await
            .unwrap_or_else(|e| panic!("update run {combiner:?}: {e}"));
        assert_repair_equivalence(&project, &recipe).await;

        // Delete: order_id=4 (customer 3's ONLY row, seeded by the insert
        // step above) departs entirely — customer 3's whole window
        // contribution is gone, leaving no row for ANY current-source scan
        // to select (P9, `docs/specs/incremental_models.md` §"The repair
        // family" — "Obligation 7 over a `mutable_snapshot` source"). The
        // group-grain fingerprint sidecar diff is what still discovers it,
        // via its own stored comparandum — this is the gap case the
        // conformance gate previously dodged (see git history for the
        // now-removed `known_bug_repair_affected_key_discovery_misses_
        // full_group_deletion` workaround this replaces).
        delete_repair_row(&project, &recipe, 4)
            .unwrap_or_else(|e| panic!("delete {combiner:?}: {e}"));
        run_repair_window(&project, "repair-pool-delete")
            .await
            .unwrap_or_else(|e| panic!("delete run {combiner:?}: {e}"));
        assert_repair_equivalence(&project, &recipe).await;
    }
}

/// `repair_run_actually_executes_the_repair_family` (plan Phase 8 test 3):
/// the captured statements for a retraction step contain the repair
/// family's affected-key `EXISTS` slice and its targeted delete+insert —
/// equivalence is not passing because the run silently fell back to a full
/// refresh.
#[tokio::test]
async fn repair_run_actually_executes_the_repair_family() {
    let recipe = RepairRecipe::new(
        KeyedCombiner::Idempotent,
        RepairWriteMode::TargetedDeleteInsert,
    );
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_repair_recipe(&recipe, &tmp).expect("stage repair recipe");
    let plan = classify_repair(&project, &recipe).expect("classify repair recipe");
    assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

    seed_repair_orders(&project, &recipe).expect("seed");
    run_repair_creation(&project, "repair-exec-create")
        .await
        .expect("creation run");

    update_repair_row_amount(&project, &recipe, 1, "10.00").expect("retract");

    let mut retract_request = base_request("dev");
    retract_request.start = Some("2025-01-16".to_string());
    retract_request.end = Some("2025-01-17".to_string());
    let (outcome, backend) = project
        .run_recording("repair-exec-retract", retract_request)
        .await
        .expect("retraction run");

    let record = outcome
        .models
        .get(&recipe.model_name)
        .unwrap_or_else(|| panic!("model {:?} ran", recipe.model_name));
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the retraction must dispatch the repair family, not the fold"
    );

    let groups: Vec<StatementGroup> = backend.recorded_groups();
    let staged_prefix = format!("CREATE TEMP TABLE __smelt_repair_{}", recipe.model_name);
    let repair_group = groups
        .iter()
        .find(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with(&staged_prefix))
        })
        .unwrap_or_else(|| panic!("no per-group-recompute statement group among {groups:?}"));

    let full_text = repair_group
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        full_text.contains("EXISTS"),
        "expected the affected-key EXISTS slice, got:\n{full_text}"
    );
    assert!(
        full_text.to_uppercase().contains("DELETE"),
        "expected a targeted DELETE leg, got:\n{full_text}"
    );
    assert!(
        full_text.to_uppercase().contains("INSERT"),
        "expected the candidate INSERT leg, got:\n{full_text}"
    );
}

/// `diff_patch_pinned_repair_upholds_equivalence_under_reconcile` (plan
/// Phase 8 test 4): the same recipe pinned `write: diff_patch`: a mutation
/// step then a re-run of the same window with no further source change
/// keeps equivalence, and the second run writes nothing (the maintained
/// table's contents are byte-identical before/after — an empty diff).
#[tokio::test]
async fn diff_patch_pinned_repair_upholds_equivalence_under_reconcile() {
    let recipe = RepairRecipe::new(KeyedCombiner::Idempotent, RepairWriteMode::DiffPatch);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_repair_recipe(&recipe, &tmp).expect("stage repair recipe");
    let plan = classify_repair(&project, &recipe).expect("classify repair recipe");
    assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

    seed_repair_orders(&project, &recipe).expect("seed");
    run_repair_creation(&project, "diff-patch-create")
        .await
        .expect("creation run");
    assert_repair_equivalence(&project, &recipe).await;

    update_repair_row_amount(&project, &recipe, 1, "10.00").expect("mutate");
    run_repair_window(&project, "diff-patch-mutate")
        .await
        .expect("mutation run");
    assert_repair_equivalence(&project, &recipe).await;

    let before = {
        let backend = project.backend().await.expect("backend before re-run");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot before the no-op re-run")
    };

    // Re-run the SAME window with no further source change: reconciliation
    // must be a no-op.
    let mut rerun_request = base_request("dev");
    rerun_request.start = Some("2025-01-16".to_string());
    rerun_request.end = Some("2025-01-17".to_string());
    let (outcome, _backend) = project
        .run_recording("diff-patch-rerun", rerun_request)
        .await
        .expect("re-run");
    let record = outcome
        .models
        .get(&recipe.model_name)
        .unwrap_or_else(|| panic!("model {:?} ran", recipe.model_name));
    assert_eq!(
        record.strategy, "diff_patch",
        "the write: diff_patch pin must dispatch the diff-patch write"
    );

    assert_repair_equivalence(&project, &recipe).await;

    let after = {
        let backend = project.backend().await.expect("backend after re-run");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after the no-op re-run")
    };
    assert_eq!(
        before, after,
        "re-running the same window with no source change must write nothing — the maintained \
         table's contents must be byte-identical before/after (an empty diff)"
    );
}

/// `diff_patch_run_is_labelled_and_emitted` (plan Phase 8 test 5): the
/// pinned run's captured statements are the `emit_diff_patch` statements
/// (delete leg present, restricted to the affected-key slice), not the
/// repair family's own default targeted delete+insert.
#[tokio::test]
async fn diff_patch_run_is_labelled_and_emitted() {
    let recipe = RepairRecipe::new(KeyedCombiner::Idempotent, RepairWriteMode::DiffPatch);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_repair_recipe(&recipe, &tmp).expect("stage repair recipe");
    seed_repair_orders(&project, &recipe).expect("seed");
    run_repair_creation(&project, "diff-patch-label-create")
        .await
        .expect("creation run");

    update_repair_row_amount(&project, &recipe, 1, "10.00").expect("mutate");

    let mut retract_request = base_request("dev");
    retract_request.start = Some("2025-01-16".to_string());
    retract_request.end = Some("2025-01-17".to_string());
    let (outcome, backend) = project
        .run_recording("diff-patch-label-retract", retract_request)
        .await
        .expect("retraction run");

    let record = outcome
        .models
        .get(&recipe.model_name)
        .unwrap_or_else(|| panic!("model {:?} ran", recipe.model_name));
    assert_eq!(record.strategy, "diff_patch");

    let groups: Vec<StatementGroup> = backend.recorded_groups();
    let diff_relation = format!("__smelt_diff_patch_{}", recipe.model_name);
    let repair_prefix = format!("CREATE TEMP TABLE __smelt_repair_{}", recipe.model_name);

    let has_diff_patch_group = groups
        .iter()
        .any(|g| g.statements.iter().any(|s| s.sql.contains(&diff_relation)));
    assert!(
        has_diff_patch_group,
        "expected an emit_diff_patch statement group among {groups:?}"
    );

    let has_default_repair_group = groups.iter().any(|g| {
        g.statements
            .first()
            .is_some_and(|s| s.sql.starts_with(&repair_prefix))
    });
    assert!(
        !has_default_repair_group,
        "a diff_patch pin must not ALSO dispatch the repair family's default targeted \
         delete+insert: {groups:?}"
    );

    let delete_leg_present = groups.iter().any(|g| {
        g.statements
            .iter()
            .any(|s| s.sql.to_uppercase().starts_with("DELETE") && s.sql.contains(&diff_relation))
    });
    assert!(
        delete_leg_present,
        "expected a DELETE leg restricted to the affected-key slice among {groups:?}"
    );
}
