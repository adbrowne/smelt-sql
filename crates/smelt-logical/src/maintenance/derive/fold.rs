use super::*;

/// Caller-supplied fold admission input for a keyed-grain model: which
/// columns fold additively, each under its **own** combiner (a mixed fold —
/// e.g. `COUNT`→`SUM`, `MIN`→`MIN`, `MAX`→`MAX` composed over the same
/// key — is the common shape, not a single shared combiner across every
/// column). Checked against the combiner-algebra classifier per column —
/// never trusted bare.
#[derive(Debug, Clone)]
pub struct FoldSpec {
    pub add_columns: Vec<(String, SqlFunction)>,
}

/// Whether `source` contributes to `sql`'s cumulative fold — i.e. appears as
/// an argument to one of the fold's own aggregate expressions
/// (`model_properties.md` §"Faithful-fold conditions";
/// `docs/specs/incremental_shapes.md` §"The key grain (`grain: key`)" binds
/// the `NewData` append-only obligation to exactly the sources this
/// classifier answers `true` for, not every source the model references).
/// [`FoldSpec`] itself carries no source attribution (`add_columns` is
/// `(alias, combiner)` only), so this re-parses `sql`'s own outermost
/// `SELECT` to see which FROM source backs each aggregate's arguments.
///
/// **Leaf classifier** — runs over the model's own already-bounded fold
/// body (the outermost `SELECT`'s own aggregate expressions and FROM-alias
/// map); feeds admission; never composes across nodes. Matches
/// `maintenance::grouping`'s per-column mutation-sensitivity classifier's
/// own v0 scope restriction: single top-level `SELECT` scope only, no
/// CTE/derived-table chase (`docs/specs/architecture.md` §"Property
/// composition walk rule" — admissible as a leaf classifier invoked over one
/// already-bounded node's own text, not a competing whole-model scan).
///
/// **Conservative by construction — false negatives are forbidden.** Every
/// case this classifier cannot resolve cleanly — an unqualified column
/// reference ambiguous among more than one joined FROM source, a qualifier
/// that does not resolve to a named FROM alias, a derived-table/subquery
/// FROM item, or a CTE/set-operation composing the fold body through more
/// than this one scope — answers `true` ("contributes"), never `false`. A
/// false positive here only costs permissiveness (the caller's separate
/// `UpstreamMutation`-coverage check still has to hold before anything is
/// admitted); a false negative would silently let an un-retractable folded
/// contribution through, which is exactly the admission hole this predicate
/// exists to close.
pub fn source_contributes_to_fold(sql: &str, source: &str) -> bool {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return true;
    };
    let Some(select) = file.select_stmt() else {
        return true;
    };
    let Some(items) = select_stmt_items(&select) else {
        return true;
    };

    // A CTE or set operation composes the fold body through more than one
    // scope — outside this leaf classifier's single-scope resolution
    // (matches `maintenance::grouping`'s own v0 restriction). Cannot prove
    // `source` absent from a scope this classifier does not chase into.
    if select.with_clause().is_some() || select.has_set_operation() {
        return true;
    }

    let Some(from_clause) = select.from_clause() else {
        return true;
    };

    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for table_ref in from_clause
        .table_refs()
        .chain(from_clause.joins().filter_map(|j| j.table_ref()))
    {
        if table_ref.subquery().is_some() {
            // A derived table this classifier does not chase into — cannot
            // prove it doesn't itself surface `source`.
            return true;
        }
        let Some(resolved) = resolve_table_ref_source_name(&table_ref) else {
            return true;
        };
        let bare = resolved
            .strip_prefix("sources.")
            .unwrap_or(resolved.as_str())
            .to_string();
        let key = table_ref
            .alias()
            .unwrap_or_else(|| bare.clone())
            .to_ascii_lowercase();
        aliases.insert(key, bare);
    }

    let source_lower = source.to_ascii_lowercase();
    // `source` does not even appear as a FROM item under any name/alias
    // this classifier resolved: the fold body cannot read it at all, under
    // any qualifier, from this scope.
    if !aliases
        .values()
        .any(|v| v.eq_ignore_ascii_case(&source_lower))
    {
        return false;
    }

    for item in &items {
        let is_aggregate = matches!(
            item,
            SelectItemKind::OtherAggregate { .. } | SelectItemKind::CountDistinct { .. }
        );
        if !is_aggregate {
            continue;
        }
        for cref in collect_fold_column_refs(item_expr(item)) {
            let resolved = match cref.qualifier() {
                Some(q) => aliases.get(&q.to_ascii_lowercase()).cloned(),
                // Unqualified inside an aggregate argument: resolvable only
                // when exactly one source is joined in this scope — with
                // more than one, it is ambiguous and cannot be proven not
                // to be `source`.
                None if aliases.len() == 1 => aliases.values().next().cloned(),
                None => None,
            };
            match resolved {
                Some(name) if name.eq_ignore_ascii_case(&source_lower) => return true,
                Some(_) => continue,
                // A qualifier this FROM-alias scan didn't resolve — cannot
                // prove it is not an alias for `source`.
                None => return true,
            }
        }
    }
    false
}

/// Recursively collect every simple (possibly qualified) column reference
/// inside `expr` — a leaf classifier over one already-parsed aggregate
/// argument's own syntax tree, mirroring `maintenance::grouping`'s private
/// helper of the same shape (kept local; that one is not `pub`).
fn collect_fold_column_refs(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_fold_column_refs_rec(expr.syntax(), &mut out);
    out
}

fn collect_fold_column_refs_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    if node.kind() == smelt_parser::SyntaxKind::EXPRESSION {
        if let Some(e) = Expr::cast(node.clone()) {
            if let Some(cref) = ColumnRef::from_expr(&e) {
                out.push(cref);
                return;
            }
        }
    }
    for child in node.children() {
        collect_fold_column_refs_rec(&child, out);
    }
}
