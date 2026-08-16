//! Per-scope self-edge exclusion in the partition-skew fold
//! (`docs/specs/incremental_shapes.md` §"Window independence and self-referential
//! models": the self-edge is never a skew anchor; `docs/specs/architecture.md`
//! §"Property composition walk rule": the exclusion is resolved by the shared
//! walk's per-scope alias map, never by a whole-text scan).
//!
//! `model_partition_skew_excluding_self` must:
//! - derive NO skew from the self-edge's own bounding relation, whether it
//!   lives in the join's `ON` clause or the `WHERE` clause, aliased or via a
//!   bare comma-join FROM entry;
//! - keep every genuine Form B relation anchored on a non-self source —
//!   including one sharing top-level `AND`-clause structure with the self
//!   bound in the same `WHERE` clause;
//! - resolve the exclusion **per scope**: an unrelated subquery or set-
//!   operation arm reusing the self-edge's alias text for a different source
//!   keeps that scope's genuine relations (excluding them would under-widen
//!   the derived output window — stale partitions).

use smelt_logical::analysis::source_bounds::{Seconds, Skew};
use smelt_logical::analysis::walk::{model_partition_skew, model_partition_skew_excluding_self};

const SELF: &str = "marts.running_balance";

/// The canonical `Ordered` self-edge shape (`window_independence`'s own unit
/// fixture): the ONLY bounding relation is the self-edge's own `WHERE`
/// condition, whose column shares the model's `partition_column` name.
const SELF_BOUND_IN_WHERE: &str = "SELECT bal.d AS d, bal.balance + t.amount AS balance \
     FROM smelt.marts.running_balance bal \
     JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
     WHERE bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d";

#[test]
fn self_edge_bound_in_where_is_not_a_skew_anchor() {
    // Without the exclusion the self bound reads as a 1-day-before anchor —
    // the false positive the exclusion targets.
    assert_ne!(
        model_partition_skew(SELF_BOUND_IN_WHERE, "d"),
        Skew::ZERO,
        "precondition: the raw scan does read the self bound as an anchor \
         (otherwise this test guards nothing)"
    );
    assert_eq!(
        model_partition_skew_excluding_self(SELF_BOUND_IN_WHERE, "d", Some(SELF)),
        Skew::ZERO,
        "the self-edge's WHERE-clause bound must not contribute a skew anchor"
    );
}

#[test]
fn self_edge_bound_in_join_on_is_not_a_skew_anchor() {
    let sql = "SELECT t.d AS d, bal.balance + t.amount AS balance \
         FROM smelt.silver.transactions t \
         LEFT JOIN smelt.marts.running_balance bal \
           ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "d", Some(SELF)),
        Skew::ZERO,
        "the self-edge's ON-clause bound must not contribute a skew anchor"
    );
}

#[test]
fn self_reference_via_bare_comma_join_from_entry_is_excluded() {
    // The self-reference as a bare FROM entry (comma join), bound entirely
    // in WHERE.
    let sql = "SELECT t.d AS d, bal.balance + t.amount AS balance \
         FROM smelt.silver.transactions t, smelt.marts.running_balance bal \
         WHERE bal.acct_id = t.acct_id \
           AND bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "d", Some(SELF)),
        Skew::ZERO,
        "a comma-join self reference's WHERE bound must not contribute a skew anchor"
    );
}

#[test]
fn genuine_form_b_relation_sharing_the_where_clause_survives() {
    // The genuine Form B relation (anchored on the driving source's own
    // partition column) shares top-level AND-clause structure with the self
    // bound in one WHERE clause: the genuine relation must survive, the self
    // bound must not anchor.
    let sql = "SELECT e.session_start_date AS session_start_date, \
                      COALESCE(bal.total, 0) + SUM(e.amt) AS total \
         FROM smelt.sources.events e \
         LEFT JOIN smelt.marts.running_balance bal ON bal.k = e.k \
         WHERE bal.session_start_date >= e.session_start_date - INTERVAL '2 days' \
           AND bal.session_start_date < e.session_start_date \
           AND e.event_date BETWEEN e.session_start_date - INTERVAL '1 day' \
               AND e.session_start_date + INTERVAL '1 day' \
         GROUP BY e.session_start_date, bal.total";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "session_start_date", Some(SELF)),
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "the genuine relation must survive; the (wider, 2-day) self bound in \
         the same WHERE clause must not anchor"
    );
}

#[test]
fn subquery_reusing_the_self_alias_for_a_different_source_keeps_its_relation() {
    // Finding-2 regression: the outer scope's self-edge uses alias `bal`;
    // a nested derived table reuses the SAME alias text `bal` for a
    // DIFFERENT source carrying a genuine Form B relation. Per-scope alias
    // resolution must keep that relation — a cross-scope alias accumulation
    // would blank it, under-deriving skew and under-widening the write
    // window (stale partitions).
    let sql = "SELECT agg.d AS d, agg.total + bal.balance AS balance \
         FROM ( \
             SELECT bal.d AS d, SUM(bal.amt) AS total \
             FROM smelt.sources.ledger bal \
             WHERE bal.event_date BETWEEN bal.d - INTERVAL '1 day' \
                 AND bal.d + INTERVAL '1 day' \
             GROUP BY bal.d \
         ) agg \
         LEFT JOIN smelt.marts.running_balance bal \
           ON bal.d >= agg.d - INTERVAL '3 days' AND bal.d < agg.d";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "d", Some(SELF)),
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "the inner scope's `bal` resolves to a different source; its genuine \
         relation must contribute — only the outer scope's (3-day) self bound \
         is excluded"
    );
}

#[test]
fn union_arm_reusing_the_self_alias_for_a_different_source_keeps_its_relation() {
    // The set-operation variant of the finding-2 regression: one arm has the
    // self-edge under alias `bal`; the other arm reuses `bal` for a genuine
    // source with a Form B relation. Per-arm (per-scope) resolution must keep
    // the genuine arm's contribution.
    let sql = "SELECT bal.d AS d, bal.balance AS v \
         FROM smelt.marts.running_balance bal \
         JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
         WHERE bal.d >= t.d - INTERVAL '5 days' AND bal.d < t.d \
         UNION ALL \
         SELECT bal.d AS d, SUM(bal.amt) AS v \
         FROM smelt.sources.ledger bal \
         WHERE bal.event_date BETWEEN bal.d - INTERVAL '1 day' \
             AND bal.d + INTERVAL '1 day' \
         GROUP BY bal.d";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "d", Some(SELF)),
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "only the self arm's (5-day) bound is excluded; the other arm's \
         genuine relation under the same alias text must contribute"
    );
}

#[test]
fn or_condition_mixing_self_and_genuine_bounds_is_never_excluded() {
    // A single WHERE condition disjoins the self bound with a genuine 5-day
    // bound anchored on the model's partition column. Excluding the whole
    // disjunction because one branch references the self alias would drop
    // the genuine bound — under-widening the derived output window. The OR
    // guard must keep the condition; the resulting over-approximation can
    // only widen.
    let sql = "SELECT e.event_date AS event_date, bal.balance AS balance \
         FROM smelt.sources.events e \
         LEFT JOIN smelt.marts.running_balance bal ON bal.k = e.k \
         WHERE (bal.d >= e.d - INTERVAL '1 day') \
            OR (e.d >= e.event_date - INTERVAL '5 days')";
    let skew = model_partition_skew_excluding_self(sql, "event_date", Some(SELF));
    assert!(
        skew.before >= Seconds::days(5),
        "the genuine 5-day bound inside the OR must survive (got {skew:?}); \
         dropping the whole disjunction would under-widen the write window"
    );
}

#[test]
fn string_literal_shaped_like_a_self_qualifier_does_not_exclude_a_genuine_bound() {
    // The genuine bound conjunct contains a string literal whose text looks
    // exactly like a self-qualified column ('bal.d'). Qualifier detection is
    // token-based: a STRING token can never read as an `IDENT DOT` qualified
    // reference, so the conjunct — and its 1-day bound — must survive.
    let sql = "SELECT e.d AS d, bal.balance AS balance \
         FROM smelt.sources.events e \
         LEFT JOIN smelt.marts.running_balance bal ON bal.k = e.k \
         WHERE bal.d >= e.d - INTERVAL '2 days' \
           AND e.event_date >= e.d - INTERVAL '1 day' + LENGTH('bal.d') * INTERVAL '0 day'";
    assert_eq!(
        model_partition_skew_excluding_self(sql, "d", Some(SELF)),
        Skew {
            before: Seconds::days(1),
            after: Seconds::ZERO,
        },
        "the literal 'bal.d' inside the genuine conjunct must not trigger \
         the self exclusion; only the real (2-day) self bound is excluded"
    );
}

#[test]
fn no_exclusion_requested_derives_identically_to_model_partition_skew() {
    for sql in [
        SELF_BOUND_IN_WHERE,
        "SELECT event_date, COUNT(*) AS n FROM smelt.silver.events GROUP BY event_date",
    ] {
        assert_eq!(
            model_partition_skew_excluding_self(sql, "d", None),
            model_partition_skew(sql, "d"),
            "with self_name: None the excluding entry is exactly the plain fold"
        );
    }
}
