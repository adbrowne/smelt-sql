//! TDD tests for the fold-contribution leaf classifier
//! (`smelt_logical::maintenance::derive::source_contributes_to_fold`) —
//! `docs/plans/20260720-prod-w10-keyed-mutable-admission.md` Phase 2.
//!
//! The predicate answers "does `source` appear as an argument to any
//! aggregate `sql`'s outermost `SELECT` folds?" It is the safety fact the
//! narrowed key-grain `NewData` admission (Phase 3 of the same plan) needs
//! to tell apart a mutable source consumed only via a covered enrichment
//! join (safe to admit) from one that both feeds the fold and is
//! enrich-joined (must stay refused — the folded contribution is
//! un-retractable).
//!
//! **The load-bearing invariant asserted throughout: false negatives are
//! forbidden.** A source that genuinely does feed the fold must never
//! classify `false` — that would be the admission hole. False positives
//! (refusing a genuinely enrich-only source) only cost permissiveness and
//! are the documented conservative fallback on any ambiguity the leaf
//! classifier cannot resolve (unqualified references among multiple
//! sources, unresolved aliases, CTEs/derived tables/set operations).

use smelt_logical::maintenance::derive::source_contributes_to_fold;

/// `SUM(dim.amount)` with `dim` the source directly qualifying the
/// aggregate's argument — the base case the predicate exists to catch.
#[test]
fn source_qualifying_aggregate_argument_contributes() {
    let sql = "SELECT fact.id, SUM(dim.amount) AS total \
               FROM smelt.sources.fact fact \
               JOIN smelt.sources.dim dim ON dim.id = fact.dim_id";
    assert!(
        source_contributes_to_fold(sql, "dim"),
        "dim.amount is a direct argument of SUM — must classify true"
    );
}

/// The fold reads only `fact.*`; `dim` is joined in but appears only in the
/// `JOIN ... ON` clause and a plain (non-aggregated) `SELECT` enrichment
/// column, never inside an aggregate's own arguments. This is the
/// enrich-only shape Phase 3's narrowing is meant to admit — the predicate
/// must say `false` here so that admission path is reachable.
#[test]
fn enrich_only_join_column_does_not_contribute() {
    let sql = "SELECT fact.id, dim.label, SUM(fact.amount) AS total \
               FROM smelt.sources.fact fact \
               JOIN smelt.sources.dim dim ON dim.id = fact.dim_id";
    assert!(
        !source_contributes_to_fold(sql, "dim"),
        "dim never appears inside an aggregate argument — must classify false"
    );
    // `fact` does feed the fold (SUM(fact.amount)) — sanity check the
    // predicate is not just returning false unconditionally.
    assert!(source_contributes_to_fold(sql, "fact"));
}

/// A source aliased in `FROM` and referenced via that alias inside an
/// aggregate. Alias resolution against a source's OWN alias is in scope
/// (the classifier builds the FROM-alias map itself), so this is the
/// positive case — `true`.
#[test]
fn aliased_source_referenced_via_alias_inside_aggregate_contributes() {
    let sql = "SELECT f.id, SUM(d.amount) AS total \
               FROM smelt.sources.fact f \
               JOIN smelt.sources.dim d ON d.id = f.dim_id";
    assert!(
        source_contributes_to_fold(sql, "dim"),
        "d is dim's own FROM alias, referenced inside SUM — must classify true"
    );
}

/// Qualified references are resolved by alias exactly like bare (unaliased)
/// references resolve by source name — same code path, asserted
/// separately so a future change that special-cases one but not the other
/// is caught.
#[test]
fn qualified_and_bare_source_references_handled_the_same_way() {
    let qualified = "SELECT fact.id, SUM(dim.amount) AS total \
                      FROM smelt.sources.fact fact \
                      JOIN smelt.sources.dim dim ON dim.id = fact.dim_id";
    let bare_alias = "SELECT f.id, SUM(dim.amount) AS total \
                       FROM smelt.sources.fact f \
                       JOIN smelt.sources.dim dim ON dim.id = f.dim_id";
    assert!(source_contributes_to_fold(qualified, "dim"));
    assert!(source_contributes_to_fold(bare_alias, "dim"));
}

/// Conservative fallback, pinned explicitly: an unqualified column
/// reference inside an aggregate, with more than one source joined in, is
/// ambiguous — the classifier cannot prove the reference is NOT `dim`, so
/// it must classify `true` rather than guess `false`. This is the fail-safe
/// direction the plan requires be pinned by an explicit test.
#[test]
fn ambiguous_unqualified_reference_inside_aggregate_conservatively_contributes() {
    let sql = "SELECT fact.id, SUM(amount) AS total \
               FROM smelt.sources.fact fact \
               JOIN smelt.sources.dim dim ON dim.id = fact.dim_id";
    assert!(
        source_contributes_to_fold(sql, "dim"),
        "an unqualified aggregate argument with two joined sources is ambiguous — \
         must conservatively classify true, never false"
    );
    // Symmetric: the same ambiguity must also bias toward true for the
    // OTHER joined source, not just `dim` — the predicate never resolves
    // an ambiguous reference in either direction.
    assert!(
        source_contributes_to_fold(sql, "fact"),
        "the same ambiguous reference must conservatively contribute for fact too"
    );
}

/// A source not referenced anywhere in the FROM clause at all cannot
/// possibly feed the fold — the one case this classifier answers `false`
/// with total confidence (no ambiguity to bias away from).
#[test]
fn source_absent_from_from_clause_does_not_contribute() {
    let sql = "SELECT fact.id, SUM(fact.amount) AS total FROM smelt.sources.fact fact";
    assert!(!source_contributes_to_fold(sql, "dim"));
}

/// A CTE composing the fold body is outside this leaf classifier's
/// single-scope resolution (mirrors `maintenance::grouping`'s own v0
/// restriction) — conservatively `true`, never a guess through the CTE.
#[test]
fn cte_composed_fold_body_conservatively_contributes() {
    let sql = "WITH enriched AS (SELECT * FROM smelt.sources.fact) \
               SELECT id, SUM(amount) AS total FROM enriched";
    assert!(
        source_contributes_to_fold(sql, "fact"),
        "a CTE-composed fold body is outside single-scope resolution — must \
         conservatively classify true"
    );
}

/// A source that both feeds the fold AND is separately projected as a
/// plain enrichment column — the both-fold-and-enrich overlap the spec
/// says must stay refused. The predicate must still say `true` (it feeds
/// the fold at all), which is what licenses the caller's refusal.
#[test]
fn source_both_folded_and_enrich_projected_still_contributes() {
    let sql = "SELECT fact.id, dim.label, SUM(dim.amount) AS total \
               FROM smelt.sources.fact fact \
               JOIN smelt.sources.dim dim ON dim.id = fact.dim_id";
    assert!(source_contributes_to_fold(sql, "dim"));
}
