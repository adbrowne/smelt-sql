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

    /// The [`ConformanceTarget`] a case's FULL-REFRESH ORACLE TWIN runs
    /// against (`docs/plans/20260817-bigquery-generative-conformance.md`
    /// Phase 5b) — `families::dags`'s `stage_pair_for` stages TWO independent
    /// projects per case (an incremental one and a full-refresh oracle one)
    /// and needs them landing in different physical storage, or the twin's
    /// first `CREATE TABLE` collides with the incremental project's own
    /// (`409 Already Exists` on a shared BigQuery dataset — harmless on
    /// DuckDB, where each project already gets its own temp-dir `.duckdb`
    /// file regardless of `target()`'s value, and on Spark's single
    /// persistent warehouse, which already shares physical tables across
    /// every case by design). Distinct BY CONSTRUCTION — never a naming
    /// convention a caller must honour — so a backend that shares physical
    /// storage across `target(case)` cannot regress into a silent collision
    /// by forgetting to vary a name somewhere. Default: identical to
    /// `target(case)`, correct for every backend except a per-case-dataset
    /// one (BigQuery overrides both this and [`Self::twin_schema`]
    /// together).
    fn twin_target(&self, case: usize) -> ConformanceTarget {
        self.target(case)
    }

    /// [`Self::twin_target`]'s schema/dataset counterpart — the twin
    /// backend's [`Self::schema`]. Kept as a SEPARATE method (rather than
    /// deriving the schema from `twin_target` inside a helper) because
    /// [`Self::schema`] itself is independent of [`Self::target`] for the
    /// same reason (Spark's schema is a fixed constant while its target is a
    /// fixed enum variant — the two just happen to agree). Default:
    /// identical to `schema(case)`, matching [`Self::twin_target`]'s
    /// default.
    fn twin_schema(&self, case: usize) -> String {
        self.schema(case)
    }

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
    /// Takes `case` so a per-case-dataset backend (BigQuery) can
    /// resolve which case's dataset to connect to, rather than being forced
    /// to guess — `schema`/`target` already take `case` for the same reason.
    async fn open_backend(&self, case: usize, db_path: &Path) -> Result<Box<dyn Backend>>;

    /// SQL text for `(left) MINUS (right)` preserving MULTISET semantics — a
    /// row present N times more on `left` than on `right` must still
    /// surface as N divergent rows, never degrade to per-value SET
    /// membership (`oracle.rs`'s own doc comment explains why: plain
    /// `EXCEPT`/`EXCEPT DISTINCT` is blind to multiplicity, exactly the
    /// divergence class the oracle exists to catch). Default: native
    /// `EXCEPT ALL` (DuckDB, Spark/Delta both support it). GoogleSQL has no
    /// `EXCEPT ALL`; BigQuery overrides this with
    /// [`crate::oracle::bigquery_multiset_diff_sql`]'s row-ranking
    /// emulation rather than falling back to plain `EXCEPT DISTINCT`.
    fn multiset_diff_sql(&self, left_sql: &str, right_sql: &str) -> String {
        crate::oracle::except_all_diff_sql(left_sql, right_sql)
    }

    /// The source-table storage clause this backend's `CREATE TABLE` needs,
    /// INCLUDING its own leading space when non-empty (e.g. `" USING
    /// DELTA"` for Spark/Delta) so callers can splice it directly after a
    /// column list — empty for engines with no such clause (BigQuery,
    /// DuckDB).
    fn storage_clause(&self) -> &str {
        ""
    }

    /// This backend's DDL spelling for a whole-number column
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 8 —
    /// the g-10 hazard staging's hand-rolled `DOUBLE` DDL surfaced live as
    /// `400 Type not found: DOUBLE`). Default `"INT"`: DuckDB, Spark SQL,
    /// and GoogleSQL (where `INT`/`INTEGER`/`BIGINT` are documented aliases
    /// of `INT64`) all accept it, so no backend needs to override this
    /// today — it exists as the same seam [`Self::double_type`] uses, so a
    /// family body never has to hand-spell an integer column type either.
    fn int_type(&self) -> &str {
        "INT"
    }

    /// This backend's DDL spelling for a floating-point column
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 8) —
    /// GoogleSQL has no `DOUBLE` type (`400 Type not found: DOUBLE`,
    /// measured live 2026-08-19 staging `families::pinned`'s g-10 hazard
    /// facts table); it spells the IEEE double-precision type `FLOAT64`.
    /// Default `"DOUBLE"`, correct for DuckDB and Spark SQL; BigQuery
    /// overrides this with `"FLOAT64"`.
    fn double_type(&self) -> &str {
        "DOUBLE"
    }

    /// This backend's SQL dialect, for a family that needs to build a
    /// dialect-aware inline row set
    /// (`smelt_core::sql::row_set::build_row_set_table`,
    /// `docs/plans/20260817-bigquery-generative-conformance.md` Phase 7) —
    /// `gate_composed.rs`'s route-3 delta query is the one family body that
    /// needs this today. Default: `BackendType::DuckDB`, matching every
    /// backend except Spark/BigQuery, which override it.
    fn dialect(&self) -> smelt_core::config::BackendType {
        smelt_core::config::BackendType::DuckDB
    }

    /// The SQL relation reference for `S_k` (design §6's "materializes `S_k`
    /// into temp tables") — the seam every family's oracle-relation call
    /// site goes through instead of calling
    /// [`crate::s_tracker::STracker::materialize_s_as_view`] directly
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7).
    ///
    /// Default implementation reproduces today's DuckDB/Spark behaviour
    /// EXACTLY: materialize `S_k` as a session-scoped temp view via
    /// [`crate::s_tracker::STracker::materialize_s_as_view`] and return its
    /// bare unqualified name (`oracle_<source>`) — DuckDB's standing per-PR
    /// gate and the Spark leg's own re-run both depend on this staying
    /// byte-identical, so this default must never change.
    ///
    /// BigQuery overrides this: `CREATE OR REPLACE TEMPORARY VIEW` is
    /// refused outright by GoogleSQL (`400 CREATE TEMP VIEW is
    /// unsupported`, measured live 2026-08-17) — its implementation issues
    /// NO DDL at all and returns an inline derived table `(<the S_k
    /// SELECT>) AS oracle_<source>` instead (the SELECT itself comes from
    /// [`crate::s_tracker::STracker::s_select_sql`], the same portable
    /// `UNION ALL` row-set builder [`crate::s_tracker::STracker::materialize_s_as_view`]
    /// already uses) — strictly better even on a dialect that DID support
    /// temp views: no round trip, nothing to clean up.
    ///
    /// The returned relation is meant for splicing into a BARE (no
    /// trailing alias) `FROM` position — every family call site that
    /// substitutes `smelt.sources.<name>` for this value satisfies that
    /// shape EXCEPT `gate_mixed.rs`'s fact-join template, whose body
    /// already supplies its own trailing alias (`f`) immediately after the
    /// substitution point; that call site strips a derived table's `AS
    /// oracle_<name>` suffix back down to a bare `(...)` before splicing
    /// (`gate_mixed.rs`'s `bare_relation_for_alias`) — the bare-identifier
    /// default form needs no such stripping (an identifier followed by an
    /// alias is ordinary SQL either way).
    async fn oracle_relation(
        &self,
        backend: &dyn Backend,
        tracker: &crate::s_tracker::STracker,
        k: usize,
    ) -> Result<String> {
        tracker.materialize_s_as_view(backend, k).await?;
        Ok(tracker.oracle_table_name())
    }
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

    /// `no_family_hardcodes_a_backend_dialect`
    /// a source-level check that no shared family body spells a
    /// backend's dialect directly — `EXCEPT ALL` (the oracle's diff
    /// operator, now backend-supplied via `ConformanceBackend::
    /// multiset_diff_sql`), `USING DELTA` (Spark/Delta's storage clause, now
    /// `ConformanceBackend::storage_clause`), or `SPARK_CONFORMANCE_SCHEMA`
    /// (now resolved through `ConformanceBackend::schema`). This is the
    /// check Phase 4's "no `match` on target" guard should have been: a
    /// hardcoded dialect assumption is the same defect as a branch on
    /// `ConformanceTarget`, wearing different clothes — it never shows up as
    /// a `match`, but it just as surely means the family body is not
    /// actually target-generic.
    ///
    /// Scoped to the family BODY files — `mod.rs` (this file) is the
    /// `ConformanceBackend` trait's own definition, where `EXCEPT ALL` is
    /// the documented DEFAULT `multiset_diff_sql` implementation (every
    /// backend except BigQuery's override), and `oracle.rs` is the general,
    /// backend-agnostic multiset-comparison primitive (also used directly by
    /// the DuckDB-only `maintenance_conformance/{gate,pinned}.rs`, outside
    /// this plan's scope) — neither is a "family body".
    ///
    /// `"(VALUES "` joined this list in
    /// `docs/plans/20260817-bigquery-generative-conformance.md` Phase 7:
    /// this exact gate existed before that phase and did NOT catch
    /// `gate_composed.rs`'s hand-rolled `format!("(VALUES {}) AS
    /// t(id, d, val)", …)` — a hardcoded table-value constructor is the
    /// SAME class of defect as a hardcoded `EXCEPT ALL`/`USING DELTA`
    /// (a dialect assumption baked into a family body instead of resolved
    /// through `ConformanceBackend`), it just happened not to be a bare
    /// string literal this test already knew to look for. Any family body
    /// that needs an inline row set must build it through
    /// `smelt_core::sql::row_set::build_row_set_table`, parametrized over
    /// `ConformanceBackend::dialect`, the way `gate_composed.rs`'s
    /// `composed_delta_values_sql` does post-fix.
    ///
    /// `" DOUBLE,"`/`" DOUBLE)"`/`" INT,"`/`" INT)"` joined this list in
    /// Phase 8 (measured live 2026-08-19:
    /// `pinned_bigquery::hazard_schedules_are_pinned_on_bigquery` failed
    /// with `400 Type not found: DOUBLE` staging `families::pinned`'s g-10
    /// hazard facts/dims tables) — a hand-spelled DuckDB/Spark DDL type
    /// (`DOUBLE`, GoogleSQL's `FLOAT64`) is the same class of defect as a
    /// hand-spelled `(VALUES `: a dialect assumption baked into a family
    /// body's `CREATE TABLE` column list instead of resolved through
    /// `ConformanceBackend::double_type`/`ConformanceBackend::int_type`.
    /// Matched on the DDL shape (a space, then the type keyword, then the
    /// column-list separator `,` or terminator `)`) rather than the bare
    /// keyword so this never fires on `INTEGER`/`BIGINT` (no `,`/`)`
    /// immediately follows `INT` in those spellings) or on a legitimate
    /// identifier like `POINT)` (no space immediately precedes `INT` there)
    /// or on the source-YAML `type: DOUBLE\n` declaration a couple of
    /// family bodies also embed (a smelt-level `DataType` keyword, uniform
    /// across every backend and NOT the dialect assumption this gate polices
    /// — followed by `\n`, not `,`/`)`).
    #[test]
    fn no_family_hardcodes_a_backend_dialect() {
        let forbidden = [
            "EXCEPT ALL",
            "USING DELTA",
            "SPARK_CONFORMANCE_SCHEMA",
            "(VALUES ",
            " DOUBLE,",
            " DOUBLE)",
            " INT,",
            " INT)",
        ];
        let family_body_files = [
            "dags.rs",
            "feed.rs",
            "gate.rs",
            "gate_composed.rs",
            "gate_keyed.rs",
            "gate_mixed.rs",
            "harness_self_check.rs",
            "pinned.rs",
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/families");
        for file in family_body_files {
            let path = dir.join(file);
            let whole_file = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read family body file {path:?}: {e}"));
            // The scan covers the family BODY only. A file's own
            // `#[cfg(test)] mod tests` legitimately quotes a dialect's
            // expected output verbatim — `gate_composed.rs` asserts the
            // DuckDB row set is byte-identical to `(VALUES …) AS t(…)`, which
            // is the very byte-stability this gate's fix has to preserve.
            // Scanning it would make asserting the old shape impossible.
            let contents = match whole_file.find("#[cfg(test)]") {
                Some(at) => whole_file[..at].to_string(),
                None => whole_file,
            };
            for needle in forbidden {
                assert!(
                    !contents.contains(needle),
                    "{path:?} hardcodes {needle:?} — a family body must resolve a backend's \
                     dialect through ConformanceBackend (multiset_diff_sql/storage_clause/\
                     schema), never embed it directly, or it is a duplicated implementation \
                     wearing a parameter"
                );
            }
        }
    }

    /// `multiset_difference_is_backend_supplied_and_stays_a_multiset`: the
    /// family-facing plumbing (`oracle::multiset_equal_via_backend_with_diff`
    /// fed a `ConformanceBackend::multiset_diff_sql`) still detects a
    /// duplicate-only divergence for the DEFAULT diff operator (`EXCEPT
    /// ALL`, what DuckDB/Spark's `ConformanceBackend` impls use), executed
    /// here against a real DuckDB connection through the SAME seam a family
    /// body reaches (`ConformanceBackend::multiset_diff_sql`, never a
    /// hardcoded `EXCEPT ALL` in the family itself). BigQuery's row-ranking
    /// emulation (`crate::oracle::bigquery_multiset_diff_sql`) is asserted
    /// OFFLINE below (SQL shape only — DuckDB has no `TO_JSON_STRING`, so
    /// there is no honest local executor for it; `oracle.rs`'s own tests
    /// carry the same offline-only proof and explain why).
    #[tokio::test]
    async fn multiset_difference_is_backend_supplied_and_stays_a_multiset() {
        use std::path::Path;

        use anyhow::Result;
        use async_trait::async_trait;
        use smelt_backend::Backend;

        use crate::families::ConformanceBackend;
        use crate::oracle::{bigquery_multiset_diff_sql, multiset_equal_via_backend_with_diff};
        use crate::recipe::ModelRecipe;

        struct DefaultDiffBackend;
        struct BigQueryLikeDiffBackend;

        #[async_trait]
        impl ConformanceBackend for DefaultDiffBackend {
            fn target(&self, _case: usize) -> ConformanceTarget {
                unimplemented!("not exercised by this test")
            }
            fn schema(&self, _case: usize) -> String {
                unimplemented!("not exercised by this test")
            }
            fn engine_name(&self) -> &str {
                unimplemented!("not exercised by this test")
            }
            fn skip_reason(&self) -> Option<String> {
                unimplemented!("not exercised by this test")
            }
            fn corrupt_sql(&self, _recipe: &ModelRecipe) -> String {
                unimplemented!("not exercised by this test")
            }
            async fn before_step(&self) {
                unimplemented!("not exercised by this test")
            }
            async fn open_backend(
                &self,
                _case: usize,
                _db_path: &Path,
            ) -> Result<Box<dyn Backend>> {
                unimplemented!("not exercised by this test")
            }
            // multiset_diff_sql: uses the trait default (EXCEPT ALL).
        }

        #[async_trait]
        impl ConformanceBackend for BigQueryLikeDiffBackend {
            fn target(&self, _case: usize) -> ConformanceTarget {
                unimplemented!("not exercised by this test")
            }
            fn schema(&self, _case: usize) -> String {
                unimplemented!("not exercised by this test")
            }
            fn engine_name(&self) -> &str {
                unimplemented!("not exercised by this test")
            }
            fn skip_reason(&self) -> Option<String> {
                unimplemented!("not exercised by this test")
            }
            fn corrupt_sql(&self, _recipe: &ModelRecipe) -> String {
                unimplemented!("not exercised by this test")
            }
            async fn before_step(&self) {
                unimplemented!("not exercised by this test")
            }
            async fn open_backend(
                &self,
                _case: usize,
                _db_path: &Path,
            ) -> Result<Box<dyn Backend>> {
                unimplemented!("not exercised by this test")
            }
            fn multiset_diff_sql(&self, left_sql: &str, right_sql: &str) -> String {
                bigquery_multiset_diff_sql(left_sql, right_sql)
            }
        }

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open DuckDbBackend");

        for stmt in [
            "CREATE TABLE main.left_t(id BIGINT, val DOUBLE)",
            "INSERT INTO main.left_t VALUES (1, 10.0), (1, 10.0)",
            "CREATE TABLE main.right_t(id BIGINT, val DOUBLE)",
            "INSERT INTO main.right_t VALUES (1, 10.0)",
        ] {
            backend
                .execute_sql(stmt)
                .await
                .unwrap_or_else(|e| panic!("seed duplicate-only divergence ({stmt}): {e}"));
        }

        let left_sql = "SELECT * FROM main.left_t";
        let right_sql = "SELECT * FROM main.right_t";

        // Executed proof: the default diff operator, reached ONLY through
        // `ConformanceBackend::multiset_diff_sql` (never a family hardcoding
        // `EXCEPT ALL` itself), still flags the seeded duplicate.
        let default_backend: Box<dyn ConformanceBackend> = Box::new(DefaultDiffBackend);
        let equal = multiset_equal_via_backend_with_diff(&backend, left_sql, right_sql, |l, r| {
            default_backend.multiset_diff_sql(l, r)
        })
        .await
        .expect("default diff operator's multiset comparison must not error");
        assert!(
            !equal,
            "the default (EXCEPT ALL) diff operator, reached through \
             ConformanceBackend::multiset_diff_sql, failed to flag a duplicate-only divergence"
        );

        // Offline proof: BigQuery's `ConformanceBackend` override reaches
        // the SAME row-ranking emulation `oracle.rs` defines. Not executed
        // here — see `oracle.rs`'s own offline shape test
        // (`bigquery_diff_sql_partitions_by_to_json_string_and_differences_ranked_rows`)
        // for why (DuckDB has no `TO_JSON_STRING`).
        let bq_backend: Box<dyn ConformanceBackend> = Box::new(BigQueryLikeDiffBackend);
        let bq_sql = bq_backend.multiset_diff_sql(left_sql, right_sql);
        assert!(
            bq_sql.contains("TO_JSON_STRING(t)") && bq_sql.contains("EXCEPT DISTINCT"),
            "BigQueryLikeDiffBackend's ConformanceBackend override must reach \
             oracle::bigquery_multiset_diff_sql's row-ranking emulation, not some other \
             diff shape: {bq_sql:?}"
        );
    }

    /// `oracle_relation_default_reproduces_materialize_s_as_view_and_returns_bare_name`
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7,
    /// Gap 1): the trait's DEFAULT `oracle_relation` implementation — the
    /// one DuckDB and Spark both use — must still call
    /// `STracker::materialize_s_as_view` and return the bare
    /// `oracle_<source>` name unchanged, executed here against a real
    /// DuckDB backend so this is a byte-for-byte behavioural guard, not
    /// just a read of the source.
    #[tokio::test]
    async fn oracle_relation_default_reproduces_materialize_s_as_view_and_returns_bare_name() {
        use std::path::Path;

        use anyhow::Result;
        use smelt_backend::Backend;
        use smelt_runtime::check_runner::batches_to_rows;

        use crate::families::ConformanceBackend;
        use crate::recipe::{KeyShape, ModelRecipe, SourcePosture, SourceRecipe};
        use crate::s_tracker::STracker;

        struct DefaultRelationBackend;

        #[async_trait::async_trait]
        impl ConformanceBackend for DefaultRelationBackend {
            fn target(&self, _case: usize) -> ConformanceTarget {
                unimplemented!("not exercised by this test")
            }
            fn schema(&self, _case: usize) -> String {
                unimplemented!("not exercised by this test")
            }
            fn engine_name(&self) -> &str {
                unimplemented!("not exercised by this test")
            }
            fn skip_reason(&self) -> Option<String> {
                unimplemented!("not exercised by this test")
            }
            fn corrupt_sql(&self, _recipe: &ModelRecipe) -> String {
                unimplemented!("not exercised by this test")
            }
            async fn before_step(&self) {
                unimplemented!("not exercised by this test")
            }
            async fn open_backend(
                &self,
                _case: usize,
                _db_path: &Path,
            ) -> Result<Box<dyn Backend>> {
                unimplemented!("not exercised by this test")
            }
            // target/schema/dialect/oracle_relation: trait defaults.
        }

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let duckdb = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open DuckDbBackend");

        let source = SourceRecipe {
            name: "events".to_string(),
            clock_column: "d".to_string(),
            key_column: "id".to_string(),
            payload_column: "val".to_string(),
            key_shape: KeyShape::Single,
            posture: SourcePosture::AppendOnly,
            key_recurrence: None,
        };
        let mut tracker = STracker::new(&source);
        let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
        let k = tracker.record_run(
            d,
            d + chrono::Duration::days(1),
            vec![crate::schedule_gen::GenRow { d, id: 1, val: 10 }],
        );

        let backend = DefaultRelationBackend;
        let relation = backend
            .oracle_relation(&duckdb, &tracker, k)
            .await
            .expect("default oracle_relation must succeed");
        assert_eq!(
            relation,
            tracker.oracle_table_name(),
            "the default oracle_relation must return the bare oracle_<source> name, unchanged \
             from today's behaviour"
        );

        // The relation must actually be queryable — proof the default
        // really did materialize the view via `materialize_s_as_view`
        // rather than merely returning a name nothing backs.
        let batches = duckdb
            .execute_sql(&format!("SELECT COUNT(*) AS n FROM {relation}"))
            .await
            .unwrap_or_else(|e| panic!("query materialized oracle relation {relation:?}: {e}"));
        let rows = batches_to_rows(&batches);
        assert_eq!(
            rows.first().and_then(|r| r.get("n")).cloned(),
            Some("1".to_string()),
            "materialized oracle view must contain exactly the one recorded row"
        );
    }

    /// `oracle_relation_bigquery_shape_emits_no_ddl_and_returns_a_derived_table`
    /// (plan Phase 7, Gap 1): a BigQuery-shaped override — built the same
    /// way `BigQueryConformanceBackend::oracle_relation`
    /// (`crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs`)
    /// is — must issue NO backend calls at all (proven with a call-counting
    /// `Backend` double that panics if touched) and must return an inline
    /// parenthesised derived table (`(<select>) AS oracle_<name>`), never a
    /// `CREATE`/`DROP` DDL statement. The returned relation is additionally
    /// executed against a real DuckDB connection (DuckDB accepts the same
    /// portable `UNION ALL` shape GoogleSQL requires) to prove it is valid,
    /// queryable SQL, not just a string shape.
    #[tokio::test]
    async fn oracle_relation_bigquery_shape_emits_no_ddl_and_returns_a_derived_table() {
        use std::path::Path;

        use anyhow::Result;
        use smelt_backend::{Backend, BackendCapabilities};

        use crate::families::ConformanceBackend;
        use crate::recipe::{KeyShape, ModelRecipe, SourcePosture, SourceRecipe};
        use crate::s_tracker::STracker;

        /// Panics on any DDL/query call — proves a caller never reaches the
        /// backend at all.
        struct PanicIfTouchedBackend;

        #[async_trait::async_trait]
        impl Backend for PanicIfTouchedBackend {
            async fn execute_sql(
                &self,
                sql: &str,
            ) -> std::result::Result<
                Vec<arrow::record_batch::RecordBatch>,
                smelt_backend::BackendError,
            > {
                panic!("BigQuery's oracle_relation must issue NO backend calls, got: {sql:?}")
            }
            async fn create_table_as(
                &self,
                _schema: &str,
                _name: &str,
                _sql: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no create_table_as call")
            }
            async fn create_view_as(
                &self,
                _schema: &str,
                _name: &str,
                _sql: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no create_view_as call")
            }
            async fn drop_table_if_exists(
                &self,
                _schema: &str,
                _name: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no drop_table_if_exists call")
            }
            async fn drop_view_if_exists(
                &self,
                _schema: &str,
                _name: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no drop_view_if_exists call")
            }
            async fn get_row_count(
                &self,
                _schema: &str,
                _name: &str,
            ) -> std::result::Result<usize, smelt_backend::BackendError> {
                unimplemented!("not exercised by this test")
            }
            async fn get_preview(
                &self,
                _schema: &str,
                _name: &str,
                _limit: usize,
            ) -> std::result::Result<
                Vec<arrow::record_batch::RecordBatch>,
                smelt_backend::BackendError,
            > {
                unimplemented!("not exercised by this test")
            }
            async fn table_exists(
                &self,
                _schema: &str,
                _name: &str,
            ) -> std::result::Result<bool, smelt_backend::BackendError> {
                unimplemented!("not exercised by this test")
            }
            async fn ensure_schema(
                &self,
                _schema: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                unimplemented!("not exercised by this test")
            }
            fn dialect(&self) -> smelt_backend::SqlDialect {
                unimplemented!("not exercised by this test")
            }
            fn capabilities(&self) -> BackendCapabilities {
                unimplemented!("not exercised by this test")
            }
            async fn load_table(
                &self,
                _schema: &str,
                _name: &str,
                _arrow_schema: arrow::datatypes::SchemaRef,
                _batches: Vec<arrow::record_batch::RecordBatch>,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                unimplemented!("not exercised by this test")
            }
            async fn delete_partitions(
                &self,
                _schema: &str,
                _name: &str,
                _partition: &smelt_backend::PartitionRange,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no delete_partitions call")
            }
            async fn insert_into_from_query(
                &self,
                _schema: &str,
                _name: &str,
                _sql: &str,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no insert_into_from_query call")
            }
            async fn insert_overwrite(
                &self,
                _schema: &str,
                _table: &str,
                _sql: &str,
                _partition: &smelt_backend::PartitionRange,
            ) -> std::result::Result<(), smelt_backend::BackendError> {
                panic!("BigQuery's oracle_relation must issue no insert_overwrite call")
            }
        }

        struct BigQueryLikeRelationBackend;

        #[async_trait::async_trait]
        impl ConformanceBackend for BigQueryLikeRelationBackend {
            fn target(&self, _case: usize) -> ConformanceTarget {
                unimplemented!("not exercised by this test")
            }
            fn schema(&self, _case: usize) -> String {
                unimplemented!("not exercised by this test")
            }
            fn engine_name(&self) -> &str {
                unimplemented!("not exercised by this test")
            }
            fn skip_reason(&self) -> Option<String> {
                unimplemented!("not exercised by this test")
            }
            fn corrupt_sql(&self, _recipe: &ModelRecipe) -> String {
                unimplemented!("not exercised by this test")
            }
            async fn before_step(&self) {
                unimplemented!("not exercised by this test")
            }
            async fn open_backend(
                &self,
                _case: usize,
                _db_path: &Path,
            ) -> Result<Box<dyn Backend>> {
                unimplemented!("not exercised by this test")
            }
            fn dialect(&self) -> smelt_core::config::BackendType {
                smelt_core::config::BackendType::BigQuery
            }
            async fn oracle_relation(
                &self,
                _backend: &dyn Backend,
                tracker: &STracker,
                k: usize,
            ) -> Result<String> {
                Ok(format!(
                    "({}) AS {}",
                    tracker.s_select_sql(k),
                    tracker.oracle_table_name()
                ))
            }
        }

        let source = SourceRecipe {
            name: "events".to_string(),
            clock_column: "d".to_string(),
            key_column: "id".to_string(),
            payload_column: "val".to_string(),
            key_shape: KeyShape::Single,
            posture: SourcePosture::AppendOnly,
            key_recurrence: None,
        };
        let mut tracker = STracker::new(&source);
        let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
        let k = tracker.record_run(
            d,
            d + chrono::Duration::days(1),
            vec![crate::schedule_gen::GenRow { d, id: 1, val: 10 }],
        );

        let backend = BigQueryLikeRelationBackend;
        let panic_backend = PanicIfTouchedBackend;
        let relation = backend
            .oracle_relation(&panic_backend, &tracker, k)
            .await
            .expect("BigQuery-shaped oracle_relation must succeed without touching the backend");

        assert!(
            relation.starts_with('(') && relation.ends_with(&tracker.oracle_table_name()),
            "expected a parenthesised derived table aliased to oracle_<source>, got: {relation:?}"
        );
        let upper = relation.to_uppercase();
        assert!(
            !upper.contains("CREATE") && !upper.contains("DROP"),
            "the BigQuery shape must issue no DDL text at all, got: {relation:?}"
        );

        // Prove it's actually valid, queryable SQL (DuckDB accepts the same
        // portable UNION ALL shape GoogleSQL requires).
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let duckdb = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open DuckDbBackend");
        let batches = duckdb
            .execute_sql(&format!("SELECT COUNT(*) AS n FROM {relation}"))
            .await
            .unwrap_or_else(|e| panic!("query inline derived-table relation {relation:?}: {e}"));
        let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
        assert_eq!(
            rows.first().and_then(|r| r.get("n")).cloned(),
            Some("1".to_string()),
            "the inline derived table must contain exactly the one recorded row"
        );
    }
}
