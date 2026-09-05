//! Fan-out/cardinality and join-contribution monotonicity proofs.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" rows **"Fan-out /
//! cardinality"** and **"Join-contribution monotonicity"**. Fan-out decides
//! whether a join multiplies rows (`OneToMany`) or enriches in place
//! (`OneToOne`); join-contribution monotonicity composes that fact with the
//! algebraic discriminants (`discriminants::Discriminants`) of the aggregate
//! the join feeds to decide whether the dimension-driven horizon MERGE (F15)
//! may be licensed. Both are fail-closed: an undecidable join shape or an
//! unclassified combiner never yields an optimistic verdict.

use crate::analysis::discriminants::{Discriminants, Monotone};
use crate::analysis::source_bounds::resolve_table_ref_source_name;
use smelt_parser::{Expr, File, JoinClause, JoinCondition, JoinType};
use std::collections::{HashMap, HashSet};

/// Whether a join multiplies rows against the target's existing cardinality,
/// or enriches it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// The join matches at most one row on the joined side per row on the
    /// probe side — enrichment, not row multiplication.
    OneToOne,
    /// The join may match more than one row on the joined side — row
    /// multiplication. Also the fail-closed verdict for any join shape whose
    /// cardinality cannot be proven `OneToOne`.
    OneToMany,
}

/// Declares which column-sets are known to uniquely identify a row of a
/// given source — the fact a catalog/PK declaration would otherwise supply.
/// `smelt-logical` has no catalog access (layering rule), so callers inject
/// these facts the same way `source_bounds::BoundContext` injects partition
/// columns. A source absent from the map has no declared unique key; a join
/// against it can only be proven `OneToOne` via a `USING`/`ON` equality whose
/// matched columns are a superset of (i.e. cover every column of) at least
/// one declared key-set — a single-column key as well as a genuine COMPOSITE
/// key (e.g. `(user_id, dt)`, jointly but not individually unique) are both
/// expressible: a single-column key is just a key-set of size one.
#[derive(Debug, Default, Clone)]
pub struct JoinContext {
    /// Source name (or alias, whichever the join condition qualifies columns
    /// with) -> the declared key-sets for that source. Each inner set is one
    /// declared key (its columns are jointly, not necessarily individually,
    /// unique); a source may have more than one declared key-set.
    pub unique_keys: HashMap<String, Vec<HashSet<String>>>,
}

impl JoinContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `column` alone uniquely identifies a row of `source` —
    /// convenience sugar for a 1-column composite key.
    pub fn with_unique_key(self, source: &str, column: &str) -> Self {
        self.with_composite_unique_key(source, &[column])
    }

    /// Declare that `columns`, taken together, uniquely identify a row of
    /// `source` (a composite key: the columns need not be individually
    /// unique, only jointly so).
    pub fn with_composite_unique_key(mut self, source: &str, columns: &[&str]) -> Self {
        self.unique_keys
            .entry(source.to_string())
            .or_default()
            .push(columns.iter().map(|c| c.to_string()).collect());
        self
    }

    /// Union `self` with `other`'s declared key-sets — every key-set either
    /// side declares for a source is kept (never dropped, never
    /// deduplicated beyond the `HashSet` identity `fan_out` already relies
    /// on). Used to combine two independently-built contexts covering
    /// disjoint declaration sources (e.g. model edges' own `unique_key`
    /// alongside external sources' `unique_key`) into the single shared
    /// context a route's proofs must all see.
    pub fn union(mut self, other: Self) -> Self {
        for (source, key_sets) in other.unique_keys {
            self.unique_keys.entry(source).or_default().extend(key_sets);
        }
        self
    }
}

/// Prove the cardinality of `join` against `ctx`'s declared unique keys.
///
/// A join is proven `OneToOne` when the equi-join's matched columns for the
/// joined table are a SUPERSET of (contain every column of) at least one
/// declared key-set for that source — this covers both single-column keys
/// and genuine composite keys. Matching only a strict SUBSET of a declared
/// composite key does NOT prove one-to-one: that is the key hazard this
/// function fails closed on.
///
/// Fail-closed: a `CROSS JOIN` (no key at all), a join with no condition, or
/// an equality condition that does not fully cover any declared key-set all
/// yield `OneToMany` — the conservative verdict is never optimistically
/// skipped.
pub fn fan_out(join: &JoinClause, ctx: &JoinContext) -> Cardinality {
    if matches!(join.join_type(), Some(JoinType::Cross)) {
        return Cardinality::OneToMany;
    }
    let Some(table_ref) = join.table_ref() else {
        return Cardinality::OneToMany;
    };
    let alias = table_ref.alias();
    let base_name = table_ref.identifier();
    let Some(condition) = join.condition() else {
        return Cardinality::OneToMany;
    };

    let equality_columns: HashSet<String> =
        equality_columns_for_table(&condition, alias.as_deref(), base_name.as_deref())
            .into_iter()
            .collect();
    let is_unique = [alias.as_deref(), base_name.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| {
            ctx.unique_keys.get(name).is_some_and(|key_sets| {
                key_sets
                    .iter()
                    .any(|key_set| key_set.is_subset(&equality_columns))
            })
        });

    if is_unique {
        Cardinality::OneToOne
    } else {
        Cardinality::OneToMany
    }
}

/// Column names referenced against `alias`/`base_name` in top-level ANDed
/// equalities of `condition` (an `ON` expression, walked recursively through
/// `AND`; or the columns of a `USING` clause, which by construction name the
/// same column on both sides).
fn equality_columns_for_table(
    condition: &JoinCondition,
    alias: Option<&str>,
    base_name: Option<&str>,
) -> Vec<String> {
    if condition.is_using() {
        return condition.using_columns();
    }
    let Some(on_expr) = condition.on_expression() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_equality_columns(&on_expr, alias, base_name, &mut out);
    out
}

fn collect_equality_columns(
    expr: &Expr,
    alias: Option<&str>,
    base_name: Option<&str>,
    out: &mut Vec<String>,
) {
    // `Expr::as_binary()` prefers descending into a nested `BinaryExpr` child
    // over self-casting; for a chained `AND` (>2 top-level equalities) that
    // means the natural left-associative parse — a `BinaryExpr` node whose
    // own children are themselves `BinaryExpr`s — mis-resolves to the
    // innermost leaf instead of the current AND node. Self-cast first so a
    // 3+ term `AND` chain still recurses through every level.
    let Some(bin) =
        smelt_parser::BinaryExpr::cast(expr.syntax().clone()).or_else(|| expr.as_binary())
    else {
        return;
    };
    let Some(op) = bin.operator() else {
        return;
    };
    if op.eq_ignore_ascii_case("AND") {
        if let Some(left) = bin.left() {
            collect_equality_columns(&left, alias, base_name, out);
        }
        if let Some(right) = bin.right() {
            collect_equality_columns(&right, alias, base_name, out);
        }
        return;
    }
    if op != "=" {
        return;
    }
    let (Some(left), Some(right)) = (bin.left(), bin.right()) else {
        return;
    };
    for side in [&left, &right] {
        if let Some(col_ref) = side.as_column_ref() {
            let matches_table = match col_ref.qualifier() {
                Some(q) => Some(q) == alias || Some(q) == base_name,
                // An unqualified column in the ON clause: only attributable
                // to this table when the join has no other table to qualify
                // against (single-table condition side) — conservatively
                // still require a qualifier match, since an unqualified
                // column could belong to either side.
                None => false,
            };
            if matches_table {
                out.push(col_ref.name().to_string());
            }
        }
    }
}

/// The verdict of composing fan-out with the downstream combiner's algebraic
/// discriminants: does this join's per-key contribution fold into the target
/// without needing an inverse (a monotone fold), or must it be refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContributionVerdict {
    /// The contribution folds without an inverse (value- or order-monotone)
    /// and does not fan into a decrementing aggregate.
    Monotone,
    /// Refused, naming why — never returned optimistically.
    Refused(String),
}

impl ContributionVerdict {
    pub fn is_monotone(&self) -> bool {
        matches!(self, ContributionVerdict::Monotone)
    }
}

/// Compose a join's [`fan_out`] verdict with the [`Discriminants`] of the
/// aggregate its contribution feeds, to decide whether the contribution folds
/// monotonically (licensing the dimension-driven horizon MERGE, F15).
///
/// Fail-closed: any combiner without a monotone discriminant is refused
/// regardless of fan-out (it is not licensed by this proof at all); any
/// row-multiplying join into a decrementing aggregate (a monoid with an
/// inverse — `SUM`/`COUNT`) is refused with that specific reason, since
/// row-multiplication changes which values are being decremented later.
pub fn join_contribution_monotone(
    fan_out: Cardinality,
    discriminants: &Discriminants,
) -> ContributionVerdict {
    if fan_out == Cardinality::OneToMany && discriminants.is_monoid && !discriminants.needs_inverse
    {
        return ContributionVerdict::Refused(
            "join fans out (row-multiplying) into a decrementing aggregate (a monoid with an \
             inverse, e.g. SUM/COUNT); exact retraction requires the un-fanned contribution"
                .to_string(),
        );
    }
    if discriminants.monotone == Monotone::None {
        return ContributionVerdict::Refused(
            "downstream aggregate combiner carries no value- or order-monotonicity".to_string(),
        );
    }
    if fan_out == Cardinality::OneToMany {
        return ContributionVerdict::Refused(
            "join fans out (row-multiplying); a monotone fold at the dimension side cannot be \
             composed across duplicated rows"
                .to_string(),
        );
    }
    ContributionVerdict::Monotone
}

/// Find the top-level join clause (plus the alias, or bare identifier when
/// unaliased, `fan_out`'s `JoinContext` lookup keys on) whose `smelt.<path>`
/// table ref resolves to `source` in `sql`'s outermost query scope. `None`
/// when `sql` doesn't parse, has no top-level `SELECT`/`FROM`, or `source` is
/// not the target of a join there (e.g. it is the `FROM`-clause driving
/// table, or is not referenced in this scope at all).
fn find_join_for_source(sql: &str, source: &str) -> Option<(JoinClause, String)> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let from = select.from_clause()?;
    for join in from.joins() {
        let table_ref = join.table_ref()?;
        let Some(resolved) = resolve_table_ref_source_name(&table_ref) else {
            continue;
        };
        let matches = resolved == source
            || resolved
                .strip_prefix("sources.")
                .is_some_and(|bare| bare == source);
        if matches {
            // No explicit `AS alias`: a `smelt.<path>` ref's implicit name is
            // its own last path segment (`table_ref.identifier()`/`alias()`
            // only see direct-child tokens, never a nested `SmeltPathRef`/
            // `SmeltPathCall` node's own segments — the same fallback
            // `meta_eval`/`maintenance::emit` already use for this exact
            // shape).
            let alias = table_ref
                .alias()
                .or_else(|| {
                    table_ref
                        .smelt_path_ref()
                        .and_then(|p| p.segments().last().cloned())
                })
                .or_else(|| {
                    table_ref
                        .smelt_path_call()
                        .and_then(|p| p.segments().last().cloned())
                })
                .or_else(|| table_ref.identifier())?;
            return Some((join, alias));
        }
    }
    None
}

/// Resolve the alias (or bare identifier, when unaliased) `fan_out`'s
/// `JoinContext` lookup keys on for the top-level join whose `smelt.<path>`
/// table ref resolves to `source` (`sources.`-prefix optional, matching
/// [`resolve_table_ref_source_name`]'s convention). `None` when `source` is
/// not joined in `sql`'s outermost query scope at all.
///
/// Moved from `smelt-runtime::maintenance_driver::find_join_alias` (this
/// crate's [`dimension_join_contribution`]-equivalent proofs are the only
/// production consumers of a join's resolved alias, and `smelt-runtime` may
/// not itself scan SQL for join shape — `architecture.md` §"Property
/// composition walk rule").
pub fn join_alias_for_source(sql: &str, source: &str) -> Option<String> {
    find_join_for_source(sql, source).map(|(_, alias)| alias)
}

/// Resolve `source`'s own top-level join `Cardinality` in `sql` against
/// `ctx`'s declared unique keys ([`fan_out`]), or `None` when `source` is not
/// joined in `sql`'s outermost query scope at all — the same "no join at
/// all" case [`dimension_join_contribution`] refuses outright, left to the
/// caller to interpret (a derivation site composing this with a downstream
/// combiner's algebra has its own "nothing new to push" reading of `None`,
/// distinct from `dimension_join_contribution`'s hard refusal).
pub fn source_join_cardinality(sql: &str, source: &str, ctx: &JoinContext) -> Option<Cardinality> {
    let (join, _alias) = find_join_for_source(sql, source)?;
    Some(fan_out(&join, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_join(sql: &str) -> JoinClause {
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let select = file.select_stmt().expect("select stmt");
        let from = select.from_clause().expect("from clause");
        let join = from.joins().next().expect("join clause");
        join
    }

    #[test]
    fn equi_join_on_declared_unique_key_is_one_to_one() {
        let join = parse_join("SELECT * FROM events e JOIN dims d ON e.dim_id = d.id");
        let ctx = JoinContext::new().with_unique_key("d", "id");
        assert_eq!(fan_out(&join, &ctx), Cardinality::OneToOne);
    }

    #[test]
    fn equi_join_on_non_unique_key_is_one_to_many() {
        let join = parse_join("SELECT * FROM events e JOIN dims d ON e.dim_id = d.category");
        let ctx = JoinContext::new().with_unique_key("d", "id");
        assert_eq!(fan_out(&join, &ctx), Cardinality::OneToMany);
    }

    #[test]
    fn unknown_cardinality_fails_closed_to_one_to_many() {
        // No declared unique keys at all — the conservative verdict.
        let join = parse_join("SELECT * FROM events e JOIN dims d ON e.dim_id = d.id");
        let ctx = JoinContext::new();
        assert_eq!(fan_out(&join, &ctx), Cardinality::OneToMany);
    }

    #[test]
    fn cross_join_fails_closed_to_one_to_many() {
        let join = parse_join("SELECT * FROM events e CROSS JOIN dims d");
        let ctx = JoinContext::new().with_unique_key("d", "id");
        assert_eq!(fan_out(&join, &ctx), Cardinality::OneToMany);
    }

    #[test]
    fn using_clause_on_declared_unique_key_is_one_to_one() {
        let join = parse_join("SELECT * FROM events e JOIN dims d USING (id)");
        let ctx = JoinContext::new().with_unique_key("d", "id");
        assert_eq!(fan_out(&join, &ctx), Cardinality::OneToOne);
    }

    fn dim_lookup_discriminants(monotone: Monotone) -> Discriminants {
        Discriminants {
            is_monoid: monotone == Monotone::Value,
            needs_inverse: monotone == Monotone::Value,
            decomposable: false,
            monotone,
        }
    }

    #[test]
    fn one_to_one_monotone_contribution_is_admitted() {
        let verdict = join_contribution_monotone(
            Cardinality::OneToOne,
            &dim_lookup_discriminants(Monotone::Value),
        );
        assert_eq!(verdict, ContributionVerdict::Monotone);
    }

    #[test]
    fn one_to_one_order_monotone_contribution_is_admitted() {
        let discriminants = Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::Order,
        };
        assert!(join_contribution_monotone(Cardinality::OneToOne, &discriminants).is_monotone());
    }

    #[test]
    fn contribution_feeding_sum_is_refused() {
        // SUM: is_monoid, !needs_inverse, no monotonicity claimed — a
        // decrementing aggregate, never admitted as a monotone contribution.
        let sum_discriminants = Discriminants {
            is_monoid: true,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::None,
        };
        let verdict = join_contribution_monotone(Cardinality::OneToOne, &sum_discriminants);
        assert!(!verdict.is_monotone());
        assert!(matches!(verdict, ContributionVerdict::Refused(reason) if !reason.is_empty()));
    }

    #[test]
    fn fan_out_into_decrementing_aggregate_is_refused() {
        let sum_discriminants = Discriminants {
            is_monoid: true,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::None,
        };
        let verdict = join_contribution_monotone(Cardinality::OneToMany, &sum_discriminants);
        assert!(!verdict.is_monotone());
        assert!(
            matches!(&verdict, ContributionVerdict::Refused(reason) if reason.contains("decrementing"))
        );
    }

    #[test]
    fn fan_out_of_otherwise_monotone_contribution_is_refused() {
        // Even a value-monotone combiner (MIN/MAX) is refused once the join
        // fans out — the row-multiplication changes what is being folded.
        let max_discriminants = Discriminants {
            is_monoid: true,
            needs_inverse: true,
            decomposable: false,
            monotone: Monotone::Value,
        };
        let verdict = join_contribution_monotone(Cardinality::OneToMany, &max_discriminants);
        assert!(!verdict.is_monotone());
    }

    #[test]
    fn undecidable_join_shape_never_yields_optimistic_monotone() {
        // A join with no condition at all: fan_out fails closed to
        // OneToMany, so even a monotone combiner is refused end-to-end.
        let join = parse_join("SELECT * FROM events e CROSS JOIN dims d");
        let ctx = JoinContext::new();
        let cardinality = fan_out(&join, &ctx);
        let discriminants = Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::Order,
        };
        let verdict = join_contribution_monotone(cardinality, &discriminants);
        assert!(!verdict.is_monotone());
    }

    #[test]
    fn join_alias_for_source_resolves_an_aliased_join() {
        let sql = "SELECT o.customer_id FROM smelt.sources.orders o \
                    JOIN smelt.sources.customers c ON o.customer_id = c.customer_id";
        assert_eq!(
            join_alias_for_source(sql, "customers"),
            Some("c".to_string())
        );
    }

    #[test]
    fn join_alias_for_source_resolves_an_unaliased_join() {
        let sql = "SELECT o.customer_id FROM smelt.sources.orders o \
                    JOIN smelt.sources.customers ON o.customer_id = customers.customer_id";
        assert_eq!(
            join_alias_for_source(sql, "customers"),
            Some("customers".to_string())
        );
    }

    #[test]
    fn join_alias_for_source_is_none_when_not_joined() {
        let sql = "SELECT o.customer_id FROM smelt.sources.orders o \
                    JOIN smelt.sources.customers c ON o.customer_id = c.customer_id";
        // `orders` is the driving FROM table, not the target of a join.
        assert_eq!(join_alias_for_source(sql, "orders"), None);
        // A source not referenced at all.
        assert_eq!(join_alias_for_source(sql, "nonexistent"), None);
    }
}
