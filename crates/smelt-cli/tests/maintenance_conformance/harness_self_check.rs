//! The harness-is-not-lying self-check
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3;
//! pattern: `crates/smelt-db/tests/nullability_property_tests.rs`'s
//! seeded-divergence check). If the S-restricted oracle never actually
//! fails, `gate.rs`'s standing pass could just mean the oracle is
//! vacuously true — this test proves it isn't by directly corrupting a
//! maintained output row and asserting the oracle catches it.

use duckdb::Connection;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::oracle::multiset_equal;
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, RecipePool};
use smelt_maintenance_testkit::schedule_gen::arb_schedule_for;
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate::{assert_equivalence, drive_and_assert, stage_recipe};

/// `oracle_flags_a_seeded_divergence` (plan Phase 3 TDD list): after a
/// green run, directly corrupt one output row via a raw connection and
/// assert the oracle reports inequality.
#[test]
fn oracle_flags_a_seeded_divergence() {
    // Pin to AdditiveAgg — `verdict.rs`'s own pinned test guarantees this
    // construct always admits `Technique::DeleteInsert` over an append-only
    // source, so this case is reliably green before corruption.
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();
    let schedule = arb_schedule_for(&recipe)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the pinned additive-agg append-only recipe to admit: {verdict:?}"
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tracker, k, _migrate_apply_exit_codes) = rt
        .block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect("green run must uphold equivalence before corruption");

    // Directly corrupt one maintained output row via a raw connection —
    // bypassing smelt's run pipeline entirely.
    {
        let conn = project.connect().expect("connect for corruption");
        conn.execute(
            &format!(
                "UPDATE main.{table} SET total = total + 999999 \
                 WHERE total = (SELECT MIN(total) FROM main.{table})",
                table = recipe.model_name,
            ),
            [],
        )
        .expect("seed a divergence");
    }

    let result = rt.block_on(assert_equivalence(&project, &recipe, &tracker, k));
    assert!(
        result.is_err(),
        "oracle failed to flag a seeded divergence — the harness would be silently \
         reporting equivalence even when the maintained output is wrong"
    );
}

/// `float_equivalence_comparison_tolerates_last_bit_only` (phase 8 task 6,
/// `docs/outcomes/20260809-rung2-state-shapes/phases/08-plan.md`): pins the
/// float-aware end-state comparison's tolerance (`gate.rs`'s
/// `rounded_select_list`, `ROUND(col, 6)` over `DOUBLE`/`FLOAT`/`REAL`
/// presented columns) so it cannot silently widen into swallowing a real
/// fold bug. DuckDB's own `STDDEV_SAMP` uses a numerically stable pass
/// while the decomposed `(n, Σx, Σx²)` recompute this outcome derives does
/// not, so the two agree only to floating-point noise (~1e-12) — a
/// perturbation at that scale must still compare equal after rounding, but
/// a perturbation three orders of magnitude coarser (1e-3, well outside
/// float noise and inside the shape of a genuine wrong-fold bug) must not.
#[test]
fn float_equivalence_comparison_tolerates_last_bit_only() {
    let conn = Connection::open_in_memory().expect("open duckdb");
    conn.execute_batch(
        "CREATE TABLE left_t(id BIGINT, val DOUBLE); \
         CREATE TABLE right_t(id BIGINT, val DOUBLE);",
    )
    .expect("create left_t/right_t");

    let rounded = |table: &str| format!("SELECT id, ROUND(val, 6) AS val FROM {table}");

    // A 1e-12 perturbation is the numerically-stable-vs-naive-recompute
    // noise floor this tolerance exists to absorb.
    conn.execute_batch(
        "DELETE FROM left_t; DELETE FROM right_t; \
         INSERT INTO left_t VALUES (1, 10.0); \
         INSERT INTO right_t VALUES (1, 10.000000000001);",
    )
    .expect("seed a last-bit perturbation");
    assert!(
        multiset_equal(&conn, &rounded("left_t"), &rounded("right_t")),
        "a 1e-12 perturbation (floating-point noise between a stable and a naive \
         variance/stddev recompute) must compare equal under the ROUND(col, 6) tolerance"
    );

    // A 1e-3 perturbation is three orders of magnitude coarser than the
    // rounding tolerance and well outside float noise — the shape a real
    // wrong-fold bug would produce. It must still be flagged.
    conn.execute_batch(
        "DELETE FROM left_t; DELETE FROM right_t; \
         INSERT INTO left_t VALUES (1, 10.0); \
         INSERT INTO right_t VALUES (1, 10.001);",
    )
    .expect("seed a real divergence");
    assert!(
        !multiset_equal(&conn, &rounded("left_t"), &rounded("right_t")),
        "a 1e-3 perturbation must NOT compare equal — the tolerance would otherwise be wide \
         enough to swallow a real fold bug"
    );
}
