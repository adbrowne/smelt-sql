//! The shared bottom-up property walk over a model's logical operator tree
//! (`model_properties.md` §"The composition walk").
//!
//! A parsed model is normalized into a [`QueryTree`] — CTE definitions in
//! dependency order, set-operation branches, FROM items including derived
//! tables — and folded bottom-up by [`walk`]. Each property contributes a
//! [`Transfer`] (a transfer function `(operator, child verdicts) → verdict`);
//! the walk applies it once per node, carrying a per-node [`NodeCx`]
//! (alias→source map and projected-column lineage).
//!
//! The fold distinguishes the three composition shapes a transfer function
//! needs:
//! - **sequential nesting** — a CTE body feeds its reference sites (the
//!   reference-site child verdict *is* the CTE subtree's verdict) and a
//!   derived table feeds the enclosing scope;
//! - **parallel branching** — set-operation arms are sibling children of a
//!   [`SetOpNode`];
//! - **joins** — multiple FROM inputs of one [`SelectNode`] are sibling
//!   children of that node.
//!
//! Fail-loud: an unrecognisable relational construct is normalized to an
//! explicit `Unsupported` node (never silently skipped), so a fail-closed
//! transfer function yields its reject verdict for the subtree above it.

use std::collections::BTreeMap;

use smelt_parser::{ColumnRef, SelectStmt};

use super::{item_alias, item_expr, resolve_scope_group_by, select_stmt_items, SelectItemKind};

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
/// its definition site) followed by the verdicts of `node.inputs` in order;
/// a `CteRef` input's verdict is a clone of its definition's verdict. For
/// `SetOp(node)` the children are `node.ctes` verdicts followed by the
/// branch verdicts in arm order. `Unsupported` has no children.
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
    /// the parser does not nest a redundantly-parenthesized derived table
    /// (`FROM ((SELECT …)) AS t`, the shape function expansion emits), so
    /// that FROM item normalizes to `Unsupported`.
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
        return QueryNode::Select(SelectNode {
            select: select.clone(),
            ctes,
            inputs,
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
        branches.push(QueryNode::Select(SelectNode {
            select: branch.clone(),
            ctes: Vec::new(),
            inputs,
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
    refs.map(|table_ref| normalize_table_ref(&table_ref, scope))
        .collect()
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

fn walk_select<T: Transfer>(
    sn: &SelectNode,
    transfer: &T,
    path: &[PathSeg],
    env: &WalkEnv<T::Verdict>,
) -> (T::Verdict, Vec<ColumnLineage>) {
    let mut env = env.clone();
    let mut children = walk_ctes(&sn.ctes, transfer, path, &mut env);

    let mut aliases = BTreeMap::new();
    // Derived-table lineages, keyed like `aliases`, for column resolution.
    let mut derived_lineage: BTreeMap<String, Vec<ColumnLineage>> = BTreeMap::new();

    for input in &sn.inputs {
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
                let (verdict, lineage) = walk_node(body, transfer, &child_path, &env);
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

    let columns = select_lineage(&sn.select, &aliases, &env, &derived_lineage);
    let cx = NodeCx {
        path: path.to_vec(),
        aliases,
        columns,
    };
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

/// Projected-column lineage of one SELECT scope: each output column, and —
/// when its expression is a simple (possibly qualified) column reference —
/// the base-relation column it resolves to, chased through CTE and
/// derived-table projections.
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
            });
            continue;
        }
        let Some(expr) = item.expression() else {
            continue;
        };
        let output = item
            .column_name()
            .unwrap_or_else(|| expr.text().trim().to_string());
        let leaf = ColumnRef::from_expr(&expr).and_then(|col_ref| {
            let source = match col_ref.qualifier() {
                Some(q) => aliases.get(&q.to_ascii_lowercase()),
                // Unqualified: unambiguous only with a single input.
                None if aliases.len() == 1 => aliases.values().next().map(Some).unwrap_or(None),
                None => None,
            }?;
            resolve_leaf(source, col_ref.name(), env, derived_lineage)
        });
        columns.push(ColumnLineage { output, leaf });
    }
    columns
}

fn resolve_leaf<V: Clone>(
    source: &RelationSource,
    column: &str,
    env: &WalkEnv<V>,
    derived_lineage: &BTreeMap<String, Vec<ColumnLineage>>,
) -> Option<LeafColumn> {
    match source {
        RelationSource::Table(table) => Some(LeafColumn {
            relation: table.clone(),
            column: column.to_string(),
        }),
        RelationSource::Cte(name) => {
            let (_, lineage) = env.ctes.get(&name.to_ascii_lowercase())?;
            lineage
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(column))?
                .leaf
                .clone()
        }
        RelationSource::DerivedTable(alias) => {
            let lineage = derived_lineage.get(&alias.to_ascii_lowercase())?;
            lineage
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(column))?
                .leaf
                .clone()
        }
    }
}

// ===== Scope enumeration: the first Transfer =====

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
    use super::{
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

/// The raw text of one SELECT scope's own region: every token of the scope's
/// node except the subtrees that are walk nodes of their own — its WITH
/// clause (CTE bodies), derived-table bodies in FROM, and a direct-child
/// SELECT (the next set-operation arm). Expression-position subqueries
/// (`EXISTS (…)`, a scalar subquery) are NOT walk nodes, so their text stays
/// in the owning scope's region — fail-closed coverage for content the tree
/// normalization does not model. Join `ON` conditions live in the FROM
/// clause and are kept (only the derived-table `SUBQUERY` bodies under a
/// `TABLE_REF` are pruned).
pub(crate) fn own_region_text(select: &SelectStmt) -> String {
    use smelt_parser::syntax_kind::SyntaxNode;
    use smelt_parser::SyntaxKind::{SELECT_STMT, SUBQUERY, TABLE_REF, WITH_CLAUSE};

    fn collect(node: &SyntaxNode, root: &SyntaxNode, out: &mut String) {
        for element in node.children_with_tokens() {
            if let Some(token) = element.as_token() {
                out.push_str(token.text());
            } else if let Some(child) = element.as_node() {
                match child.kind() {
                    WITH_CLAUSE => {}
                    SELECT_STMT if node == root => {}
                    SUBQUERY if node.kind() == TABLE_REF => {}
                    _ => collect(child, root, out),
                }
            }
        }
    }

    let root = select.syntax();
    let mut out = String::new();
    collect(root, root, &mut out);
    out
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
/// `exclude_source` names the model's own source path (dotted, as it appears
/// in a `smelt.<path>` self-reference) for a self-referential model
/// (`docs/specs/incremental_models.md` §"Window independence and self-referential
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
        let acc = children
            .iter()
            .fold(Skew::ZERO, |acc, child| acc.union(*child));
        match op {
            OpNode::Select(sn) => {
                let own = match self.exclude_source {
                    Some(self_name) => {
                        let quals = scope_self_qualifiers(cx, self_name);
                        own_region_text_excluding_self_relations(&sn.select, &quals)
                    }
                    None => own_region_text(&sn.select),
                };
                acc.union(derive_partition_skew(&own, self.partition_column))
            }
            OpNode::SetOp(_) => acc,
            // An `Unsupported` node carries no readable text; whole-tree
            // coverage is restored by [`model_partition_skew`]'s whole-text
            // fallback (it never walks a tree containing one).
            OpNode::Unsupported { .. } => acc,
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
/// (`docs/specs/incremental_models.md` §"Window independence and self-referential
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
            collect_self_conjunct_ranges(expr.syntax(), self_quals, &mut excluded);
        }
    }
    if let Some(from_clause) = select.from_clause() {
        for join in from_clause.joins() {
            if let Some(on_expr) = join.condition().and_then(|c| c.on_expression()) {
                collect_self_conjunct_ranges(on_expr.syntax(), self_quals, &mut excluded);
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

/// Split `node` (a `WHERE`/`ON` expression) into its top-level
/// `AND`-separated conditions — structurally, by descending `BINARY_EXPR`
/// nodes whose own operator token is `AND` (a `BETWEEN`'s `AND` lives inside
/// its own `BETWEEN_EXPR` node and is never split) — and record the range of
/// each condition that references one of `self_quals`.
///
/// A condition containing an `OR` anywhere in its subtree is **never**
/// recorded, even when it references a self qualifier: an `OR` may
/// disjunctively mix the self bound with a genuine relation, and dropping
/// the whole disjunction would silently under-widen the derived output
/// window. Keeping it can only over-widen — the fail-safe direction.
fn collect_self_conjunct_ranges(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    self_quals: &[String],
    out: &mut Vec<smelt_parser::TextRange>,
) {
    use smelt_parser::SyntaxKind::{AND_KW, BINARY_EXPR, EXPRESSION};

    // Unwrap EXPRESSION wrappers.
    if node.kind() == EXPRESSION {
        let children: Vec<_> = node.children().collect();
        if children.len() == 1 {
            return collect_self_conjunct_ranges(&children[0], self_quals, out);
        }
    }

    let is_and = node.kind() == BINARY_EXPR
        && node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == AND_KW);
    if is_and {
        for child in node.children() {
            collect_self_conjunct_ranges(&child, self_quals, out);
        }
        return;
    }

    if !conjunct_contains_or(node) && conjunct_references_qualifier(node, self_quals) {
        out.push(node.text_range());
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
/// (`docs/specs/incremental_models.md` §"Window independence and self-referential
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

// ===== The model property vector: grain, FDs, discriminants, determinism =====

use crate::analysis::discriminants::{combiner_discriminants, Discriminants};
use crate::analysis::join_shape::{fan_out, Cardinality, JoinContext};
use crate::analysis::monotonicity::{classify_function_determinism, FunctionDeterminism};

/// One proven key of a relation: a set of output-column names that together
/// uniquely identify a row. The empty grain (no keys) is the fail-closed
/// default — an unkeyed relation is `OneToMany`.
pub type KeySet = Vec<String>;

/// The proven grain of a relation node — the keys the walk can establish from
/// query structure (a `GROUP BY` factory key, a `DISTINCT` whole-row key, a
/// discriminated-union key). Empty ⇒ no key proven ⇒ `OneToMany` (fail-closed;
/// grain is never optimistically assumed, `model_properties.md` §Constraints).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedFd {
    pub key: KeySet,
    pub determines: String,
}

/// Determinism level of a projected column — the lattice `Clean < Run < Row`
/// (`model_properties.md` §"Determinism (run vs row) and the nondeterminism
/// predicate"). A columnar union takes the per-position lub (`clean ∪ clean =
/// clean`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Determinism {
    /// Deterministic every run (a plain column, a deterministic expression).
    Clean,
    /// One value per run (`NOW`, `CURRENT_DATE`) — a per-run constant.
    Run,
    /// A fresh value per row (`RANDOM`, `UUID`) — unpinnable.
    Row,
}

/// Per-column determinism fact.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnComparability {
    pub output: String,
    pub comparability: Comparability,
}

/// Per-column algebraic discriminants of an aggregate output column (the
/// combiner classifier applied at the aggregate's defining scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDiscriminant {
    pub output: String,
    pub discriminants: Discriminants,
}

/// The whole-model property vector — one derivation every current and future
/// consumer reads (`model_properties.md` §"The composition walk"). Grain,
/// functional dependencies, per-column discriminants, and the determinism
/// predicate are folded bottom-up by [`PropertyTransfer`]; the fields are the
/// four fact families, all fail-closed by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// through CTE/derived inputs where the input's own determinism is known.
    fn scope_determinism(
        &self,
        sn: &SelectNode,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &PropertyVector>,
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
            out.push(ColumnDeterminism { output, level });
        }
        out
    }

    /// Per-column change-comparability of one SELECT scope, reducing plain
    /// column refs through CTE/derived inputs where the input's own
    /// comparability is known (`model_properties.md` §"Change
    /// comparability").
    fn scope_comparability(
        &self,
        sn: &SelectNode,
        cx: &NodeCx,
        input_props: &BTreeMap<String, &PropertyVector>,
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

                let determinism = self.scope_determinism(sn, cx, &input_props);
                let comparability = self.scope_comparability(sn, cx, &input_props);
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
/// alias key in `cx.aliases`.
fn resolve_alias_source(cx: &NodeCx, qualifier: Option<&str>) -> Option<String> {
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

/// The determinism of an expression from the nondeterminism predicate over
/// every function call it contains — the fail-closed leaf classifier
/// (`monotonicity::classify_function_determinism`).
fn expr_determinism(expr: &smelt_parser::Expr) -> Determinism {
    let mut level = Determinism::Clean;
    for node in expr.syntax().descendants() {
        if node.kind() != smelt_parser::SyntaxKind::FUNCTION_CALL {
            continue;
        }
        let Some(func) = smelt_parser::FunctionCall::cast(node) else {
            continue;
        };
        let Some(name) = func.name() else { continue };
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
/// function call it contains. A known run-/row-nondeterministic function
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
    for node in expr.syntax().descendants() {
        if node.kind() != smelt_parser::SyntaxKind::FUNCTION_CALL {
            continue;
        }
        let Some(func) = smelt_parser::FunctionCall::cast(node) else {
            continue;
        };
        let Some(name) = func.name() else { continue };
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

/// Whether `expr` is a constant literal — a string/number literal with no
/// column reference or function call (a discriminator/tag candidate).
fn is_constant_literal(expr: &smelt_parser::Expr) -> bool {
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

/// The whole-model property vector — the single walk-derived derivation of
/// grain, functional dependencies, discriminants, and determinism
/// (`model_properties.md` §"The composition walk"). `ctx` supplies the
/// declared unique keys the fan-out proof reads. `None` when the text has no
/// SELECT statement at all.
pub fn model_property_vector(sql: &str, ctx: &JoinContext) -> Option<PropertyVector> {
    let tree = QueryTree::from_sql(sql)?;
    Some(walk(&tree, &PropertyTransfer { ctx }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes_of(sql: &str) -> ScopeEnumeration {
        enumerate_scopes(sql).expect("model parses to a SELECT")
    }

    fn find<'a>(e: &'a ScopeEnumeration, kind: ScopeKind, path: &[PathSeg]) -> Vec<&'a Scope> {
        e.scopes
            .iter()
            .filter(|s| s.kind == kind && s.path == path)
            .collect()
    }

    /// The pre-walk admission helpers in `rules::incremental` judge scopes
    /// by iterating the outer UNION chain only
    /// (`check_having_alignment_all_scopes` / `check_distinct_alignment_all_scopes`:
    /// `select.having_clause()` / `select.is_distinct()` on each
    /// `union_select()` link). This mirrors that chain and shows it never
    /// reaches a CTE body — the hole the shared walk closes.
    fn outer_chain_sees_having_or_distinct(sql: &str) -> bool {
        let parse = smelt_parser::parse(crate::types::Frontmatter::strip(sql));
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let mut current = file.select_stmt().expect("select stmt");
        loop {
            if current.having_clause().is_some() || current.is_distinct() {
                return true;
            }
            match current.union_select() {
                Some(next) => current = next,
                None => return false,
            }
        }
    }

    #[test]
    fn enumerates_scopes_inside_cte_bodies() {
        let sql = "WITH dedup AS (\
             SELECT DISTINCT user_id, event_date FROM events \
             GROUP BY user_id, event_date \
             HAVING COUNT(*) > 1\
         ) \
         SELECT user_id FROM dedup";

        // The existing outer-chain walk does not see the CTE-internal
        // DISTINCT/HAVING at all — this is the hole being documented.
        assert!(
            !outer_chain_sees_having_or_distinct(sql),
            "outer-chain walk unexpectedly sees CTE-internal scopes"
        );

        let e = scopes_of(sql);
        let cte_path = vec![PathSeg::Cte("dedup".to_string())];
        assert_eq!(
            find(&e, ScopeKind::Distinct, &cte_path).len(),
            1,
            "DISTINCT inside the CTE body must be enumerated; got: {:?}",
            e.scopes
        );
        let having = find(&e, ScopeKind::Having, &cte_path);
        assert_eq!(
            having.len(),
            1,
            "HAVING inside the CTE body must be enumerated; got: {:?}",
            e.scopes
        );
        let group_by = find(&e, ScopeKind::GroupBy, &cte_path);
        assert_eq!(group_by.len(), 1);
        assert_eq!(group_by[0].keys, vec!["user_id", "event_date"]);
        assert!(
            e.unsupported.is_empty(),
            "nothing unrecognised here: {:?}",
            e.unsupported
        );
    }

    #[test]
    fn expression_position_distinct_is_judged_by_admission() {
        // A `SELECT DISTINCT` in an expression-position scalar subquery is the
        // SC-7 cross-partition-dedup hazard one nesting level down: it dedups
        // `profiles` rows across partitions, unaligned to the model's `d`
        // partition. It is not a walk node, so it must be judged in the owning
        // scope's region — the admission gate must flag it, not silently pass.
        let sql = "SELECT e.d, e.user_id, \
                   (SELECT DISTINCT tier FROM smelt.sources.profiles p \
                     WHERE p.user_id = e.user_id) AS tier \
                   FROM smelt.sources.events e";
        let violations = batched_admission_violations(sql, "d").expect("model parses");
        assert!(
            violations.iter().any(|v| v.gate == AdmissionGate::Distinct),
            "expression-position SELECT DISTINCT must trip the Distinct gate; got {violations:?}"
        );
    }

    #[test]
    fn expression_position_aligned_distinct_is_admitted() {
        // An expression-position DISTINCT whose key includes the partition
        // column is partition-local — it must NOT trip the gate (no
        // over-refusal of a legitimately-aligned dedup).
        let sql = "SELECT e.d, \
                   (SELECT DISTINCT d FROM smelt.sources.profiles p \
                     WHERE p.user_id = e.user_id) AS d2 \
                   FROM smelt.sources.events e";
        let violations = batched_admission_violations(sql, "d").expect("model parses");
        assert!(
            !violations.iter().any(|v| v.gate == AdmissionGate::Distinct),
            "an aligned expression-position DISTINCT must be admitted; got {violations:?}"
        );
    }

    #[test]
    fn enumerates_set_op_branch_scopes_per_branch() {
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM events_a GROUP BY event_date \
             UNION ALL \
             SELECT event_date, COUNT(*) AS cnt FROM events_b GROUP BY event_date";
        let e = scopes_of(sql);

        let b0 = find(&e, ScopeKind::GroupBy, &[PathSeg::SetOpBranch(0)]);
        let b1 = find(&e, ScopeKind::GroupBy, &[PathSeg::SetOpBranch(1)]);
        assert_eq!(
            (b0.len(), b1.len()),
            (1, 1),
            "each branch's GROUP BY is its own scope with its own path; got: {:?}",
            e.scopes
        );

        let setop = find(&e, ScopeKind::SetOp, &[]);
        assert_eq!(setop.len(), 1);
        assert_eq!(setop[0].keys, vec!["UNION ALL"]);
        assert!(e.unsupported.is_empty());
    }

    #[test]
    fn derived_table_and_nested_cte_scopes() {
        let sql = "WITH base AS (\
             SELECT user_id, event_date FROM events GROUP BY user_id, event_date\
         ), daily AS (\
             SELECT event_date FROM base GROUP BY event_date\
         ) \
         SELECT d.event_date FROM (\
             SELECT event_date FROM daily GROUP BY event_date\
         ) d";
        let e = scopes_of(sql);

        let base = find(&e, ScopeKind::GroupBy, &[PathSeg::Cte("base".to_string())]);
        let daily = find(&e, ScopeKind::GroupBy, &[PathSeg::Cte("daily".to_string())]);
        let derived = find(
            &e,
            ScopeKind::GroupBy,
            &[PathSeg::DerivedTable("d".to_string())],
        );
        assert_eq!(
            (base.len(), daily.len(), derived.len()),
            (1, 1, 1),
            "CTE-referencing-CTE and subquery-in-FROM scopes all visited once; got: {:?}",
            e.scopes
        );

        // Dependency order is stable: base (defined first) before daily
        // (which references it), both before the derived table's scope.
        let pos = |path: &[PathSeg]| {
            e.scopes
                .iter()
                .position(|s| s.kind == ScopeKind::GroupBy && s.path == path)
                .expect("scope present")
        };
        let base_pos = pos(&[PathSeg::Cte("base".to_string())]);
        let daily_pos = pos(&[PathSeg::Cte("daily".to_string())]);
        let derived_pos = pos(&[PathSeg::DerivedTable("d".to_string())]);
        assert!(
            base_pos < daily_pos && daily_pos < derived_pos,
            "dependency order must be stable: base < daily < derived; got {:?}",
            e.scopes
        );
        assert!(e.unsupported.is_empty());
    }

    #[test]
    fn alias_resolution_through_cte_rename() {
        // A column renamed through a CTE projection resolves to its source
        // leaf in the consuming node's context.
        let sql = "WITH c AS (SELECT user_id AS uid FROM events) SELECT uid FROM c";

        struct LineageProbe;
        impl Transfer for LineageProbe {
            type Verdict = Vec<ColumnLineage>;
            fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Self::Verdict {
                Vec::new()
            }
            fn operator(
                &self,
                _op: &OpNode<'_>,
                _children: &[Self::Verdict],
                cx: &NodeCx,
            ) -> Self::Verdict {
                cx.columns.clone()
            }
        }

        let tree = QueryTree::from_sql(sql).expect("parses");
        let columns = walk(&tree, &LineageProbe);
        assert_eq!(columns.len(), 1, "one projected column; got {columns:?}");
        assert_eq!(columns[0].output, "uid");
        assert_eq!(
            columns[0].leaf,
            Some(LeafColumn {
                relation: "events".to_string(),
                column: "user_id".to_string(),
            }),
            "uid must chase through the CTE rename to events.user_id"
        );
    }

    #[test]
    fn unrecognised_from_construct_is_fail_loud() {
        // A table function in FROM is not yet a recognised leaf: the walk
        // must surface an explicit Unsupported entry, never an empty
        // enumeration that consumers could mistake for "no scopes, admit".
        let sql = "SELECT a FROM read_csv('data.csv')";
        let e = scopes_of(sql);
        assert!(
            !e.unsupported.is_empty(),
            "table function in FROM must yield an Unsupported entry"
        );
    }

    /// The walk-composed skew fold: a sessions-shaped model (the Form B
    /// relation lives in the outer scope, below a CTE) composes to a
    /// symmetric 1-day skew; an identity model composes to zero; and a
    /// relation buried inside a CTE body still surfaces at the root (the
    /// union fold across walk nodes).
    #[test]
    fn skew_fold_composes_across_scopes() {
        use super::super::source_bounds::{Seconds, Skew};

        let sessions_shaped = "WITH sessionized AS (\
             SELECT user_id, event_date, session_start_date FROM smelt.silver.events\
         ) \
         SELECT * FROM sessionized \
         WHERE event_date BETWEEN session_start_date - INTERVAL '1 day' \
             AND session_start_date + INTERVAL '1 day'";
        let skew = model_partition_skew(sessions_shaped, "session_start_date");
        assert_eq!(
            skew,
            Skew {
                before: Seconds::days(1),
                after: Seconds::days(1),
            },
            "sessions-shaped model must compose a symmetric 1-day skew"
        );

        let identity = "SELECT event_date, COUNT(*) AS n \
             FROM smelt.silver.events GROUP BY event_date";
        assert_eq!(
            model_partition_skew(identity, "event_date"),
            Skew::ZERO,
            "identity model must compose zero skew"
        );

        // The Form B relation inside a CTE body (its own walk node) must
        // reach the root verdict via the union fold.
        let nested = "WITH capped AS (\
             SELECT * FROM smelt.silver.events \
             WHERE event_date BETWEEN session_start_date - INTERVAL '2 days' \
                 AND session_start_date + INTERVAL '1 day'\
         ) \
         SELECT * FROM capped";
        let skew = model_partition_skew(nested, "session_start_date");
        assert_eq!(
            skew,
            Skew {
                before: Seconds::days(2),
                after: Seconds::days(1),
            },
            "a CTE-scope Form B relation must surface in the root verdict"
        );
    }

    #[test]
    fn reach_series_adds_parallel_maxes() {
        use super::super::source_bounds::{
            derive_model_bounds, BoundContext, BoundResult, Seconds,
        };

        // Stacked frames across a CTE boundary compose in SERIES: an output
        // row reads 3d of s7 values, each of which reads 7d of source rows —
        // true backward reach 10d, not max(7, 3) = 7.
        let stacked = "WITH seven AS (\
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS s7 \
             FROM smelt.sources.metrics\
         ) \
         SELECT d, MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3 \
         FROM seven";
        let ctx = BoundContext::new().with_source("sources.metrics", "d");
        let bounds = derive_model_bounds(stacked, &ctx);
        assert_eq!(
            bounds.get("sources.metrics"),
            Some(&BoundResult::Bounded {
                source_partition_col: "d".to_string(),
                before: Seconds::days(10),
                after: Seconds::ZERO,
            }),
            "stacked frames must series-add (7d + 3d = 10d)"
        );

        // Set-operation arms are PARALLEL: the source is read independently
        // by each arm, so the reach is the max across arms, not the sum.
        let unioned = "SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS x \
             FROM smelt.sources.metrics \
             UNION ALL \
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS x \
             FROM smelt.sources.metrics";
        let bounds = derive_model_bounds(unioned, &ctx);
        assert_eq!(
            bounds.get("sources.metrics"),
            Some(&BoundResult::Bounded {
                source_partition_col: "d".to_string(),
                before: Seconds::days(7),
                after: Seconds::ZERO,
            }),
            "set-op arms must parallel-max (max(7d, 3d) = 7d)"
        );

        // A symbolic (month/year) offset anywhere on the series path is
        // absorbing: the source's bound is NotDerivable, never approximated.
        let symbolic = "WITH seven AS (\
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '1 month' PRECEDING AND CURRENT ROW) AS s7 \
             FROM smelt.sources.metrics\
         ) \
         SELECT d, MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3 \
         FROM seven";
        let bounds = derive_model_bounds(symbolic, &ctx);
        assert_eq!(
            bounds.get("sources.metrics"),
            Some(&BoundResult::NotDerivable),
            "a symbolic offset in a series position must absorb to NotDerivable"
        );
    }

    /// Set-operation arms carry a per-branch trace VECTOR: arms anchored to
    /// different sources each keep their own verdict — there is no collapsed
    /// single-source reduction — and a `StaticSeed` arm keeps its per-branch
    /// refusal (the consumer's per-branch policy refuses that branch's push),
    /// never averaged away.
    #[test]
    fn set_op_branch_trace_vector() {
        use super::super::monotonicity::{
            trace_event_time_composed, ComposedTrace, EventTimeTrace,
        };
        use super::super::source_bounds::BoundContext;

        let ctx = BoundContext::new()
            .with_source("sources.a", "a_ts")
            .with_source("sources.b", "b_ts");

        let sql = "SELECT a_ts AS event_time FROM smelt.sources.a \
                   UNION ALL \
                   SELECT b_ts AS event_time FROM smelt.sources.b";
        match trace_event_time_composed(sql, "event_time", &ctx, false) {
            Some(ComposedTrace::Branches(traces)) => {
                assert_eq!(traces.len(), 2, "one trace per arm; got {traces:?}");
                match (&traces[0], &traces[1]) {
                    (
                        EventTimeTrace::Traceable { source: s0, .. },
                        EventTimeTrace::Traceable { source: s1, .. },
                    ) => {
                        assert_eq!(s0, "sources.a");
                        assert_eq!(s1, "sources.b");
                    }
                    other => {
                        panic!("each branch must trace to its own source, got {other:?}")
                    }
                }
            }
            other => panic!("expected a per-branch trace vector, got {other:?}"),
        }

        let sql = "SELECT a_ts AS event_time FROM smelt.sources.a \
                   UNION ALL \
                   SELECT NULL AS event_time FROM smelt.sources.b";
        match trace_event_time_composed(sql, "event_time", &ctx, false) {
            Some(ComposedTrace::Branches(traces)) => {
                assert_eq!(traces.len(), 2);
                assert!(matches!(traces[0], EventTimeTrace::Traceable { .. }));
                assert!(
                    matches!(traces[1], EventTimeTrace::StaticSeed { .. }),
                    "the seed branch must keep its own refusal in the vector, got {:?}",
                    traces[1]
                );
            }
            other => {
                panic!("expected a per-branch vector with the seed branch preserved, got {other:?}")
            }
        }
    }

    #[test]
    fn chained_join_bands_add_along_path() {
        use super::super::source_bounds::{
            derive_model_bounds, BoundContext, BoundResult, Seconds,
        };

        // Chained interval-join bands: b within 1d of a, c within 2d of b —
        // source c's reach relative to the run window is 3d. Structurally the
        // chain nests (the inner hop is a CTE the outer hop joins), so the
        // series-add composes the bands along the path.
        let sql = "WITH ab AS (\
             SELECT b.d AS d, b.v AS v \
             FROM smelt.sources.a a \
             JOIN smelt.sources.b b ON b.d >= a.d - INTERVAL '1 day' AND b.d <= a.d\
         ) \
         SELECT c.d, c.v \
         FROM ab \
         JOIN smelt.sources.c c ON c.d >= ab.d - INTERVAL '2 days' AND c.d <= ab.d";
        let ctx = BoundContext::new()
            .with_source("sources.a", "d")
            .with_source("sources.b", "d")
            .with_source("sources.c", "d");
        let bounds = derive_model_bounds(sql, &ctx);
        assert_eq!(
            bounds.get("sources.c"),
            Some(&BoundResult::Bounded {
                source_partition_col: "d".to_string(),
                before: Seconds::days(3),
                after: Seconds::ZERO,
            }),
            "the far source's bands must add along the join path (1d + 2d = 3d)"
        );
        // The nearer sources stay bounded (the composition may widen them —
        // a wider scan is sound — but must never lose their bound).
        for src in ["sources.a", "sources.b"] {
            assert!(
                matches!(bounds.get(src), Some(BoundResult::Bounded { .. })),
                "{src} must stay Bounded; got {:?}",
                bounds.get(src)
            );
        }
    }

    #[test]
    fn expression_subquery_reference_site_keeps_flat_scan_floor() {
        use super::super::source_bounds::{
            derive_model_bounds, BoundContext, BoundResult, Seconds,
        };

        // `metrics` is referenced twice: once as a plain FROM leaf (the outer
        // JOIN, zero margin) and once inside an expression-position scalar
        // subquery carrying a 30-day lookback band. The subquery is not a walk
        // node, so the leaf-path walk verdict only carries the zero-margin FROM
        // reach; the flat-scan floor must widen it back to 30 days so the
        // injected scan filter covers the subquery's reference site.
        let sql = "WITH agg AS (\
             SELECT u.d, \
                    (SELECT MAX(m.v) FROM smelt.sources.metrics m \
                      WHERE m.d >= u.d - INTERVAL '30 days' AND m.d <= u.d) AS peak \
             FROM smelt.sources.activity u\
         ) \
         SELECT r.d, r.v, a.peak \
         FROM smelt.sources.metrics r JOIN agg a ON a.d = r.d";
        let ctx = BoundContext::new()
            .with_source("sources.metrics", "d")
            .with_source("sources.activity", "d");
        let bounds = derive_model_bounds(sql, &ctx);
        match bounds.get("sources.metrics") {
            Some(BoundResult::Bounded { before, .. }) => {
                assert!(
                    *before >= Seconds::days(30),
                    "metrics scan must cover the subquery's 30-day band; got before={:?}",
                    before
                );
            }
            other => panic!("expected metrics Bounded with >=30d before, got {other:?}"),
        }
    }

    // ===== Property-vector transfer functions =====

    fn vector_of(sql: &str) -> PropertyVector {
        model_property_vector(sql, &JoinContext::new()).expect("model parses to a SELECT")
    }

    fn keyset(cols: &[&str]) -> std::collections::BTreeSet<String> {
        cols.iter().map(|c| c.to_ascii_lowercase()).collect()
    }

    #[test]
    fn group_by_establishes_grain_and_fds() {
        // The GROUP BY factory: the grouping key uniquely identifies an output
        // row, so `customer_id → every output column` by construction (§3.5).
        let v =
            vector_of("SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id");

        assert!(
            v.grain.keys.iter().any(
                |k| keyset(&k.iter().map(String::as_str).collect::<Vec<_>>())
                    == keyset(&["customer_id"])
            ),
            "GROUP BY customer_id must establish [customer_id] as a grain key; got {:?}",
            v.grain
        );
        assert!(
            v.fds
                .iter()
                .any(|fd| fd.key == vec!["customer_id".to_string()] && fd.determines == "total"),
            "the factory must carry customer_id → total; got {:?}",
            v.fds
        );
    }

    #[test]
    fn union_all_drops_grain_and_fds_unless_discriminated() {
        // Both arms keyed by [customer_id], but the union has no discriminator
        // — the same customer_id may appear in both arms, so the union is
        // unkeyed (§3.8: FD/grain destroyed by a bare UNION ALL).
        let undiscriminated = vector_of(
            "SELECT customer_id FROM crm_a GROUP BY customer_id \
             UNION ALL \
             SELECT customer_id FROM crm_b GROUP BY customer_id",
        );
        assert!(
            undiscriminated.grain.keys.is_empty(),
            "a bare UNION ALL of two keyed arms is unkeyed; got {:?}",
            undiscriminated.grain
        );
        assert!(
            undiscriminated.has_set_op_barrier,
            "the union node must record its FD barrier"
        );

        // A distinct literal tag column per arm, added to the key, makes the
        // arms provably disjoint — (src, customer_id) survives as a key.
        let discriminated = vector_of(
            "SELECT 'a' AS src, customer_id FROM crm_a GROUP BY customer_id \
             UNION ALL \
             SELECT 'b' AS src, customer_id FROM crm_b GROUP BY customer_id",
        );
        assert!(
            discriminated.grain.keys.iter().any(|k| keyset(
                &k.iter().map(String::as_str).collect::<Vec<_>>()
            ) == keyset(&["src", "customer_id"])),
            "a literal discriminator in the key preserves the union key; got {:?}",
            discriminated.grain
        );
    }

    #[test]
    fn determinism_predicate_registered_as_leaf() {
        // clean ∪ clean = clean across a UNION ALL (columnar union lub).
        let clean =
            vector_of("SELECT user_id FROM events_a UNION ALL SELECT user_id FROM events_b");
        assert_eq!(
            clean
                .determinism
                .iter()
                .find(|d| d.output == "user_id")
                .map(|d| d.level),
            Some(Determinism::Clean),
            "clean ∪ clean must stay clean; got {:?}",
            clean.determinism
        );

        // A row-nondeterministic function taints its column (leaf predicate).
        let row = vector_of("SELECT random() AS r FROM t");
        assert_eq!(
            row.determinism
                .iter()
                .find(|d| d.output == "r")
                .map(|d| d.level),
            Some(Determinism::Row),
            "random() must classify Row; got {:?}",
            row.determinism
        );

        // A run-deterministic clock is a per-run constant, not row-tainted.
        let run = vector_of("SELECT now() AS t FROM src");
        assert_eq!(
            run.determinism
                .iter()
                .find(|d| d.output == "t")
                .map(|d| d.level),
            Some(Determinism::Run),
            "now() must classify Run; got {:?}",
            run.determinism
        );
    }

    // ===== Change-comparability lattice fold (P3) =====

    fn comparability_of<'a>(v: &'a PropertyVector, output: &str) -> Option<&'a Comparability> {
        v.comparability
            .iter()
            .find(|c| c.output == output)
            .map(|c| &c.comparability)
    }

    #[test]
    fn comparability_leaf_classification() {
        // A plain aggregate column is a pure function of processed inputs —
        // comparable across runs.
        let agg =
            vector_of("SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id");
        assert_eq!(
            comparability_of(&agg, "total"),
            Some(&Comparability::Comparable),
            "a plain SUM aggregate must be Comparable; got {:?}",
            agg.comparability
        );

        // A run-pinned clock is comparable *within* a run but not *across*
        // runs — Incomparable.
        let run = vector_of("SELECT now() AS t FROM src");
        assert_eq!(
            comparability_of(&run, "t"),
            Some(&Comparability::Incomparable),
            "now() must be Incomparable across runs; got {:?}",
            run.comparability
        );

        // A row-nondeterministic value is unpinnable — Incomparable.
        let row = vector_of("SELECT random() AS r FROM t");
        assert_eq!(
            comparability_of(&row, "r"),
            Some(&Comparability::Incomparable),
            "random() must be Incomparable; got {:?}",
            row.comparability
        );

        // An unrecognised (opaque) function call is not known to be a pure
        // deterministic function of its inputs — fail-closed to Incomparable,
        // never a default Comparable.
        let opaque = vector_of("SELECT my_opaque_udf(amount) AS y FROM orders");
        assert_eq!(
            comparability_of(&opaque, "y"),
            Some(&Comparability::Incomparable),
            "an unrecognised function call must fail-closed to Incomparable; got {:?}",
            opaque.comparability
        );
    }

    #[test]
    fn comparability_folds_through_cte() {
        // A NOW()-tainted CTE column, read plainly by the outer scope, must
        // still be Incomparable — comparability composes through the walk's
        // CTE reduction, not re-derived from the outer scope's own text.
        let v = vector_of(
            "WITH staged AS (SELECT now() AS t, customer_id FROM src) \
             SELECT t, customer_id FROM staged",
        );
        assert_eq!(
            comparability_of(&v, "t"),
            Some(&Comparability::Incomparable),
            "a NOW()-tainted CTE column must stay Incomparable through the outer read; got {:?}",
            v.comparability
        );
        assert_eq!(
            comparability_of(&v, "customer_id"),
            Some(&Comparability::Comparable),
            "a plain passthrough CTE column must stay Comparable; got {:?}",
            v.comparability
        );
    }

    #[test]
    fn comparability_union_lub() {
        // A column comparable in one arm and incomparable in the other folds
        // Incomparable (the union operator rule = lub, same shape as
        // determinism's per-position max).
        let v = vector_of(
            "SELECT customer_id AS id FROM crm_a \
             UNION ALL \
             SELECT now() AS id FROM crm_b",
        );
        assert_eq!(
            comparability_of(&v, "id"),
            Some(&Comparability::Incomparable),
            "one Incomparable arm must dominate the union; got {:?}",
            v.comparability
        );
    }
}
