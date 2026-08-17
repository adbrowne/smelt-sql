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
        // with `case`.
        ConformanceTarget::SparkDelta
    }

    fn schema(&self, _case: usize) -> String {
        SPARK_CONFORMANCE_SCHEMA.to_string()
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

    fn corrupt_sql(&self, recipe: &ModelRecipe) -> String {
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

    async fn open_backend(&self, db_path: &Path) -> anyhow::Result<Box<dyn Backend>> {
        open_spark_conformance_backend(db_path).await
    }
}
