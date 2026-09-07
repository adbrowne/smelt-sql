//! Registry-first inference for the [`REGISTRY_MIGRATED`] allowlist of
//! built-ins, plus the grouped-nullability helpers it depends on.

use smelt_parser::ast::{Expr, FunctionCall, SelectStmt};
use smelt_types::signatures::{unify_call_with_expected, BuiltinRegistry, ExprKind};
use smelt_types::{DataType, TypedColumn};

use crate::type_inference::type_context::TypeContext;
use crate::type_inference::{infer_expression_kind, infer_expression_type, promote_types};

/// LUB adapter: the canonical numeric-promotion routine lives in
/// [`promote_types`] (this module) but signatures-side [`unify_call`] needs a
/// plain `Fn(&DataType, &DataType) -> DataType`. This wrapper keeps
/// `smelt-types` dependency-free per the plan's cross-phase design choice.
fn registry_lub(a: &DataType, b: &DataType) -> DataType {
    let lhs = TypedColumn {
        data_type: a.clone(),
        nullable: true,
    };
    let rhs = TypedColumn {
        data_type: b.clone(),
        nullable: true,
    };
    promote_types(&lhs, &rhs).data_type
}

/// Names whose typing is driven registry-first (via [`try_registry_inference`])
/// rather than by the hand-written `match` in [`infer_function_type`](super::legacy::infer_function_type).
/// Every entry's `BuiltinRegistry` signature reproduces the legacy arm's return type,
/// and [`registry_result_nullable`] reproduces its nullability. Names NOT on
/// this list stay on the legacy path; that residual set is the
/// `legacy_match_ratchet` count and shrinks as functions migrate.
///
/// Excluded on purpose (the **named exception list** — their return type or
/// nullability depends on argument types/values in a way a static `Signature`
/// cannot express, so they remain hand-written):
///   * `SUM` — precision/width widening (`SUM(DECIMAL(p,s)) → DECIMAL(38,s)`).
///   * `CEIL`/`CEILING`/`FLOOR` — `DECIMAL(p,s) → DECIMAL(p,0)`, else Double.
///   * `ROUND`/`TRUNC`/`TRUNCATE`/`MOD` — first-argument identity else Double/Integer.
///   * `MEDIAN` — integer inputs widen to Double, others keep their type.
///   * `ARRAY_AGG` — wraps the argument type in `Array<…>`.
///   * `DATE_TRUNC` — return tz-axis mirrors the second argument.
///   * `COALESCE`/`IFNULL`/`NULLIF`/`GREATEST`/`LEAST` — first-concrete-of-N
///     with argument-derived nullability.
///   * `MODE`/`ANY_VALUE`/`FIRST`/`LAST` and the window-navigation
///     family (`LAG`/`LEAD`/`FIRST_VALUE`/`LAST_VALUE`/`NTH_VALUE`) — first-
///     argument identity with optional trailing arguments (variable arity).
///     `ARG_MAX`/`ARG_MIN` (aliases `MAX_BY`/`MIN_BY`) migrated below — fixed
///     2-arity, first-argument-identity return type, no trailing arguments.
///   * `BIT_AND`/`BIT_OR`/`BIT_XOR` — first-argument identity else BigInt.
///   * `EXTRACT` — routed as `ExtractExpr` syntax, not a plain call.
///
/// The list is ordered by family for easy review.
pub(super) const REGISTRY_MIGRATED: &[&str] = &[
    // ── Aggregates ──────────────────────────────────────────────────────────
    "AVG",   // <T: Numeric>(T) → Double
    "MIN",   // <T: Ordered>(T) → T (first-arg identity, nullable=true)
    "MAX",   // <T: Ordered>(T) → T
    "COUNT", // (Any) → BigInt (nullable=false)
    "STDDEV",
    "STDDEV_POP",
    "STDDEV_SAMP",
    "VARIANCE",
    "VAR_POP",
    "VAR_SAMP",
    "CORR",
    "COVAR_POP",
    "COVAR_SAMP",
    "REGR_SLOPE",
    "PERCENTILE_CONT",
    "PERCENTILE_DISC",
    "BOOL_AND",
    "BOOL_OR",
    "EVERY",
    "STRING_AGG",
    "LISTAGG",
    "GROUP_CONCAT",
    "APPROX_COUNT_DISTINCT", // (Any) → BigInt (nullable=false)
    "ARG_MAX",               // <T: Any>(T, K) → T (first-arg identity, nullable=true)
    "ARG_MIN",               // <T: Any>(T, K) → T (first-arg identity, nullable=true)
    // ── Window (fixed return) ───────────────────────────────────────────────
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "NTILE",
    "CUME_DIST",
    "PERCENT_RANK",
    // ── Arithmetic / numeric scalars (fixed return) ─────────────────────────
    "ABS", // <T: Numeric>(T) → T (preserves arg nullability)
    "SIGN",
    "POWER",
    "POW",
    "SQRT",
    "EXP",
    "LN",
    "LOG",
    "LOG10",
    "LOG2",
    "SIN",
    "COS",
    "TAN",
    "ASIN",
    "ACOS",
    "ATAN",
    "ATAN2",
    "SINH",
    "COSH",
    "TANH",
    "PI",     // () → Double (nullable=false)
    "RANDOM", // () → Double (nullable=false)
    // ── Text scalars ────────────────────────────────────────────────────────
    "LOWER",
    "UPPER",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "CONCAT",
    "REPLACE",
    "TRANSLATE",
    "REVERSE",
    "REPEAT",
    "LPAD",
    "RPAD",
    "INITCAP",
    "QUOTE_IDENT",
    "QUOTE_LITERAL",
    "LEFT",
    "RIGHT",
    "SUBSTRING",
    "SUBSTR",
    "SPLIT_PART",
    "TO_CHAR",
    "LENGTH",           // → BigInt
    "CHAR_LENGTH",      // → BigInt
    "CHARACTER_LENGTH", // → BigInt
    "POSITION",         // → BigInt
    "STRPOS",           // → BigInt
    "MD5",
    // ── Date / time (fixed return) ──────────────────────────────────────────
    "DATE",
    "CURRENT_DATE",
    "NOW",               // () → Timestamp{tz} (nullable=false)
    "CURRENT_TIMESTAMP", // () → Timestamp{tz} (nullable=false)
    "MAKE_DATE",
    "MAKE_TIME",
    "MAKE_TIMESTAMP",
    "MAKE_TIMESTAMPTZ",
    "AGE",
    "TO_SECONDS",
    "YEAR",
    "MONTH",
    "DAY",
    "DAYOFWEEK",
    "QUARTER",
    "DATE_PART",
    "DATE_ADD", // (Date, Interval) → Timestamp (fixed return)
    "DATE_SUB", // (Date, Interval) → Timestamp (fixed return)
    // ── JSON (fixed return) ─────────────────────────────────────────────────
    "JSON_OBJECT",
    "JSON_ARRAY",
    "TO_JSON",
    "JSON_EXTRACT",
    "JSON_EXTRACT_TEXT",
    "JSON_ARRAY_LENGTH",
    "JSON_OBJECT_KEYS",
    "JSON_CONTAINS",
];

/// The residual set of recognised functions still typed by the hand-written
/// match, exposed for the `legacy_match_ratchet` gate. A name is registry-first
/// iff it appears in [`REGISTRY_MIGRATED`].
pub fn registry_migrated_names() -> &'static [&'static str] {
    REGISTRY_MIGRATED
}

/// Policy for deriving [`TypedColumn::nullable`] on a registry-resolved call.
///
/// The registry itself doesn't track nullability for most functions (§16
/// defers it — see "Out of scope" in the plan), so Phase 9 mirrors the
/// legacy per-function rule via a tiny lookup table. Migrating a new entry to
/// the registry means adding a row here.
///
/// The one exception is a signature carrying a
/// [`smelt_types::signatures::NullabilityPropagation`] tag (currently
/// `MIN`/`MAX`'s grouped-extremal rule): that registry-declared policy is
/// consulted first and takes precedence over the name-matched default below,
/// per the function-registry single-ownership invariant.
fn registry_result_nullable(
    sig: &smelt_types::signatures::Signature,
    name: &str,
    arg_nullable: &[bool],
    grouped: bool,
) -> bool {
    use smelt_types::signatures::NullabilityPropagation;
    match sig.nullability {
        // Grouped extremal fold (MIN/MAX): a NOT NULL argument is NOT NULL
        // under a GROUP BY (every group has ≥1 row); ungrouped input stays
        // nullable (a zero-row table folds to a single NULL row).
        NullabilityPropagation::GroupedExtremal => {
            let all_args_not_null = !arg_nullable.is_empty() && arg_nullable.iter().all(|&n| !n);
            !(grouped && all_args_not_null)
        }
        NullabilityPropagation::None => match name {
            // Non-nullable aggregates / niladic clocks / ranking windows /
            // deterministic niladic scalars — mirrors the legacy per-function rule.
            "COUNT"
            | "NOW"
            | "CURRENT_DATE"
            | "CURRENT_TIMESTAMP"
            | "APPROX_COUNT_DISTINCT"
            | "ROW_NUMBER"
            | "RANK"
            | "DENSE_RANK"
            | "NTILE"
            | "CUME_DIST"
            | "PERCENT_RANK"
            | "PI"
            | "RANDOM" => false,
            // ABS preserves its arg's nullability — legacy returns the arg
            // TypedColumn verbatim when a single-arg inference succeeds.
            "ABS" => arg_nullable.first().copied().unwrap_or(true),
            // Everything else is nullable per legacy.
            _ => true,
        },
    }
}

/// Whether `func` sits inside a `SELECT ... GROUP BY ...` that guarantees
/// every group produces at least one output row — the enclosing
/// `SelectStmt` (nearest CST ancestor) carries a `GROUP_BY_CLAUSE`, and that
/// clause is not undermined by an explicit empty `GROUPING SETS (...)`
/// member.
///
/// Grouped-scope detection only: this walks up from the call site to find
/// its own statement's GROUP BY, not any outer query's — a scalar subquery
/// or CTE body nested inside `func`'s arguments never affects this, since
/// `ancestors()` only visits nodes strictly containing `func` itself.
///
/// Two further hazards, beyond the grand-total-row shapes handled by
/// [`group_by_has_grand_total_row`], defeat the "every group has ≥1 row"
/// guarantee even when a non-empty `GROUP_BY_CLAUSE` is present:
///
/// - A `FILTER (WHERE ...)` clause attached to `func` itself: even though
///   every group has ≥1 row, if every row in a group fails the filter
///   predicate, the aggregate result for that group is NULL regardless of
///   the argument's nullability.
/// - `GROUP BY ALL` (DuckDB) when the select list contains only aggregate
///   items: `GROUP BY ALL` groups by every non-aggregate select item, so
///   zero non-aggregate items means zero actual grouping keys — this
///   degenerates to the ungrouped case (a zero-row table folds to a single
///   NULL row), not a real "every group has ≥1 row" guarantee.
fn is_grouped_query(func: &FunctionCall, ctx: &TypeContext) -> bool {
    if is_window_function_call(func) {
        return false;
    }
    if func.filter_clause().is_some() {
        return false;
    }
    let Some(select) = func.syntax().ancestors().find_map(SelectStmt::cast) else {
        return false;
    };
    let Some(group_by) = select.group_by_clause() else {
        return false;
    };
    if group_by_has_grand_total_row(group_by.syntax()) {
        return false;
    }
    if group_by.is_all() && !select_list_has_non_aggregate_item(&select, ctx) {
        return false;
    }
    true
}

/// True if `select`'s select list contains at least one item whose
/// expression is not an aggregate/window call (i.e. would form a real
/// `GROUP BY ALL` grouping key). Used only to disambiguate the DuckDB
/// `GROUP BY ALL` form, where an all-aggregate select list means zero
/// actual grouping keys.
fn select_list_has_non_aggregate_item(select: &SelectStmt, ctx: &TypeContext) -> bool {
    let Some(select_list) = select.select_list() else {
        return false;
    };
    let has_non_aggregate = select_list.items().any(|item| {
        item.expression()
            .is_some_and(|expr| infer_expression_kind(&expr, ctx) == ExprKind::Scalar)
    });
    has_non_aggregate
}

/// True when `func` is invoked as a window function — i.e. its enclosing
/// `Expr` carries an `OVER (...)` clause (a [`WindowSpec`]).
///
/// `FUNCTION_CALL` and its `OVER` clause are parsed as sibling children of
/// the same wrapping `EXPRESSION` node (see `smelt-parser`'s
/// `parse_primary_expr`: `FUNCTION_CALL` is finished first, then
/// `parse_window_spec()` appends `WINDOW_SPEC` as the next sibling before the
/// wrapping `EXPRESSION` node closes) — so walking from `func`'s syntax node
/// to its parent and casting to [`Expr`] recovers that wrapper, and
/// `Expr::window_spec()` finds the sibling `WINDOW_SPEC` if present.
///
/// This matters for grouped-extremal nullability: the "every group has ≥1
/// row" guarantee that justifies inferring `MIN`/`MAX` as NOT NULL under a
/// `GROUP BY` says nothing about a per-row *window frame* within a group — a
/// bounded frame (e.g. `ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING`) can be
/// empty at a partition's boundary rows, and engines including DuckDB return
/// NULL for `MIN`/`MAX` over an empty window frame regardless of the
/// argument's nullability or the enclosing query's `GROUP BY`.
fn is_window_function_call(func: &FunctionCall) -> bool {
    func.syntax()
        .parent()
        .and_then(Expr::cast)
        .is_some_and(|expr| expr.window_spec().is_some())
}

/// True if `group_by`'s CST subtree guarantees an always-present grand-total
/// row — i.e. is NOT safely "grouped" for grouped-extremal nullability
/// purposes, even though it has a non-empty `GROUP_BY_CLAUSE`. Two shapes
/// trigger this:
///
/// - An explicit empty `GROUPING SETS (...)` member (`()`), anywhere,
///   including nested inside another `GROUPING SETS`.
/// - A top-level or nested `ROLLUP(...)`/`CUBE(...)` call. Per
///   `smelt-parser`'s grammar (see `parser/select.rs` around
///   `parse_grouping_set_element`), smelt has no dedicated CUBE/ROLLUP
///   grammar: `ROLLUP(a, b)` / `CUBE(a, b)` appearing directly in a
///   `GROUP BY` list, or nested inside `GROUPING SETS (...)`, parses via the
///   generic `parse_expression()` path as an ordinary `FUNCTION_CALL` node.
///   Both `ROLLUP` and `CUBE` always include the empty (grand-total) grouping
///   in their expansion.
///
/// An empty/grand-total grouping set produces a row that, over a zero-row
/// input table, still collapses to a single NULL row — the same
/// empty-input soundness boundary the plain ungrouped case hits (`SELECT
/// MIN(x) FROM t` stays nullable even for NOT NULL `x`). So a `GROUP BY`
/// containing either shape does not establish the guarantee that every
/// group produces at least one row, which grouped-extremal nullability
/// inference relies on. Plain `GROUP BY k` (no grouping sets, no
/// ROLLUP/CUBE) is unaffected and returns `false`, staying safely grouped.
fn group_by_has_grand_total_row(group_by: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    group_by.descendants().any(|node| {
        (node.kind() == smelt_parser::SyntaxKind::GROUPING_SET && node.children().next().is_none())
            || FunctionCall::cast(node.clone()).is_some_and(|call| {
                call.name().is_some_and(|name| {
                    let upper = name.to_ascii_uppercase();
                    upper == "ROLLUP" || upper == "CUBE"
                })
            })
    })
}

/// Registry-first inference for the allowlisted subset of built-ins.
///
/// Returns:
/// * `Some(Some(tc))` when the registry resolved the call cleanly — the caller
///   uses this directly and skips the legacy match.
/// * `Some(None)` when the function is known to the registry but arg types
///   couldn't be inferred up-front — the caller should fall through to the
///   legacy match which handles Unknown args more gracefully.
/// * `None` when the function isn't on [`REGISTRY_MIGRATED`] or the registry
///   doesn't know about it — caller uses the legacy match.
pub(super) fn try_registry_inference(
    upper_name: &str,
    func: &FunctionCall,
    ctx: &TypeContext,
) -> Option<Option<TypedColumn>> {
    if !REGISTRY_MIGRATED.contains(&upper_name) {
        return None;
    }
    let sig = BuiltinRegistry::resolve(upper_name)?;

    // Collect arg DataTypes + nullability. If any arg fails to infer, defer
    // to the legacy match — it has per-function fallback behaviour for
    // Unknown args that the registry doesn't model.
    let args = func.arguments();
    let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len());
    let mut arg_nullable: Vec<bool> = Vec::with_capacity(args.len());
    for arg in &args {
        match infer_expression_type(arg, ctx) {
            Some(tc) => {
                arg_types.push(tc.data_type);
                arg_nullable.push(tc.nullable);
            }
            None => {
                // Missing arg inference — fall back to legacy, which has
                // function-specific Unknown handling (e.g. `first_arg_type_or`
                // supplies a sensible default for MIN/MAX).
                return Some(None);
            }
        }
    }

    match unify_call_with_expected(sig, &arg_types, ctx.expected_return.as_ref(), &registry_lub) {
        Ok(result) => {
            let nullable = registry_result_nullable(
                sig,
                upper_name,
                &arg_nullable,
                is_grouped_query(func, ctx),
            );
            Some(Some(TypedColumn {
                data_type: result.return_type,
                nullable,
            }))
        }
        // intentionally ignored: unification failed → fall back to the legacy
        // match so permissive behaviour (LOWER on Integer, MIN on Unknown) is
        // preserved. The caller handles None as "type unknown, use legacy path".
        Err(_) => Some(None),
    }
}
