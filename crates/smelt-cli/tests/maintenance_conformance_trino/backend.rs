//! `TrinoConformanceBackend` — the Trino-arm implementation of
//! `smelt_maintenance_testkit::families::ConformanceBackend`
//! (`docs/outcomes/20260913-trino-incremental/phases/08-plan.md`). Every
//! `#[test]` wrapper in this binary constructs one (naming the family it
//! belongs to, so schemas from different families never collide) and calls
//! the matching `families::<family>::run_<family>` entry point — the family
//! bodies themselves never know they are talking to Trino.
//!
//! Unlike Spark's one persistent schema shared by every case
//! (`SparkConformanceBackend::target`), a `TrinoConformanceBackend` derives a
//! FRESH schema per case
//! (`smelt_maintenance_testkit::recipe::trino_conformance_schema`) — the same
//! per-case-schema shape `BigQueryConformanceBackend` uses, since Trino's
//! Iceberg REST catalog is likewise one shared coordinator/catalog where only
//! the schema separates two cases' tables (3e's isolation ruling, already
//! landed elsewhere in this outcome for the other live Trino suites).
//!
//! No pacing and no credential-window preflight: unlike BigQuery's live
//! warehouse, this binary talks to a local Docker Trino coordinator with no
//! per-table modification quota and no short-lived OAuth token to budget
//! against.

use std::path::Path;

use smelt_backend::Backend;
use smelt_maintenance_testkit::families::ConformanceBackend;
use smelt_maintenance_testkit::link_c_harness::open_trino_conformance_backend;
use smelt_maintenance_testkit::recipe::{
    trino_conformance_schema, trino_env, ConformanceTarget, ModelRecipe,
};

pub struct TrinoConformanceBackend {
    /// Names the family this backend instance belongs to
    /// (`trino_conformance_schema`'s `family` argument) — keeps two
    /// families' schemas apart the same way `BigQueryConformanceBackend`
    /// keeps every BigQuery case apart, except Trino additionally varies per
    /// `case` within a family (mirroring BigQuery exactly, not Spark's one
    /// constant schema).
    family: &'static str,
}

impl TrinoConformanceBackend {
    pub fn new(family: &'static str) -> Self {
        Self { family }
    }
}

#[async_trait::async_trait]
impl ConformanceBackend for TrinoConformanceBackend {
    fn target(&self, case: usize) -> ConformanceTarget {
        ConformanceTarget::Trino {
            schema: trino_conformance_schema(self.family, &case.to_string()),
        }
    }

    fn schema(&self, case: usize) -> String {
        trino_conformance_schema(self.family, &case.to_string())
    }

    fn twin_target(&self, case: usize) -> ConformanceTarget {
        // A fresh schema, distinct from `target(case)`'s — mirrors
        // `BigQueryConformanceBackend::twin_target`'s reasoning exactly:
        // `families::dags`'s full-refresh oracle twin must never race the
        // incremental project to create the same table in the SAME schema.
        ConformanceTarget::Trino {
            schema: trino_conformance_schema(self.family, &format!("{case}-full")),
        }
    }

    fn twin_schema(&self, case: usize) -> String {
        trino_conformance_schema(self.family, &format!("{case}-full"))
    }

    fn engine_name(&self) -> &str {
        "trino"
    }

    fn skip_reason(&self) -> Option<String> {
        if trino_env().is_some() {
            None
        } else {
            Some("SMELT_TRINO_URL unset".to_string())
        }
    }

    fn corrupt_sql(&self, case: usize, recipe: &ModelRecipe) -> String {
        // An unconditional, syntactically legal whole-table mutation — no
        // subquery needed (mirrors Spark's own whole-table bump, not
        // DuckDB's subquery-based one).
        format!(
            "UPDATE {schema}.{table} SET total = total + 999999",
            schema = self.schema(case),
            table = recipe.model_name,
        )
    }

    async fn before_step(&self) {
        // No pacing needed against a local Docker Trino coordinator.
    }

    async fn open_backend(&self, case: usize, _db_path: &Path) -> anyhow::Result<Box<dyn Backend>> {
        open_trino_conformance_backend(&self.schema(case)).await
    }

    fn dialect(&self) -> smelt_core::config::BackendType {
        smelt_core::config::BackendType::Trino
    }

    fn string_type(&self) -> &str {
        // Trino has no `STRING` type (`Unknown type: STRING`, measured live
        // against a real coordinator) — it spells the same concept
        // `VARCHAR`.
        "VARCHAR"
    }

    fn excluded_constructs(&self) -> &[smelt_maintenance_testkit::recipe::ConstructKind] {
        // `MEDIAN` has no native Trino form and no exact ordered-set
        // aggregate either (measured live: `Function 'median' not
        // registered`; Trino's only percentile function is the approximate
        // `approx_percentile`, which would make the equivalence oracle flag
        // divergences that are artefacts of the approximation rather than
        // real bugs). Tracked as a registry gap in
        // `crates/smelt-db/tests/dialect_audit/ledger.rs` (`MEDIAN`, Trino,
        // issue #209) and `docs/specs/multi_backend.md` §"Exact-median
        // lowering" — not a fix this harness makes on its own.
        &[smelt_maintenance_testkit::recipe::ConstructKind::HolisticAgg]
    }

    async fn oracle_relation(
        &self,
        _backend: &dyn Backend,
        tracker: &smelt_maintenance_testkit::s_tracker::STracker,
        k: usize,
    ) -> anyhow::Result<String> {
        // Trino has no session-scoped temporary view — the same hook
        // BigQuery's `oracle_relation` override uses: issue NO DDL at all
        // and return an inline derived table over the SAME portable `S_k`
        // row-set query `STracker::materialize_s_as_view` would have
        // materialized, aliased to the same bare name every family body
        // already substitutes for `smelt.sources.<name>`.
        Ok(format!(
            "({}) AS {}",
            tracker.s_select_sql(k),
            tracker.oracle_table_name()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use smelt_maintenance_testkit::families::ConformanceBackend;

    use super::TrinoConformanceBackend;

    /// `the_full_refresh_twin_lands_in_different_storage_than_the_incremental_project`
    /// (test 1, `docs/outcomes/20260913-trino-incremental/phases/08-plan.md`):
    /// a case's incremental project and its full-refresh oracle twin resolve
    /// different `(target, schema)` — mirrors
    /// `maintenance_conformance_bigquery::backend::tests`'s own guard (there
    /// is no equivalent test in the Spark twin, since Spark's target does
    /// not vary by case at all and the twin override is what makes it safe
    /// there; here the trait's per-case DEFAULT is what would be unsafe if
    /// `twin_target`/`twin_schema` were dropped).
    #[test]
    fn the_full_refresh_twin_lands_in_different_storage_than_the_incremental_project() {
        let b = TrinoConformanceBackend::new("harness_self_check");
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

    /// `every_case_gets_its_own_schema` (test 2): `schema(case)` is distinct
    /// across cases and legal per `trino_ci_wiring.rs`'s
    /// `assert_legal_trino_identifier` rules (3e's isolation ruling: no
    /// shared namespace between live tests) — pure, no credentials needed.
    #[test]
    fn every_case_gets_its_own_schema() {
        let b = TrinoConformanceBackend::new("every_case_gets_its_own_schema");
        let names: Vec<String> = (0..20).map(|case| b.schema(case)).collect();
        let unique: HashSet<&String> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "20 cases in one run must resolve to 20 distinct schemas"
        );
        for name in &names {
            let first = name.chars().next().expect("non-empty schema name");
            assert!(
                first.is_ascii_alphabetic(),
                "schema name {name:?} must start with an ASCII letter"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "schema name {name:?} must contain only [a-z0-9_]"
            );
        }
    }

    /// A different family must never collide with this one's schema names —
    /// mirrors `BigQueryConformanceBackend`'s own family-isolation property.
    #[test]
    fn different_families_never_share_a_schema() {
        let a = TrinoConformanceBackend::new("family_a");
        let b = TrinoConformanceBackend::new("family_b");
        for case in 0..5 {
            assert_ne!(
                a.schema(case),
                b.schema(case),
                "case {case}: two different families must not share a schema name"
            );
        }
    }
}
