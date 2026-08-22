//! `SparkConformanceBackend` — the Spark-arm implementation of
//! `smelt_maintenance_testkit::families::ConformanceBackend`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4). Every
//! `#[test]` wrapper in this binary constructs one and calls the matching
//! `families::<family>::run_<family>` entry point — the family bodies
//! themselves never know they are talking to Spark.

use std::path::Path;

use smelt_backend::Backend;
use smelt_maintenance_testkit::families::ConformanceBackend;
use smelt_maintenance_testkit::link_c_harness::open_spark_conformance_backend;
use smelt_maintenance_testkit::recipe::{ConformanceTarget, ModelRecipe, SPARK_CONFORMANCE_SCHEMA};

pub struct SparkConformanceBackend;

/// `SPARK_CONNECT_URL` from the environment — mirrors
/// `crates/smelt-cli/tests/common/mod.rs::spark_connect_url`'s convention,
/// kept local (rather than pulling in that whole shared module) since this
/// test target is a standalone binary, like `maintenance_conformance` is.
pub fn spark_connect_url() -> Option<String> {
    std::env::var("SPARK_CONNECT_URL").ok()
}

#[async_trait::async_trait]
impl ConformanceBackend for SparkConformanceBackend {
    fn target(&self, _case: usize) -> ConformanceTarget {
        // The Spark/Delta warehouse is one persistent schema shared across
        // every case in this binary (`SPARK_CONFORMANCE_SCHEMA`) — unlike a
        // per-case-dataset backend (BigQuery), Spark's target does not vary
        // with `case`. Staging is drop-before-seed, so consecutive cases
        // reusing these tables is safe; what is NOT safe is a case's
        // full-refresh oracle twin sharing them, which is why
        // `twin_target`/`twin_schema` below are overridden.
        ConformanceTarget::spark_delta()
    }

    fn schema(&self, _case: usize) -> String {
        SPARK_CONFORMANCE_SCHEMA.to_string()
    }

    /// Spark MUST override the twin seam, even though its target does not
    /// vary by case.
    ///
    /// The trait default makes the twin identical to the incremental
    /// project, which is only safe when the target itself separates the two
    /// projects' physical storage. DuckDB satisfies that incidentally — each
    /// staged project gets its own temp-dir `.duckdb` file. Spark does not:
    /// one persistent Delta warehouse, one `spark_catalog`, one warehouse
    /// directory, and fixed DAG node names, so under the default a case's
    /// incremental project and its full-refresh oracle wrote the *same*
    /// tables. That made `families::dags`'s per-node equality assertion
    /// compare a table against itself — it would have passed even if the
    /// incremental engine were wrong — and it also let the twin's source
    /// inserts pile on top of the incremental project's, doubling the rows
    /// the oracle refreshed from.
    fn twin_target(&self, case: usize) -> ConformanceTarget {
        ConformanceTarget::SparkDelta {
            schema: self.twin_schema(case),
        }
    }

    fn twin_schema(&self, _case: usize) -> String {
        format!("{SPARK_CONFORMANCE_SCHEMA}_twin")
    }

    fn engine_name(&self) -> &str {
        "spark"
    }

    fn skip_reason(&self) -> Option<String> {
        if spark_connect_url().is_some() {
            None
        } else {
            Some("SPARK_CONNECT_URL unset".to_string())
        }
    }

    fn corrupt_sql(&self, _case: usize, recipe: &ModelRecipe) -> String {
        // `case` is unused here: Spark's conformance schema is one constant
        // shared by every case, unlike BigQuery's per-case dataset.
        //
        // Delta's `UPDATE` refuses a subquery in the SET/WHERE clause
        // (`DELTA_UNSUPPORTED_SUBQUERY`), unlike DuckDB's own self-check
        // (`WHERE total = (SELECT MIN(total) ...)`) — an unconditional
        // whole-table bump needs no subquery and is just as effective a
        // seeded divergence (every row's `total` no longer matches the
        // oracle).
        format!(
            "UPDATE {schema}.{table} SET total = total + 999999",
            schema = SPARK_CONFORMANCE_SCHEMA,
            table = recipe.model_name,
        )
    }

    async fn before_step(&self) {
        // No pacing needed against a local/dev Spark Connect server.
    }

    async fn open_backend(&self, _case: usize, db_path: &Path) -> anyhow::Result<Box<dyn Backend>> {
        open_spark_conformance_backend(db_path).await
    }

    fn storage_clause(&self) -> &str {
        // Delta's own storage clause — the mixed-pool/pinned-hazard staging
        // paths splice this directly after a `CREATE TABLE` column list.
        // `multiset_diff_sql` uses the trait default (`EXCEPT ALL`, which
        // Spark SQL supports natively) — no override needed here.
        " USING DELTA"
    }

    fn dialect(&self) -> smelt_core::config::BackendType {
        // Spark SQL keeps the `VALUES` table-value constructor
        // (`smelt_core::sql::row_set::row_set_body`'s DuckDB/Spark arm) —
        // `gate_composed.rs`'s route-3 delta query stays byte-identical to
        // today for this backend.
        smelt_core::config::BackendType::Spark
    }
}

#[cfg(test)]
mod tests {
    use super::SparkConformanceBackend;
    use smelt_maintenance_testkit::families::ConformanceBackend;

    /// A case's incremental project and its full-refresh oracle twin must
    /// land in DIFFERENT physical storage on Spark, or
    /// `families::dags`'s `assert_every_node_equal_for` compares one table
    /// against itself and passes no matter what the incremental engine did.
    ///
    /// Spark shares one persistent Delta warehouse across every project in
    /// this binary — same connect URL, same `spark_catalog`, and (because
    /// `SMELT_SPARK_WAREHOUSE` is exported by `scripts/spark-env.sh`) the
    /// same warehouse directory regardless of a project's `db_path`. The DAG
    /// node names are fixed literals, so "different storage" can only come
    /// from the schema. Unlike DuckDB — where each staged project gets its
    /// own temp-dir `.duckdb` file, so the trait default (twin identical to
    /// the incremental target) is already physically distinct — Spark must
    /// override the twin seam explicitly.
    #[test]
    fn the_full_refresh_twin_lands_in_different_storage_than_the_incremental_project() {
        let b = SparkConformanceBackend;
        for case in 0..3 {
            assert_ne!(
                (
                    ConformanceBackend::target(&b, case),
                    ConformanceBackend::schema(&b, case)
                ),
                (
                    ConformanceBackend::twin_target(&b, case),
                    ConformanceBackend::twin_schema(&b, case)
                ),
                "case {case}: the full-refresh twin shares physical storage with the \
                 incremental project, so every dags equality assertion reads one table \
                 twice and would pass even if the incremental engine were wrong"
            );
        }
    }
}
