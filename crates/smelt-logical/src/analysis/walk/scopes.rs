// ===== Scope enumeration: the first Transfer =====

use smelt_parser::SelectStmt;

use crate::analysis::monotonicity::{classify_function_determinism, FunctionDeterminism};
use crate::analysis::{item_expr, resolve_scope_group_by, select_stmt_items};

use super::tree::*;

/// The kind of a composition-relevant scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    GroupBy,
    Distinct,
    Over,
    Having,
    SetOp,
    Limit,
}

/// One scope found by the walk: its kind, its key expressions (meaning
/// depends on the kind — GROUP BY keys, window PARTITION BY keys, set-op
/// operator names, …), and the nesting path of the node it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub keys: Vec<String>,
    pub path: Vec<PathSeg>,
}

/// An unrecognisable construct encountered by the walk — surfaced so
/// fail-closed consumers reject instead of silently under-enumerating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedConstruct {
    pub reason: String,
    pub path: Vec<PathSeg>,
}

/// The exhaustive scope enumeration of a model — every GROUP BY, DISTINCT,
/// OVER, HAVING, set-op, and LIMIT scope in the whole tree, including
/// CTE-internal ones, each visited exactly once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeEnumeration {
    pub scopes: Vec<Scope>,
    pub unsupported: Vec<UnsupportedConstruct>,
}

impl ScopeEnumeration {
    fn merge(&mut self, other: &ScopeEnumeration) {
        self.scopes.extend(other.scopes.iter().cloned());
        self.unsupported.extend(other.unsupported.iter().cloned());
    }
}

/// The scope-enumeration transfer function: the union of the children's
/// enumerations plus the node's own scopes. CTE-reference children are
/// skipped when merging — a CTE body's scopes are counted once at its
/// definition site, however many times it is referenced.
pub struct ScopeEnum;

impl Transfer for ScopeEnum {
    type Verdict = ScopeEnumeration;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> ScopeEnumeration {
        ScopeEnumeration::default()
    }

    fn operator(
        &self,
        op: &OpNode<'_>,
        children: &[ScopeEnumeration],
        cx: &NodeCx,
    ) -> ScopeEnumeration {
        let mut out = ScopeEnumeration::default();
        match op {
            OpNode::Unsupported { reason } => {
                out.unsupported.push(UnsupportedConstruct {
                    reason: reason.to_string(),
                    path: cx.path.clone(),
                });
            }
            OpNode::Select(sn) => {
                let n_ctes = sn.ctes.len();
                for child in &children[..n_ctes] {
                    out.merge(child);
                }
                for (input, child) in sn.inputs.iter().zip(&children[n_ctes..]) {
                    if !matches!(input, InputItem::CteRef { .. }) {
                        out.merge(child);
                    }
                }
                out.scopes.extend(select_own_scopes(&sn.select, &cx.path));
            }
            OpNode::SetOp(so) => {
                let n_ctes = so.ctes.len();
                for child in children {
                    out.merge(child);
                }
                debug_assert_eq!(children.len(), n_ctes + so.branches.len());
                out.scopes.push(Scope {
                    kind: ScopeKind::SetOp,
                    keys: so.ops.iter().map(|op| op.as_str().to_string()).collect(),
                    path: cx.path.clone(),
                });
            }
        }
        out
    }
}

/// The scopes declared by one SELECT statement's own clauses (its direct
/// children only — nested scopes belong to their own nodes).
fn select_own_scopes(select: &SelectStmt, path: &[PathSeg]) -> Vec<Scope> {
    let mut scopes = Vec::new();
    let items = select_stmt_items(select).unwrap_or_default();

    let group_by_keys = resolve_scope_group_by(select, &items);
    if !group_by_keys.is_empty() {
        scopes.push(Scope {
            kind: ScopeKind::GroupBy,
            keys: group_by_keys,
            path: path.to_vec(),
        });
    }

    if select.is_distinct() {
        scopes.push(Scope {
            kind: ScopeKind::Distinct,
            keys: items
                .iter()
                .map(|item| item_expr(item).text().trim().to_string())
                .collect(),
            path: path.to_vec(),
        });
    }

    for item in &items {
        if let Some(window) = item_expr(item).window_spec() {
            let keys = window
                .partition_by()
                .map(|pb| {
                    pb.expressions()
                        .map(|e| e.text().trim().to_string())
                        .collect()
                })
                .unwrap_or_default();
            scopes.push(Scope {
                kind: ScopeKind::Over,
                keys,
                path: path.to_vec(),
            });
        }
    }

    if let Some(having) = select.having_clause() {
        scopes.push(Scope {
            kind: ScopeKind::Having,
            keys: having
                .expression()
                .map(|e| vec![e.text().trim().to_string()])
                .unwrap_or_default(),
            path: path.to_vec(),
        });
    }

    if select.limit_clause().is_some() {
        scopes.push(Scope {
            kind: ScopeKind::Limit,
            keys: Vec::new(),
            path: path.to_vec(),
        });
    }

    scopes
}

/// Convenience entry: the exhaustive scope enumeration of a model's SQL.
pub fn enumerate_scopes(sql: &str) -> Option<ScopeEnumeration> {
    let tree = QueryTree::from_sql(sql)?;
    Some(walk(&tree, &ScopeEnum))
}

// ===== Partition-grain admission: alignment judged per walk-enumerated scope =====

/// Which batched admission gate a violation falls under. Each gate maps to
/// one `safety_overrides.allow_*` escape; `Unsupported` has no escape short
/// of disabling every gate (an unenumerable construct defeats them all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionGate {
    /// A window `OVER` whose `PARTITION BY` omits the partition column and
    /// whose frame is not a bounded `RANGE BETWEEN INTERVAL` (Form A).
    WindowOver,
    /// A `HAVING` whose owning scope's `GROUP BY` omits the partition column
    /// (HAVING inherits its GROUP BY scope's alignment verdict).
    Having,
    /// A `SELECT DISTINCT` scope that does not project the partition column.
    Distinct,
    /// A `LIMIT` clause (global top-k — never partition-local).
    Limit,
    /// A construct the walk could not normalize; the gates cannot prove the
    /// absence of an inadmissible scope inside it, so it refuses fail-closed.
    Unsupported,
}

/// One admission violation found by the walk: the gate it trips, a
/// human-readable detail (the alignment reason or construct description),
/// and the nesting path of the scope it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionViolation {
    pub gate: AdmissionGate,
    pub detail: String,
    pub path: Vec<PathSeg>,
}

impl AdmissionViolation {
    /// Render the scope's nesting path for a diagnostic message; empty for
    /// the top scope.
    pub fn path_display(&self) -> String {
        if self.path.is_empty() {
            return String::new();
        }
        let segs: Vec<String> = self
            .path
            .iter()
            .map(|seg| match seg {
                PathSeg::Cte(name) => format!("CTE '{name}'"),
                PathSeg::SetOpBranch(i) => format!("set-operation branch {}", i + 1),
                PathSeg::DerivedTable(alias) if alias.is_empty() => {
                    "an unaliased derived table".to_string()
                }
                PathSeg::DerivedTable(alias) => format!("derived table '{alias}'"),
                PathSeg::ExprScope { kind, index } => {
                    let kind = match kind {
                        ExprScopeKind::Scalar => "scalar",
                        ExprScopeKind::Exists => "EXISTS",
                        ExprScopeKind::In => "IN",
                        ExprScopeKind::Quantified => "quantified",
                    };
                    format!("{kind} subquery {}", index + 1)
                }
            })
            .collect();
        format!(" (in {})", segs.join(" → "))
    }
}

/// The batched-admission transfer function: every scope the walk enumerates —
/// CTE bodies, derived tables, and set-operation arms included — is judged by
/// the same AST-pure per-scope alignment classifiers the top scope is judged
/// by (`scope_group_by_alignment` / `scope_distinct_alignment` /
/// `window_over_alignment`), and an unnormalizable construct yields an
/// `Unsupported` violation (fail-closed by construction,
/// `model_properties.md` §"The composition walk").
struct PartitionGrainAdmission<'a> {
    partition_col: &'a str,
}

impl Transfer for PartitionGrainAdmission<'_> {
    type Verdict = Vec<AdmissionViolation>;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Self::Verdict {
        Vec::new()
    }

    fn operator(&self, op: &OpNode<'_>, children: &[Self::Verdict], cx: &NodeCx) -> Self::Verdict {
        let mut out = Vec::new();
        match op {
            OpNode::Unsupported { reason } => {
                out.push(AdmissionViolation {
                    gate: AdmissionGate::Unsupported,
                    detail: reason.to_string(),
                    path: cx.path.clone(),
                });
            }
            OpNode::Select(sn) => {
                let n_ctes = sn.ctes.len();
                for child in &children[..n_ctes] {
                    out.extend(child.iter().cloned());
                }
                for (input, child) in sn.inputs.iter().zip(&children[n_ctes..]) {
                    // A CTE body's violations are counted once at its
                    // definition site, however many times it is referenced.
                    if !matches!(input, InputItem::CteRef { .. }) {
                        out.extend(child.iter().cloned());
                    }
                }
                out.extend(select_own_admission_violations(
                    &sn.select,
                    self.partition_col,
                    &cx.path,
                ));
            }
            OpNode::SetOp(_) => {
                for child in children {
                    out.extend(child.iter().cloned());
                }
            }
        }
        out
    }
}

/// The admission violations declared by one SELECT scope's own clauses
/// (nested walk nodes — CTE bodies, derived tables, set-op arms — belong to
/// their own nodes and are pruned from this scope's region).
fn select_own_admission_violations(
    select: &SelectStmt,
    partition_col: &str,
    path: &[PathSeg],
) -> Vec<AdmissionViolation> {
    use crate::analysis::{
        scope_distinct_alignment, scope_group_by_alignment,
        window_has_bounded_range_interval_frame, window_over_alignment, PartitionAlignment,
    };

    let mut out = Vec::new();

    if select.having_clause().is_some() {
        if let PartitionAlignment::NotAligned { reason } =
            scope_group_by_alignment(select, partition_col)
        {
            out.push(AdmissionViolation {
                gate: AdmissionGate::Having,
                detail: reason,
                path: path.to_vec(),
            });
        }
    }

    if select.is_distinct() {
        if let PartitionAlignment::NotAligned { reason } =
            scope_distinct_alignment(select, partition_col)
        {
            out.push(AdmissionViolation {
                gate: AdmissionGate::Distinct,
                detail: reason,
                path: path.to_vec(),
            });
        }
    }

    let (windows, has_limit, subqueries) = collect_scope_region(select);

    // Expression-position subqueries are not walk nodes; their own
    // DISTINCT/HAVING are judged here, in the owning scope, with the same
    // per-scope leaf classifiers — a `SELECT DISTINCT` / `HAVING` one nesting
    // level down (a scalar subquery, `EXISTS (…)`) is the same cross-partition
    // hazard and must trip the same gate. The violation is attributed to the
    // owning scope's `path` (the subquery has no walk-node path of its own).
    for sub in &subqueries {
        if sub.having_clause().is_some() {
            if let PartitionAlignment::NotAligned { reason } =
                scope_group_by_alignment(sub, partition_col)
            {
                out.push(AdmissionViolation {
                    gate: AdmissionGate::Having,
                    detail: reason,
                    path: path.to_vec(),
                });
            }
        }
        if sub.is_distinct() {
            if let PartitionAlignment::NotAligned { reason } =
                scope_distinct_alignment(sub, partition_col)
            {
                out.push(AdmissionViolation {
                    gate: AdmissionGate::Distinct,
                    detail: reason,
                    path: path.to_vec(),
                });
            }
        }
    }

    for window in &windows {
        // A bounded RANGE BETWEEN INTERVAL frame (Form A) is a reach
        // obligation the bound deriver picks up, not an alignment one.
        if window_has_bounded_range_interval_frame(window) {
            continue;
        }
        if let PartitionAlignment::NotAligned { reason } =
            window_over_alignment(window, partition_col)
        {
            out.push(AdmissionViolation {
                gate: AdmissionGate::WindowOver,
                detail: reason,
                path: path.to_vec(),
            });
        }
    }

    if has_limit {
        out.push(AdmissionViolation {
            gate: AdmissionGate::Limit,
            detail: "LIMIT clause".to_string(),
            path: path.to_vec(),
        });
    }

    out
}

/// The window specs, LIMIT presence, and expression-position subquery scopes
/// of one SELECT scope's own region: every descendant of the scope's node
/// except the subtrees that are walk nodes of their own — its WITH clause (CTE
/// bodies), its FROM clause (derived tables), and a direct-child SELECT (the
/// next set-operation arm). Expression-position subqueries (`EXISTS (…)`, a
/// scalar subquery) are NOT walk nodes, so their windows, LIMITs, **and**
/// DISTINCT/HAVING scopes are judged here, in the owning scope — fail-closed
/// coverage for scopes the tree normalization does not model. The returned
/// `SelectStmt`s are every such expression-position `SELECT_STMT` in the
/// region; the caller judges each one's DISTINCT/HAVING alignment with the
/// same per-scope leaf classifiers the owning scope is judged by.
fn collect_scope_region(
    select: &SelectStmt,
) -> (Vec<smelt_parser::WindowSpec>, bool, Vec<SelectStmt>) {
    use smelt_parser::SyntaxKind::{FROM_CLAUSE, SELECT_STMT, WITH_CLAUSE};

    fn collect_rec(
        node: &smelt_parser::syntax_kind::SyntaxNode,
        windows: &mut Vec<smelt_parser::WindowSpec>,
        has_limit: &mut bool,
        subqueries: &mut Vec<SelectStmt>,
    ) {
        use smelt_parser::SyntaxKind::{LIMIT_CLAUSE, WINDOW_SPEC};
        match node.kind() {
            WINDOW_SPEC => {
                if let Some(window) = smelt_parser::WindowSpec::cast(node.clone()) {
                    windows.push(window);
                }
            }
            LIMIT_CLAUSE => {
                *has_limit = true;
            }
            SELECT_STMT => {
                // An expression-position subquery reached inside the region (a
                // scalar subquery or `EXISTS (…)`) — not a walk node, so its
                // own DISTINCT/HAVING must be judged in this owning scope.
                if let Some(sub) = SelectStmt::cast(node.clone()) {
                    subqueries.push(sub);
                }
            }
            _ => {}
        }
        for child in node.children() {
            collect_rec(&child, windows, has_limit, subqueries);
        }
    }

    let mut windows = Vec::new();
    let mut has_limit = false;
    let mut subqueries = Vec::new();
    for child in select.syntax().children() {
        if matches!(child.kind(), WITH_CLAUSE | FROM_CLAUSE | SELECT_STMT) {
            continue;
        }
        collect_rec(&child, &mut windows, &mut has_limit, &mut subqueries);
    }
    (windows, has_limit, subqueries)
}

/// Visit every element (token or node) of one SELECT scope's own region:
/// everything under the scope's node except the subtrees that are walk nodes
/// of their own — its WITH clause (CTE bodies), a direct-child SELECT (the
/// next set-operation arm), and every `SUBQUERY` (a FROM-position derived
/// table's own, nested in `TABLE_REF`, or an expression-position scope's,
/// `SelectNode::expr_scopes`) — each already folded through its own child
/// verdict elsewhere in the walk, so visiting it here would double-count.
/// Join `ON` conditions live in the FROM clause and are visited (only the
/// derived-table `SUBQUERY` bodies under a `TABLE_REF` are pruned). Shared
/// by [`own_region_text`] (collects token text) and every per-scope leaf
/// classifier that instead needs to inspect the region's own *nodes*
/// ([`scope_has_window_function`], [`scope_nondeterministic_fn`]).
fn visit_own_region_elements(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    root: &smelt_parser::syntax_kind::SyntaxNode,
    visit: &mut dyn FnMut(&smelt_parser::syntax_kind::SyntaxElement),
) {
    use smelt_parser::SyntaxKind::{SELECT_STMT, SUBQUERY, WITH_CLAUSE};

    for element in node.children_with_tokens() {
        if element.as_token().is_some() {
            visit(&element);
        } else if let Some(child) = element.as_node() {
            match child.kind() {
                WITH_CLAUSE => {}
                SELECT_STMT if node == root => {}
                SUBQUERY => {}
                _ => {
                    visit(&element);
                    visit_own_region_elements(child, root, visit);
                }
            }
        }
    }
}

/// The raw text of one SELECT scope's own region ([`visit_own_region_elements`]).
pub(crate) fn own_region_text(select: &SelectStmt) -> String {
    let root = select.syntax();
    let mut out = String::new();
    visit_own_region_elements(root, root, &mut |element| {
        if let Some(token) = element.as_token() {
            out.push_str(token.text());
        }
    });
    out
}

/// Visit every *node* (not token) of one SELECT scope's own region
/// ([`visit_own_region_elements`]) — the entry point a per-scope leaf
/// classifier uses to inspect the region's own parsed shape (a `WINDOW_SPEC`,
/// a `FUNCTION_CALL`, …) without re-parsing text and without descending into
/// a child that is itself a walk node.
pub(crate) fn visit_own_region(
    select: &SelectStmt,
    visit: &mut dyn FnMut(&smelt_parser::syntax_kind::SyntaxNode),
) {
    let root = select.syntax();
    visit_own_region_elements(root, root, &mut |element| {
        if let Some(node) = element.as_node() {
            visit(node);
        }
    });
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): whether one SELECT scope's own region ([`visit_own_region`])
/// contains a window function (`OVER (...)`) — the keyed-admission rule
/// `KeyedForbidsWindowFunctions` (`incremental_shapes.md` §"Key-grain
/// codes"). Folded across the whole model by [`first_scope_hit`].
pub(crate) fn scope_has_window_function(select: &SelectStmt) -> bool {
    let mut found = false;
    visit_own_region(select, &mut |node| {
        if node.kind() == smelt_parser::SyntaxKind::WINDOW_SPEC {
            found = true;
        }
    });
    found
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): the first non-deterministic function call (by
/// `monotonicity::classify_function_determinism`) in one SELECT scope's own
/// region ([`visit_own_region`]), matched as a parsed `FUNCTION_CALL` node's
/// name — never a substring — so `SNOW(x)` never matches `NOW` and a listed
/// name inside a string literal never fires. Named for
/// `KeyedForbidsNondeterministic` (`incremental_shapes.md` §"Key-grain
/// codes"). Folded across the whole model by [`first_scope_hit`].
pub(crate) fn scope_nondeterministic_fn(select: &SelectStmt) -> Option<&'static str> {
    let mut hit: Option<&'static str> = None;
    visit_own_region(select, &mut |node| {
        if hit.is_some() || node.kind() != smelt_parser::SyntaxKind::FUNCTION_CALL {
            return;
        }
        let Some(func) = smelt_parser::FunctionCall::cast(node.clone()) else {
            return;
        };
        let Some(name) = func.name() else {
            return;
        };
        if !matches!(
            classify_function_determinism(&name),
            FunctionDeterminism::Neither
        ) {
            hit = crate::analysis::monotonicity::NONDETERMINISTIC_FUNCTIONS
                .iter()
                .find(|nd| nd.eq_ignore_ascii_case(&name))
                .copied();
        }
    });
    hit
}

/// A `Transfer` folding a per-scope `Option<T>` classifier as parallel OR
/// (first `Some` wins) over the whole children slice (`ctes ++ inputs ++
/// expr_scopes`) — the composition shape `model_properties.md` §"The
/// composition walk" gives the keyed-admission presence verdicts: an `OVER`
/// or a non-deterministic call anywhere in the model's scope tree refuses,
/// regardless of which scope it sits in. Each node's own contribution comes
/// from `classify` invoked over that node's own region only ([`ExprScope`]
/// children participate exactly like any other child — there is no
/// join-sibling carve-out to protect here, since this fold has no sibling
/// slack computation, only presence).
struct ScopePresenceTransfer<'a, T> {
    classify: &'a dyn Fn(&SelectStmt) -> Option<T>,
}

impl<T: Clone> Transfer for ScopePresenceTransfer<'_, T> {
    type Verdict = Option<T>;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Option<T> {
        None
    }

    fn operator(&self, op: &OpNode<'_>, children: &[Option<T>], _cx: &NodeCx) -> Option<T> {
        match op {
            OpNode::Unsupported { .. } | OpNode::SetOp(_) => {
                children.iter().find_map(|c| c.clone())
            }
            OpNode::Select(sn) => children
                .iter()
                .find_map(|c| c.clone())
                .or_else(|| (self.classify)(&sn.select)),
        }
    }
}

/// The shared scope-presence entry point: `classify` judges one SELECT
/// scope's own region ([`visit_own_region`]) and the verdict is folded as
/// parallel OR (first `Some`) over every scope of the model
/// ([`ScopePresenceTransfer`]). For a tree the walk cannot normalize (no
/// SELECT statement at all, or an `Unsupported` subtree), falls back to
/// classifying every `SelectStmt` scope of the parsed CST directly — the
/// same fallback shape [`model_partition_skew`] and
/// `footprint::model_has_trajectory_column` use, so coverage never degrades
/// below the flat enumeration.
pub(crate) fn first_scope_hit<T: Clone>(
    sql: &str,
    classify: &dyn Fn(&SelectStmt) -> Option<T>,
) -> Option<T> {
    match QueryTree::from_sql(sql) {
        Some(tree) if !tree.root.has_unsupported() => {
            walk(&tree, &ScopePresenceTransfer { classify })
        }
        _ => {
            let stripped = crate::types::Frontmatter::strip(sql);
            let parse = smelt_parser::parse(stripped);
            parse
                .syntax()
                .descendants()
                .filter_map(SelectStmt::cast)
                .find_map(|s| classify(&s))
        }
    }
}

/// Convenience entry: every batched-admission violation in a model's SQL,
/// judged per walk-enumerated scope. `None` when the text has no SELECT
/// statement at all.
pub fn batched_admission_violations(
    sql: &str,
    partition_col: &str,
) -> Option<Vec<AdmissionViolation>> {
    let tree = QueryTree::from_sql(sql)?;
    Some(walk(&tree, &PartitionGrainAdmission { partition_col }))
}

// ===== The partition-skew fold: the model's own output-window skew =====

use crate::analysis::source_bounds::{derive_partition_skew, Skew};

/// The partition-skew transfer function: the model's own partition-column
/// skew bound (`docs/specs/model_transforms.md` §Semantics "The output
/// window is derived, never assumed") folded over the walk. Per SELECT
/// scope, the leaf classifier [`derive_partition_skew`] is invoked over the
/// scope's own region text ([`own_region_text`] — every token except the
/// subtrees that are walk nodes of their own); verdicts compose by
/// [`Skew::union`] (max before, max after), since a Form B relation in any
/// scope can push rows into a neighbouring partition.
///
/// This transfer folds the *whole* children slice (`ctes ++ inputs ++
/// expr_scopes`) by [`Skew::union`] — an expression-position scope composes
/// exactly as any other child does: a Form B relation living inside a
/// scalar/`EXISTS`/`IN`/quantified subquery body can push rows into a
/// neighbouring partition just as one in a `FROM`-position derived table
/// can, so it is not excluded from the fold the way grain/key derivation
/// excludes an expr scope's contribution (`model_properties.md` §"The
/// composition walk"). Unlike bound/reach there is no join-sibling
/// carve-out to make here: this transfer has no sibling-slack computation,
/// and the fold's conservative direction is *more* skew, so widening it can
/// only over-approximate, never under-derive.
///
/// `exclude_source` names the model's own source path (dotted, as it appears
/// in a `smelt.<path>` self-reference) for a self-referential model
/// (`docs/specs/incremental_shapes.md` §"Window independence and self-referential
/// models": the self-edge is never a skew anchor). When set, each scope's
/// region text is filtered by [`own_region_text_excluding_self_relations`]
/// before the leaf classifier runs: the scope's own alias→source map
/// (`NodeCx::aliases`, resolved structurally per scope by the walk) decides
/// which aliases are the self-reference *in that scope*, so a nested
/// subquery or set-operation arm reusing the same short alias for a
/// different source keeps its genuine relations intact.
pub struct SkewTransfer<'a> {
    pub partition_column: &'a str,
    pub exclude_source: Option<&'a str>,
}

impl Transfer for SkewTransfer<'_> {
    type Verdict = Skew;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Skew {
        Skew::ZERO
    }

    fn operator(&self, op: &OpNode<'_>, children: &[Skew], cx: &NodeCx) -> Skew {
        match op {
            OpNode::Select(sn) => {
                // Folds the whole children slice (ctes ++ inputs ++
                // expr_scopes): a Form B relation in any scope — including
                // an expression-position subquery body — can push rows into
                // a neighbouring partition (`model_properties.md` §"The
                // composition walk").
                let acc = children
                    .iter()
                    .fold(Skew::ZERO, |acc, child| acc.union(*child));
                let own = match self.exclude_source {
                    Some(self_name) => {
                        let quals = scope_self_qualifiers(cx, self_name);
                        own_region_text_excluding_self_relations(&sn.select, &quals)
                    }
                    None => own_region_text(&sn.select),
                };
                acc.union(derive_partition_skew(&own, self.partition_column))
            }
            OpNode::SetOp(_) => children
                .iter()
                .fold(Skew::ZERO, |acc, child| acc.union(*child)),
            // An `Unsupported` node carries no readable text and no
            // children; whole-tree coverage is restored by
            // [`model_partition_skew`]'s whole-text fallback (it never
            // walks a tree containing one).
            OpNode::Unsupported { .. } => Skew::ZERO,
        }
    }
}

/// The qualifiers under which this scope's FROM items reference the model's
/// own source (`self_name`): each alias (or bare-name key) whose resolved
/// [`RelationSource::Table`] is the self source, per the walk's own per-scope
/// alias map — never a cross-scope accumulation, so an unrelated scope
/// reusing the same alias text for a different source is unaffected. For an
/// unaliased dotted self-reference the map key is the full dotted path; its
/// last segment is added as well, since an unaliased table's columns are
/// qualified by the bare table name.
fn scope_self_qualifiers(cx: &NodeCx, self_name: &str) -> Vec<String> {
    let mut quals = Vec::new();
    for (key, source) in &cx.aliases {
        let RelationSource::Table(name) = source else {
            continue;
        };
        if !name.eq_ignore_ascii_case(self_name) {
            continue;
        }
        quals.push(key.clone());
        if let Some(last) = key.rsplit('.').next() {
            if last != key {
                quals.push(last.to_string());
            }
        }
    }
    quals
}

/// [`own_region_text`], minus the self-edge's own bounding relations: every
/// top-level `AND`-separated condition — of the scope's `WHERE` clause and of
/// each join's `ON` expression — that references one of `self_quals` (a
/// `qual.`-qualified column of the self source, per this scope's own alias
/// resolution) is omitted from the returned text, so the skew leaf classifier
/// never reads the self-edge's bound as a partition-column skew anchor
/// (`docs/specs/incremental_shapes.md` §"Window independence and self-referential
/// models"). Conditions that do not reference a self qualifier — including a
/// genuine Form B relation sharing the same `WHERE` clause — survive
/// verbatim. A condition containing an `OR` anywhere within it is never
/// omitted, even when it references a self qualifier (the disjunction may
/// mix a self bound with a genuine relation; keeping it can only over-widen
/// the derived output window, never narrow it — [`conjunct_contains_or`]).
fn own_region_text_excluding_self_relations(select: &SelectStmt, self_quals: &[String]) -> String {
    use smelt_parser::syntax_kind::SyntaxNode;
    use smelt_parser::SyntaxKind::{SELECT_STMT, SUBQUERY, TABLE_REF, WITH_CLAUSE};
    use smelt_parser::TextRange;

    if self_quals.is_empty() {
        return own_region_text(select);
    }

    // Collect the ranges of self-referencing conditions in this scope's own
    // WHERE clause and join ON expressions.
    let mut excluded: Vec<TextRange> = Vec::new();
    if let Some(where_clause) = select.where_clause() {
        if let Some(expr) = where_clause.expression() {
            collect_self_conjunct_ranges(&expr, self_quals, &mut excluded);
        }
    }
    if let Some(from_clause) = select.from_clause() {
        for join in from_clause.joins() {
            if let Some(on_expr) = join.condition().and_then(|c| c.on_expression()) {
                collect_self_conjunct_ranges(&on_expr, self_quals, &mut excluded);
            }
        }
    }

    fn collect(node: &SyntaxNode, root: &SyntaxNode, excluded: &[TextRange], out: &mut String) {
        for element in node.children_with_tokens() {
            let range = element.text_range();
            if excluded.iter().any(|ex| ex.contains_range(range)) {
                // A skipped region is replaced by one space so neighbouring
                // tokens never fuse into a new identifier.
                out.push(' ');
                continue;
            }
            if let Some(token) = element.as_token() {
                out.push_str(token.text());
            } else if let Some(child) = element.as_node() {
                match child.kind() {
                    WITH_CLAUSE => {}
                    SELECT_STMT if node == root => {}
                    SUBQUERY if node.kind() == TABLE_REF => {}
                    _ => collect(child, root, excluded, out),
                }
            }
        }
    }

    let root = select.syntax();
    let mut out = String::new();
    collect(root, root, &excluded, &mut out);
    out
}

/// Split `expr` (a `WHERE`/`ON` expression) into its top-level `AND`-joined
/// conjuncts via the shared [`crate::analysis::expr_util::split_top_level_conjuncts`]
/// splitter, then record the range of each conjunct that references one of
/// `self_quals`.
///
/// This function's own output is text *ranges* for region carving
/// (`own_region_text_excluding_self_relations` blanks the excluded ranges
/// out of the scope's own SQL text), not split expressions — genuinely a
/// different shape from the two `Vec<Expr>`-returning splitters unified in
/// `expr_util`, so it consumes the shared splitter internally rather than
/// being folded into its signature.
///
/// A condition containing an `OR` anywhere in its subtree is **never**
/// recorded, even when it references a self qualifier: an `OR` may
/// disjunctively mix the self bound with a genuine relation, and dropping
/// the whole disjunction would silently under-widen the derived output
/// window. Keeping it can only over-widen — the fail-safe direction.
fn collect_self_conjunct_ranges(
    expr: &smelt_parser::Expr,
    self_quals: &[String],
    out: &mut Vec<smelt_parser::TextRange>,
) {
    let mut conjuncts = Vec::new();
    crate::analysis::expr_util::split_top_level_conjuncts(expr, &mut conjuncts);
    for conjunct in &conjuncts {
        let node = conjunct.syntax();
        if !conjunct_contains_or(node) && conjunct_references_qualifier(node, self_quals) {
            out.push(node.text_range());
        }
    }
}

/// Whether the condition's subtree contains an `OR` operator token — the
/// structural guard behind [`collect_self_conjunct_ranges`]'s never-exclude
/// rule for disjunctions. Token-kind based: an `'or'` inside a string
/// literal is one `STRING` token and never matches.
fn conjunct_contains_or(node: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    use smelt_parser::SyntaxKind::OR_KW;
    node.descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == OR_KW)
}

/// Whether one already-isolated condition references any of `self_quals` as
/// a column qualifier — structurally, over the condition's own token stream:
/// an `IDENT` token whose text equals a qualifier (case-insensitive),
/// immediately followed (ignoring trivia) by a `DOT` token, i.e. a real
/// `qual.column` reference. String-literal contents are single `STRING`
/// tokens and can never match, and token identity makes partial-identifier
/// matches (`total_bal.d` for qualifier `bal`) impossible. Invoked by
/// [`collect_self_conjunct_ranges`] over a single conjunct the structural
/// split has already bounded; the qualifiers themselves come from the walk's
/// per-scope alias resolution.
fn conjunct_references_qualifier(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    self_quals: &[String],
) -> bool {
    use smelt_parser::SyntaxKind::{DOT, IDENT};
    let tokens: Vec<_> = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();
    tokens.windows(2).any(|pair| {
        pair[0].kind() == IDENT
            && pair[1].kind() == DOT
            && self_quals
                .iter()
                .any(|qual| pair[0].text().eq_ignore_ascii_case(qual))
    })
}

/// Convenience entry: the model's own partition-column skew bound, composed
/// per walk-enumerated scope by [`SkewTransfer`]. Falls back to the
/// whole-text [`derive_partition_skew`] when the tree normalization cannot
/// model the SQL (no SELECT statement at all, or an `Unsupported` subtree)
/// — under-deriving skew would silently narrow the derived output window,
/// so exact whole-tree coverage wins over fail-closed rejection here (the
/// [`QueryNode::has_unsupported`] consumer pattern).
pub fn model_partition_skew(sql: &str, partition_column: &str) -> Skew {
    model_partition_skew_excluding_self(sql, partition_column, None)
}

/// [`model_partition_skew`] with an optional self-source exclusion for
/// self-referential models: relations arising from a reference to
/// `self_name` (the model's own dotted path) never contribute skew anchors
/// (`docs/specs/incremental_shapes.md` §"Window independence and self-referential
/// models" — the self-edge is never a skew anchor). Exclusion is resolved
/// per scope by the shared walk (see [`SkewTransfer`]), so an unrelated
/// scope reusing the self-edge's alias text for a different source keeps its
/// genuine relations.
///
/// When the tree normalization cannot model the SQL, the whole-text fallback
/// runs **without** the exclusion: it cannot resolve aliases per scope, and
/// omitting the exclusion can only over-widen the derived output window (a
/// correct, wider rebase), never narrow it — the fail-safe direction.
pub fn model_partition_skew_excluding_self(
    sql: &str,
    partition_column: &str,
    self_name: Option<&str>,
) -> Skew {
    match QueryTree::from_sql(sql) {
        Some(tree) if !tree.root.has_unsupported() => walk(
            &tree,
            &SkewTransfer {
                partition_column,
                exclude_source: self_name,
            },
        ),
        _ => derive_partition_skew(sql, partition_column),
    }
}
