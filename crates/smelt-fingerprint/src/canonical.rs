//! Canonical normal form of a model's SELECT.
//!
//! The form is a structured value so each part can be normalised independently
//! and (in later phases) rewritten — projection reorder, alias rename, CTE
//! inline. Every field is a normalised token string with trivia
//! (whitespace/comments) stripped, which is what makes formatting changes
//! collapse.
//!
//! Soundness over completeness: anything the builder cannot safely canonicalise
//! drops the whole query to a [`Canon::Verbatim`] hash of its normalised token
//! stream — sound (a change always re-fingerprints) but conservative.

use smelt_parser::ast::{Cte, FromClause, SelectItem, SelectStmt, TableRef, WithClause};
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::hash::Encoder;
use crate::MissedReuse;

/// The canonicalised query, either fully structured or a conservative verbatim
/// fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Canon {
    Structured(CanonForm),
    /// Normalised token stream of the whole statement; used when the builder
    /// declines to canonicalise structurally.
    Verbatim(String),
}

/// Top-level projection. By-name when output column order is not observable
/// (the common case for a model boundary, matched downstream by name); an
/// ordered list when position is observable or names are ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Projection {
    ByName(BTreeMap<String, String>),
    Ordered(Vec<(Option<String>, String)>),
}

/// Structured normal form of a single-block SELECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonForm {
    /// Normalised `WITH` clause (replaced by inlining in a later phase).
    pub with: Option<String>,
    pub projection: Projection,
    /// Output column name → type rendering, folded only when a schema was
    /// supplied. Empty otherwise.
    pub types: BTreeMap<String, String>,
    /// Normalised `FROM` clause.
    pub source: Option<String>,
    /// Normalised `WHERE` predicate (keyword stripped).
    pub filter: Option<String>,
    /// Normalised `GROUP BY` keys (order-insensitive).
    pub group_by: BTreeSet<String>,
    /// Normalised `HAVING` predicate.
    pub having: Option<String>,
    pub distinct: bool,
}

/// Result of building the canonical form.
pub(crate) struct Built {
    pub canon: Canon,
    pub missed: Vec<MissedReuse>,
}

/// Normalise a CST node to its leaf token stream with trivia removed, tokens
/// joined by a single space. This is the whitespace/comment-insensitive core.
pub(crate) fn norm(node: &SyntaxNode) -> String {
    let mut out = String::new();
    for tok in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        if tok.kind().is_trivia() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        // SQL keywords are case-insensitive, so fold them to a canonical case.
        // Identifiers and literals are left verbatim: quoted identifiers are
        // case-sensitive and literal spelling can carry semantics (decimal
        // scale), so normalising them would risk unsoundness.
        if tok.kind().is_keyword() {
            out.push_str(&tok.text().to_ascii_lowercase());
        } else {
            out.push_str(tok.text());
        }
    }
    out
}

fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|c| c.kind() == kind)
}

/// Map from an internal binding name to its positional canonical name.
type AliasMap = HashMap<String, String>;

/// Collect the internal binding names of `select` — top-level CTE names and
/// top-level FROM aliases — and assign each a positional canonical name `r0`,
/// `r1`, … in deterministic binding order (CTEs first in `WITH` order, then
/// FROM aliases in `FROM` order).
///
/// Returns `(map, safe)`. `safe` is `false` for a recursive `WITH`, which must
/// not be alias-renamed (a recursive CTE references its own binding name, so
/// renaming changes meaning); the caller falls back to verbatim.
fn collect_alias_map(select: &SelectStmt) -> (AliasMap, bool) {
    let mut map = AliasMap::new();
    let mut next = 0usize;

    if let Some(with) = select.syntax().children().find_map(WithClause::cast) {
        if with.is_recursive() {
            return (map, false);
        }
        for cte in with.ctes() {
            if let Some(name) = cte.name() {
                map.entry(name).or_insert_with(|| {
                    let r = format!("r{next}");
                    next += 1;
                    r
                });
            }
        }
    }

    if let Some(from) = select.syntax().children().find_map(FromClause::cast) {
        for tref in from.table_refs() {
            if let Some(alias) = tref.alias() {
                map.entry(alias).or_insert_with(|| {
                    let r = format!("r{next}");
                    next += 1;
                    r
                });
            }
        }
    }

    (map, true)
}

/// Normalise a node to its trivia-free token stream, lowercasing keywords and
/// alpha-renaming binding names from `map` to their canonical positional names.
///
/// An identifier whose text is in `map` is renamed only in a position where it
/// denotes the binding, never where it denotes a column:
/// - immediately before `.` (a qualifier) — renamed;
/// - immediately after `.` (a column under a qualifier) — never renamed;
/// - immediately before `AS` (a CTE binding name) — renamed;
/// - immediately after `AS` (an alias definition) — renamed;
/// - a bare identifier inside the `FROM` clause (a table reference), when
///   `in_from` is set — renamed.
///
/// Everything else is left verbatim, which keeps the rename sound: a column that
/// merely shares a name with an alias is never rewritten.
fn norm_aliased(node: &SyntaxNode, map: &AliasMap, in_from: bool) -> String {
    use SyntaxKind::{AS_KW, DOT, IDENT};

    let toks: Vec<_> = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();

    let mut out = String::new();
    for (i, tok) in toks.iter().enumerate() {
        if !out.is_empty() {
            out.push(' ');
        }
        if tok.kind() == IDENT {
            if let Some(canon) = map.get(tok.text()) {
                let prev = i.checked_sub(1).and_then(|p| toks.get(p)).map(|t| t.kind());
                let next = toks.get(i + 1).map(|t| t.kind());
                let is_qualifier = next == Some(DOT);
                let is_column_segment = prev == Some(DOT);
                let is_binding = next == Some(AS_KW);
                let is_alias_def = prev == Some(AS_KW);
                let is_from_table_ref = in_from && !is_column_segment;
                let rename = !is_column_segment
                    && (is_qualifier || is_binding || is_alias_def || is_from_table_ref);
                if rename {
                    out.push_str(canon);
                    continue;
                }
            }
        }
        if tok.kind().is_keyword() {
            out.push_str(&tok.text().to_ascii_lowercase());
        } else {
            out.push_str(tok.text());
        }
    }
    out
}

/// Normalise `node`, applying alias renaming only when `map` is non-empty.
fn nf(node: &SyntaxNode, map: &AliasMap, in_from: bool) -> String {
    if map.is_empty() {
        norm(node)
    } else {
        norm_aliased(node, map, in_from)
    }
}

/// First descendant `SELECT` of `node` (preorder) — the body of a subquery or
/// CTE.
fn first_select(node: &SyntaxNode) -> Option<SelectStmt> {
    node.descendants().find_map(SelectStmt::cast)
}

/// Recursive fingerprint of a nested `SELECT`, hex-encoded, used to represent a
/// FROM subquery by its content rather than its written form. Soundness is
/// inherited: equal nested fingerprints denote equal nested relations.
fn subquery_fp_hex(inner: &SelectStmt) -> String {
    let bytes = build(inner, &[]).canon.fingerprint();
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Attempt to canonicalise a single-subquery FROM by content. Returns
/// `Some((with, source))` with rewritten fields when it applies, else `None`.
///
/// Bounded to the sound, common case: the FROM has exactly one table reference,
/// which is either a written derived table or a reference to the query's single,
/// single-use CTE. Joins, multiple tables, and multi-CTE queries keep the flat
/// representation — incomplete, never unsound (a single-use CTE referenced once
/// is semantically identical to inlining its body as a derived table).
fn try_inline(
    select: &SelectStmt,
    alias_map: &AliasMap,
    missed: &mut Vec<MissedReuse>,
) -> Option<(Option<String>, String)> {
    let from = select.syntax().children().find_map(FromClause::cast)?;
    // A join's tables live in `JOIN_CLAUSE` nodes, not as direct `table_refs()`
    // of the FROM, so a join leaves `table_refs().len() == 1`. Bail explicitly:
    // inlining the single derived table here would represent the query by that
    // subquery alone and silently drop the join — a false equivalence.
    if from.joins().next().is_some() {
        return None;
    }
    let trefs: Vec<TableRef> = from.table_refs().collect();
    if trefs.len() != 1 {
        return None;
    }
    let tref = &trefs[0];

    // Case 1: a written derived table `(Q) AS alias`. Canonicalise the subquery;
    // a derived table removes no CTE, so WITH is preserved.
    if tref.subquery().is_some() {
        let inner = first_select(tref.syntax())?;
        let alias = tref.alias()?;
        let canon = alias_map.get(&alias).cloned().unwrap_or(alias);
        let with = select
            .syntax()
            .children()
            .find_map(WithClause::cast)
            .map(|n| nf(n.syntax(), alias_map, false));
        return Some((
            with,
            format!("from subq:{} as {canon}", subquery_fp_hex(&inner)),
        ));
    }

    // Case 2: a reference to the query's single, single-use CTE. Inline its body
    // and drop the now-empty WITH.
    let with = select.syntax().children().find_map(WithClause::cast)?;
    let ctes: Vec<Cte> = with.ctes().collect();
    if ctes.len() != 1 {
        return None;
    }
    let cte = &ctes[0];
    let cte_name = cte.name()?;
    if tref.identifier().as_deref() != Some(cte_name.as_str()) {
        return None;
    }
    let inner = first_select(cte.syntax())?;
    let canon = alias_map.get(&cte_name).cloned().unwrap_or(cte_name);
    missed.push(MissedReuse {
        reason: "inlined single-use CTE into FROM".into(),
    });
    Some((
        None,
        format!("from subq:{} as {canon}", subquery_fp_hex(&inner)),
    ))
}

/// Build the canonical form of `select`, folding `schema` (name → type) when
/// non-empty.
pub(crate) fn build(select: &SelectStmt, schema: &[(String, String)]) -> Built {
    let node = select.syntax();
    let mut missed = Vec::new();

    // Set operations make top-level column position observable across branches;
    // do not collapse the projection by name. Conservative: verbatim.
    if select.has_set_operation() {
        missed.push(MissedReuse {
            reason: "set operation (UNION/INTERSECT/EXCEPT) — verbatim fallback".into(),
        });
        return Built {
            canon: Canon::Verbatim(norm(node)),
            missed,
        };
    }

    let (alias_map, safe) = collect_alias_map(select);
    if !safe {
        missed.push(MissedReuse {
            reason: "recursive WITH — verbatim fallback".into(),
        });
        return Built {
            canon: Canon::Verbatim(norm(node)),
            missed,
        };
    }

    let projection = match build_projection(node, &alias_map, &mut missed) {
        Some(p) => p,
        None => {
            return Built {
                canon: Canon::Verbatim(norm(node)),
                missed,
            }
        }
    };

    let mut with = child(node, SyntaxKind::WITH_CLAUSE).map(|n| nf(&n, &alias_map, false));
    let mut source = child(node, SyntaxKind::FROM_CLAUSE).map(|n| nf(&n, &alias_map, true));

    // CTE inline / derived-table canonicalisation: when the FROM is a single
    // subquery — a written derived table OR a single-use CTE — represent it by
    // the *recursive* fingerprint of its inner SELECT. This collapses the
    // distinction between `WITH c AS (Q) … FROM c` and `… FROM (Q) AS c`.
    if let Some((new_with, new_source)) = try_inline(select, &alias_map, &mut missed) {
        with = new_with;
        source = Some(new_source);
    }
    let filter = child(node, SyntaxKind::WHERE_CLAUSE).map(|n| norm_clause_body(&n, &alias_map));
    let having = child(node, SyntaxKind::HAVING_CLAUSE).map(|n| norm_clause_body(&n, &alias_map));
    let group_by = child(node, SyntaxKind::GROUP_BY_CLAUSE)
        .map(|n| group_keys(n, &alias_map))
        .unwrap_or_default();

    let types: BTreeMap<String, String> =
        schema.iter().map(|(n, t)| (n.clone(), t.clone())).collect();

    Built {
        canon: Canon::Structured(CanonForm {
            with,
            projection,
            types,
            source,
            filter,
            group_by,
            having,
            distinct: select.is_distinct(),
        }),
        missed,
    }
}

/// Build the by-name projection. Falls back to `None` (⇒ verbatim) when a
/// wildcard is present (cannot enumerate columns without schema resolution) or a
/// column has no determinable output name, and to [`Projection::Ordered`] when
/// two items share an output name (a by-name map would silently drop one).
fn build_projection(
    select_node: &SyntaxNode,
    alias_map: &AliasMap,
    missed: &mut Vec<MissedReuse>,
) -> Option<Projection> {
    let list = child(select_node, SyntaxKind::SELECT_LIST)?;

    let mut ordered: Vec<(Option<String>, String)> = Vec::new();
    for item_node in list
        .children()
        .filter(|c| c.kind() == SyntaxKind::SELECT_ITEM)
    {
        let item = SelectItem::cast(item_node.clone())?;
        if item.is_wildcard() {
            missed.push(MissedReuse {
                reason: "wildcard projection — verbatim fallback".into(),
            });
            return None;
        }
        let expr_norm = match item.expression() {
            Some(e) => nf(e.syntax(), alias_map, false),
            None => nf(&item_node, alias_map, false),
        };
        ordered.push((item.column_name(), expr_norm));
    }

    // Any column without a determinable name → cannot key by name.
    if ordered.iter().any(|(name, _)| name.is_none()) {
        missed.push(MissedReuse {
            reason: "projection column without a determinable output name".into(),
        });
        return Some(Projection::Ordered(ordered));
    }

    // Duplicate output names → by-name map would drop a column.
    let mut seen = BTreeSet::new();
    let mut dup = false;
    for (name, _) in &ordered {
        if let Some(n) = name {
            if !seen.insert(n.clone()) {
                dup = true;
            }
        }
    }
    if dup {
        missed.push(MissedReuse {
            reason: "duplicate output column names — order-sensitive projection".into(),
        });
        return Some(Projection::Ordered(ordered));
    }

    let map = ordered
        .into_iter()
        .map(|(name, expr)| (name.unwrap(), expr))
        .collect();
    Some(Projection::ByName(map))
}

/// Normalise a clause node's body, dropping the leading keyword token(s)
/// (`WHERE`, `HAVING`). The body is everything after the first keyword.
fn norm_clause_body(clause: &SyntaxNode, alias_map: &AliasMap) -> String {
    // The clause's first non-trivia token is the keyword; the predicate is the
    // child EXPRESSION (or remaining tokens). Normalise the child expression
    // node if present, else fall back to the full clause text minus keyword.
    if let Some(expr) = clause
        .children()
        .find(|c| c.kind() == SyntaxKind::EXPRESSION)
    {
        return nf(&expr, alias_map, false);
    }
    // Fallback: drop the first non-trivia token (the keyword).
    let mut out = String::new();
    let mut skipped_kw = false;
    for tok in clause
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        if tok.kind().is_trivia() {
            continue;
        }
        if !skipped_kw {
            skipped_kw = true;
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(tok.text());
    }
    out
}

/// Collect `GROUP BY` keys as a set of normalised expressions (order-insensitive).
fn group_keys(clause: SyntaxNode, alias_map: &AliasMap) -> BTreeSet<String> {
    clause
        .children()
        .filter(|c| c.kind() == SyntaxKind::EXPRESSION)
        .map(|e| nf(&e, alias_map, false))
        .collect()
}

impl Canon {
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut enc = Encoder::new();
        match self {
            Canon::Verbatim(s) => {
                enc.tag("verbatim");
                enc.field("sql", s);
            }
            Canon::Structured(f) => {
                enc.tag("structured");
                if let Some(w) = &f.with {
                    enc.field("with", w);
                }
                match &f.projection {
                    Projection::ByName(map) => {
                        enc.tag("proj_by_name");
                        for (name, expr) in map {
                            enc.field("col", name);
                            enc.field("expr", expr);
                        }
                    }
                    Projection::Ordered(items) => {
                        enc.tag("proj_ordered");
                        for (name, expr) in items {
                            enc.field("col", name.as_deref().unwrap_or(""));
                            enc.field("expr", expr);
                        }
                    }
                }
                for (name, ty) in &f.types {
                    enc.field("type_col", name);
                    enc.field("type", ty);
                }
                if let Some(s) = &f.source {
                    enc.field("from", s);
                }
                if let Some(s) = &f.filter {
                    enc.field("where", s);
                }
                for g in &f.group_by {
                    enc.field("group", g);
                }
                if let Some(s) = &f.having {
                    enc.field("having", s);
                }
                if f.distinct {
                    enc.tag("distinct");
                }
            }
        }
        enc.finish()
    }
}
