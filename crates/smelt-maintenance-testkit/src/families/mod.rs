//! Target-parametrized owner for the maintenance-conformance test families
//! the Spark leg (`crates/smelt-cli/tests/maintenance_conformance_spark/`)
//! used to re-derive per backend
//! (`docs/plans/20260817-bigquery-generative-conformance.md`;
//! `docs/specs/multi_backend.md` §"Generative equivalence coverage").
//!
//! Each submodule owns one shared family: the staging/insert/assert/drive
//! helpers a family needs, plus one or more `run_<family>` entry points that
//! a thin per-backend test wrapper calls. A family body never branches on
//! [`crate::recipe::ConformanceTarget`] — every backend-specific decision
//! (which target/schema a case runs against, whether to skip, how to seed a
//! divergence, how to pace writes) is resolved once through the
//! [`ConformanceBackend`] trait and threaded through as an ordinary
//! parameter. Adding a backend means implementing the trait, never editing a
//! family.
//!
//! The DuckDB leg (`crates/smelt-cli/tests/maintenance_conformance/`) is
//! deliberately NOT folded in here — it is the reference leg, runs per-PR,
//! and owns families (`contract_points`, `fact_violations`, `probes`,
//! `registry`, `repair`) neither Spark nor BigQuery exercise. See this
//! plan's "Explicitly deferred" section.

#![cfg(any(feature = "spark", feature = "bigquery"))]

use std::path::Path;

use anyhow::Result;
use smelt_backend::Backend;

use crate::recipe::{ConformanceTarget, ModelRecipe};

pub mod dags;
pub mod feed;
pub mod gate;
pub mod gate_composed;
pub mod gate_keyed;
pub mod gate_mixed;
pub mod harness_self_check;
pub mod pinned;

/// The seam every shared family is parametrized over. One implementation
/// per backend (`SparkConformanceBackend` in the Spark test binary today; a
/// `BigQueryConformanceBackend` follows the same shape). Every method
/// resolves ONE backend-specific fact — never a branch a family body would
/// otherwise have to make itself.
#[async_trait::async_trait]
pub trait ConformanceBackend: Sync {
    /// The [`ConformanceTarget`] `case` runs against. Most backends ignore
    /// `case` and return one fixed target (Spark's persistent Delta
    /// warehouse); a per-case-dataset backend (BigQuery) derives a fresh
    /// target from it.
    fn target(&self, case: usize) -> ConformanceTarget;

    /// The schema/dataset `case`'s model and source tables live under —
    /// resolved once per case by the caller and threaded through every
    /// staging/insert/assert helper explicitly, so no helper needs to
    /// re-derive it (or match on `target()` to get it).
    fn schema(&self, case: usize) -> String;

    /// The `ExecuteRequest.target`/`smelt.yml` engine name
    /// (`link_c_harness::base_request`'s parameter) for this backend.
    fn engine_name(&self) -> &str;

    /// `Some(reason)` when the leg must skip green rather than run (e.g. no
    /// live server reachable) — checked once by the test wrapper before any
    /// family function runs.
    fn skip_reason(&self) -> Option<String>;

    /// Backend-specific corruption SQL for the harness self-check family —
    /// an UNCONDITIONAL, GoogleSQL/Delta-valid whole-table mutation (Delta's
    /// `UPDATE` refuses a subquery in the SET/WHERE clause, unlike DuckDB's
    /// own self-check).
    fn corrupt_sql(&self, recipe: &ModelRecipe) -> String;

    /// Pacing hook invoked before every write-ish step (insert/run). A
    /// no-op on Spark; BigQuery's implementation applies the
    /// `bigquery_session` pacing delay measured against the live warehouse's
    /// per-table modification limit.
    async fn before_step(&self);

    /// Open a backend connection directly against `db_path` (before a
    /// [`crate::link_c_harness::LinkCProject`] exists yet) — needed by
    /// staging helpers that seed physical tables ahead of the first run.
    async fn open_backend(&self, db_path: &Path) -> Result<Box<dyn Backend>>;
}

#[cfg(test)]
mod tests {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    use crate::recipe::{arb_recipe, ConformanceTarget, RecipePool};
    use crate::render;

    /// Guards the code MOVE this phase makes: every shared family used to
    /// call `render::stage_for_target` directly, inline, hardcoded to
    /// `ConformanceTarget::SparkDelta`
    /// (`crates/smelt-cli/tests/maintenance_conformance_spark/gate_spark.rs`'s
    /// pre-extraction `stage_recipe_spark`); after the move,
    /// `families::gate::stage_recipe_for` reaches the exact same
    /// `render::stage_for_target` call, now parametrized over an explicit
    /// `target` argument instead of a hardcoded literal. `render.rs` itself
    /// is untouched by this phase (out of its Critical-files scope), so it
    /// is the fixed baseline this test compares against directly, rather
    /// than a hand-captured snapshot — exercising it twice through two
    /// independent call sites (the direct call below, standing in for the
    /// pre-extraction inline call; and the extracted wrapper) is the
    /// strongest available guard that the move changed no argument, no
    /// order, and no staged byte.
    ///
    /// Runs against `ConformanceTarget::DuckDb` (not `SparkDelta`) so this
    /// stays a network-free, always-on guard — `stage_for_target`'s DuckDb
    /// arm both writes files AND seeds its source table entirely locally.
    /// `render::stage_for_target`'s own `match` dispatches every target
    /// through the identical write-then-seed shape (only the seeding
    /// backend differs), so this exercises the same code path the Spark arm
    /// takes for its own file-writing half. The live Spark re-run (this
    /// plan's "Verification" section) is the complementary check that
    /// actually needs a reachable server.
    #[test]
    fn extracted_families_stage_byte_identical_projects_for_spark() {
        let mut runner = TestRunner::deterministic();
        let recipe = arb_recipe(RecipePool::partition_append_only())
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let direct_tmp = tempfile::TempDir::new().expect("tempdir");
        let direct_dir = direct_tmp.path().join("project");
        let direct_db = direct_tmp.path().join("unused.duckdb");
        std::fs::create_dir_all(&direct_dir).expect("mkdir");
        render::stage_for_target(&recipe, &direct_dir, &direct_db, ConformanceTarget::DuckDb)
            .expect("direct stage_for_target call");

        let extracted_tmp = tempfile::TempDir::new().expect("tempdir");
        let extracted_project = crate::families::gate::stage_recipe_for(
            &recipe,
            &extracted_tmp,
            ConformanceTarget::DuckDb,
        )
        .expect("extracted families::gate::stage_recipe_for call");

        for rel in [
            format!("models/{}.sql", recipe.model_name),
            format!("models/sources/{}.yml", recipe.source.name),
            "smelt.yml".to_string(),
        ] {
            let direct_contents = std::fs::read_to_string(direct_dir.join(&rel))
                .unwrap_or_else(|e| panic!("read direct-path staged file {rel}: {e}"));
            let extracted_contents =
                std::fs::read_to_string(extracted_project.project_dir.join(&rel))
                    .unwrap_or_else(|e| panic!("read extracted-path staged file {rel}: {e}"));
            // `smelt.yml`'s `database:` field embeds the per-call temp-dir
            // path, which necessarily differs between the two independent
            // `TempDir`s this test creates — normalize both db paths to a
            // fixed placeholder before comparing so the assertion is about
            // staged CONTENT, not incidental path identity.
            let normalize = |s: String, db: &std::path::Path| {
                s.replace(db.to_str().expect("utf8 db path"), "<db_path>")
            };
            let direct_contents = normalize(direct_contents, &direct_db);
            let extracted_contents = normalize(extracted_contents, &extracted_project.db_path);
            assert_eq!(
                direct_contents, extracted_contents,
                "staged file {rel} diverged between the direct render::stage_for_target call \
                 and the extracted families::gate::stage_recipe_for wrapper — the move changed \
                 staged output"
            );
        }
    }
}
