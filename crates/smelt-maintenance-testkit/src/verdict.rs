//! Admitted/refused verdict protocol
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 2).
//!
//! [`classify`] runs a staged recipe through the **real** derivation —
//! [`smelt_db::maintenance_plan_report`] (which itself only assembles
//! Salsa-visible inputs and calls the pure
//! `smelt_logical::maintenance::derive::derive_maintenance_plan`) plus
//! [`smelt_db::file_diagnostics`] — never re-deriving admission, locality,
//! or ledger logic itself (root `CLAUDE.md` §"Maintenance-plan purity":
//! "consumers never re-derive it").
//!
//! A recipe with at least one admitted cell is [`Verdict::Admitted`]; a
//! recipe with none is [`Verdict::Refused`], but only when a named
//! `Maintenance*`/admission diagnostic backs the refusal — an admission
//! failure with no diagnostic at all is a fail-loud discipline violation
//! (`architecture.md` §"Fail-loud discipline") and [`classify`] returns an
//! `Err` for it rather than silently producing an unexplained `Refused`.

#![allow(dead_code)]

use std::path::Path;

use smelt_logical::maintenance::MaintenancePlan;

use crate::link_c_harness::LinkCProject;
use crate::recipe::{AdversarialLeafRecipe, ModelRecipe};
use crate::render;

/// The outcome of classifying one staged recipe against the real
/// maintenance derivation.
#[derive(Debug)]
pub enum Verdict {
    /// At least one plan cell was admitted (`plan.cells` non-empty). The
    /// full derived plan is exposed so a caller (a later phase's schedule
    /// driver) can inspect cells/clamps/techniques directly — never
    /// re-derived.
    Admitted(MaintenancePlan),
    /// No plan cell was admitted, backed by at least one named
    /// `Maintenance*`/admission diagnostic (never an unexplained empty
    /// plan — see module docs).
    Refused(Vec<smelt_db::Diagnostic>),
}

/// Every `DiagnosticCode` this protocol accepts as a "named" backing for a
/// refusal — the `Maintenance*` family `check_file_diagnostics` maps
/// admission refusals onto (`smelt-db/src/lib.rs`'s `maintenance_plan`
/// Salsa query). Mirrors `render.rs`'s `is_maintenance_family` predicate
/// (kept as a separate copy here since `render.rs` is outside this phase's
/// edit scope — see the plan's Critical files list).
fn is_admission_diagnostic(code: Option<smelt_db::DiagnosticCode>) -> bool {
    matches!(
        code,
        Some(
            smelt_db::DiagnosticCode::MaintenanceNoAdmissibleTechnique
                | smelt_db::DiagnosticCode::MaintenanceScanUnbounded
                | smelt_db::DiagnosticCode::MaintenanceGranularityMismatch
        )
    )
}

/// Build a throwaway `smelt_db::Database` over `project_dir` (mirrors
/// `render::staged_diagnostics`'s Salsa setup) and return the workspace plus
/// the specific `SourceFile` handle for `models/<model_name>.sql`, so the
/// caller can invoke `maintenance_plan_report`/`file_diagnostics` for that
/// model exactly as production does.
fn build_db_and_target(
    project_dir: &Path,
    model_name: &str,
) -> anyhow::Result<(
    smelt_db::Database,
    smelt_db::Workspace,
    smelt_db::SourceFile,
)> {
    let config = smelt_core::config::Config::load(project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models()?;

    let target_path = project_dir.join(format!("models/{model_name}.sql"));

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file =
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf());
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged model {model_name:?} (expected at {}) not found among discovered models",
            target_path.display()
        )
    })?;
    Ok((db, workspace, target))
}

/// Fold an already-derived plan + diagnostics list into a [`Verdict`],
/// enforcing the fail-loud refusal check. Split out from [`classify_model`]
/// so the fail-loud check is directly unit-testable without staging a
/// project (`refusal_without_named_diagnostic_fails_the_case`).
fn verdict_from_parts(
    plan: Option<MaintenancePlan>,
    diagnostics: &[smelt_db::Diagnostic],
    model_name: &str,
) -> anyhow::Result<Verdict> {
    let named: Vec<smelt_db::Diagnostic> = diagnostics
        .iter()
        .filter(|d| {
            d.severity == smelt_db::DiagnosticSeverity::Error && is_admission_diagnostic(d.code)
        })
        .cloned()
        .collect();

    match plan {
        Some(plan) if !plan.cells.is_empty() => Ok(Verdict::Admitted(plan)),
        _ => {
            if named.is_empty() {
                anyhow::bail!(
                    "model {model_name:?} was refused (no admitted maintenance cell) but carries \
                     no named Maintenance*/admission diagnostic — fail-loud discipline violation \
                     (architecture.md §\"Fail-loud discipline\")"
                );
            }
            Ok(Verdict::Refused(named))
        }
    }
}

/// Classify the staged model `model_name` in `project` through the real
/// derivation.
fn classify_model(project: &LinkCProject, model_name: &str) -> anyhow::Result<Verdict> {
    let (db, workspace, file) = build_db_and_target(&project.project_dir, model_name)?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, file);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, file);
    verdict_from_parts(plan_result.map(|r| r.plan), &diagnostics, model_name)
}

/// Classify a staged [`ModelRecipe`] (the plan's Phase 2 Implementation
/// shape: `classify(&LinkCProject, &ModelRecipe) -> Verdict`).
pub fn classify(project: &LinkCProject, recipe: &ModelRecipe) -> anyhow::Result<Verdict> {
    classify_model(project, &recipe.model_name)
}

/// Classify a staged [`AdversarialLeafRecipe`] — the adversarial-pool
/// counterpart of [`classify`].
pub fn classify_adversarial(
    project: &LinkCProject,
    recipe: &AdversarialLeafRecipe,
) -> anyhow::Result<Verdict> {
    classify_model(project, &recipe.model_name)
}

/// Classify a staged [`crate::recipe::KeyedRecipe`] — the `grain: key` pool's
/// counterpart of [`classify`] (`docs/plans/20260712-generative-maintenance-conformance.md`
/// Phase 7). `crate::gate`'s own `classify_keyed` (`smelt-cli`'s test binary)
/// cannot be imported back into this crate, and Phase 7's probes need a keyed
/// verdict from inside the testkit itself, so this is a second thin wrapper
/// around the same [`classify_model`] this module already uses for
/// [`classify`]/[`classify_adversarial`] — no admission logic is re-derived.
pub fn classify_keyed(
    project: &LinkCProject,
    recipe: &crate::recipe::KeyedRecipe,
) -> anyhow::Result<Verdict> {
    classify_model(project, &recipe.model_name)
}

/// Stage an [`AdversarialLeafRecipe`] to disk + create its empty source
/// table, mirroring [`render::stage`]'s contract for [`ModelRecipe`]
/// (kept here rather than in `render.rs` for the same reason
/// [`AdversarialLeafRecipe`] self-renders — see its doc comment).
pub fn stage_adversarial(
    recipe: &AdversarialLeafRecipe,
    project_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<LinkCProject> {
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        recipe.source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(db_path),
    )?;

    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        name = recipe.source.name,
        d = recipe.source.clock_column,
        id = recipe.source.key_column,
        val = recipe.source.payload_column,
    ))?;
    drop(conn);

    LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// An in-memory tally of refused cases: `(matrix_cell, refusal_kind)` pairs
/// — non-gating, reportable (plan Phase 2 Implementation shape:
/// `OverRefusalLedger` "rendered into the reachability report").
#[derive(Debug, Default)]
pub struct OverRefusalLedger {
    entries: Vec<(String, String)>,
}

impl OverRefusalLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one refused case under `matrix_cell` with `refusal_kind` (a
    /// stable label — this crate uses each refusing diagnostic's
    /// `DiagnosticCode` `Debug` string, but any caller-chosen label works).
    pub fn record(&mut self, matrix_cell: impl Into<String>, refusal_kind: impl Into<String>) {
        self.entries.push((matrix_cell.into(), refusal_kind.into()));
    }

    /// Record every diagnostic in a [`Verdict::Refused`] against
    /// `matrix_cell`, one ledger entry per diagnostic (labelled by its
    /// `DiagnosticCode`, or `"unnamed"` — unreachable in practice since
    /// [`classify`]/[`classify_adversarial`] only ever produce `Refused`
    /// diagnostics with a `Some(code)`, per [`is_admission_diagnostic`]).
    pub fn record_refusal(
        &mut self,
        matrix_cell: impl Into<String>,
        diags: &[smelt_db::Diagnostic],
    ) {
        let cell = matrix_cell.into();
        for diag in diags {
            let kind = diag
                .code
                .map(|c| format!("{c:?}"))
                .unwrap_or_else(|| "unnamed".to_string());
            self.record(cell.clone(), kind);
        }
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{arb_adversarial_recipe, arb_recipe, ConstructKind, RecipePool};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use smelt_logical::maintenance::{Corner, Technique, Trigger};

    fn stage_model_recipe(
        recipe: &ModelRecipe,
        tmp: &tempfile::TempDir,
    ) -> anyhow::Result<LinkCProject> {
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir)?;
        render::stage(recipe, &project_dir, &db_path)
    }

    /// `additive_agg_append_only_admits_delete_insert` (plan Phase 2 TDD
    /// list): an additive-aggregate recipe over a clocked append-only
    /// source derives a `Trigger::NewData` cell with
    /// `Technique::DeleteInsert`.
    #[test]
    fn additive_agg_append_only_admits_delete_insert() {
        let mut runner = TestRunner::deterministic();
        let pool = RecipePool {
            constructs: vec![ConstructKind::AdditiveAgg],
        };
        let strat = arb_recipe(pool);
        let recipe = strat.new_tree(&mut runner).unwrap().current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_model_recipe(&recipe, &tmp).expect("stage");

        let verdict = classify(&project, &recipe).expect("classify");
        let plan = match verdict {
            Verdict::Admitted(plan) => plan,
            Verdict::Refused(diags) => panic!(
                "expected additive-agg append-only recipe to admit, got refusals: {diags:#?}"
            ),
        };

        let cell = plan
            .cell_for(&Trigger::NewData {
                source: recipe.source.name.clone(),
            })
            .unwrap_or_else(|| {
                panic!(
                    "no NewData cell admitted for source {:?}: {plan:#?}",
                    recipe.source.name
                )
            });
        assert_eq!(
            cell.technique,
            Technique::DeleteInsert,
            "additive-agg append-only cell must admit DeleteInsert, got {:?}",
            cell.technique
        );
    }

    /// `adversarial_leaves_refuse_or_collapse_conservatively` (plan Phase 2
    /// TDD list): proptest over the adversarial pool — each case yields
    /// either a named refusal diagnostic or a plan whose every admitted
    /// cell is full-input recompute (`Corner::RecomputeRegion`), never a
    /// targeted technique.
    #[test]
    fn adversarial_leaves_refuse_or_collapse_conservatively() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_adversarial_recipe();

        for i in 0..40 {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().join("project");
            let db_path = tmp.path().join("db.duckdb");

            let project = stage_adversarial(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
                panic!("case {i}: adversarial recipe {recipe:?} failed to stage: {e}")
            });

            let verdict = classify_adversarial(&project, &recipe).unwrap_or_else(|e| {
                panic!("case {i}: adversarial recipe {recipe:?} violated fail-loud discipline: {e}")
            });

            match verdict {
                Verdict::Refused(diags) => assert!(
                    !diags.is_empty(),
                    "case {i}: adversarial recipe {recipe:?} refused with no diagnostics \
                     (should have been caught by classify's own fail-loud check)"
                ),
                Verdict::Admitted(plan) => {
                    for cell in &plan.cells {
                        assert_eq!(
                            cell.corner,
                            Corner::RecomputeRegion,
                            "case {i}: adversarial recipe {recipe:?} admitted a targeted \
                             (non-full-input-recompute) cell: {cell:#?}"
                        );
                    }
                }
            }
        }
    }

    /// `refusal_without_named_diagnostic_fails_the_case` (plan Phase 2 TDD
    /// list): the protocol function returns an error (test failure) when a
    /// recipe is refused but no `Maintenance*`/admission diagnostic is
    /// present.
    #[test]
    fn refusal_without_named_diagnostic_fails_the_case() {
        let empty_plan = MaintenancePlan::default();
        assert!(empty_plan.cells.is_empty());

        let result = verdict_from_parts(Some(empty_plan), &[], "unnamed_refusal_case");
        assert!(
            result.is_err(),
            "verdict_from_parts must fail loud on a refused case with no named diagnostic"
        );

        // Sanity check the positive case: the same empty plan WITH a named
        // diagnostic backing it must classify as Refused, not error.
        let diag = smelt_db::Diagnostic {
            severity: smelt_db::DiagnosticSeverity::Error,
            message: "no admissible technique".to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(smelt_db::DiagnosticCode::MaintenanceNoAdmissibleTechnique),
            data: None,
        };
        let result = verdict_from_parts(
            Some(MaintenancePlan::default()),
            std::slice::from_ref(&diag),
            "named_refusal_case",
        );
        assert!(matches!(result, Ok(Verdict::Refused(_))));
    }

    /// `over_refusal_ledger_records_cell_ids` (plan Phase 2 TDD list):
    /// refused cases append `(matrix_cell, refusal_kind)` to the in-run
    /// ledger; the ledger is reportable (counted, non-gating).
    #[test]
    fn over_refusal_ledger_records_cell_ids() {
        let mut ledger = OverRefusalLedger::new();
        assert!(ledger.is_empty());

        let diag = smelt_db::Diagnostic {
            severity: smelt_db::DiagnosticSeverity::Error,
            message: "maintenance scan cannot be partition-bounded".to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(smelt_db::DiagnosticCode::MaintenanceScanUnbounded),
            data: None,
        };
        ledger.record_refusal("intersect_body×adversarial", std::slice::from_ref(&diag));
        ledger.record_refusal(
            "nondeterministic_skeleton×adversarial",
            std::slice::from_ref(&diag),
        );

        assert_eq!(ledger.len(), 2);
        let cells: Vec<&str> = ledger
            .entries()
            .iter()
            .map(|(cell, _)| cell.as_str())
            .collect();
        assert_eq!(
            cells,
            vec![
                "intersect_body×adversarial",
                "nondeterministic_skeleton×adversarial"
            ]
        );
        for (_, kind) in ledger.entries() {
            assert_eq!(kind, "MaintenanceScanUnbounded");
        }
    }
}
