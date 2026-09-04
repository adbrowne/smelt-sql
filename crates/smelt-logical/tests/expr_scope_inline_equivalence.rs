//! Phase 2 of `docs/outcomes/20260904-walk-migration-residue`: bound/reach and
//! grain (key set, per-column determinism, per-column change-comparability)
//! consume the `expr_scopes` verdicts the walk enumerates
//! (`docs/specs/model_properties.md` §"The composition walk"). The contract
//! under test is equivalence: an expression-position subquery's verdict must
//! equal the verdict of the same subquery rewritten as a `FROM`-position
//! derived table.

use proptest::prelude::*;

use smelt_logical::analysis::footprint::{reflect_footprint, FootprintResult};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::source_bounds::{
    derive_model_bounds, BoundContext, BoundResult, Seconds, Skew,
};
use smelt_logical::analysis::walk::{
    model_partition_skew, model_property_vector, Comparability, Determinism,
};

fn ctx() -> BoundContext {
    BoundContext::new()
        .with_source("events", "event_date")
        .with_source("other", "other_date")
}

/// A source read only through a select-list scalar subquery must appear in
/// the bound map — before phase 2 it was invisible (the expr-scopes tail was
/// not a reach contributor).
#[test]
fn scalar_subquery_source_appears_in_bound_map() {
    let sql = "SELECT a, (SELECT max(b) FROM other) AS m FROM events";
    let bounds = derive_model_bounds(sql, &ctx());
    assert_eq!(
        bounds.get("other"),
        Some(&BoundResult::Bounded {
            source_partition_col: "other_date".to_string(),
            before: Seconds::ZERO,
            after: Seconds::ZERO,
        }),
        "a source read only via a select-list scalar subquery must get a bound: {bounds:?}"
    );
}

/// Same claim, via `WHERE EXISTS (…)`.
#[test]
fn exists_subquery_source_appears_in_bound_map() {
    let sql = "SELECT a FROM events WHERE EXISTS (SELECT 1 FROM other WHERE other.k = events.k)";
    let bounds = derive_model_bounds(sql, &ctx());
    assert_eq!(
        bounds.get("other"),
        Some(&BoundResult::Bounded {
            source_partition_col: "other_date".to_string(),
            before: Seconds::ZERO,
            after: Seconds::ZERO,
        }),
        "a source read only via a WHERE EXISTS subquery must get a bound: {bounds:?}"
    );
}

/// A window frame inside the subquery body contributes reach for that
/// source, the same as a frame in the outer scope would.
#[test]
fn subquery_frame_reach_reaches_model_bound() {
    let sql = "SELECT a, \
               (SELECT SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '1 day' PRECEDING \
                AND CURRENT ROW) FROM other) AS m \
               FROM events";
    let bounds = derive_model_bounds(sql, &ctx());
    assert_eq!(
        bounds.get("other"),
        Some(&BoundResult::Bounded {
            source_partition_col: "other_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        }),
        "the subquery body's own RANGE BETWEEN frame must reach the model bound: {bounds:?}"
    );
}

/// `UNBOUNDED PRECEDING` inside a subquery body must make the model's
/// verdict `Unbounded`, not silently invisible (fail-loud: absence of a
/// proof is a rejection, never an under-derivation).
#[test]
fn unbounded_construct_in_subquery_is_fail_closed() {
    let sql = "SELECT a, \
               (SELECT SUM(v) OVER (ORDER BY d RANGE BETWEEN UNBOUNDED PRECEDING \
                AND CURRENT ROW) FROM other) AS m \
               FROM events";
    let bounds = derive_model_bounds(sql, &ctx());
    assert_eq!(
        bounds.get("events"),
        Some(&BoundResult::Unbounded),
        "an unbounded construct anywhere beneath the model must reject every context source, \
         including ones not read inside the subquery: {bounds:?}"
    );
    assert_eq!(
        bounds.get("other"),
        Some(&BoundResult::Unbounded),
        "the subquery's own source must also be rejected: {bounds:?}"
    );
}

/// A `NOW()`-bearing subquery body taints the projected column's determinism
/// through the child verdict — and only through it: a fixture where `NOW()`
/// sits *only* inside the subquery (never in the outer scope's own text)
/// still taints the outer item, proving the outer classifier is reading the
/// child verdict rather than (impossibly, since it isn't there) finding
/// `NOW()` itself.
#[test]
fn scalar_subquery_column_determinism_comes_from_child_verdict() {
    let sql = "SELECT a, (SELECT NOW()) AS m FROM events";
    let vector = model_property_vector(sql, &JoinContext::new()).expect("model parses");
    let m = vector
        .determinism
        .iter()
        .find(|d| d.output.eq_ignore_ascii_case("m"))
        .expect("column m is projected");
    assert_eq!(
        m.level,
        Determinism::Run,
        "the outer column's determinism must come from the subquery body's own NOW() taint"
    );
}

/// Adding an `EXISTS` filter to a `GROUP BY` scope must leave grain and
/// `has_fan_out_join` unchanged — an expr scope contributes no key, no
/// output columns, and no fan-out.
#[test]
fn exists_filter_does_not_change_grain() {
    let baseline = "SELECT k, COUNT(*) AS c FROM events GROUP BY k";
    let with_exists = "SELECT k, COUNT(*) AS c FROM events \
               WHERE EXISTS (SELECT 1 FROM other WHERE other.k = events.k) GROUP BY k";
    let base_vector = model_property_vector(baseline, &JoinContext::new()).expect("parses");
    let exists_vector = model_property_vector(with_exists, &JoinContext::new()).expect("parses");
    assert_eq!(base_vector.grain, exists_vector.grain);
    assert_eq!(base_vector.has_fan_out_join, exists_vector.has_fan_out_join);
}

/// A `UNION`-bodied scalar subquery sets `has_set_op_barrier` on the
/// enclosing scope, matching the barrier a `UNION`-bodied derived table
/// would set.
#[test]
fn set_op_bodied_subquery_propagates_barrier() {
    let sql = "SELECT k, COUNT(*) AS c, \
               (SELECT x FROM p UNION SELECT x FROM q) AS m \
               FROM events GROUP BY k";
    let vector = model_property_vector(sql, &JoinContext::new()).expect("model parses");
    assert!(
        vector.has_set_op_barrier,
        "a UNION-bodied expression-position subquery must propagate the set-op barrier"
    );
}

// ===== Inline-equivalence proptest =====
// An uncorrelated single-column scalar subquery rendered at expression
// position must give exactly the same verdicts as the same subquery rendered
// as a cross-joined derived table (`model_properties.md` §"The composition
// walk": bound/reach folds an expr scope as a read).

fn agg_strategy() -> impl Strategy<Value = String> {
    prop_oneof!["MAX", "MIN", "SUM", "COUNT"].prop_map(|s| s.to_string())
}

/// Render the same uncorrelated scalar subquery both at expression position
/// and inlined as a cross-joined derived table `__e`.
fn render_pair(agg: &str, col: &str, taint: bool) -> (String, String) {
    let inner_expr = if taint {
        format!("{agg}({col}) + RANDOM() * 0")
    } else {
        format!("{agg}({col})")
    };
    let expr_position = format!("SELECT t.a, (SELECT {inner_expr} FROM u) AS m FROM t");
    let inlined =
        format!("SELECT t.a, __e.m AS m FROM t, (SELECT {inner_expr} AS m FROM u) AS __e");
    (expr_position, inlined)
}

fn prop_ctx() -> BoundContext {
    BoundContext::new()
        .with_source("t", "t_date")
        .with_source("u", "u_date")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_expr_scope_bound_equals_inlined(agg in agg_strategy(), taint in any::<bool>()) {
        let (expr_position, inlined) = render_pair(&agg, "b", taint);
        let ctx = prop_ctx();
        prop_assert_eq!(
            derive_model_bounds(&expr_position, &ctx),
            derive_model_bounds(&inlined, &ctx)
        );
    }

    #[test]
    fn prop_expr_scope_property_vector_equals_inlined(agg in agg_strategy(), taint in any::<bool>()) {
        let (expr_position, inlined) = render_pair(&agg, "b", taint);
        let expr_vector = model_property_vector(&expr_position, &JoinContext::new())
            .expect("expr-position form parses");
        let inlined_vector = model_property_vector(&inlined, &JoinContext::new())
            .expect("inlined form parses");
        prop_assert_eq!(&expr_vector.grain, &inlined_vector.grain);
        let expr_m = expr_vector
            .determinism
            .iter()
            .find(|d| d.output.eq_ignore_ascii_case("m"))
            .map(|d| d.level);
        let inlined_m = inlined_vector
            .determinism
            .iter()
            .find(|d| d.output.eq_ignore_ascii_case("m"))
            .map(|d| d.level);
        prop_assert_eq!(expr_m, inlined_m);
        let expr_c = expr_vector
            .comparability
            .iter()
            .find(|c| c.output.eq_ignore_ascii_case("m"))
            .map(|c| c.comparability);
        let inlined_c = inlined_vector
            .comparability
            .iter()
            .find(|c| c.output.eq_ignore_ascii_case("m"))
            .map(|c| c.comparability);
        prop_assert_eq!(expr_c, inlined_c);
    }
}

/// Smoke check that the taint path actually varies determinism (otherwise
/// the proptest above could pass vacuously without exercising `Comparability`
/// beyond the default).
#[test]
fn taint_flag_actually_taints() {
    let (expr_position, _) = render_pair("MAX", "b", true);
    let vector = model_property_vector(&expr_position, &JoinContext::new()).expect("parses");
    let m = vector
        .determinism
        .iter()
        .find(|d| d.output.eq_ignore_ascii_case("m"))
        .expect("column m is projected");
    assert_eq!(m.level, Determinism::Row);
    let comparability = vector
        .comparability
        .iter()
        .find(|c| c.output.eq_ignore_ascii_case("m"))
        .expect("column m is projected");
    assert_eq!(comparability.comparability, Comparability::Incomparable);
}

// ===== Phase 3: skew and footprint-trajectory consume the expr-scope tail =====
// (`docs/outcomes/20260904-walk-migration-residue` phase 3). Both transfers
// were bounded to `ctes ++ inputs` after phase 1/2; phase 3 retires the
// bound.

/// A Form B relation living only inside a `WHERE EXISTS (…)` filter body
/// must be seen by skew — before phase 3 the expr-scopes tail was not a
/// skew contributor at all, so this scope's relation was invisible.
#[test]
fn skew_from_expr_position_subquery_is_seen() {
    let sql = "SELECT e.d AS d, e.amt AS amt \
               FROM smelt.sources.events e \
               WHERE EXISTS ( \
                   SELECT 1 FROM smelt.marts.running_balance bal \
                   WHERE bal.d >= e.d - INTERVAL '2 days' AND bal.d < e.d \
               )";
    let skew = model_partition_skew(sql, "d");
    assert_eq!(
        skew,
        Skew {
            before: Seconds::days(2),
            after: Seconds::ZERO,
        },
        "a Form B relation living only inside an EXISTS filter body must be seen by skew: {skew:?}"
    );
}

fn render_skew_pair(agg: &str, before_days: i64) -> (String, String) {
    let cond = format!("bal.d >= t.d - INTERVAL '{before_days} day' AND bal.d < t.d");
    let expr_position = format!(
        "SELECT t.a, (SELECT {agg}(bal.balance) FROM smelt.marts.running_balance bal \
         WHERE {cond}) AS m FROM smelt.sources.events t"
    );
    let inlined = format!(
        "SELECT t.a, __e.m AS m FROM smelt.sources.events t, \
         (SELECT {agg}(bal.balance) AS m FROM smelt.marts.running_balance bal WHERE {cond}) AS __e"
    );
    (expr_position, inlined)
}

/// The skew verdict for a model reading a Form B relation via an
/// uncorrelated scalar subquery equals the verdict for the same relation
/// rewritten as a cross-joined derived table.
#[test]
fn skew_expr_scope_equals_inlined_derived_table() {
    let (expr_position, inlined) = render_skew_pair("MAX", 1);
    assert_eq!(
        model_partition_skew(&expr_position, "d"),
        model_partition_skew(&inlined, "d")
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_skew_expr_scope_inline_equivalence(agg in agg_strategy(), before_days in 1i64..30) {
        let (expr_position, inlined) = render_skew_pair(&agg, before_days);
        prop_assert_eq!(
            model_partition_skew(&expr_position, "d"),
            model_partition_skew(&inlined, "d")
        );
    }
}

fn footprint_ctx() -> BoundContext {
    BoundContext::new()
        .with_source("silver.events", "event_date")
        .with_source("silver.other", "other_date")
}

/// A running `SUM(…) OVER (ORDER BY <axis>)` inside a select-list scalar
/// subquery's body makes the enclosing model a trajectory of that source —
/// before phase 3 this fold sat outside the `ctes ++ inputs` bound and was
/// invisible to `TrajectoryTransfer`.
#[test]
fn trajectory_from_expr_position_subquery_is_seen() {
    let sql = "SELECT event_date, \
               (SELECT SUM(amount) OVER (ORDER BY event_date) FROM smelt.silver.other) AS m \
               FROM smelt.silver.events";
    let fps = reflect_footprint(sql, &footprint_ctx(), Some("event_date"));
    assert_eq!(
        fps.get("silver.other"),
        Some(&FootprintResult::Unbounded),
        "a running fold over the output axis inside a select-list scalar subquery must be seen \
         as a trajectory column: {fps:?}"
    );
}

/// Same claim as an equivalence: the reflected footprint for a model reading
/// the running fold via a select-list scalar subquery equals the footprint
/// for the same fold rewritten as a cross-joined derived table.
#[test]
fn trajectory_expr_scope_equals_inlined_derived_table() {
    let expr_position = "SELECT t.event_date, \
        (SELECT SUM(amount) OVER (ORDER BY event_date) FROM smelt.silver.other) AS m \
        FROM smelt.silver.events t";
    let inlined = "SELECT t.event_date, __e.m AS m \
        FROM smelt.silver.events t, \
        (SELECT SUM(amount) OVER (ORDER BY event_date) AS m FROM smelt.silver.other) AS __e";
    let ctx = footprint_ctx();
    assert_eq!(
        reflect_footprint(expr_position, &ctx, Some("event_date")),
        reflect_footprint(inlined, &ctx, Some("event_date"))
    );
}
