// ===== The model property vector: grain, FDs, discriminants, determinism =====

use std::collections::BTreeMap;

use serde::Serialize;

use crate::analysis::discriminants::{combiner_discriminants, Discriminants};
use crate::analysis::join_shape::{fan_out, Cardinality, JoinContext};
use crate::analysis::monotonicity::{classify_function_determinism, FunctionDeterminism};
use crate::analysis::{
    item_alias, item_expr, resolve_scope_group_by, select_stmt_items, SelectItemKind,
};

use super::tree::*;

/// One proven key of a relation: a set of output-column names that together
/// uniquely identify a row. The empty grain (no keys) is the fail-closed
/// default — an unkeyed relation is `OneToMany`.
pub type KeySet = Vec<String>;

/// The proven grain of a relation node — the keys the walk can establish from
/// query structure (a `GROUP BY` factory key, a `DISTINCT` whole-row key, a
/// discriminated-union key). Empty ⇒ no key proven ⇒ `OneToMany` (fail-closed;
/// grain is never optimistically assumed, `model_properties.md` §Constraints).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Grain {
    /// Column sets each of which uniquely identifies an output row.
    pub keys: Vec<KeySet>,
}

impl Grain {
    /// The fail-closed grain: no key proven.
    pub fn unkeyed() -> Self {
        Grain { keys: Vec::new() }
    }

    /// Whether some proven key is a subset of `candidate` (augmentation: a
    /// superkey of a key is itself a key, so it determines every column).
    pub fn has_subset_key(&self, candidate: &std::collections::BTreeSet<String>) -> bool {
        self.keys.iter().any(|k| {
            k.iter()
                .all(|c| candidate.contains(&c.to_ascii_lowercase()))
        })
    }
}

/// A functional dependency derived by the walk from query structure:
/// `key → determines` (output-column names in the node's own scope). An empty
/// `key` is the constant-column FD (`∅ → c` for a literal column).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DerivedFd {
    pub key: KeySet,
    pub determines: String,
}

/// Determinism level of a projected column — the lattice `Clean < Run < Row`
/// (`model_properties.md` §"Determinism (run vs row) and the nondeterminism
/// predicate"). A columnar union takes the per-position lub (`clean ∪ clean =
/// clean`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Determinism {
    /// Deterministic every run (a plain column, a deterministic expression).
    Clean,
    /// One value per run (`NOW`, `CURRENT_DATE`) — a per-run constant.
    Run,
    /// A fresh value per row (`RANDOM`, `UUID`) — unpinnable.
    Row,
}

/// Per-column determinism fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnDeterminism {
    pub output: String,
    pub level: Determinism,
}

/// Change-comparability of a projected column — is its value a pure
/// function of the *processed* inputs, and therefore comparable across
/// separate runs (not merely stable within one run) for future
/// write-suppression purposes (`model_properties.md` §"Change
/// comparability"). A two-point lattice, `Comparable ⊑ Incomparable`; a
/// columnar union takes the per-position lub, same shape as
/// [`Determinism`]'s. `Incomparable` is the fail-closed default — an
/// unrecognised construct never defaults to `Comparable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Comparability {
    /// A pure function of processed inputs — safe to diff against a prior
    /// run's stored value.
    Comparable,
    /// Not provably stable across runs (a run-/row-nondeterministic value,
    /// or an unrecognised construct the classifier has no rule for).
    Incomparable,
}

impl Default for Comparability {
    /// Fail-closed: absence of a proof is `Incomparable`, never an
    /// optimistic `Comparable`.
    fn default() -> Self {
        Comparability::Incomparable
    }
}

/// Per-column change-comparability fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnComparability {
    pub output: String,
    pub comparability: Comparability,
}

/// Per-column algebraic discriminants of an aggregate output column (the
/// combiner classifier applied at the aggregate's defining scope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnDiscriminant {
    pub output: String,
    pub discriminants: Discriminants,
}

/// The whole-model property vector — one derivation every current and future
/// consumer reads (`model_properties.md` §"The composition walk"). Grain,
/// functional dependencies, per-column discriminants, and the determinism
/// predicate are folded bottom-up by [`PropertyTransfer`]; the fields are the
/// four fact families, all fail-closed by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PropertyVector {
    /// Output columns of this node, in projection order.
    pub columns: Vec<String>,
    /// The proven grain (keys). Empty ⇒ unkeyed.
    pub grain: Grain,
    /// Query-derived functional dependencies (from grain + literal columns).
    pub fds: Vec<DerivedFd>,
    /// Per-column determinism.
    pub determinism: Vec<ColumnDeterminism>,
    /// Per-column change-comparability (P3, `model_properties.md` §"Change
    /// comparability") — is the column's value a pure function of the
    /// processed inputs, comparable across separate runs.
    pub comparability: Vec<ColumnComparability>,
    /// Per-column aggregate discriminants (aggregate outputs only).
    pub discriminants: Vec<ColumnDiscriminant>,
    /// Output columns that are constant literals here, name → literal text.
    pub literal_columns: Vec<(String, String)>,
    /// Whether an output column crosses a set operation (`UNION`/`INTERSECT`/
    /// `EXCEPT`) whose branches are not proven key-disjoint — a structural
    /// barrier for FD survival (`20260707-property-per-key-constancy.md` §3.8).
    pub has_set_op_barrier: bool,
    /// Whether an input join proves `OneToMany` (row-multiplying) — a
    /// structural disproof of per-key constancy for probe-side FDs.
    pub has_fan_out_join: bool,
}

impl PropertyVector {
    /// The FD set implied by grain and literal columns: every proven key
    /// determines every other output column, and every literal column is
    /// `∅ → c`.
    fn fds_from_facts(&self) -> Vec<DerivedFd> {
        let mut out = Vec::new();
        for key in &self.grain.keys {
            let key_lower: std::collections::BTreeSet<String> =
                key.iter().map(|c| c.to_ascii_lowercase()).collect();
            for c in &self.columns {
                if !key_lower.contains(&c.to_ascii_lowercase()) {
                    out.push(DerivedFd {
                        key: key.clone(),
                        determines: c.clone(),
                    });
                }
            }
        }
        for (name, _lit) in &self.literal_columns {
            out.push(DerivedFd {
                key: Vec::new(),
                determines: name.clone(),
            });
        }
        out
    }
}

/// The property-vector transfer function: grain / FD / discriminant /
/// determinism folded together over the walk (`model_properties.md` §"The
/// composition walk"). Leaf-level classifiers (the join-fan-out proof, the
/// aggregate discriminant classifier, the nondeterminism predicate) are the
/// existing pure functions, invoked per node; the operator rules apply the
/// per-construct transfer rules of `20260707-property-per-key-constancy.md`
/// §§3–5 (the `GROUP BY`/`DISTINCT` factory; undiscriminated-union barrier;
/// discriminated-union key survival).
pub struct PropertyTransfer<'a> {
    pub ctx: &'a JoinContext,
}

impl PropertyTransfer<'_> {
    /// Map this scope's `GROUP BY` keys to their output-column names. Returns
    /// `None` (fail-closed ⇒ unkeyed) when any grouping key is not a projected
    /// output column (grouped by a non-projected expression), since such a key
    /// cannot be named on the output relation.
    fn group_by_output_keys(&self, sn: &SelectNode) -> Option<Vec<String>> {
        let items = select_stmt_items(&sn.select)?;
        let gb_keys = resolve_scope_group_by(&sn.select, &items);
        if gb_keys.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(gb_keys.len());
        for key in &gb_keys {
            let named = items.iter().find_map(|item| {
                if item_expr(item).text().trim() == key {
                    let alias = item_alias(item);
                    Some(if alias.is_empty() {
                        key.clone()
                    } else {
                        alias.to_string()
                    })
                } else {
                    None
                }
            });
            out.push(named?);
        }
        Some(out)
    }

    /// Per-column determinism of one SELECT scope, reducing plain column refs
    /// through CTE/derived inputs where the input's own determinism is known,
    /// and folding in the worst column verdict of any `ExprScope` a select
    /// item embeds (`model_properties.md` §"The composition walk": a select
    /// item containing an expression-position subquery takes the max of its
    /// own syntactic verdict, excluding the subquery subtree, and that
    /// scope's own worst column verdict).
    fn scope_determinism(
        &self,
        sn: &SelectNode,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &PropertyVector>,
        expr_scopes: &[(&ExprScope, &PropertyVector)],
    ) -> Vec<ColumnDeterminism> {
        let Some(select_list) = sn.select.select_list() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in select_list.items() {
            if item.is_wildcard() {
                continue;
            }
            let Some(expr) = item.expression() else {
                continue;
            };
            let output = item
                .column_name()
                .unwrap_or_else(|| expr.text().trim().to_string());
            let mut level = expr_determinism(&expr);
            // Reduce a plain column reference through a CTE/derived input.
            if let Some(col_ref) = smelt_parser::ColumnRef::from_expr(&expr) {
                if let Some(source) = resolve_alias_source(cx, col_ref.qualifier()) {
                    if let Some(inner) = input_props.get(&source) {
                        if let Some(d) = inner
                            .determinism
                            .iter()
                            .find(|d| d.output.eq_ignore_ascii_case(col_ref.name()))
                        {
                            level = level.max(d.level);
                        }
                    }
                }
            }
            let item_range = expr.syntax().text_range();
            for (es, verdict) in expr_scopes {
                if !item_range.contains_range(es.range) {
                    continue;
                }
                let worst = verdict
                    .determinism
                    .iter()
                    .map(|d| d.level)
                    .max()
                    .unwrap_or(Determinism::Clean);
                level = level.max(worst);
            }
            out.push(ColumnDeterminism { output, level });
        }
        out
    }

    /// Per-column change-comparability of one SELECT scope, reducing plain
    /// column refs through CTE/derived inputs where the input's own
    /// comparability is known (`model_properties.md` §"Change
    /// comparability"), and folding in the worst column verdict of any
    /// `ExprScope` a select item embeds — same rule as
    /// [`Self::scope_determinism`].
    fn scope_comparability(
        &self,
        sn: &SelectNode,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &PropertyVector>,
        expr_scopes: &[(&ExprScope, &PropertyVector)],
    ) -> Vec<ColumnComparability> {
        let Some(select_list) = sn.select.select_list() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in select_list.items() {
            if item.is_wildcard() {
                continue;
            }
            let Some(expr) = item.expression() else {
                continue;
            };
            let output = item
                .column_name()
                .unwrap_or_else(|| expr.text().trim().to_string());
            let mut comparability = expr_comparability(&expr);
            // Reduce a plain column reference through a CTE/derived input.
            if let Some(col_ref) = smelt_parser::ColumnRef::from_expr(&expr) {
                if let Some(source) = resolve_alias_source(cx, col_ref.qualifier()) {
                    if let Some(inner) = input_props.get(&source) {
                        if let Some(c) = inner
                            .comparability
                            .iter()
                            .find(|c| c.output.eq_ignore_ascii_case(col_ref.name()))
                        {
                            comparability = comparability.max(c.comparability);
                        }
                    }
                }
            }
            let item_range = expr.syntax().text_range();
            for (es, verdict) in expr_scopes {
                if !item_range.contains_range(es.range) {
                    continue;
                }
                let worst = verdict
                    .comparability
                    .iter()
                    .map(|c| c.comparability)
                    .max()
                    .unwrap_or(Comparability::Comparable);
                comparability = comparability.max(worst);
            }
            out.push(ColumnComparability {
                output,
                comparability,
            });
        }
        out
    }

    /// Per-column aggregate discriminants of one SELECT scope.
    fn scope_discriminants(&self, sn: &SelectNode) -> Vec<ColumnDiscriminant> {
        let Some(items) = select_stmt_items(&sn.select) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in &items {
            let (name, distinct, alias) = match item {
                SelectItemKind::CountDistinct { alias, .. } => {
                    (Some("COUNT".to_string()), true, alias.clone())
                }
                SelectItemKind::OtherAggregate { expr, alias, .. } => (
                    expr.as_function_call().and_then(|f| f.name()),
                    false,
                    alias.clone(),
                ),
                SelectItemKind::GroupByKey { .. } => continue,
            };
            let Some(name) = name else { continue };
            let Some(func) = smelt_types::SqlFunction::from_name(&name.to_uppercase()) else {
                continue;
            };
            if alias.is_empty() {
                continue;
            }
            out.push(ColumnDiscriminant {
                output: alias,
                discriminants: combiner_discriminants(func, distinct),
            });
        }
        out
    }

    /// The constant-literal output columns of one SELECT scope, name → text.
    fn scope_literals(&self, sn: &SelectNode) -> Vec<(String, String)> {
        let Some(select_list) = sn.select.select_list() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in select_list.items() {
            if item.is_wildcard() {
                continue;
            }
            let Some(expr) = item.expression() else {
                continue;
            };
            if !is_constant_literal(&expr) {
                continue;
            }
            let output = item
                .column_name()
                .unwrap_or_else(|| expr.text().trim().to_string());
            out.push((output, expr.text().trim().to_string()));
        }
        out
    }

    /// Whether any join in this scope's FROM clause proves `OneToMany`.
    fn scope_has_fan_out(&self, sn: &SelectNode) -> bool {
        let Some(from) = sn.select.from_clause() else {
            return false;
        };
        let joins: Vec<_> = from.joins().collect();
        joins
            .iter()
            .any(|join| fan_out(join, self.ctx) == Cardinality::OneToMany)
    }
}

impl Transfer for PropertyTransfer<'_> {
    type Verdict = PropertyVector;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> PropertyVector {
        // A base relation contributes no projections of its own; grain is
        // established by the consuming scope's own operators (fail-closed:
        // no key is assumed for a bare table).
        PropertyVector::default()
    }

    fn operator(
        &self,
        op: &OpNode<'_>,
        children: &[PropertyVector],
        cx: &NodeCx,
    ) -> PropertyVector {
        match op {
            OpNode::Unsupported { .. } => PropertyVector::default(),
            OpNode::Select(sn) => {
                let columns: Vec<String> = cx.columns.iter().map(|c| c.output.clone()).collect();

                // Input verdicts keyed like the walk's alias map, for
                // determinism reduction through CTE / derived-table inputs.
                let mut input_props: BTreeMap<String, &PropertyVector> = BTreeMap::new();
                let mut input_barrier = false;
                for (input, child) in sn.inputs.iter().zip(&children[sn.ctes.len()..]) {
                    input_barrier |= child.has_set_op_barrier;
                    match input {
                        InputItem::CteRef { name, alias } => {
                            let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                            input_props.insert(key, child);
                        }
                        InputItem::Derived {
                            alias: Some(alias), ..
                        } => {
                            input_props.insert(alias.to_ascii_lowercase(), child);
                        }
                        _ => {}
                    }
                }

                // Expr-scope children, zipped to their own `ExprScope`
                // definitions for range-containment matching — a select item
                // takes no key/output/fan-out from a scope it embeds, but
                // does take its set-op barrier
                // (`model_properties.md` §"The composition walk").
                let expr_scope_children: Vec<(&ExprScope, &PropertyVector)> = sn
                    .expr_scopes
                    .iter()
                    .zip(&children[sn.ctes.len() + sn.inputs.len()..])
                    .collect();
                for (_, child) in &expr_scope_children {
                    input_barrier |= child.has_set_op_barrier;
                }

                let has_fan_out_join = self.scope_has_fan_out(sn);
                let literal_columns = self.scope_literals(sn);

                // Grain: DISTINCT ⇒ whole projected row; GROUP BY ⇒ factory
                // key on the grouping columns; otherwise fail-closed unkeyed
                // (a plain scan or an unproven join establishes no key).
                let grain = if sn.select.is_distinct() && !columns.is_empty() {
                    Grain {
                        keys: vec![columns.clone()],
                    }
                } else if let Some(key) = self.group_by_output_keys(sn) {
                    Grain { keys: vec![key] }
                } else {
                    Grain::unkeyed()
                };

                let determinism =
                    self.scope_determinism(sn, cx, &input_props, &expr_scope_children);
                let comparability =
                    self.scope_comparability(sn, cx, &input_props, &expr_scope_children);
                let discriminants = self.scope_discriminants(sn);

                let mut vector = PropertyVector {
                    columns,
                    grain,
                    fds: Vec::new(),
                    determinism,
                    comparability,
                    discriminants,
                    literal_columns,
                    has_set_op_barrier: input_barrier,
                    has_fan_out_join,
                };
                vector.fds = vector.fds_from_facts();
                vector
            }
            OpNode::SetOp(so) => {
                let branches = &children[so.ctes.len()..];
                let is_union_all = so.ops.iter().all(|o| *o == SetOpKind::UnionAll);

                // Output columns and names come from the first arm.
                let columns = branches
                    .first()
                    .map(|b| b.columns.clone())
                    .unwrap_or_default();

                // Determinism: per output position, the lub across arms.
                let determinism = union_determinism(branches);

                // Comparability: per output position, the lub across arms —
                // a column comparable in one arm and incomparable in another
                // folds Incomparable.
                let comparability = union_comparability(branches);

                // Grain survival: only the discriminated-union case keeps a
                // key. A literal-discriminator column (a distinct constant per
                // arm) added to a key shared by every arm makes the arms
                // provably key-disjoint (§3.8 survival case 1).
                let grain = if is_union_all {
                    union_discriminated_grain(branches)
                } else {
                    Grain::unkeyed()
                };

                let mut vector = PropertyVector {
                    columns,
                    grain,
                    fds: Vec::new(),
                    determinism,
                    comparability,
                    discriminants: Vec::new(),
                    literal_columns: Vec::new(),
                    // A set operation is always an FD barrier for any key it
                    // does not itself preserve as grain.
                    has_set_op_barrier: true,
                    has_fan_out_join: branches.iter().any(|b| b.has_fan_out_join),
                };
                vector.fds = vector.fds_from_facts();
                vector
            }
        }
    }
}

/// Resolve `qualifier` (or the sole input when unqualified) to its lowercased
/// alias key in `cx.aliases`. `pub(crate)`: reused by [`crate::analysis::output_delta`]
/// to resolve a select-item's embedded column references the same way
/// [`PropertyTransfer`]'s own determinism/comparability reduction does.
pub(crate) fn resolve_alias_source(cx: &NodeCx, qualifier: Option<&str>) -> Option<String> {
    match qualifier {
        Some(q) => {
            let key = q.to_ascii_lowercase();
            cx.aliases.contains_key(&key).then_some(key)
        }
        None if cx.aliases.len() == 1 => cx.aliases.keys().next().cloned(),
        None => None,
    }
}

/// The per-position determinism lub across set-operation arms — `clean ∪ clean
/// = clean`, escalating to the strongest arm otherwise.
fn union_determinism(branches: &[PropertyVector]) -> Vec<ColumnDeterminism> {
    let Some(first) = branches.first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(first.determinism.len());
    for (i, col) in first.determinism.iter().enumerate() {
        let mut level = col.level;
        for other in &branches[1..] {
            if let Some(o) = other.determinism.get(i) {
                level = level.max(o.level);
            }
        }
        out.push(ColumnDeterminism {
            output: col.output.clone(),
            level,
        });
    }
    out
}

/// The per-position change-comparability lub across set-operation arms — a
/// column `Comparable` in every arm stays `Comparable`; any arm that folds
/// `Incomparable` dominates (same shape as [`union_determinism`]).
fn union_comparability(branches: &[PropertyVector]) -> Vec<ColumnComparability> {
    let Some(first) = branches.first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(first.comparability.len());
    for (i, col) in first.comparability.iter().enumerate() {
        let mut comparability = col.comparability;
        for other in &branches[1..] {
            if let Some(o) = other.comparability.get(i) {
                comparability = comparability.max(o.comparability);
            }
        }
        out.push(ColumnComparability {
            output: col.output.clone(),
            comparability,
        });
    }
    out
}

/// The grain surviving a `UNION ALL`: unkeyed unless a literal discriminator
/// (a distinct constant column per arm, by position) exists and every arm
/// shares a key — then the discriminator column joins that key
/// (`20260707-property-per-key-constancy.md` §3.8 survival case 1).
fn union_discriminated_grain(branches: &[PropertyVector]) -> Grain {
    if branches.len() < 2 {
        return branches
            .first()
            .map(|b| b.grain.clone())
            .unwrap_or_default();
    }
    // Find a discriminator: an output-column position that is a distinct
    // constant literal in every arm.
    let width = branches[0].columns.len();
    let mut discriminator: Option<String> = None;
    for pos in 0..width {
        let mut lits: Vec<String> = Vec::with_capacity(branches.len());
        let mut all_literal = true;
        for b in branches {
            let Some(name) = b.columns.get(pos) else {
                all_literal = false;
                break;
            };
            match b
                .literal_columns
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                Some((_, lit)) => lits.push(lit.clone()),
                None => {
                    all_literal = false;
                    break;
                }
            }
        }
        if !all_literal {
            continue;
        }
        // Pairwise-distinct literal values ⇒ the arms cannot collide on it.
        let distinct: std::collections::BTreeSet<&String> = lits.iter().collect();
        if distinct.len() == branches.len() {
            discriminator = branches[0].columns.get(pos).cloned();
            break;
        }
    }
    let Some(disc) = discriminator else {
        return Grain::unkeyed();
    };

    // The keys shared (as name-sets) by every arm.
    let shared: Vec<KeySet> = branches[0]
        .grain
        .keys
        .iter()
        .filter(|k| {
            let ks: std::collections::BTreeSet<String> =
                k.iter().map(|c| c.to_ascii_lowercase()).collect();
            branches[1..].iter().all(|b| {
                b.grain.keys.iter().any(|k2| {
                    let k2s: std::collections::BTreeSet<String> =
                        k2.iter().map(|c| c.to_ascii_lowercase()).collect();
                    k2s == ks
                })
            })
        })
        .cloned()
        .collect();

    let keys = shared
        .into_iter()
        .map(|mut k| {
            if !k.iter().any(|c| c.eq_ignore_ascii_case(&disc)) {
                k.push(disc.clone());
            }
            k.sort();
            k
        })
        .collect::<Vec<_>>();
    Grain { keys }
}

/// Function-call names in `expr`'s own text, not descending into a nested
/// `SUBQUERY` node — that subtree is a walk node of its own
/// (`SelectNode::expr_scopes`), and descending into it would be exactly the
/// cross-node ad-hoc scan `model_properties.md` §"The composition walk"
/// forbids. Shared by [`expr_determinism`] and [`expr_comparability`], whose
/// per-column verdicts are folded with the matching `ExprScope` child's own
/// verdict by [`PropertyTransfer::scope_determinism`]/`scope_comparability`
/// instead.
fn own_function_call_names(expr: &smelt_parser::Expr) -> Vec<String> {
    fn collect(node: &smelt_parser::syntax_kind::SyntaxNode, out: &mut Vec<String>) {
        if node.kind() == smelt_parser::SyntaxKind::SUBQUERY {
            return;
        }
        if node.kind() == smelt_parser::SyntaxKind::FUNCTION_CALL {
            if let Some(func) = smelt_parser::FunctionCall::cast(node.clone()) {
                if let Some(name) = func.name() {
                    out.push(name);
                }
            }
        }
        for child in node.children() {
            collect(&child, out);
        }
    }
    let mut out = Vec::new();
    collect(expr.syntax(), &mut out);
    out
}

/// The determinism of an expression from the nondeterminism predicate over
/// every function call it contains, excluding any nested expression-position
/// subquery — the fail-closed leaf classifier
/// (`monotonicity::classify_function_determinism`).
fn expr_determinism(expr: &smelt_parser::Expr) -> Determinism {
    let mut level = Determinism::Clean;
    for name in own_function_call_names(expr) {
        let contrib = match classify_function_determinism(&name) {
            FunctionDeterminism::RowNondeterministic => Determinism::Row,
            FunctionDeterminism::RunDeterministic => Determinism::Run,
            FunctionDeterminism::Neither => Determinism::Clean,
        };
        level = level.max(contrib);
    }
    level
}

/// The change-comparability of an expression (`model_properties.md`
/// §"Change comparability") — fail-closed leaf classifier over every
/// function call it contains, excluding any nested expression-position
/// subquery. A known run-/row-nondeterministic function
/// (`NOW`/`RANDOM`/…) is `Incomparable` (comparable only *within* one run,
/// per the determinism predicate). A recognised function outside that set
/// (registry-backed, `smelt_types::SqlFunction`) is treated as a pure
/// function of its arguments and does not itself taint the result.
/// A function call the registry does not recognise (a UDF, an opaque body)
/// is the fail-closed case: `Incomparable`, never a default `Comparable`,
/// since smelt cannot prove it is a pure function of processed inputs. A
/// bare column reference or literal (no function calls) is `Comparable`.
fn expr_comparability(expr: &smelt_parser::Expr) -> Comparability {
    let mut result = Comparability::Comparable;
    for name in own_function_call_names(expr) {
        let contrib = match classify_function_determinism(&name) {
            FunctionDeterminism::RowNondeterministic | FunctionDeterminism::RunDeterministic => {
                Comparability::Incomparable
            }
            FunctionDeterminism::Neither => {
                if smelt_types::SqlFunction::from_name(&name.to_ascii_uppercase()).is_some() {
                    Comparability::Comparable
                } else {
                    Comparability::Incomparable
                }
            }
        };
        result = result.max(contrib);
    }
    result
}

/// Whether `expr` is a constant literal — a string/number literal (bare, or
/// a typed literal such as `DATE '2026-01-01'`/`INTERVAL '1' DAY`) with no
/// column reference or function call. The shared constant-literal
/// recognizer (`docs/specs/architecture.md` §"Property composition walk
/// rule") — a discriminator/tag candidate for [`union_discriminated_grain`]
/// and, via [`constant_literal_tag`], for backbuild's F2 branch-removal
/// discriminator proof.
pub(crate) fn is_constant_literal(expr: &smelt_parser::Expr) -> bool {
    if smelt_parser::ColumnRef::from_expr(expr).is_some() {
        return false;
    }
    use smelt_parser::SyntaxKind::{FUNCTION_CALL, IDENT, NUMBER, STRING};
    let mut saw_literal = false;
    for element in expr.syntax().descendants_with_tokens() {
        if let Some(n) = element.as_node() {
            if n.kind() == FUNCTION_CALL {
                return false;
            }
        } else if let Some(t) = element.as_token() {
            match t.kind() {
                STRING | NUMBER => saw_literal = true,
                // A bare identifier that is not a type keyword (DATE '…') is a
                // column reference, not a constant.
                IDENT => {
                    let up = t.text().to_ascii_uppercase();
                    if !matches!(up.as_str(), "DATE" | "TIME" | "TIMESTAMP" | "INTERVAL") {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }
    saw_literal
}

/// `expr`'s discriminator identity — `(kind, raw source text)` — if it is
/// [`is_constant_literal`], else `None`. `kind` is the literal's coercion
/// family: `"NUMBER"`/`"STRING"` for a bare literal, or the uppercase type
/// keyword (`"DATE"`, `"TIME"`, `"TIMESTAMP"`, `"INTERVAL"`) for a typed
/// one. Two literals of different kinds are not safely comparable by value
/// after a `UNION ALL`'s column-type coercion, even when their raw text
/// differs — callers building a coercion-safe discriminator (backbuild's F2
/// branch-removal proof) compare `kind` before comparing `text`. The
/// per-branch counterpart of [`union_discriminated_grain`]: that function
/// works over already-walked `PropertyVector`s; this one works directly
/// over one branch's own already-bounded SELECT-item expression, the shape
/// backbuild's per-`SelectStmt` diffing needs without building a full
/// [`QueryTree`].
pub(crate) fn constant_literal_tag(expr: &smelt_parser::Expr) -> Option<(String, String)> {
    if !is_constant_literal(expr) {
        return None;
    }
    let kind = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .map(|t| match t.kind() {
            smelt_parser::SyntaxKind::NUMBER => "NUMBER".to_string(),
            smelt_parser::SyntaxKind::STRING => "STRING".to_string(),
            _ => t.text().to_ascii_uppercase(),
        })
        .unwrap_or_default();
    Some((kind, expr.text().trim().to_string()))
}

/// The whole-model property vector — the single walk-derived derivation of
/// grain, functional dependencies, discriminants, and determinism
/// (`model_properties.md` §"The composition walk"). `ctx` supplies the
/// declared unique keys the fan-out proof reads. `None` when the text has no
/// SELECT statement at all.
pub fn model_property_vector(sql: &str, ctx: &JoinContext) -> Option<PropertyVector> {
    let tree = QueryTree::from_sql(sql)?;
    Some(walk(&tree, &PropertyTransfer { ctx }))
}

use crate::analysis::succession::{
    classify_keyed_succession, SuccessionContext, SuccessionVerdict,
};

/// The sole call site of [`classify_keyed_succession`]: apply the
/// keyed-succession leaf classifier to `tree`'s top scope only. A set
/// operation or unrecognised construct at the outermost query refuses
/// outright — a succession-shaped projection nested inside a CTE or
/// `UNION` arm is future work (`docs/specs/incremental_shapes.md`
/// §Future Extensions), never silently missed.
pub fn model_keyed_succession(tree: &QueryTree, ctx: &SuccessionContext) -> SuccessionVerdict {
    use crate::analysis::succession::NotSuccessionReason;
    match &tree.root {
        QueryNode::Select(node) => classify_keyed_succession(node, ctx),
        QueryNode::SetOp(_) => SuccessionVerdict::NotSuccession {
            reason: NotSuccessionReason::SingleSourceOnly(
                "outermost query is a set operation, not a single SELECT scope".into(),
            ),
        },
        QueryNode::Unsupported { reason } => SuccessionVerdict::NotSuccession {
            reason: NotSuccessionReason::SingleSourceOnly(format!(
                "unrecognised outermost query construct: {reason}"
            )),
        },
    }
}
