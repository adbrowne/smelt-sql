//! Classification and the end-state equivalence oracle for the keyed pool: the presented-column projection, the float-aware select list, and the per-window driver.

use super::keyed_support::insert_row_keyed;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{KeyedRecipe, KeyedSchedule};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::read_source_snapshot;

/// Classify a staged [`KeyedRecipe`] through the real maintenance derivation
/// — the keyed-pool counterpart of `classify`/`classify_mixed`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files). Returns the derived plan (possibly with zero cells) plus
/// every diagnostic on the target model, so a refusal case can name the
/// diagnostic that backs it.
pub(crate) fn classify_keyed_full(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
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
            "staged keyed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Classify a staged [`KeyedRecipe`], requiring an admitted (non-empty) plan
/// — the happy-path wrapper around [`classify_keyed_full`] for cases that
/// expect admission.
pub(crate) fn classify_keyed(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    let (plan, diags) = classify_keyed_full(project, recipe)?;
    match plan {
        Some(plan) if !plan.cells.is_empty() => Ok(plan),
        _ => anyhow::bail!(
            "keyed recipe {:?} admitted no cells: diagnostics={diags:#?}",
            recipe.model_name
        ),
    }
}

/// The maintained table's PRESENTED columns only, as `(name, data_type)`
/// pairs in physical column order — excludes any physical column whose name
/// contains the reserved `__` decomposed-state marker
/// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in keyed
/// models"). A state-bearing model's physical table carries its hidden
/// state columns alongside the presented ones (`MAX_BY`/`MIN_BY`, row 5);
/// a bare `SELECT *` against the live table — unlike a `ref()`-mediated
/// read through smelt's own compiler, which the `presentation_projection`
/// mechanism already rewrites — would leak them into an oracle comparison
/// with a different column count. Column names/types/order come off
/// `information_schema.columns`, mirroring `probes::read_full_output_as_text`.
pub(crate) fn presented_columns_with_types(
    project: &LinkCProject,
    model_name: &str,
) -> Vec<(String, String)> {
    let conn = project
        .connect()
        .expect("connect for presented-column listing");
    let columns: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = 'main' AND table_name = '{model_name}' \
                 AND column_name NOT LIKE '%\\_\\_%' ESCAPE '\\' \
                 ORDER BY ordinal_position",
            ))
            .expect("prepare presented-column listing");
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query presented-column listing")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect presented-column listing")
    };
    assert!(
        !columns.is_empty(),
        "model {model_name:?} reported zero presented columns via information_schema — \
         staging bug or an over-eager state-column filter"
    );
    columns
}

/// EVERY physical column name of `model_name`'s maintained table, in
/// ordinal order — unlike [`presented_columns_with_types`], applies no `__`
/// filter. Used both to prove a state-bearing recipe's table really does
/// carry hidden state columns (a vacuity guard,
/// `state_bearing_recipes_physically_carry_state_columns`) and to prove a
/// downstream `ref()`-mediated consumer carries none
/// (`assert_downstream_hides_state`).
pub(crate) fn all_physical_column_names(project: &LinkCProject, model_name: &str) -> Vec<String> {
    let conn = project
        .connect()
        .expect("connect for physical-column listing");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = '{model_name}' \
             ORDER BY ordinal_position",
        ))
        .expect("prepare physical-column listing");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query physical-column listing")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect physical-column listing")
}

/// `(name, data_type)` pairs (from [`presented_columns_with_types`]) into a
/// float-aware select-list fragment: `DOUBLE`/`FLOAT`/`REAL` columns are
/// wrapped `ROUND(col, 6) AS col`, every other column is selected bare. Used
/// to build BOTH sides of a keyed end-state comparison from the exact same
/// column list, so a maintained/oracle pair only ever disagrees in the
/// column list itself (a real bug) rather than in which side got rounded.
///
/// Float-aware, not exact, because DuckDB's own `STDDEV_SAMP` uses a
/// numerically stable (Welford-style) accumulation pass while the
/// decomposed `(n, Σx, Σx²)` state this outcome derives recomputes variance
/// from the raw sums (`incremental_shapes.md` §"Decomposed state (rung 2) in
/// keyed models") — the two agree only to floating-point noise (~1e-12),
/// so an exact `EXCEPT ALL` would flake. [`harness_self_check`]'s
/// `float_equivalence_comparison_tolerates_last_bit_only` pins this
/// tolerance so it cannot silently widen into swallowing a real fold bug.
pub(crate) fn rounded_select_list(columns: &[(String, String)]) -> String {
    columns
        .iter()
        .map(|(name, data_type)| {
            if matches!(data_type.as_str(), "DOUBLE" | "FLOAT" | "REAL") {
                format!("ROUND({name}, 6) AS {name}")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The end-state equivalence assertion for a [`KeyedRecipe`] (design §6
/// "Keyed-grain carve-outs"; `incremental_shapes.md` §"End-state equivalence"):
/// materialize `S_k` (the union, across every run so far, of that run's own
/// window's rows — exactly [`STracker::s_at`]'s definition, which coincides
/// with "every delta row a window-forward keyed run has folded so far" since
/// a keyed run merges every row landing in its own window and no
/// re-delivery occurs in a generated [`KeyedSchedule`]), then compare the
/// maintained table's full contents against the recipe's own body evaluated
/// over `S_k`. Both sides are selected through the same float-aware,
/// presented-columns-only projection ([`rounded_select_list`]) built from
/// one `information_schema`-derived column list.
pub(crate) async fn assert_keyed_equivalence(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let columns = presented_columns_with_types(project, &recipe.model_name);
    let select_list = rounded_select_list(&columns);
    let maintained_sql = format!("SELECT {select_list} FROM main.{}", recipe.model_name);
    let oracle_body =
        render::render_keyed_oracle_body_over(recipe, &format!("oracle_{}", recipe.source.name));
    let oracle_sql = format!("SELECT {select_list} FROM ({oracle_body}) AS oracle_sub");
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "keyed end-state equivalence violated for model {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Phase 8 task 5: asserts a staged downstream `SELECT * FROM
/// smelt.<model_name>` consumer (`stage_keyed_recipe_with_downstream`,
/// model file `<model_name>_downstream.sql`) materializes with EXACTLY the
/// upstream's presented columns (no `__`-marked names — `presentation_projection`
/// rewrites the wildcard at compile time, `incremental_shapes.md` §"Decomposed
/// state (rung 2) in keyed models") and multiset-equals the upstream's
/// presented contents — the end-to-end DuckDB witness for the hiding
/// mechanism (unit-tested at compile time in row 4) proven against a real
/// run. Float-aware via the same [`rounded_select_list`]
/// [`assert_keyed_equivalence`] uses.
pub(crate) async fn assert_downstream_hides_state(project: &LinkCProject, model_name: &str) {
    let downstream_name = format!("{model_name}_downstream");

    let downstream_physical_columns = all_physical_column_names(project, &downstream_name);
    let leaked: Vec<_> = downstream_physical_columns
        .iter()
        .filter(|c| c.contains("__"))
        .collect();
    assert!(
        leaked.is_empty(),
        "downstream consumer {downstream_name:?} carries `__`-marked physical column(s) \
         {leaked:?} — presentation_projection failed to hide upstream state from a \
         ref()-mediated read"
    );

    let upstream_columns = presented_columns_with_types(project, model_name);
    let upstream_names: Vec<&String> = upstream_columns.iter().map(|(n, _)| n).collect();
    let downstream_names: Vec<&String> = downstream_physical_columns.iter().collect();
    assert_eq!(
        upstream_names, downstream_names,
        "downstream consumer {downstream_name:?}'s physical column list does not match \
         upstream {model_name:?}'s presented columns"
    );

    let select_list = rounded_select_list(&upstream_columns);
    let upstream_sql = format!("SELECT {select_list} FROM main.{model_name}");
    let downstream_sql = format!("SELECT {select_list} FROM main.{downstream_name}");
    let backend = project
        .backend()
        .await
        .expect("backend for downstream comparison");
    let equal = multiset_equal_via_backend(backend.as_ref(), &upstream_sql, &downstream_sql)
        .await
        .expect("compare downstream consumer to upstream presented contents");
    assert!(
        equal,
        "downstream consumer {downstream_name:?} does not multiset-equal upstream \
         {model_name:?}'s presented contents: upstream ({upstream_sql:?}) != downstream \
         ({downstream_sql:?})"
    );
}

/// Drive `schedule` against `project`/`recipe` (a [`KeyedRecipe`] under the
/// window-forward posture) through the real `execute_project` pipeline,
/// asserting end-state equivalence after every window.
pub(crate) async fn drive_keyed_and_assert(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let mut tracker = STracker::new(&recipe.source);

    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            insert_row_keyed(project, recipe, row)?;
        }

        let snapshot = {
            let conn = project.connect()?;
            read_source_snapshot(&conn, &recipe.source)
        };

        let mut request = base_request("dev");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_quiet(&format!("keyed-run-{i}"), request)
            .await?;

        let k = tracker.record_run(window.start, window.end, snapshot);
        assert_keyed_equivalence(project, recipe, &tracker, k).await?;
    }
    Ok(())
}
