//! The keyed-succession (SCD2) family's own renderer + staging
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/07a-plan.md`),
//! split out of `render.rs` proper once the succession additions crossed
//! this crate's large-file baseline (plan task 8).

use super::*;
use crate::recipe::SuccessionRecipe;

/// The succession model's `SELECT` body (design §4 "renders once, serves
/// three", the succession family's own counterpart of
/// [`render_keyed_model_body`]): [`SuccessionRecipe::projection`]'s row-local
/// columns verbatim, then one `LEAD(<clock>) OVER (PARTITION BY <key> ORDER
/// BY <clock>) AS <alias>` per [`SuccessionRecipe::lead_cols`] entry and the
/// `LAG` counterpart per `lag_cols`, an optional `WHERE <clamp>` pre-window
/// filter, and an optional `QUALIFY NOT <delete_flag_column>` post-window
/// filter — exactly the shape `crates/smelt-runtime/tests/fixtures/
/// succession/models/customer_history.sql` pins and
/// `analysis::succession::classify_keyed_succession` recognises.
pub fn render_succession_model_body(recipe: &SuccessionRecipe) -> String {
    let src = format!("smelt.sources.{}", recipe.source.name);
    let clock = &recipe.source.clock_column;
    let partition_by = &recipe.source.key_column;

    let mut items: Vec<String> = recipe
        .projection
        .iter()
        .map(|(alias, expr)| {
            if alias == expr {
                alias.clone()
            } else {
                format!("{expr} AS {alias}")
            }
        })
        .collect();
    for alias in &recipe.lead_cols {
        items.push(format!(
            "LEAD({clock}) OVER (PARTITION BY {partition_by} ORDER BY {clock}) AS {alias}"
        ));
    }
    for alias in &recipe.lag_cols {
        items.push(format!(
            "LAG({clock}) OVER (PARTITION BY {partition_by} ORDER BY {clock}) AS {alias}"
        ));
    }

    let mut sql = format!("SELECT {} FROM {src}", items.join(", "));
    if let Some(clamp) = &recipe.clamp {
        sql.push_str(&format!(" WHERE {clamp}"));
    }
    if recipe.delete_filter {
        let flag = recipe
            .source
            .delete_flag_column
            .as_deref()
            .expect("delete_filter requires the source to declare a delete_flag_column");
        sql.push_str(&format!(" QUALIFY NOT {flag}"));
    }
    sql
}

/// The full succession model file: `refresh: incremental` frontmatter — no
/// declared `grain:` (`incremental_shapes.md` §"Succession-grain admission
/// (no declaration)": the succession grain is recognised, never declared) —
/// followed by [`render_succession_model_body`].
pub fn render_succession_model_file(recipe: &SuccessionRecipe) -> String {
    format!(
        "---\nrefresh: incremental\n---\n{body}\n",
        body = render_succession_model_body(recipe)
    )
}

/// The succession model's oracle query evaluated over `source_table_ref`
/// instead of `smelt.sources.<name>` — [`render_keyed_oracle_body_over`]'s
/// succession-family counterpart.
pub fn render_succession_oracle_body_over(
    recipe: &SuccessionRecipe,
    source_table_ref: &str,
) -> String {
    render_succession_model_body(recipe).replace(
        &format!("smelt.sources.{}", recipe.source.name),
        source_table_ref,
    )
}

/// The succession driving source's YAML: `mutation_profile: append_only`,
/// `timeseries.event_time_column` = the clock, `timeseries.partition_column`
/// = the declared `partition_column` (falling back to the clock when unset,
/// matching [`crate::recipe::SourceRecipe::source_yaml`]'s own convention),
/// the key/clock/payload columns, and — when the source declares one — a
/// `NOT NULL` (`nullable: false`) delete-flag column
/// (`docs/specs/sources.md` §"Source YAML shape").
pub fn render_succession_source_file(recipe: &SuccessionRecipe) -> String {
    let src = &recipe.source;
    let partition_col = src.partition_column.as_deref().unwrap_or(&src.clock_column);

    let mut columns = format!(
        "  - name: {key}\n    type: INTEGER\n  - name: {clock}\n    type: TIMESTAMP\n",
        key = src.key_column,
        clock = src.clock_column,
    );
    if partition_col != src.clock_column {
        columns.push_str(&format!("  - name: {partition_col}\n    type: DATE\n"));
    }
    columns.push_str(&format!(
        "  - name: {val}\n    type: VARCHAR\n",
        val = src.payload_column,
    ));
    if let Some(flag) = &src.delete_flag_column {
        columns.push_str(&format!(
            "  - name: {flag}\n    type: BOOLEAN\n    nullable: false\n"
        ));
    }

    format!(
        "description: generative-conformance succession driving source, arrival-partitioned.\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: {clock}\n  partition_column: {partition_col}\n  granularity: day\n\
         columns:\n{columns}",
        clock = src.clock_column,
    )
}

/// Stage a [`SuccessionRecipe`] into a fresh project dir + DuckDB file,
/// targeting `target` — mirrors [`stage_keyed_for_target`]'s shape, but
/// DuckDB only: Spark/BigQuery take the recorded availability downgrade for
/// this grain (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md`
/// §Out of scope), so a non-DuckDB target refuses loudly rather than
/// silently staging nothing.
pub fn stage_succession_for_target(
    recipe: &SuccessionRecipe,
    project_dir: &Path,
    db_path: &Path,
    target: ConformanceTarget,
) -> anyhow::Result<crate::link_c_harness::LinkCProject> {
    if !matches!(target, ConformanceTarget::DuckDb) {
        anyhow::bail!(
            "the succession family (`SuccessionRecipe`) supports ConformanceTarget::DuckDb \
             only today — got {target:?}"
        );
    }

    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render_succession_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        render_succession_source_file(recipe),
    )?;
    std::fs::write(project_dir.join("smelt.yml"), render_smelt_yml(db_path))?;

    let src = &recipe.source;
    let partition_col = src.partition_column.as_deref().unwrap_or(&src.clock_column);
    let mut column_defs = format!(
        "{key} INTEGER, {clock} TIMESTAMP",
        key = src.key_column,
        clock = src.clock_column,
    );
    if partition_col != src.clock_column {
        column_defs.push_str(&format!(", {partition_col} DATE"));
    }
    column_defs.push_str(&format!(", {val} VARCHAR", val = src.payload_column));
    if let Some(flag) = &src.delete_flag_column {
        column_defs.push_str(&format!(", {flag} BOOLEAN NOT NULL"));
    }
    create_source_table_via_backend(db_path, &src.name, &column_defs)?;

    crate::link_c_harness::LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `rendered_succession_source_declares_append_only_and_both_axes`
    /// (phase 7a test 2): the rendered succession source YAML carries
    /// `mutation_profile: append_only`, both timeseries axes (event-time and
    /// partition, distinct), and the flag column typed `BOOLEAN` with
    /// `nullable: false`.
    #[test]
    fn rendered_succession_source_declares_append_only_and_both_axes() {
        let recipe = crate::recipe::SuccessionRecipe::new_lead();
        let yaml = render_succession_source_file(&recipe);
        assert!(
            yaml.contains("mutation_profile: append_only"),
            "expected mutation_profile: append_only in:\n{yaml}"
        );
        assert!(
            yaml.contains("event_time_column: changed_at"),
            "expected the event-time axis in:\n{yaml}"
        );
        assert!(
            yaml.contains("partition_column: arrival_date"),
            "expected the (distinct) partition axis in:\n{yaml}"
        );
        assert!(
            yaml.contains("name: is_deleted\n    type: BOOLEAN\n    nullable: false"),
            "expected a NOT NULL BOOLEAN delete-flag column in:\n{yaml}"
        );
    }

    /// `rendered_succession_recipe_stages_cleanly` (phase 7a test 3): a
    /// rendered [`crate::recipe::SuccessionRecipe`] project parses,
    /// type-checks and stages with zero diagnostics — mirrors
    /// `rendered_recipe_stages_cleanly` above for the succession family.
    #[test]
    fn rendered_succession_recipe_stages_cleanly() {
        for recipe in [
            crate::recipe::SuccessionRecipe::new_lead(),
            crate::recipe::SuccessionRecipe::new_lag(),
        ] {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().join("project");
            let db_path = tmp.path().join("db.duckdb");
            std::fs::create_dir_all(&project_dir).expect("create project dir");

            stage_succession_for_target(&recipe, &project_dir, &db_path, ConformanceTarget::DuckDb)
                .unwrap_or_else(|e| panic!("recipe {recipe:?} failed to stage: {e}"));

            let diags = staged_diagnostics(&project_dir)
                .unwrap_or_else(|e| panic!("recipe {recipe:?} diagnostics query failed: {e}"));
            let bad: Vec<_> = diags
                .iter()
                .filter(|d| d.severity == smelt_db::DiagnosticSeverity::Error)
                .collect();
            assert!(
                bad.is_empty(),
                "recipe {recipe:?} rendered to model:\n{}\nwith diagnostics (generator bug): \
                 {bad:#?}",
                render_succession_model_file(&recipe),
            );
        }
    }

    /// The staged project's derived maintenance plan for `model_name` — the
    /// succession-family counterpart of `crates/smelt-cli/tests/
    /// maintenance_conformance/gate/keyed_oracle.rs::classify_keyed_full`.
    fn classify_succession_plan(
        project_dir: &Path,
        model_name: &str,
    ) -> smelt_logical::maintenance::MaintenancePlan {
        let config = smelt_core::config::Config::load(project_dir).expect("load smelt.yml");
        let discovery =
            smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
        let sql_models = discovery.discover_models().expect("discover models");
        let target_path = project_dir.join(format!("models/{model_name}.sql"));

        let mut db = smelt_db::Database::default();
        let project_input = db.set_project_input(project_dir.to_path_buf(), String::new());
        let mut target: Option<smelt_db::SourceFile> = None;
        let source_files: Vec<_> = sql_models
            .iter()
            .map(|m| {
                let file = db.set_source_file(
                    m.path.clone(),
                    m.content.clone(),
                    project_dir.to_path_buf(),
                );
                if m.path == target_path {
                    target = Some(file);
                }
                file
            })
            .collect();
        db.set_workspace(source_files, vec![project_input]);
        let workspace = db.workspace();
        let target = target.unwrap_or_else(|| {
            panic!(
                "staged succession model {model_name:?} (expected at {}) not found among \
                 discovered models",
                target_path.display()
            )
        });
        smelt_db::maintenance_plan_report(&db, workspace, target)
            .expect("maintenance_plan_report must return a plan")
            .plan
    }

    /// `rendered_succession_model_is_classified_as_the_succession_grain`
    /// (phase 7a test 4): the staged model's derived maintenance plan
    /// carries `Grain::Succession` + `Technique::SuccessionPatch`, not a
    /// refusal.
    #[test]
    fn rendered_succession_model_is_classified_as_the_succession_grain() {
        let recipe = crate::recipe::SuccessionRecipe::new_lead();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        stage_succession_for_target(&recipe, &project_dir, &db_path, ConformanceTarget::DuckDb)
            .expect("stage succession recipe");

        let plan = classify_succession_plan(&project_dir, &recipe.model_name);
        assert!(
            plan.refusals.is_empty(),
            "expected the succession cell to admit cleanly: {:?}",
            plan.refusals
        );
        assert!(
            plan.cells
                .iter()
                .any(|c| c.technique == smelt_logical::maintenance::Technique::SuccessionPatch),
            "expected an admitted Technique::SuccessionPatch cell (the succession grain is \
             recognised, not declared — `incremental_shapes.md` §\"Succession-grain admission \
             (no declaration)\"), got: {plan:#?}"
        );
    }

    /// `succession_oracle_body_is_the_model_sql_over_the_named_relation`
    /// (phase 7a test 5): [`render_succession_oracle_body_over`] is the
    /// model's own SQL (including `QUALIFY NOT <flag>` and the clamp) with
    /// the source reference swapped for the named relation.
    #[test]
    fn succession_oracle_body_is_the_model_sql_over_the_named_relation() {
        let mut recipe = crate::recipe::SuccessionRecipe::new_lead();
        recipe.clamp = Some("changed_at >= DATE '2026-01-01'".to_string());
        recipe.delete_filter = true;

        let oracle = render_succession_oracle_body_over(&recipe, "oracle_customer_changes");
        let expected_body = render_succession_model_body(&recipe).replace(
            &format!("smelt.sources.{}", recipe.source.name),
            "oracle_customer_changes",
        );
        assert_eq!(oracle, expected_body);
        assert!(oracle.contains("WHERE changed_at >= DATE '2026-01-01'"));
        assert!(oracle.contains("QUALIFY NOT is_deleted"));
        assert!(!oracle.contains(&format!("smelt.sources.{}", recipe.source.name)));
        assert!(oracle.contains("FROM oracle_customer_changes"));
    }
}
