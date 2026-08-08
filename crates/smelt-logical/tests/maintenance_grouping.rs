//! TDD tests for `smelt_logical::maintenance::grouping` — the
//! mutation-sensitivity column-grouping derivation
//! (`docs/specs/incremental_models.md` §Design "Factoring by
//! mutation-sensitivity"; `docs/specs/model_properties.md` §"Per-column
//! mutation-sensitivity / column provenance").

use std::collections::BTreeSet;

use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::{MutationProfile, SourceFacts};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn source(
    name: &str,
    mutation: MutationProfile,
    partition_col: Option<&str>,
    unique_key: &[&str],
) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col: partition_col.map(|s| s.to_string()),
        unique_key: unique_key.iter().map(|s| s.to_string()).collect(),
        allow_full_scan: false,
    }
}

/// The load-bearing append-only case (`incremental_models.md` §Design
/// "Factoring by mutation-sensitivity"): a column that reads only an
/// append-only source, without aggregating over it, is immutable at
/// creation — the source can only ever add new rows, never revise the one
/// this column already read. `tier`, which joins the mutable `customers`
/// dimension, must still land in its own sensitive group so the test also
/// proves the append-only reference does not drag the whole row along.
#[test]
fn immutable_at_creation_reference_contributes_no_sensitivity() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    let sql = "SELECT o.order_id, o.order_date, o.amount, c.tier \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id";
    let skeleton = set(&["order_id", "order_date"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    let amount_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"amount".to_string()))
        .expect("amount is grouped");
    assert!(
        amount_group.mutation_sensitivity.is_empty(),
        "an append-only, non-aggregated reference must contribute no sensitivity: {:?}",
        amount_group.mutation_sensitivity
    );

    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier".to_string()))
        .expect("tier is grouped");
    assert_eq!(tier_group.mutation_sensitivity, set(&["customers"]));
    // The two must be different groups: the append-only reference did not
    // drag `amount` into `tier`'s mutation-sensitive group.
    assert_ne!(amount_group.columns, tier_group.columns);
}

/// A column that reaches two independently mutation-sensitive sources must
/// carry the *union* of both — never silently keep just one (fail-closed) —
/// and that union forms its own group, distinct from either single-source
/// group.
#[test]
fn two_source_projection_merges_groups_fail_closed() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
        source(
            "warehouses",
            MutationProfile::MutableSnapshot,
            None,
            &["warehouse_id"],
        ),
    ];
    let sql = "SELECT o.order_id, o.order_date, c.tier, w.region, \
               c.tier || w.region AS tier_region \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id \
               JOIN smelt.sources.warehouses w ON w.warehouse_id = o.warehouse_id";
    let skeleton = set(&["order_id", "order_date"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );

    let tier_region_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier_region".to_string()))
        .expect("tier_region is grouped");
    assert_eq!(
        tier_region_group.mutation_sensitivity,
        set(&["customers", "warehouses"]),
        "must carry both sources' sensitivity, never drop one"
    );

    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns == vec!["tier".to_string()])
        .expect("tier has its own single-source group");
    assert_eq!(tier_group.mutation_sensitivity, set(&["customers"]));
    let region_group = result
        .groups
        .iter()
        .find(|g| g.columns == vec!["region".to_string()])
        .expect("region has its own single-source group");
    assert_eq!(region_group.mutation_sensitivity, set(&["warehouses"]));

    // Three distinct sensitivity sets ⇒ three distinct groups — the merged
    // column never silently joins either single-source group.
    assert_ne!(tier_region_group.columns, tier_group.columns);
    assert_ne!(tier_region_group.columns, region_group.columns);
}

/// An unqualified column reference ambiguous among more than one joined
/// source cannot be resolved to a single provenance — the derivation must
/// fail closed (collapse to the whole model's sources) and *surface* the
/// collapse, never silently drop coverage or silently pick one source.
#[test]
fn degenerate_collapse_is_surfaced() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    // `amount` is unqualified with two joined inputs in scope — ambiguous.
    let sql = "SELECT o.order_id, o.order_date, amount \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id";
    let skeleton = set(&["order_id", "order_date"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        !result.degenerate.is_empty(),
        "the collapse must be named, never silent"
    );
    assert!(
        result.degenerate.iter().any(|d| d.column == "amount"),
        "degenerate: {:?}",
        result.degenerate
    );

    // Fail-closed: still produces a plan (widened, never dropped) — one
    // group over every declared source ("the whole table").
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].mutation_sensitivity,
        set(&["orders", "customers"])
    );
    assert!(result.groups[0].columns.contains(&"amount".to_string()));
}

/// The keyed-enriched shape (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 1): a mutable dimension read only in the JOIN's `ON` predicate —
/// never in any select item — must still drive a plan cell. Value
/// sensitivity alone would leave it invisible (no select-item expression
/// reads `dim` at all); membership sensitivity is its own derived kind that
/// attaches to every payload column group the admission read governs
/// (`model_properties.md` §"Per-column mutation-sensitivity / column
/// provenance", membership paragraph).
#[test]
fn join_only_mutable_dim_is_membership_sensitive() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.id, COUNT(f.val) AS val_count \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.id = d.id \
               GROUP BY f.id";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    assert!(group.columns.contains(&"val_count".to_string()));
    assert_eq!(
        group.mutation_sensitivity,
        set(&["fact"]),
        "value sensitivity: only fact is read in a select item"
    );
    assert_eq!(
        group.membership_sensitivity,
        set(&["dim"]),
        "membership sensitivity: dim is read only in the JOIN ON predicate, \
         in row-admission position"
    );
}

/// The complement of the previous test: an `AppendOnly` join partner's
/// retroactive-admission hazard (a later-arriving append could match a row
/// already materialized) is a *different*, out-of-scope question
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 1 "Explicitly
/// deferred"). Membership sensitivity derives only from `MutableSnapshot`
/// sources read in row-admission position — an `AppendOnly` partner
/// contributes nothing to it, even though it too is read only in the `ON`
/// predicate.
#[test]
fn append_only_join_partner_contributes_no_membership() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::AppendOnly, None, &["id"]),
    ];
    let sql = "SELECT f.id, COUNT(f.val) AS val_count \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.id = d.id \
               GROUP BY f.id";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    for group in &result.groups {
        assert!(
            group.membership_sensitivity.is_empty(),
            "an AppendOnly join partner must contribute no membership \
             sensitivity: {:?}",
            group.membership_sensitivity
        );
    }
}

/// The collector-swap red test (`docs/plans/20260808-membership-
/// sensitivity.md` Phase 1): `SUM(a.x)` must contribute `a.x`'s value
/// sensitivity, never a bogus `SUM` "column", and never force a degenerate
/// collapse — the gated `expr_util::collect_column_refs` shape, not the
/// ungated one that misreads a function call's own name as a bare column.
#[test]
fn function_wrapped_ref_collects_arguments() {
    let sources = vec![source(
        "orders",
        MutationProfile::MutableSnapshot,
        None,
        &["id"],
    )];
    let sql = "SELECT a.id, SUM(a.x) AS total FROM smelt.sources.orders a";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "SUM(a.x) must not force a degenerate collapse: {:?}",
        result.degenerate
    );
    let total_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"total".to_string()))
        .expect("total is grouped");
    assert_eq!(
        total_group.mutation_sensitivity,
        set(&["orders"]),
        "SUM(a.x) must contribute a.x's sensitivity, not a bogus 'SUM' column"
    );
}

/// Reviewer finding (`docs/plans/20260808-membership-sensitivity.md` Phase
/// 1 follow-up): `WHERE x IN (SELECT ... FROM smelt.sources.<mutable>)` is
/// a semi-join admission read the spec explicitly names alongside `ON`
/// predicates (`docs/specs/model_properties.md` §"Per-column
/// mutation-sensitivity / column provenance", membership paragraph). This
/// leaf classifier never resolves into the subquery's own FROM/aliases, so
/// it must fail closed to the whole-model collapse rather than silently
/// deriving zero sensitivity of either kind — the exact silent-hole shape
/// the spec paragraph forbids, relocated from `ON` to `WHERE`.
#[test]
fn where_in_subquery_over_mutable_source_collapses_fail_closed() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    let sql = "SELECT o.order_id, o.amount \
               FROM smelt.sources.orders o \
               WHERE o.user_id IN (SELECT user_id FROM smelt.sources.customers)";
    let skeleton = set(&["order_id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        !result.degenerate.is_empty(),
        "a WHERE-clause subquery over a mutable source must collapse, never \
         silently derive zero sensitivity: {result:?}"
    );
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].membership_sensitivity,
        set(&["orders", "customers"]),
        "the collapse must widen membership sensitivity too, not just value \
         sensitivity: {:?}",
        result.groups[0]
    );
}

/// A mutable dimension read directly in a top-level `WHERE` conjunct (no
/// subquery) is a row-admission read exactly like a `JOIN`'s `ON`
/// predicate — isolated here from the `ON`-predicate path by joining `dim`
/// on a constant (`ON 1 = 1`, no column reference to `dim` at all), so the
/// only source of `dim`'s membership sensitivity is the `WHERE` conjunct.
#[test]
fn direct_where_read_of_mutable_dim_is_membership_sensitive() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.order_id, f.amount \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON 1 = 1 \
               WHERE d.tier = 'gold'";
    let skeleton = BTreeSet::new();
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "a direct WHERE read of a mutable dim column must mark every payload \
         group membership-sensitive to it: {:?}",
        result.groups
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.mutation_sensitivity.is_empty()),
        "value sensitivity must stay empty — dim is never read in a select item: {:?}",
        result.groups
    );
}

/// Guard: the ubiquitous `WHERE <clocked column> > <literal>` time-window
/// filter on an append-only fact — the ordinary shape of nearly every
/// incremental model — must stay clean. An `AppendOnly` source read in
/// `WHERE` contributes no membership sensitivity, exactly as it contributes
/// none read in an `ON` predicate.
#[test]
fn where_filter_on_append_only_fact_contributes_no_membership() {
    let sources = vec![source(
        "fact",
        MutationProfile::AppendOnly,
        Some("event_date"),
        &[],
    )];
    let sql = "SELECT f.order_id, f.amount \
               FROM smelt.sources.fact f \
               WHERE f.event_time > '2024-01-01'";
    let skeleton = BTreeSet::new();
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    for group in &result.groups {
        assert!(
            group.membership_sensitivity.is_empty(),
            "an AppendOnly source read in WHERE must contribute no membership \
             sensitivity: {:?}",
            group.membership_sensitivity
        );
        assert!(
            group.mutation_sensitivity.is_empty(),
            "value sensitivity verdicts must stay unchanged: {:?}",
            group.mutation_sensitivity
        );
    }
}

/// Phase 3 (`docs/plans/20260809-sensitivity-precision.md`): a mutable
/// dimension joined *inside* a CTE must still attach membership sensitivity
/// at the model level — the outer scope only projects the CTE's payload
/// columns and never itself joins `dim`, so before this phase the model
/// would have collapsed whole-model rather than silently miss the join.
/// Now the interior scope's own `ON` predicate is scanned directly.
#[test]
fn cte_interior_mutable_join_attaches_membership() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "WITH enriched AS ( \
                   SELECT f.id, f.val, d.tier \
                   FROM smelt.sources.fact f \
                   JOIN smelt.sources.dim d ON f.id = d.id \
               ) \
               SELECT e.id, e.val, e.tier FROM enriched e";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        !result.groups.is_empty(),
        "must not collapse to zero groups: {result:?}"
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "a mutable dim joined inside a CTE must attach membership sensitivity \
         at the model level: {:?}",
        result.groups
    );
    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier".to_string()))
        .expect("tier is grouped");
    assert_eq!(
        tier_group.mutation_sensitivity,
        set(&["dim"]),
        "value sensitivity for the dim-sourced payload column must still \
         chase through the CTE rename: {:?}",
        tier_group
    );
    let val_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"val".to_string()))
        .expect("val is grouped");
    assert!(
        val_group.mutation_sensitivity.is_empty(),
        "val reads only the append-only fact, non-aggregated: {:?}",
        val_group
    );
}

/// Phase 3: two distinct scopes each contribute a distinct mutable
/// admission source — a CTE's own `WHERE` conjunct reads one mutable
/// dimension, and the top-level scope joins a second, different mutable
/// dimension. Both must compose into the top-level membership union.
#[test]
fn membership_composes_across_nested_scopes() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim_a", MutationProfile::MutableSnapshot, None, &["id"]),
        source("dim_b", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "WITH filtered AS ( \
                   SELECT f.id, f.val \
                   FROM smelt.sources.fact f \
                   JOIN smelt.sources.dim_a a ON f.id = a.id \
                   WHERE a.tier = 'gold' \
               ) \
               SELECT ft.id, ft.val \
               FROM filtered ft \
               JOIN smelt.sources.dim_b b ON ft.id = b.id";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        !result.groups.is_empty(),
        "must not collapse to zero groups: {result:?}"
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim_a", "dim_b"])),
        "both scopes' mutable admission sources must union at the top: {:?}",
        result.groups
    );
}

/// Phase 3: unchanged fail-closed posture for a subquery admission
/// predicate — now proven inside a nested (CTE) scope too, not just the
/// top-level scope.
#[test]
fn subquery_conjunct_still_fails_closed() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    let sql = "WITH filtered AS ( \
                   SELECT f.order_id, f.amount, f.user_id \
                   FROM smelt.sources.fact f \
                   WHERE f.user_id IN (SELECT user_id FROM smelt.sources.customers) \
               ) \
               SELECT ft.order_id, ft.amount FROM filtered ft";
    let skeleton = set(&["order_id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        !result.degenerate.is_empty(),
        "a WHERE-clause subquery inside a nested scope must collapse, never \
         silently derive zero sensitivity: {result:?}"
    );
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].membership_sensitivity,
        set(&["fact", "customers"]),
        "the collapse must widen membership sensitivity too, not just value \
         sensitivity: {:?}",
        result.groups[0]
    );
}

// ---------------------------------------------------------------------------
// Phase 4 (`docs/plans/20260809-sensitivity-precision.md`): closure-pruned
// membership sensitivity — an enrichment join whose skeleton-source closure
// (`model_properties.md` §"Skeleton-source closure") is proven `Closed` with
// row preservation established by the join SHAPE itself (a provably outer
// join, never a declared `referential_integrity` world-fact) contributes no
// membership sensitivity through its own `ON` read.
// ---------------------------------------------------------------------------

/// The load-bearing pruning case: a `LEFT JOIN` against a one-to-one,
/// payload-only, no-membership-predicate `dim` closes all five conjuncts —
/// its `ON` read must no longer attach membership sensitivity. Value
/// sensitivity is untouched: `attr` still reads `dim`'s mutable value.
#[test]
fn closed_outer_enrichment_join_prunes_membership() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.id, f.amount, d.attr AS attr \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.id = d.id";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    for group in &result.groups {
        assert!(
            group.membership_sensitivity.is_empty(),
            "a closure-Closed LEFT JOIN's own ON read must contribute no \
             membership sensitivity: {:?}",
            group.membership_sensitivity
        );
    }
    let attr_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"attr".to_string()))
        .expect("attr group");
    assert_eq!(
        attr_group.mutation_sensitivity,
        set(&["dim"]),
        "value sensitivity is untouched by the pruning rule: {attr_group:?}"
    );
}

/// The declared-`referential_integrity` route is excluded from the pruning
/// rule (`model_properties.md` §"Semantics"): `derive_column_groups` has no
/// access to a source's declared `referential_integrity` world-fact at all
/// (only `maintenance::derive`'s `SourceReferentialIntegrity` map, consulted
/// separately for the `UpstreamMutation` cell's own closure verdict, ever
/// sees it) — so a bare inner `JOIN` that a declared `referential_integrity`
/// world-fact would close at the full-plan level must still attach
/// membership sensitivity here: this module can only ever prune via the
/// join-shape (LEFT JOIN) route.
#[test]
fn declared_ri_inner_join_does_not_prune() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.id, f.amount, d.attr AS attr \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON f.id = d.id";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "an inner JOIN's closure cannot be proven via join shape alone — \
         membership sensitivity must stay attached even though a declared \
         referential_integrity world-fact (invisible to this module) could \
         close it at the full-plan level: {:?}",
        result.groups
    );
}

/// An `Open` closure — here, a `WHERE` conjunct testing the enrichment-side
/// column (conjunct 5, no-membership-predicate, fails) — never prunes:
/// membership stays attached exactly as it did before Phase 4.
#[test]
fn open_closure_does_not_prune_membership_predicate() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.id, f.amount, d.attr AS attr \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.id = d.id \
               WHERE d.attr = 'gold'";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "a WHERE predicate on the enrichment-side column breaks the closure \
         (conjunct 5) — membership sensitivity must stay attached: {:?}",
        result.groups
    );
}

/// An `Open` closure via the one-to-one conjunct (3): `dim`'s declared
/// `unique_key` (`id`) does not cover the join's equality column
/// (`category`), so fan-out cannot prove `OneToOne` — the join may be
/// `OneToMany`, and membership sensitivity must stay attached.
#[test]
fn open_closure_fan_out_does_not_prune_membership() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT f.id, f.amount, d.attr AS attr \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.id = d.category";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "an unproven one-to-one join contribution (fan-out/cardinality) \
         must not prune membership sensitivity: {:?}",
        result.groups
    );
}

/// The v1 scope restriction (`skeleton_closure`'s own aggregation
/// restriction, folded into conjunct evaluation): a top-level scope with a
/// `GROUP BY` above the enrichment join is `Open` regardless of the other
/// four conjuncts, even though the join itself is a `LEFT JOIN` against a
/// one-to-one, no-membership-predicate `dim` — an aggregating scope changes
/// the skeleton question (which rows survive the fold, not which rows
/// survive the join), so membership sensitivity must stay attached.
#[test]
fn aggregating_scope_does_not_prune_membership() {
    let sources = vec![
        source("fact", MutationProfile::AppendOnly, Some("event_date"), &[]),
        source("dim", MutationProfile::MutableSnapshot, None, &["id"]),
    ];
    let sql = "SELECT d.category, COUNT(*) AS n \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.dim_id = d.id \
               GROUP BY d.category";
    let skeleton = set(&["category"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    assert!(
        result
            .groups
            .iter()
            .all(|g| g.membership_sensitivity == set(&["dim"])),
        "an aggregating top-level scope (GROUP BY) must not prune \
         membership sensitivity through the v1 scope restriction, even \
         though the enrichment join is otherwise a closeable LEFT JOIN: \
         {:?}",
        result.groups
    );
}
