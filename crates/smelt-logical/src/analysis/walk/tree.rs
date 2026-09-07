use std::collections::BTreeMap;

use smelt_parser::{ColumnRef, SelectStmt};

/// One segment of a node's nesting path, from the model's top scope down.
/// The top scope itself has an empty path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    /// Inside the body of the named CTE.
    Cte(String),
    /// Inside the i-th arm of a set operation (0-based, source order).
    SetOpBranch(usize),
    /// Inside a derived table (subquery in FROM) with the given alias
    /// (empty string when unaliased).
    DerivedTable(String),
    /// Inside the body of the enclosing scope's i-th expression-position
    /// subquery (0-based, source order across the scope's own select list,
    /// `WHERE`, `HAVING`, `QUALIFY`, then `ORDER BY`).
    ExprScope { kind: ExprScopeKind, index: usize },
}

/// The syntactic position of an [`ExprScope`]'s subquery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprScopeKind {
    /// A bare scalar subquery, e.g. `SELECT (SELECT max(b) FROM u) FROM t`.
    Scalar,
    /// `EXISTS (SELECT …)`.
    Exists,
    /// `expr [NOT] IN (SELECT …)`.
    In,
    /// `expr {op} {ANY|ALL|SOME} (SELECT …)`.
    Quantified,
}

/// The normalized query tree for one model.
#[derive(Debug, Clone)]
pub struct QueryTree {
    pub root: QueryNode,
}

/// A node of the normalized query tree.
#[derive(Debug, Clone)]
pub enum QueryNode {
    /// A single SELECT scope.
    Select(SelectNode),
    /// A set operation (UNION / INTERSECT / EXCEPT) over two or more arms.
    SetOp(SetOpNode),
    /// An unrecognisable relational construct — fail-loud placeholder.
    Unsupported { reason: String },
}

/// A single SELECT scope: its own clauses plus its inputs.
#[derive(Debug, Clone)]
pub struct SelectNode {
    /// The scope's own parsed statement (clause accessors read only this
    /// scope's direct children).
    pub select: SelectStmt,
    /// CTE definitions visible in this scope, in dependency (source) order.
    pub ctes: Vec<CteDef>,
    /// FROM items in source order (comma items and join arms alike);
    /// more than one input means a join.
    pub inputs: Vec<InputItem>,
    /// Expression-position subquery scopes in this scope's own select list,
    /// `WHERE`, `HAVING`, `QUALIFY`, then `ORDER BY`, in that order. Folded
    /// into the walk's children tail *after* `inputs` (`ctes ++ inputs ++
    /// expr_scopes`, see [`OpNode`]) — every production [`Transfer`] impl
    /// (`footprint.rs`, `monotonicity.rs`, `source_bounds.rs`,
    /// `fingerprint.rs`, `affected_keys.rs`, `output_delta.rs`, and this
    /// module's `ScopeEnum`, `PartitionGrainAdmission`, `SkewTransfer`,
    /// `PropertyTransfer`, `Discard`) either consumes the tail or explicitly
    /// slices it off, so no production transfer opts out by silent omission
    /// (`model_properties.md` §"The composition walk"). Bound/reach
    /// (`source_bounds.rs`'s `ReachTransfer`) folds an expr scope as a read;
    /// grain/determinism/comparability (this module's `PropertyTransfer`)
    /// take the per-column verdict and the set-op barrier; partition skew
    /// (`SkewTransfer`) folds the whole tail by `Skew::union`, since a Form
    /// B relation in any scope can push rows into a neighbouring partition;
    /// footprint trajectory (`footprint.rs`'s `TrajectoryTransfer`) folds
    /// only the sub-slice whose `ExprScope::range` sits inside one of this
    /// scope's own select-list items — a running fold buried in a
    /// `WHERE`/`HAVING`/`QUALIFY`/`ORDER BY` subquery never becomes a
    /// stored output column, so it is not a trajectory contributor.
    pub expr_scopes: Vec<ExprScope>,
}

/// One expression-position subquery scope of a [`SelectNode`] — see
/// [`SelectNode::expr_scopes`].
#[derive(Debug, Clone)]
pub struct ExprScope {
    pub kind: ExprScopeKind,
    pub body: QueryNode,
    /// The `SUBQUERY` syntax node's own text range in the source — the
    /// select item that owns this scope is found by range containment
    /// (`PropertyTransfer::scope_determinism`/`scope_comparability` match a
    /// select item's expression range against this).
    pub range: smelt_parser::TextRange,
}

/// One CTE definition.
#[derive(Debug, Clone)]
pub struct CteDef {
    pub name: String,
    pub body: QueryNode,
}

/// A set operation over `branches`, with `ops[i]` joining
/// `branches[i]` and `branches[i + 1]`.
#[derive(Debug, Clone)]
pub struct SetOpNode {
    /// CTE definitions hoisted to the whole compound query (a `WITH`
    /// preceding any arm scopes over all subsequent arms).
    pub ctes: Vec<CteDef>,
    pub ops: Vec<SetOpKind>,
    pub branches: Vec<QueryNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOpKind {
    Union,
    UnionAll,
    Intersect,
    IntersectAll,
    Except,
    ExceptAll,
}

impl SetOpKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SetOpKind::Union => "UNION",
            SetOpKind::UnionAll => "UNION ALL",
            SetOpKind::Intersect => "INTERSECT",
            SetOpKind::IntersectAll => "INTERSECT ALL",
            SetOpKind::Except => "EXCEPT",
            SetOpKind::ExceptAll => "EXCEPT ALL",
        }
    }
}

/// One FROM item of a [`SelectNode`].
#[derive(Debug, Clone)]
pub enum InputItem {
    /// A base relation (table or `smelt.<path>` source/model ref).
    Table { name: String, alias: Option<String> },
    /// A reference to a CTE in scope. Its child verdict in the walk is the
    /// CTE subtree's verdict (sequential composition at the reference site).
    CteRef { name: String, alias: Option<String> },
    /// A derived table (subquery in FROM).
    Derived {
        alias: Option<String>,
        body: QueryNode,
    },
    /// An unrecognisable FROM construct — fail-loud placeholder.
    Unsupported { reason: String },
}

/// Where an alias (or bare relation name) in a scope points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationSource {
    Table(String),
    Cte(String),
    DerivedTable(String),
}

/// The resolved source leaf of a projected column: a column of a base
/// relation, reached by chasing renames through CTE / derived-table
/// projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafColumn {
    pub relation: String,
    pub column: String,
}

/// Lineage of one projected column of a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnLineage {
    /// The column's output name in this scope.
    pub output: String,
    /// The base-relation column it resolves to, when the projection is a
    /// rename-chain of simple column references; `None` for computed
    /// expressions, wildcards, and unresolvable references.
    pub leaf: Option<LeafColumn>,
    /// Whether this column's resolution passes through a self-join — a
    /// scope whose FROM tree reaches the same base relation under more than
    /// one alias, at this scope or *any* scope the reference was chased
    /// through (a CTE body, a derived table) — substrate-unification Phase
    /// 5 review finding 2. `orders o1 JOIN orders o2 ON …` makes `o1.id` and
    /// `o2.id` chase to the identical [`LeafColumn`] even though they are
    /// different rows on different join legs; `leaf` may still be populated
    /// here (it is the best-effort chase result, kept for diagnostics) but
    /// a consumer matching on stored-column *identity*
    /// (`backbuild::resolve_representative`'s lineage fallback, via
    /// [`resolve_reference_leaf`]) must never trust it when this is `true`.
    /// This is a property of the lineage entry itself, decided once where
    /// the entry is built ([`select_lineage`]) and propagated unchanged
    /// through every further CTE/derived-table hop — never re-derived
    /// per call site by counting aliases in whichever scope happens to be
    /// current there (that was the bypassable, scope-local version of this
    /// guard).
    pub ambiguous: bool,
}

/// Per-node context handed to every transfer function.
#[derive(Debug, Clone)]
pub struct NodeCx {
    /// Nesting path of this node from the top scope (empty = top).
    pub path: Vec<PathSeg>,
    /// alias (or bare name) → relation source, for this scope's FROM items.
    /// Keys are lowercased.
    pub aliases: BTreeMap<String, RelationSource>,
    /// Projected-column lineage of this node's output.
    pub columns: Vec<ColumnLineage>,
}

/// A leaf source relation seen by the walk.
#[derive(Debug)]
pub struct LeafInput<'a> {
    pub name: &'a str,
    pub alias: Option<&'a str>,
}

/// The operator at a tree node, as seen by a transfer function.
///
/// Child-verdict convention: for `Select(node)` the children slice is the
/// verdicts of `node.ctes` in order (each CTE body folded exactly once at
/// its definition site), followed by the verdicts of `node.inputs` in
/// order (a `CteRef` input's verdict is a clone of its definition's
/// verdict), followed by the verdicts of `node.expr_scopes` in order —
/// `ctes ++ inputs ++ expr_scopes`. A transfer that indexes `inputs` by
/// slicing off `node.ctes.len()` leading children, or by `.zip()`ing
/// `node.inputs` against that slice, is automatically safe against the
/// `expr_scopes` tail (the zip truncates); one that folds the *whole*
/// children slice unconditionally is not, and must slice to
/// `node.ctes.len() + node.inputs.len()` first. For `SetOp(node)` the
/// children are `node.ctes` verdicts followed by the branch verdicts in arm
/// order (`SetOpNode` has no `expr_scopes` of its own). `Unsupported` has no
/// children.
#[derive(Debug)]
pub enum OpNode<'a> {
    Select(&'a SelectNode),
    SetOp(&'a SetOpNode),
    Unsupported { reason: &'a str },
}

/// A property's transfer function over the query tree
/// (`model_properties.md` §"The composition walk").
pub trait Transfer {
    type Verdict: Clone;

    /// Verdict for a leaf source relation.
    fn leaf(&self, leaf: &LeafInput<'_>, cx: &NodeCx) -> Self::Verdict;

    /// Verdict for an operator node from its children's verdicts
    /// (see [`OpNode`] for the children convention). A fail-closed
    /// implementation must map `OpNode::Unsupported` to its reject verdict.
    fn operator(&self, op: &OpNode<'_>, children: &[Self::Verdict], cx: &NodeCx) -> Self::Verdict;
}

// ===== Tree construction =====

impl QueryTree {
    /// Build the tree for a model's SQL text (frontmatter stripped, parsed
    /// with the shared parser entry). Returns `None` when the text has no
    /// SELECT statement at all (e.g. a pipe query or pure meta file).
    pub fn from_sql(sql: &str) -> Option<QueryTree> {
        let stripped = crate::types::Frontmatter::strip(sql);
        let parse = smelt_parser::parse(stripped);
        let file = smelt_parser::File::cast(parse.syntax())?;
        let select = file.select_stmt()?;
        Some(QueryTree::from_select(&select))
    }

    /// Build the tree from an already-parsed outermost `SelectStmt`.
    pub fn from_select(select: &SelectStmt) -> QueryTree {
        let scope = CteScope::default();
        QueryTree {
            root: normalize(select, &scope),
        }
    }
}

impl QueryNode {
    /// Whether this subtree contains any construct the normalization could
    /// not model (an `Unsupported` node or FROM item). Consumers that need
    /// exact whole-tree coverage — rather than fail-closed rejection — use
    /// this to fall back to their legacy whole-text derivation. Known case:
    /// a `RECURSIVE` CTE (`normalize_ctes` rejects the self-reference
    /// explicitly, not yet modelled by the walk).
    pub(crate) fn has_unsupported(&self) -> bool {
        match self {
            QueryNode::Unsupported { .. } => true,
            QueryNode::Select(sn) => {
                sn.ctes.iter().any(|c| c.body.has_unsupported())
                    || sn.inputs.iter().any(|i| match i {
                        InputItem::Unsupported { .. } => true,
                        InputItem::Derived { body, .. } => body.has_unsupported(),
                        InputItem::Table { .. } | InputItem::CteRef { .. } => false,
                    })
                    || sn.expr_scopes.iter().any(|es| es.body.has_unsupported())
            }
            QueryNode::SetOp(so) => {
                so.ctes.iter().any(|c| c.body.has_unsupported())
                    || so.branches.iter().any(|b| b.has_unsupported())
            }
        }
    }
}

/// The set of CTE names visible at a point of the normalization
/// (lowercased for case-insensitive SQL identifier matching).
#[derive(Debug, Clone, Default)]
struct CteScope {
    names: std::collections::BTreeSet<String>,
}

impl CteScope {
    fn contains(&self, name: &str) -> bool {
        self.names.contains(&name.to_ascii_lowercase())
    }

    fn insert(&mut self, name: &str) {
        self.names.insert(name.to_ascii_lowercase());
    }
}

fn normalize(select: &SelectStmt, scope: &CteScope) -> QueryNode {
    // Flatten the set-operation chain. The parser nests it recursively
    // (`A UNION B UNION C` parses as A{… B{… C}}), so repeatedly following
    // `set_operation_select()` yields the arms in source order, and the
    // operator token joining arm i and arm i+1 lives among arm i's tokens.
    let mut chain = vec![select.clone()];
    let mut current = select.clone();
    while let Some(next) = current.set_operation_select() {
        chain.push(next.clone());
        current = next;
    }

    if chain.len() == 1 {
        let mut scope = scope.clone();
        let ctes = normalize_ctes(select, &mut scope);
        let inputs = normalize_from(select, &scope);
        let expr_scopes = normalize_expr_scopes(select, &scope);
        return QueryNode::Select(SelectNode {
            select: select.clone(),
            ctes,
            inputs,
            expr_scopes,
        });
    }

    // A compound query: hoist every arm's WITH to the SetOp node. A WITH
    // attaches syntactically to the arm it precedes but scopes over that
    // arm and (because later arms are nested inside it) all subsequent
    // arms, so the scope set accumulates across the loop.
    let mut scope = scope.clone();
    let mut ctes = Vec::new();
    let mut branches = Vec::with_capacity(chain.len());
    let mut ops = Vec::with_capacity(chain.len() - 1);
    for (i, branch) in chain.iter().enumerate() {
        ctes.extend(normalize_ctes(branch, &mut scope));
        let inputs = normalize_from(branch, &scope);
        let expr_scopes = normalize_expr_scopes(branch, &scope);
        branches.push(QueryNode::Select(SelectNode {
            select: branch.clone(),
            ctes: Vec::new(),
            inputs,
            expr_scopes,
        }));
        if i + 1 < chain.len() {
            ops.push(match setop_kind_after(branch) {
                Some(kind) => kind,
                None => {
                    // The chain said there is a next arm but no operator
                    // token was found — a parse shape the walk does not
                    // understand; reject the whole compound fail-loud.
                    return QueryNode::Unsupported {
                        reason: "set operation without a recognisable operator token".to_string(),
                    };
                }
            });
        }
    }
    QueryNode::SetOp(SetOpNode {
        ctes,
        ops,
        branches,
    })
}

/// Normalize a statement's own WITH clause (if any) into CTE definitions in
/// source order — a valid dependency order, since a non-recursive CTE may
/// only reference CTEs defined before it. Extends `scope` with each name so
/// later bodies (and the consumer) resolve references.
fn normalize_ctes(select: &SelectStmt, scope: &mut CteScope) -> Vec<CteDef> {
    let Some(with) = select.with_clause() else {
        return Vec::new();
    };
    let recursive = with.is_recursive();
    let mut defs = Vec::new();
    for cte in with.ctes() {
        let name = match cte.name() {
            Some(name) => name,
            None => {
                defs.push(CteDef {
                    name: String::new(),
                    body: QueryNode::Unsupported {
                        reason: "CTE without a recognisable name".to_string(),
                    },
                });
                continue;
            }
        };
        // A recursive CTE's self-reference would be misread as a base
        // table by the fold; reject it explicitly until modelled.
        let body = if recursive {
            QueryNode::Unsupported {
                reason: format!("RECURSIVE CTE '{name}' is not supported by the property walk"),
            }
        } else {
            scope.insert(&name);
            match cte.query().and_then(|q| q.select_stmt()) {
                Some(body_select) => normalize(&body_select, scope),
                None => QueryNode::Unsupported {
                    reason: format!("CTE '{name}' body is not a SELECT statement"),
                },
            }
        };
        if recursive {
            scope.insert(&name);
        }
        defs.push(CteDef { name, body });
    }
    defs
}

/// Normalize one SELECT scope's FROM items (comma items and join arms
/// alike) in source order. A scope with no FROM clause has no inputs.
fn normalize_from(select: &SelectStmt, scope: &CteScope) -> Vec<InputItem> {
    let Some(from) = select.from_clause() else {
        return Vec::new();
    };
    let refs = from
        .table_refs()
        .chain(from.joins().filter_map(|j| j.table_ref()));
    refs.flat_map(|table_ref| normalize_table_ref_items(&table_ref, scope))
        .collect()
}

/// Normalize one FROM-position `TableRef`, flattening a parenthesised join
/// group (`FROM (a JOIN b ON …)`) into its member items rather than one
/// opaque item — the parser nests the group as a `TABLE_REF` wrapping a
/// nested `TABLE_REF` plus `JOIN_CLAUSE` children (`parser/select.rs`'s
/// `LPAREN` branch of `parse_table_ref`), so recursion here mirrors the
/// parser's own recursion and an arbitrarily nested group
/// (`FROM ((a JOIN b) JOIN c)`) flattens fully. The group's own alias (if
/// any — `FROM (a JOIN b) AS g`, aliasing the joined-table result as a
/// whole) is not modelled; that shape is rare enough, and different enough
/// from every other `InputItem`, to defer rather than guess at here.
fn normalize_table_ref_items(
    table_ref: &smelt_parser::TableRef,
    scope: &CteScope,
) -> Vec<InputItem> {
    if let Some(nested) = table_ref.nested_table_ref() {
        let mut items = normalize_table_ref_items(&nested, scope);
        for join in table_ref.nested_joins() {
            if let Some(member) = join.table_ref() {
                items.extend(normalize_table_ref_items(&member, scope));
            }
        }
        return items;
    }
    vec![normalize_table_ref(table_ref, scope)]
}

fn normalize_table_ref(table_ref: &smelt_parser::TableRef, scope: &CteScope) -> InputItem {
    let alias = table_ref.alias();

    if let Some(subquery) = table_ref.subquery() {
        let body = match subquery.select_stmt() {
            Some(inner) => normalize(&inner, scope),
            None => QueryNode::Unsupported {
                reason: "derived table body is not a SELECT statement (e.g. VALUES)".to_string(),
            },
        };
        return InputItem::Derived { alias, body };
    }

    if let Some(path_ref) = table_ref.smelt_path_ref() {
        return InputItem::Table {
            name: path_ref.segments().join("."),
            alias,
        };
    }
    if let Some(path_call) = table_ref.smelt_path_call() {
        return InputItem::Table {
            name: path_call.segments().join("."),
            alias,
        };
    }

    if table_ref.is_function_call() {
        let name = table_ref
            .function_call()
            .and_then(|f| f.name())
            .unwrap_or_else(|| "<unknown>".to_string());
        return InputItem::Unsupported {
            reason: format!("table function '{name}' in FROM is not a recognised leaf source"),
        };
    }

    if let Some(name) = table_ref.identifier() {
        if scope.contains(&name) {
            return InputItem::CteRef { name, alias };
        }
        return InputItem::Table { name, alias };
    }

    InputItem::Unsupported {
        reason: format!(
            "unrecognised FROM construct: '{}'",
            table_ref.syntax().text().to_string().trim()
        ),
    }
}

/// The set-operation token (and optional ALL) among `select`'s own direct
/// tokens — the operator joining this arm to the next one in the chain.
fn setop_kind_after(select: &SelectStmt) -> Option<SetOpKind> {
    use smelt_parser::SyntaxKind::*;
    let tokens: Vec<_> = select
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();
    for (i, token) in tokens.iter().enumerate() {
        let base = match token.kind() {
            UNION_KW => SetOpKind::Union,
            INTERSECT_KW => SetOpKind::Intersect,
            EXCEPT_KW => SetOpKind::Except,
            _ => continue,
        };
        let all = tokens[i + 1..]
            .iter()
            .find(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
            .is_some_and(|t| t.kind() == ALL_KW);
        return Some(match (base, all) {
            (SetOpKind::Union, true) => SetOpKind::UnionAll,
            (SetOpKind::Intersect, true) => SetOpKind::IntersectAll,
            (SetOpKind::Except, true) => SetOpKind::ExceptAll,
            (kind, _) => kind,
        });
    }
    None
}

/// Normalize `select`'s own expression-position subquery scopes: every
/// `SUBQUERY` node reachable from the select list, `WHERE`, `HAVING`,
/// `QUALIFY`, then `ORDER BY` clauses, in that order, without descending
/// past a found `SUBQUERY` into its own nested `SELECT_STMT` (that body's
/// own expression scopes are collected by the recursive [`normalize`] call
/// on it, not repeated here) and without visiting the `FROM` clause (its
/// derived tables and join groups are [`normalize_from`]'s job, not this
/// one's). `scope` is the CTE names visible at this point — the same
/// environment a `FROM`-position derived table normalizes its body against,
/// so a correlated reference to an outer CTE resolves the same way in
/// either position.
fn normalize_expr_scopes(select: &SelectStmt, scope: &CteScope) -> Vec<ExprScope> {
    let mut nodes = Vec::new();
    if let Some(select_list) = select.select_list() {
        collect_expr_scope_nodes(select_list.syntax(), &mut nodes);
    }
    if let Some(where_clause) = select.where_clause() {
        collect_expr_scope_nodes(where_clause.syntax(), &mut nodes);
    }
    if let Some(having) = select.having_clause() {
        if let Some(expr) = having.expression() {
            collect_expr_scope_nodes(expr.syntax(), &mut nodes);
        }
    }
    if let Some(qualify) = select.qualify_clause() {
        if let Some(expr) = qualify.expression() {
            collect_expr_scope_nodes(expr.syntax(), &mut nodes);
        }
    }
    if let Some(order_by) = select.order_by_clause() {
        collect_expr_scope_nodes(order_by.syntax(), &mut nodes);
    }

    nodes
        .into_iter()
        .map(|(kind, subquery_node)| {
            let range = subquery_node.text_range();
            let body =
                match smelt_parser::Subquery::cast(subquery_node).and_then(|sq| sq.select_stmt()) {
                    Some(inner) => normalize(&inner, scope),
                    None => QueryNode::Unsupported {
                        reason: "expression-position subquery body is not a SELECT statement \
                             (e.g. VALUES)"
                            .to_string(),
                    },
                };
            ExprScope { kind, body, range }
        })
        .collect()
}

/// Depth-first, source-order collection of top-level `SUBQUERY` syntax
/// nodes under `node`, each tagged with its [`ExprScopeKind`] by its
/// immediate parent construct. Recursion stops at a found `SUBQUERY` (its
/// interior is a separate scope's business).
fn collect_expr_scope_nodes(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    out: &mut Vec<(ExprScopeKind, smelt_parser::syntax_kind::SyntaxNode)>,
) {
    for child in node.children() {
        if child.kind() == smelt_parser::SyntaxKind::SUBQUERY {
            let kind = classify_expr_scope_kind(&child);
            out.push((kind, child));
        } else {
            collect_expr_scope_nodes(&child, out);
        }
    }
}

/// Classify a `SUBQUERY` node's [`ExprScopeKind`] by its immediate parent —
/// `EXISTS_EXPR`, `IN_EXPR`, `ANY_EXPR` (the shared node for `ANY`/`ALL`/
/// `SOME`), or a bare scalar subquery otherwise.
fn classify_expr_scope_kind(
    subquery_node: &smelt_parser::syntax_kind::SyntaxNode,
) -> ExprScopeKind {
    use smelt_parser::SyntaxKind::*;
    match subquery_node.parent().map(|p| p.kind()) {
        Some(EXISTS_EXPR) => ExprScopeKind::Exists,
        Some(IN_EXPR) => ExprScopeKind::In,
        Some(ANY_EXPR) => ExprScopeKind::Quantified,
        _ => ExprScopeKind::Scalar,
    }
}

// ===== The fold =====

/// Per-walk environment: CTE name → (verdict, output lineage), lowercased
/// keys. Cloned into child scopes so inner definitions shadow without
/// leaking back out.
#[derive(Clone)]
struct WalkEnv<V: Clone> {
    ctes: BTreeMap<String, (V, Vec<ColumnLineage>)>,
}

impl<V: Clone> Default for WalkEnv<V> {
    fn default() -> Self {
        WalkEnv {
            ctes: BTreeMap::new(),
        }
    }
}

/// Fold `tree` bottom-up with the property's transfer function, returning
/// the whole-tree verdict.
pub fn walk<T: Transfer>(tree: &QueryTree, transfer: &T) -> T::Verdict {
    let env = WalkEnv::default();
    walk_node(&tree.root, transfer, &[], &env).0
}

fn walk_node<T: Transfer>(
    node: &QueryNode,
    transfer: &T,
    path: &[PathSeg],
    env: &WalkEnv<T::Verdict>,
) -> (T::Verdict, Vec<ColumnLineage>) {
    match node {
        QueryNode::Unsupported { reason } => {
            let cx = NodeCx {
                path: path.to_vec(),
                aliases: BTreeMap::new(),
                columns: Vec::new(),
            };
            (
                transfer.operator(&OpNode::Unsupported { reason }, &[], &cx),
                Vec::new(),
            )
        }
        QueryNode::Select(sn) => walk_select(sn, transfer, path, env),
        QueryNode::SetOp(so) => walk_setop(so, transfer, path, env),
    }
}

/// Fold a list of CTE definitions in order, extending `env` so each body
/// (and everything after it) sees the earlier definitions. Returns the
/// definition verdicts in order.
fn walk_ctes<T: Transfer>(
    ctes: &[CteDef],
    transfer: &T,
    path: &[PathSeg],
    env: &mut WalkEnv<T::Verdict>,
) -> Vec<T::Verdict> {
    let mut verdicts = Vec::with_capacity(ctes.len());
    for cte in ctes {
        let mut cte_path = path.to_vec();
        cte_path.push(PathSeg::Cte(cte.name.clone()));
        let (verdict, lineage) = walk_node(&cte.body, transfer, &cte_path, env);
        env.ctes
            .insert(cte.name.to_ascii_lowercase(), (verdict.clone(), lineage));
        verdicts.push(verdict);
    }
    verdicts
}

/// Build one scope's FROM-tree alias table and derived-table lineages — the
/// `InputItem` → `RelationSource` mapping [`walk_select`] and
/// [`resolve_reference_leaf`] both need (substrate-unification Phase 5
/// review: factored out so this mapping exists once rather than forked
/// between a full property walk and the standalone lineage-only resolver).
/// Each input's own transfer verdict (a leaf verdict, a cloned CTE-body
/// verdict, a derived-table subtree's verdict, or an `Unsupported` verdict)
/// is pushed onto `children` in source order, exactly as
/// [`walk_select`] needs them interleaved after the CTE verdicts already
/// there; [`resolve_reference_leaf`]'s `Discard` transfer populates
/// `children` too but never reads it back.
fn build_scope_aliases<T: Transfer>(
    inputs: &[InputItem],
    transfer: &T,
    path: &[PathSeg],
    env: &WalkEnv<T::Verdict>,
    children: &mut Vec<T::Verdict>,
) -> (
    BTreeMap<String, RelationSource>,
    BTreeMap<String, Vec<ColumnLineage>>,
) {
    let mut aliases = BTreeMap::new();
    // Derived-table lineages, keyed like `aliases`, for column resolution.
    let mut derived_lineage: BTreeMap<String, Vec<ColumnLineage>> = BTreeMap::new();

    for input in inputs {
        match input {
            InputItem::Table { name, alias } => {
                let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                aliases.insert(key, RelationSource::Table(name.clone()));
                let leaf = LeafInput {
                    name,
                    alias: alias.as_deref(),
                };
                let cx = NodeCx {
                    path: path.to_vec(),
                    aliases: BTreeMap::new(),
                    columns: Vec::new(),
                };
                children.push(transfer.leaf(&leaf, &cx));
            }
            InputItem::CteRef { name, alias } => {
                let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                aliases.insert(key, RelationSource::Cte(name.clone()));
                match env.ctes.get(&name.to_ascii_lowercase()) {
                    Some((verdict, _)) => children.push(verdict.clone()),
                    None => {
                        // Normalization classified the name as a CTE that the
                        // walk cannot find — a walk bug, surfaced fail-loud.
                        let reason = format!("unresolved CTE reference '{name}'");
                        let cx = NodeCx {
                            path: path.to_vec(),
                            aliases: BTreeMap::new(),
                            columns: Vec::new(),
                        };
                        children.push(transfer.operator(
                            &OpNode::Unsupported { reason: &reason },
                            &[],
                            &cx,
                        ));
                    }
                }
            }
            InputItem::Derived { alias, body } => {
                let alias_text = alias.clone().unwrap_or_default();
                let mut child_path = path.to_vec();
                child_path.push(PathSeg::DerivedTable(alias_text.clone()));
                let (verdict, lineage) = walk_node(body, transfer, &child_path, env);
                if let Some(alias) = alias {
                    let key = alias.to_ascii_lowercase();
                    aliases.insert(key.clone(), RelationSource::DerivedTable(alias.clone()));
                    derived_lineage.insert(key, lineage);
                }
                children.push(verdict);
            }
            InputItem::Unsupported { reason } => {
                let cx = NodeCx {
                    path: path.to_vec(),
                    aliases: BTreeMap::new(),
                    columns: Vec::new(),
                };
                children.push(transfer.operator(&OpNode::Unsupported { reason }, &[], &cx));
            }
        }
    }

    (aliases, derived_lineage)
}

fn walk_select<T: Transfer>(
    sn: &SelectNode,
    transfer: &T,
    path: &[PathSeg],
    env: &WalkEnv<T::Verdict>,
) -> (T::Verdict, Vec<ColumnLineage>) {
    let mut env = env.clone();
    let mut children = walk_ctes(&sn.ctes, transfer, path, &mut env);

    let (aliases, derived_lineage) =
        build_scope_aliases(&sn.inputs, transfer, path, &env, &mut children);

    let columns = select_lineage(&sn.select, &aliases, &env, &derived_lineage);
    let cx = NodeCx {
        path: path.to_vec(),
        aliases,
        columns,
    };

    // Expression-position subquery scopes fold last, after ctes and inputs
    // (`ctes ++ inputs ++ expr_scopes`, see `OpNode`'s doc comment) — an
    // outer-scope reference cannot resolve into one (it is not a FROM-tree
    // alias), so it needs no `cx.aliases` entry, only its own child verdict.
    for (index, es) in sn.expr_scopes.iter().enumerate() {
        let mut expr_path = path.to_vec();
        expr_path.push(PathSeg::ExprScope {
            kind: es.kind,
            index,
        });
        let (verdict, _lineage) = walk_node(&es.body, transfer, &expr_path, &env);
        children.push(verdict);
    }

    let verdict = transfer.operator(&OpNode::Select(sn), &children, &cx);
    (verdict, cx.columns)
}

fn walk_setop<T: Transfer>(
    so: &SetOpNode,
    transfer: &T,
    path: &[PathSeg],
    env: &WalkEnv<T::Verdict>,
) -> (T::Verdict, Vec<ColumnLineage>) {
    let mut env = env.clone();
    let mut children = walk_ctes(&so.ctes, transfer, path, &mut env);

    let mut first_branch_lineage: Option<Vec<ColumnLineage>> = None;
    for (i, branch) in so.branches.iter().enumerate() {
        let mut branch_path = path.to_vec();
        branch_path.push(PathSeg::SetOpBranch(i));
        let (verdict, lineage) = walk_node(branch, transfer, &branch_path, &env);
        if first_branch_lineage.is_none() {
            first_branch_lineage = Some(lineage);
        }
        children.push(verdict);
    }

    // SQL takes a compound query's output column names from its first arm.
    let columns = first_branch_lineage.unwrap_or_default();
    let cx = NodeCx {
        path: path.to_vec(),
        aliases: BTreeMap::new(),
        columns,
    };
    let verdict = transfer.operator(&OpNode::SetOp(so), &children, &cx);
    (verdict, cx.columns)
}

/// Whether `source` is a base-relation reference (`RelationSource::Table`)
/// whose own table is reachable through more than one alias in `aliases` —
/// a self-join at this scope (substrate-unification Phase 5 review finding
/// 2). `false` for a `Cte`/`DerivedTable` source: their own ambiguity, if
/// any, is already carried on the resolved [`ColumnLineage::ambiguous`]
/// flag from whichever inner scope built it — this only ever decides the
/// *local*, direct-Table leg of the ambiguity, at the exact point a lineage
/// entry is built ([`select_lineage`]) or a standalone reference is resolved
/// ([`resolve_reference_leaf`]) — the two call sites that ever need it, so
/// the duplicate-alias count is computed in exactly one place.
fn table_is_self_joined(
    source: &RelationSource,
    aliases: &BTreeMap<String, RelationSource>,
) -> bool {
    let RelationSource::Table(name) = source else {
        return false;
    };
    aliases
        .values()
        .filter(|r| matches!(r, RelationSource::Table(n) if n == name))
        .count()
        > 1
}

/// Projected-column lineage of one SELECT scope: each output column, and —
/// when its expression is a simple (possibly qualified) column reference —
/// the base-relation column it resolves to, chased through CTE and
/// derived-table projections. Each entry's [`ColumnLineage::ambiguous`] flag
/// is decided right here, the lineage's single build site, and never
/// recomputed downstream.
fn select_lineage<V: Clone>(
    select: &SelectStmt,
    aliases: &BTreeMap<String, RelationSource>,
    env: &WalkEnv<V>,
    derived_lineage: &BTreeMap<String, Vec<ColumnLineage>>,
) -> Vec<ColumnLineage> {
    let Some(select_list) = select.select_list() else {
        return Vec::new();
    };
    let mut columns = Vec::new();
    for item in select_list.items() {
        if item.is_wildcard() {
            columns.push(ColumnLineage {
                output: "*".to_string(),
                leaf: None,
                ambiguous: false,
            });
            continue;
        }
        let Some(expr) = item.expression() else {
            continue;
        };
        let output = item
            .column_name()
            .unwrap_or_else(|| expr.text().trim().to_string());
        let resolved = ColumnRef::from_expr(&expr).and_then(|col_ref| {
            let source = match col_ref.qualifier() {
                Some(q) => aliases.get(&q.to_ascii_lowercase()),
                // Unqualified: unambiguous only with a single input.
                None if aliases.len() == 1 => aliases.values().next().map(Some).unwrap_or(None),
                None => None,
            }?;
            let mut lineage = resolve_leaf(source, col_ref.name(), env, derived_lineage)?;
            // Monotonic: only ever set `ambiguous` to `true` here, never
            // clear it — a `Cte`/`DerivedTable` source may already have
            // propagated `true` from a self-join several hops down.
            if table_is_self_joined(source, aliases) {
                lineage.ambiguous = true;
            }
            Some(lineage)
        });
        let (leaf, ambiguous) = match resolved {
            Some(lineage) => (lineage.leaf, lineage.ambiguous),
            None => (None, false),
        };
        columns.push(ColumnLineage {
            output,
            leaf,
            ambiguous,
        });
    }
    columns
}

/// Resolve `column` against `source`, returning the matched lineage entry
/// (leaf + its own `ambiguous` flag, propagated unchanged from whichever
/// scope originally built it for a `Cte`/`DerivedTable` source — never
/// recomputed here). Callers combine this with their own scope's
/// [`table_is_self_joined`] check for the direct-`Table` leg.
fn resolve_leaf<V: Clone>(
    source: &RelationSource,
    column: &str,
    env: &WalkEnv<V>,
    derived_lineage: &BTreeMap<String, Vec<ColumnLineage>>,
) -> Option<ColumnLineage> {
    match source {
        RelationSource::Table(table) => Some(ColumnLineage {
            output: column.to_string(),
            leaf: Some(LeafColumn {
                relation: table.clone(),
                column: column.to_string(),
            }),
            ambiguous: false,
        }),
        RelationSource::Cte(name) => {
            let (_, lineage) = env.ctes.get(&name.to_ascii_lowercase())?;
            lineage
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(column))
                .cloned()
        }
        RelationSource::DerivedTable(alias) => {
            let lineage = derived_lineage.get(&alias.to_ascii_lowercase())?;
            lineage
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(column))
                .cloned()
        }
    }
}

/// Resolve a single column reference — `(qualifier, raw_name)`, read
/// straight off some expression in `tree`'s own top-level scope — against
/// that scope's own lineage, chasing CTE / derived-table renames to the
/// reference's base-relation leaf. This generalizes [`select_lineage`]'s
/// per-projected-column resolution to an arbitrary reference (e.g. a
/// dependency inside an expression that is not itself a SELECT-list item —
/// `backbuild::resolve_representative`'s consumer, substrate-unification
/// Phase 5) — the same alias/CTE-lineage machinery [`walk_select`] builds,
/// run with a [`Transfer`] whose verdict is discarded, since only the
/// lineage side-channel is wanted here.
///
/// `None` when the top-level node is not a single `SELECT` scope (a set
/// operation or an unrecognised construct — fail-closed, no lineage to
/// chase), the qualifier does not resolve to a FROM-tree alias in this
/// scope, an unqualified reference is ambiguous (more than one FROM input),
/// the resolved source has no lineage entry for `raw_name` at all, or the
/// resolved [`ColumnLineage::ambiguous`] flag is set — a self-join
/// anywhere along the chase, at this scope or nested arbitrarily deep
/// through CTE bodies (substrate-unification Phase 5 review finding 2: a
/// self-join — `FROM orders o1 JOIN orders o2` — chases `o1.id` and `o2.id`
/// to the identical `LeafColumn{relation: "orders", column: "id"}`, which
/// would otherwise let a caller conflate two different join legs' columns
/// as "the same stored data" purely because they share a base table name —
/// the exact C2 self-read hazard `backbuild::resolve_representative`'s flat
/// qualifier-match rule was written to prevent). The ambiguity check itself
/// lives once, in [`select_lineage`]/[`table_is_self_joined`] — this
/// function only ever *reads* the flag [`resolve_leaf`] returns, it never
/// recomputes an alias count of its own (a prior version did, scoped to
/// only the top-level call site, which a self-join hidden inside a
/// referenced CTE body bypassed entirely).
pub fn resolve_reference_leaf(
    tree: &QueryTree,
    qualifier: Option<&str>,
    raw_name: &str,
) -> Option<LeafColumn> {
    let QueryNode::Select(sn) = &tree.root else {
        return None;
    };

    struct Discard;
    impl Transfer for Discard {
        type Verdict = ();
        fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) {}
        fn operator(&self, _op: &OpNode<'_>, _children: &[()], _cx: &NodeCx) {}
    }

    let mut env = WalkEnv::<()>::default();
    walk_ctes(&sn.ctes, &Discard, &[], &mut env);

    let mut discarded_children = Vec::new();
    let (aliases, derived_lineage) =
        build_scope_aliases(&sn.inputs, &Discard, &[], &env, &mut discarded_children);

    let source = match qualifier {
        Some(q) => aliases.get(&q.to_ascii_lowercase())?,
        None if aliases.len() == 1 => aliases.values().next()?,
        None => return None,
    };

    let mut lineage = resolve_leaf(source, raw_name, &env, &derived_lineage)?;
    if table_is_self_joined(source, &aliases) {
        lineage.ambiguous = true;
    }

    if lineage.ambiguous {
        return None;
    }
    lineage.leaf
}

/// Per-scope resolution context, built once for every [`SelectNode`] scope
/// [`enumerate_select_scopes`] finds — the top scope, every CTE body, every
/// derived-table body, and every set-operation branch, each at its own
/// definition site. Resolves a bare `(qualifier, name)` reference collected
/// from *that scope's own* clauses to its base-relation leaf, chasing
/// through that scope's own CTE/derived-table aliases exactly as
/// [`resolve_reference_leaf`] does for the top scope alone — the
/// generalization `maintenance::grouping`'s membership pass needs to scan
/// every scope the walk enumerates rather than only the outermost one
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 3).
pub struct ScopeResolver {
    aliases: BTreeMap<String, RelationSource>,
    env: WalkEnv<()>,
    derived_lineage: BTreeMap<String, Vec<ColumnLineage>>,
}

impl ScopeResolver {
    /// Resolve one reference against this scope's own alias table. `None`
    /// on the same fail-closed conditions [`resolve_reference_leaf`]
    /// documents (unresolved qualifier, ambiguous unqualified reference, a
    /// self-join anywhere along the chase, or a chase shape the walk cannot
    /// normalize) — scoped to this scope's own alias table rather than the
    /// tree's top scope.
    pub fn resolve(&self, qualifier: Option<&str>, name: &str) -> Option<LeafColumn> {
        let source = match qualifier {
            Some(q) => self.aliases.get(&q.to_ascii_lowercase())?,
            None if self.aliases.len() == 1 => self.aliases.values().next()?,
            None => return None,
        };
        let mut lineage = resolve_leaf(source, name, &self.env, &self.derived_lineage)?;
        if table_is_self_joined(source, &self.aliases) {
            lineage.ambiguous = true;
        }
        if lineage.ambiguous {
            return None;
        }
        lineage.leaf
    }
}

/// Every [`SelectNode`] scope the walk enumerates, each paired with its own
/// [`ScopeResolver`]: the top scope, every CTE body, every derived-table
/// body, and every set-operation branch — each visited exactly once, at its
/// own definition site (a `CteRef` input's target is not revisited per
/// reference site, matching the value-pass fold's "each CTE body evaluated
/// once at its definition site" convention: [`walk_ctes`]). Consumer:
/// `maintenance::grouping`'s membership-sensitivity pass, which scans each
/// returned scope's own `JOIN`-`ON`/`WHERE`/`HAVING` conjuncts
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 3) — this function
/// is the same recursion shape [`walk_node`]/[`walk_select`] use for the
/// generic [`Transfer`] fold, specialized to collect resolvers instead of
/// running a transfer function.
pub fn enumerate_select_scopes(tree: &QueryTree) -> Vec<(&SelectNode, ScopeResolver)> {
    let mut out = Vec::new();
    collect_select_scopes(&tree.root, &WalkEnv::default(), &mut out);
    out
}

fn collect_select_scopes<'a>(
    node: &'a QueryNode,
    env: &WalkEnv<()>,
    out: &mut Vec<(&'a SelectNode, ScopeResolver)>,
) -> Vec<ColumnLineage> {
    match node {
        QueryNode::Unsupported { .. } => Vec::new(),
        QueryNode::Select(sn) => {
            let mut env = env.clone();
            for cte in &sn.ctes {
                let lineage = collect_select_scopes(&cte.body, &env, out);
                env.ctes
                    .insert(cte.name.to_ascii_lowercase(), ((), lineage));
            }

            let mut aliases = BTreeMap::new();
            let mut derived_lineage: BTreeMap<String, Vec<ColumnLineage>> = BTreeMap::new();
            for input in &sn.inputs {
                match input {
                    InputItem::Table { name, alias } => {
                        let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                        aliases.insert(key, RelationSource::Table(name.clone()));
                    }
                    InputItem::CteRef { name, alias } => {
                        let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                        aliases.insert(key, RelationSource::Cte(name.clone()));
                    }
                    InputItem::Derived { alias, body } => {
                        let lineage = collect_select_scopes(body, &env, out);
                        if let Some(alias) = alias {
                            let key = alias.to_ascii_lowercase();
                            aliases
                                .insert(key.clone(), RelationSource::DerivedTable(alias.clone()));
                            derived_lineage.insert(key, lineage);
                        }
                    }
                    InputItem::Unsupported { .. } => {}
                }
            }

            let columns = select_lineage(&sn.select, &aliases, &env, &derived_lineage);
            out.push((
                sn,
                ScopeResolver {
                    aliases,
                    env,
                    derived_lineage,
                },
            ));
            columns
        }
        QueryNode::SetOp(so) => {
            let mut env = env.clone();
            for cte in &so.ctes {
                let lineage = collect_select_scopes(&cte.body, &env, out);
                env.ctes
                    .insert(cte.name.to_ascii_lowercase(), ((), lineage));
            }
            let mut first_branch_lineage = None;
            for branch in &so.branches {
                let lineage = collect_select_scopes(branch, &env, out);
                if first_branch_lineage.is_none() {
                    first_branch_lineage = Some(lineage);
                }
            }
            first_branch_lineage.unwrap_or_default()
        }
    }
}
