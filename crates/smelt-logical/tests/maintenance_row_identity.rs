//! TDD tests for region row identity (P2, `docs/specs/model_properties.md`
//! §"Region row identity"): `smelt_logical::maintenance::derive::row_identity`
//! derives what a conditional write joins stored rows to candidate rows on —
//! the declared `unique_key` where present, else the walk's proven grain
//! key, else the identity-free `WholeRow` fallback
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase C3).

use smelt_logical::maintenance::derive::row_identity;
use smelt_logical::maintenance::RowIdentity;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A declared `unique_key` wins outright — no SQL construct is needed to
/// establish it, and this model's SQL would not prove any grain on its own
/// (a bare projection, no `GROUP BY`).
#[test]
fn declared_unique_key_derives_key_identity() {
    let sql = "SELECT order_id, customer_id, amount FROM smelt.sources.orders";
    let verdict = row_identity(&strings(&["order_id"]), sql);
    assert_eq!(verdict.identity, RowIdentity::Key(strings(&["order_id"])));
    assert_eq!(
        verdict.proven_mismatch, None,
        "no proven key exists here (no GROUP BY) — nothing to disagree with"
    );
}

/// No declared key: the walk's own proven `GROUP BY` grain key is used
/// instead — a `GROUP BY` output is unique per group by construction.
#[test]
fn proven_grain_key_derives_key_identity_when_undeclared() {
    let sql = "SELECT customer_id, order_date, SUM(amount) AS total \
               FROM smelt.sources.orders GROUP BY customer_id, order_date";
    let verdict = row_identity(&[], sql);
    assert_eq!(
        verdict.identity,
        RowIdentity::Key(strings(&["customer_id", "order_date"])),
        "the walk should prove the GROUP BY columns as the grain key: {verdict:?}"
    );
    assert_eq!(verdict.proven_mismatch, None);
}

/// Both a declared key and a differing proven grain key are present: the
/// declared key wins the precedence (never silently averaged or preferring
/// the proof), but the disagreement is surfaced in `proven_mismatch` rather
/// than dropped.
#[test]
fn declared_key_wins_precedence_but_mismatch_is_surfaced() {
    let sql = "SELECT customer_id, order_date, SUM(amount) AS total \
               FROM smelt.sources.orders GROUP BY customer_id, order_date";
    // A declared key that disagrees with the GROUP BY-proven key (e.g. a
    // caller-supplied `unique_key:` that only names one of the two grouping
    // columns) — contrived to exercise the precedence rule; whether such a
    // model is otherwise well-formed is out of this function's scope.
    let verdict = row_identity(&strings(&["customer_id"]), sql);
    assert_eq!(
        verdict.identity,
        RowIdentity::Key(strings(&["customer_id"])),
        "declared must win over proven: {verdict:?}"
    );
    assert_eq!(
        verdict.proven_mismatch,
        Some(strings(&["customer_id", "order_date"])),
        "the proven key that was overridden must be surfaced, not silently dropped: {verdict:?}"
    );
}

/// Identical declared and proven keys (order-independent, case-insensitive)
/// must NOT be reported as a mismatch.
#[test]
fn declared_and_proven_agreement_reports_no_mismatch() {
    let sql = "SELECT customer_id, order_date, SUM(amount) AS total \
               FROM smelt.sources.orders GROUP BY customer_id, order_date";
    let verdict = row_identity(&strings(&["order_date", "customer_id"]), sql);
    assert_eq!(
        verdict.identity,
        RowIdentity::Key(strings(&["order_date", "customer_id"]))
    );
    assert_eq!(
        verdict.proven_mismatch, None,
        "same key set in different order/case must not be flagged as a mismatch: {verdict:?}"
    );
}

/// No declared key, no provable grain (a keyless partition-grain model: a
/// bare projection with no `GROUP BY`/`DISTINCT`) — falls back to the
/// identity-free `WholeRow` multiset diff.
#[test]
fn keyless_model_derives_whole_row_identity() {
    let sql = "SELECT event_date, event_type, payload FROM smelt.sources.events";
    let verdict = row_identity(&[], sql);
    assert_eq!(verdict.identity, RowIdentity::WholeRow);
    assert_eq!(verdict.proven_mismatch, None);
}

/// Fail-closed: a proven grain key that does not cover the output — a
/// fan-out join contaminates the `GROUP BY` grain the walk would otherwise
/// prove — must fall back to `WholeRow`, never a partial/unsound key. The
/// join has no declared unique key for the joined source (an empty
/// `JoinContext`, matching `row_identity`'s own fail-closed default), so
/// `fan_out` defaults to `OneToMany`.
#[test]
fn fan_out_join_forces_whole_row_even_with_a_provable_group_by_key() {
    let sql = "SELECT o.customer_id, o.order_date, SUM(o.amount) AS total \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.order_items i ON o.order_id = i.order_id \
               GROUP BY o.customer_id, o.order_date";
    let verdict = row_identity(&[], sql);
    assert_eq!(
        verdict.identity,
        RowIdentity::WholeRow,
        "a fan-out join must veto the proven GROUP BY key entirely — never a partial key: \
         {verdict:?}"
    );
    assert_eq!(verdict.proven_mismatch, None);
}

/// The fan-out veto only applies to the *proven* key — a declared key must
/// still be trusted even when the model also contains a fan-out join,
/// exactly the same precedence as the no-join case.
#[test]
fn declared_key_survives_a_fan_out_join() {
    let sql = "SELECT o.order_id, o.customer_id, i.sku \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.order_items i ON o.order_id = i.order_id";
    let verdict = row_identity(&strings(&["order_id", "sku"]), sql);
    assert_eq!(
        verdict.identity,
        RowIdentity::Key(strings(&["order_id", "sku"]))
    );
}
