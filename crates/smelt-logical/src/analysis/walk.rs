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

use super::{item_expr, resolve_scope_group_by, select_stmt_items};

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
}
