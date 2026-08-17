//! `BigQueryConformanceBackend` — the BigQuery-arm implementation of
//! `smelt_maintenance_testkit::families::ConformanceBackend`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5). Every
//! `#[test]` wrapper in this binary constructs one (naming the family it
//! belongs to, so datasets from different families never collide) and calls
//! the matching `families::<family>::run_<family>` entry point — the family
//! bodies themselves never know they are talking to BigQuery.
//!
//! Unlike Spark's one persistent schema shared by every case
//! (`SparkConformanceBackend::target`), a `BigQueryConformanceBackend`
//! derives a FRESH dataset per case
//! (`smelt_maintenance_testkit::recipe::bq_conformance_dataset`) — the design
//! Phase 2 built the `ConformanceTarget::BigQuery { dataset }` seam for.
//! `open_backend` now takes `case`, so this implementation opens the same
//! per-case dataset `target`/`schema` already resolve, rather than guessing
//! or refusing. The mixed-dimension staging path
//! (`smelt_maintenance_testkit::families::gate_mixed::stage_mixed_recipe_for`
//! and `families::pinned`'s mixed/g-10 hazard cases) also no longer
//! hardcodes Delta-only `USING DELTA` DDL — it splices
//! `ConformanceBackend::storage_clause()` (empty for BigQuery) after its
//! column lists instead. The multiset-equality oracle's diff operator also
//! moved onto this trait (`multiset_diff_sql`, overridden below with
//! `smelt_maintenance_testkit::oracle::bigquery_multiset_diff_sql`'s
//! row-ranking emulation of `EXCEPT ALL`, which GoogleSQL lacks), so every
//! wrapper in this binary that reaches a live comparison now emits
//! GoogleSQL-valid SQL rather than a syntax error for THIS operator
//! specifically. Confirmed live (2026-08-17): none of the above three fixes
//! is implicated in any of the first live run's 14 failures — see `main.rs`'s
//! doc comment for the three further defects the run surfaced instead
//! (`s_tracker.rs`'s `VALUES`-table-constructor oracle materialization,
//! `smelt-runtime`'s `ephemeral_seed_ctes` compiler path hitting the same
//! GoogleSQL gap, and `families::dags`'s two-project-per-case dataset
//! collision), none of which is this file's own code.

use std::path::Path;
use std::time::Duration;

use smelt_backend::Backend;
use smelt_maintenance_testkit::bigquery_session::{pace, preflight, TABLE_MODIFICATION_PACING};
use smelt_maintenance_testkit::families::ConformanceBackend;
use smelt_maintenance_testkit::recipe::{
    bq_conformance_dataset, bq_project, ConformanceTarget, ModelRecipe,
};

/// Coarse per-test ceiling passed to [`preflight`] before any wrapper starts
/// its sweep. Not a tight measured bound (Phase 1 measured per-STATEMENT
/// pacing, not whole-sweep wall-clock for every family) — a generous, uniform
/// estimate is enough to catch the case that actually matters: a token with
/// only a few minutes left starting a sweep that needs the better part of an
/// hour.
const PREFLIGHT_ESTIMATE: Duration = Duration::from_secs(600);

/// Pure predicate behind [`ConformanceBackend::skip_reason`] — credentials
/// ABSENT (no project configured at all) is the only condition that skips
/// green, mirrors `bq_env`'s convention (`crates/smelt-cli/tests/common/mod.rs`)
/// and `docs/specs/multi_backend.md` §Surface's `SMELT_BQ_PROJECT` entry. A
/// project set but a missing/expired/invalid token is deliberately NOT a
/// skip: that surfaces as a loud failure from `preflight_or_panic` or from
/// the live connection attempt itself (`classify_bq_error`'s `Auth` class),
/// never silently absorbed. Factored out as a pure function (rather than
/// asserted only through `skip_reason` on a live-constructed backend) so
/// `skips_green_when_SMELT_BQ_PROJECT_is_unset` can assert this WITHOUT
/// mutating process-global environment state (which would race against every
/// other test in this binary reading the same variable).
pub fn skip_reason_for_project(project: Option<&str>) -> Option<String> {
    if project.is_none() {
        Some("SMELT_BQ_PROJECT unset".to_string())
    } else {
        None
    }
}

pub struct BigQueryConformanceBackend {
    /// Names the family this backend instance belongs to
    /// (`bq_conformance_dataset`'s `family` argument) — keeps two families'
    /// datasets apart the same way `SPARK_CONFORMANCE_SCHEMA` keeps every
    /// Spark case in one fixed schema, except BigQuery additionally varies
    /// per `case` within a family.
    family: &'static str,
}

impl BigQueryConformanceBackend {
    pub fn new(family: &'static str) -> Self {
        Self { family }
    }

    /// Refuse to start a sweep the current token cannot outlive — see
    /// [`PREFLIGHT_ESTIMATE`]'s doc comment. Called once at the top of every
    /// `#[test]` wrapper, after the `skip_reason` check (credentials absent
    /// ⇒ skip green; credentials present but the token's window is too short
    /// ⇒ this panics, a loud failure, never a skip).
    pub fn preflight_or_panic(&self) {
        preflight(
            smelt_maintenance_testkit::bigquery_session::default_token_path(),
            PREFLIGHT_ESTIMATE,
        )
        .unwrap_or_else(|e| {
            panic!(
                "BigQuery token preflight failed for family {:?}: {e}",
                self.family
            )
        });
    }
}

#[async_trait::async_trait]
impl ConformanceBackend for BigQueryConformanceBackend {
    fn target(&self, case: usize) -> ConformanceTarget {
        ConformanceTarget::BigQuery {
            dataset: bq_conformance_dataset(self.family, &case.to_string()),
        }
    }

    fn schema(&self, case: usize) -> String {
        bq_conformance_dataset(self.family, &case.to_string())
    }

    fn engine_name(&self) -> &str {
        "bq"
    }

    fn skip_reason(&self) -> Option<String> {
        skip_reason_for_project(bq_project().as_deref())
    }

    fn corrupt_sql(&self, recipe: &ModelRecipe) -> String {
        // GoogleSQL's `UPDATE` also refuses a bare unconditional statement
        // with no WHERE unless the target uses `WHERE true` explicitly —
        // this is unconditional-but-syntactically-legal, mirroring Spark's
        // own whole-table bump (`SparkConformanceBackend::corrupt_sql`)
        // rather than DuckDB's subquery-based one.
        format!(
            "UPDATE {schema}.{table} SET total = total + 999999 WHERE true",
            schema = self.schema(0),
            table = recipe.model_name,
        )
    }

    async fn before_step(&self) {
        // Phase 1's measured per-table modification floor
        // (`docs/research/20260816-bigquery-backend.md` §"Measured against
        // the live warehouse"): 3s spacing is the floor at which BigQuery's
        // table-update burst limit stops binding.
        pace(TABLE_MODIFICATION_PACING);
    }

    async fn open_backend(&self, case: usize, _db_path: &Path) -> anyhow::Result<Box<dyn Backend>> {
        // `ConformanceBackend::open_backend` now takes `case`, so a
        // per-case-dataset backend no longer has to guess or refuse — it
        // opens the SAME dataset `target(case)`/`schema(case)` name.
        smelt_maintenance_testkit::link_c_harness::open_bigquery_backend(&self.schema(case)).await
    }

    fn multiset_diff_sql(&self, left_sql: &str, right_sql: &str) -> String {
        // GoogleSQL has no `EXCEPT ALL` — see
        // `smelt_maintenance_testkit::oracle::bigquery_multiset_diff_sql`'s
        // own doc comment for the row-ranking emulation this delegates to
        // (never a degraded `EXCEPT DISTINCT`, which would go blind to a
        // duplicate-only divergence).
        smelt_maintenance_testkit::oracle::bigquery_multiset_diff_sql(left_sql, right_sql)
    }
}
