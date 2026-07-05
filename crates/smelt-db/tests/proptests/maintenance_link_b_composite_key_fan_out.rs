//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-10` (`docs/research/20260705-property-discovery-loop.md` §2.2;
//! `docs/plans/20260705-property-discovery-loop.md`).
//!
//! Link-B classification diagnostic for `join_shape::fan_out`
//! (`crates/smelt-logical/src/analysis/join_shape.rs`). `JoinContext`'s own
//! doc comment says a declared key "alone uniquely identifies a row" — the
//! API has no way to express a COMPOSITE unique key (a pair of columns that
//! is jointly, but not individually, unique). This cell builds a genuine
//! composite-key fact/dim join, proves via DuckDB ground truth that the join
//! is actually one-to-one, then shows `fan_out` cannot be told that (no
//! single column is unique alone) and so classifies it `OneToMany`.
//!
//! This is a **classification-diagnostic finding, not a bug**: `fan_out`
//! fails closed (`OneToMany` is the safe default), so the mismatch is
//! over-conservative, never unsound. Also note `fan_out`/`JoinContext`
//! (and their sole production consumer, `dimension_horizon_merge`) have zero
//! production call sites today (`rg -n "JoinContext|dimension_horizon_merge\("`
//! outside tests finds none) — this classifier is dormant, so there is
//! nothing to wire or fix; wiring it to a real caller would be the same
//! behaviour-defining design fork already BLOCKed for `input_delta_discovery`
//! (cell `FIX-2`).

use proptest::prelude::*;

use smelt_logical::analysis::join_shape::{fan_out, Cardinality, JoinContext};

use crate::prop_helpers::duckdb_oracle::DuckDbOracle;

const JOIN_SQL: &str = "SELECT * FROM facts f JOIN dims d ON f.user_id = d.user_id AND f.dt = d.dt";

fn parse_join() -> smelt_parser::JoinClause {
    let parse = smelt_parser::parse(JOIN_SQL);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select stmt");
    let from = select.from_clause().expect("from clause");
    let join = from.joins().next().expect("join clause");
    join
}

/// Composite-key dim rows: `(user_id, dt_offset)` pairs are unique as a
/// PAIR, but neither column alone is unique — every `user_id` repeats across
/// several `dt_offset`s, and every `dt_offset` repeats across several
/// `user_id`s.
fn arb_composite_dim() -> impl Strategy<Value = Vec<(i64, i64)>> {
    let user_ids = 0_i64..4;
    let dt_offsets = 0_i64..4;
    proptest::collection::hash_set((user_ids, dt_offsets), 1..16)
        .prop_map(|set| set.into_iter().collect())
}

fn setup_composite_tables(oracle: &DuckDbOracle, dim_rows: &[(i64, i64)]) {
    oracle
        .execute_ddl(
            "CREATE TABLE dims (user_id BIGINT, dt BIGINT, dim_payload BIGINT); \
             CREATE TABLE facts (fact_id BIGINT, user_id BIGINT, dt BIGINT)",
        )
        .expect("create tables");
    for (i, (user_id, dt)) in dim_rows.iter().enumerate() {
        oracle
            .execute_ddl(&format!("INSERT INTO dims VALUES ({user_id}, {dt}, {i})"))
            .expect("insert dim row");
    }
    // One fact row per distinct user_id present in the dim data — every
    // user_id alone matches MULTIPLE dim rows (across different dt), so a
    // single-column-only join on user_id would fan out; only the composite
    // (user_id, dt) equality is actually one-to-one.
    let mut user_ids: Vec<i64> = dim_rows.iter().map(|(u, _)| *u).collect();
    user_ids.sort_unstable();
    user_ids.dedup();
    for (fact_id, user_id) in user_ids.iter().enumerate() {
        // Use the first dt this user_id appears under, so the fact row has
        // exactly one true composite match.
        let dt = dim_rows
            .iter()
            .find(|(u, _)| u == user_id)
            .map(|(_, d)| *d)
            .expect("user_id sourced from dim_rows");
        oracle
            .execute_ddl(&format!(
                "INSERT INTO facts VALUES ({fact_id}, {user_id}, {dt})"
            ))
            .expect("insert fact row");
    }
}

/// Ground truth: the max number of dim rows matched per fact row under the
/// TRUE composite-key join. `fan_out`'s `OneToOne` claim is only warranted
/// when this is never more than 1.
fn max_matches_per_fact_row(oracle: &DuckDbOracle) -> i64 {
    let rows = oracle
        .execute_query(
            "SELECT MAX(match_count) FROM (\
                 SELECT f.fact_id, COUNT(*) AS match_count \
                 FROM facts f JOIN dims d ON f.user_id = d.user_id AND f.dt = d.dt \
                 GROUP BY f.fact_id\
             ) counts",
        )
        .expect("query must succeed");
    let cell = rows
        .first()
        .and_then(|r| r.first())
        .expect("at least one fact row");
    let inner = cell
        .split_once('(')
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .unwrap_or(cell.as_str());
    inner
        .parse()
        .unwrap_or_else(|e| panic!("match_count column {cell:?} must parse as i64: {e}"))
}

/// Also confirm the precondition that makes this a genuine composite-key
/// case: at least one `user_id` alone matches more than one dim row (so a
/// single-column unique-key declaration on `user_id` would be FALSE, and
/// `fan_out` is right not to accept it).
fn some_user_id_alone_is_not_unique(oracle: &DuckDbOracle) -> bool {
    let rows = oracle
        .execute_query(
            "SELECT MAX(cnt) FROM (SELECT user_id, COUNT(*) AS cnt FROM dims GROUP BY user_id) t",
        )
        .expect("query must succeed");
    let cell = rows
        .first()
        .and_then(|r| r.first())
        .expect("dims non-empty");
    let inner = cell
        .split_once('(')
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .unwrap_or(cell.as_str());
    let max_count: i64 = inner
        .parse()
        .unwrap_or_else(|e| panic!("cnt column {cell:?} must parse as i64: {e}"));
    max_count > 1
}

proptest! {
    /// Ground truth (design §2.2, Link-B): a genuine composite-key equi-join
    /// (`ON f.user_id = d.user_id AND f.dt = d.dt` over dim data unique only
    /// on the PAIR) is actually one-to-one over every generated composite
    /// key set — never more than one dim row per fact row.
    #[test]
    fn composite_key_equality_join_is_truly_one_to_one_in_ground_truth(dim_rows in arb_composite_dim()) {
        let oracle = DuckDbOracle::new();
        setup_composite_tables(&oracle, &dim_rows);
        prop_assume!(some_user_id_alone_is_not_unique(&oracle));

        prop_assert_eq!(
            max_matches_per_fact_row(&oracle),
            1,
            "composite dim rows {:?} must make the (user_id, dt) join one-to-one in ground truth",
            dim_rows
        );
    }
}

/// The classification mismatch itself: `JoinContext` cannot express "the
/// PAIR (user_id, dt) is unique" (only per-column keys), so with no
/// single-column key declared, `fan_out` conservatively classifies the same
/// join `OneToMany` even though the ground-truth property test above proves
/// it is `OneToOne`. Fail-closed and safe — a false negative, not a false
/// positive — recorded as an over-conservative classification gap, not a bug.
#[test]
fn fan_out_cannot_express_composite_unique_key_and_conservatively_classifies_one_to_many() {
    let join = parse_join();
    let ctx = JoinContext::new(); // no single-column key to declare
    assert_eq!(
        fan_out(&join, &ctx),
        Cardinality::OneToMany,
        "fan_out has no way to express joint (user_id, dt) uniqueness and must fail closed to \
         OneToMany here, even though ground truth (see the proptest in this module) proves the \
         join is actually one-to-one — this is the over-conservative gap cell G-10 predicted"
    );
}
