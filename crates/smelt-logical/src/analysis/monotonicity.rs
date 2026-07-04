//! Static structural classifier for event-time monotonicity.
//!
//! `trace_event_time` decides whether a projected `event_time` expression
//! traces back monotonically (non-decreasing) to a real source partition
//! column. This is a pure, structural walk of the `smelt_parser::Expr` tree
//! — no substring/text-matching over SQL. It is the shared primitive that
//! (in later plans) unlocks UNION-branch/subquery/join relaxations for
//! incremental bound derivation. This module does not wire itself into any
//! consumer; it is a standalone, exhaustively-tested classifier.
//!
//! Cf. ClickHouse's `getMonotonicityForRange` — the verdict struct
//! ([`Monotonicity`]) intentionally mirrors that shape: monotonic /
//! direction / always-monotonic / strict.
//!
//! NOTE: `col AT TIME ZONE '<const>'` is in the spec's whitelist (and named
//! DST zones are in the blacklist), but `smelt-parser` does not currently
//! parse `AT TIME ZONE` syntax at all. That gap is safely covered by the
//! fail-closed default below (whatever such an expression parses as today
//! either fails to parse into a `SelectAnalysis` item, or falls through the
//! classifier's unrecognised-shape arm to `NotTraceable`) — both outcomes
//! are conservative/sound, so no special-case is implemented here.

use serde::Serialize;
use smelt_parser::{BinaryExpr, CastExpr, ColumnRef, Expr, FunctionCall};

use crate::analysis::source_bounds::{self, BoundContext, Seconds};

/// Constant temporal shift folded out of a monotone chain (col ± INTERVAL const).
///
/// Re-exported from `source_bounds` — the unified interval-literal parser
/// (`source_bounds::parse_interval`) lives there and is shared by every
/// interval-literal call site in this crate; see
/// `docs/specs/model_properties.md` "Unified bound / reach derivation".
pub use source_bounds::Offset;

/// Determinism classification of a bare SQL function name — the single
/// shared predicate replacing the three private `NONDETERMINISTIC_FUNCTIONS`
/// copies formerly duplicated in `rules::incremental`, `rules::cumulative`,
/// and this module's own inline match arm.
///
/// See `docs/specs/model_properties.md` §"Determinism (run vs row) and the
/// nondeterminism predicate".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FunctionDeterminism {
    /// One value per run (`NOW`, `CURRENT_TIMESTAMP`, `CURRENT_DATE`) —
    /// frozen at compile time, so pinnable / safe as a direct projection.
    RunDeterministic,
    /// A fresh value per row (`RANDOM`, `RAND`, `UUID`, `GEN_RANDOM_UUID`,
    /// `SETSEED`) — unpinnable.
    RowNondeterministic,
    /// Neither class; an ordinary (or unrecognised) function.
    Neither,
}

/// Every function name covered by the nondeterminism predicate (run- or
/// row-nondeterministic), for text-scanning call sites that need the flat
/// list rather than a per-name query.
pub const NONDETERMINISTIC_FUNCTIONS: &[&str] = &[
    "RANDOM",
    "RAND",
    "NOW",
    "CURRENT_TIMESTAMP",
    "CURRENT_DATE",
    "UUID",
    "GEN_RANDOM_UUID",
    "SETSEED",
];

/// Classify a bare function name (case-insensitive) into its determinism
/// class. An unrecognised name is conservatively `Neither` — callers that
/// need fail-closed treatment of unknown functions handle that at their own
/// call site (the predicate itself only answers "is this a known
/// non-deterministic function").
pub fn classify_function_determinism(name: &str) -> FunctionDeterminism {
    match name.to_ascii_uppercase().as_str() {
        "NOW" | "CURRENT_TIMESTAMP" | "CURRENT_DATE" => FunctionDeterminism::RunDeterministic,
        "RANDOM" | "RAND" | "UUID" | "GEN_RANDOM_UUID" | "SETSEED" => {
            FunctionDeterminism::RowNondeterministic
        }
        _ => FunctionDeterminism::Neither,
    }
}

/// ClickHouse-style verdict for the traced chain (cf. ClickHouse getMonotonicityForRange).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Monotonicity {
    /// Chain is monotone over the value range.
    pub is_monotonic: bool,
    /// Direction: non-decreasing (true) vs non-increasing.
    pub is_positive: bool,
    /// Monotone across the whole domain, not just a sub-range.
    pub is_always_monotonic: bool,
    /// Strictly injective (true) vs weakly monotone with plateaus (false,
    /// e.g. `DATE_TRUNC`).
    pub is_strict: bool,
}

/// Verdict for a traced `event_time` expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EventTimeTrace {
    /// Monotone non-decreasing image of `source_column` on `source`, shifted by `offset`.
    Traceable {
        source: String,
        source_column: String,
        offset: Offset,
        monotonicity: Monotonicity,
    },
    /// Constant or NULL-injecting — static seed, not a partitionable stream.
    StaticSeed { reason: String },
    /// Cannot prove monotone traceability — conservative; consumer must not push.
    NotTraceable { reason: String },
}

/// Entry point: trace `event_time_expr` back to a source partition column,
/// classifying the shape of the chain along the way. `ctx` supplies the
/// source → partition-column mapping used to resolve the traced leaf column.
///
/// Resolution is deliberately name-based and conservative: no FROM-clause /
/// alias-resolution machinery exists at this layer yet. If the leaf column's
/// name (ignoring qualifier) matches zero or more-than-one source's
/// partition column in `ctx`, the result is `NotTraceable` (fail closed).
pub fn trace_event_time(event_time_expr: &Expr, ctx: &BoundContext) -> EventTimeTrace {
    match classify(event_time_expr) {
        Classification::Trace(chain) => resolve_against_ctx(chain, ctx),
        Classification::StaticSeed(reason) => EventTimeTrace::StaticSeed { reason },
        Classification::NotTraceable(reason) => EventTimeTrace::NotTraceable { reason },
    }
}

/// Internal, ctx-free classification result — the leaf column name is
/// carried but not yet resolved against `BoundContext`.
struct Chain {
    source_column: String,
    offset: Offset,
    monotonicity: Monotonicity,
}

enum Classification {
    Trace(Chain),
    StaticSeed(String),
    NotTraceable(String),
}

fn resolve_against_ctx(chain: Chain, ctx: &BoundContext) -> EventTimeTrace {
    let matches: Vec<&String> = ctx
        .source_partition_cols
        .iter()
        .filter(|(_, partition_col)| partition_col.as_str() == chain.source_column.as_str())
        .map(|(source, _)| source)
        .collect();

    match matches.len() {
        1 => EventTimeTrace::Traceable {
            source: matches[0].clone(),
            source_column: chain.source_column,
            offset: chain.offset,
            monotonicity: chain.monotonicity,
        },
        0 => EventTimeTrace::NotTraceable {
            reason: format!(
                "leaf column '{}' does not match any known source partition column",
                chain.source_column
            ),
        },
        _ => EventTimeTrace::NotTraceable {
            reason: format!(
                "leaf column '{}' matches more than one source's partition column (ambiguous)",
                chain.source_column
            ),
        },
    }
}

/// Find the single column reference `expr` traces down to, if any — a
/// call-site addition (not part of the `classify`/`Traceable` decision
/// logic above) used by join-input driving-fact resolution
/// (`analysis::source_bounds::resolve_join_driving_fact`) to read off the
/// leaf column's qualifier (e.g. the `f` in `f.event_ts`) so a candidate
/// join input can be scoped by its FROM/alias identity rather than by
/// column name alone. Walks the same shapes `classify`/`expr_contains_column`
/// recognise; returns `None` for anything with zero or ambiguous
/// column-bearing structure (fail-closed — callers must treat `None` as "no
/// qualifier available", not "assume unqualified").
pub fn find_leaf_column_ref(expr: &Expr) -> Option<ColumnRef> {
    if let Some(col) = expr.as_column_ref() {
        return Some(col);
    }
    if let Some(bin) = expr.as_binary() {
        let left = bin.left().and_then(|e| find_leaf_column_ref(&e));
        let right = bin.right().and_then(|e| find_leaf_column_ref(&e));
        return match (left, right) {
            (Some(l), None) => Some(l),
            (None, Some(r)) => Some(r),
            _ => None,
        };
    }
    if let Some(func) = expr.as_function_call() {
        let mut found = None;
        for arg in func.arguments() {
            if let Some(col) = find_leaf_column_ref(&arg) {
                if found.is_some() {
                    return None; // ambiguous — more than one column-bearing arg
                }
                found = Some(col);
            }
        }
        return found;
    }
    if let Some(cast) = expr.as_cast() {
        return cast.expression().and_then(|e| find_leaf_column_ref(&e));
    }
    None
}

/// Base-case monotonicity: a bare column reference is trivially strictly
/// monotone, non-decreasing, over its whole domain, with zero offset.
fn identity_monotonicity() -> Monotonicity {
    Monotonicity {
        is_monotonic: true,
        is_positive: true,
        is_always_monotonic: true,
        is_strict: true,
    }
}

/// Structural, top-down classification of a single expression layer.
/// NEVER falls back to text/substring matching — every branch below uses
/// typed AST accessors. An expression shape with no explicit arm always
/// resolves to `NotTraceable`, never `Traceable` (fail closed).
fn classify(expr: &Expr) -> Classification {
    // Base case: bare or qualified column reference.
    if let Some(col) = expr.as_column_ref() {
        return Classification::Trace(Chain {
            source_column: col.name().to_string(),
            offset: Offset::Seconds(Seconds::ZERO),
            monotonicity: identity_monotonicity(),
        });
    }

    // Constant / NULL literal — static seed.
    if let Some(reason) = classify_as_literal(expr) {
        return Classification::StaticSeed(reason);
    }

    if let Some(func) = expr.as_function_call() {
        return classify_function(&func);
    }

    if expr.as_extract().is_some() {
        return Classification::NotTraceable("periodic function is not monotone".to_string());
    }

    if let Some(cast) = expr.as_cast() {
        return classify_cast(&cast);
    }

    if let Some(bin) = expr.as_binary() {
        return classify_binary(&bin);
    }

    if expr.as_case().is_some() {
        return Classification::NotTraceable(
            "CASE expression is piecewise, not monotone".to_string(),
        );
    }

    Classification::NotTraceable(format!(
        "unrecognised expression head: {}",
        expr.text().trim()
    ))
}

/// Detect a pure NUMBER/STRING literal (with no column-bearing content) or a
/// bare `NULL` literal. Returns `Some(reason)` when the expression is such a
/// literal.
fn classify_as_literal(expr: &Expr) -> Option<String> {
    use smelt_parser::SyntaxKind::{IDENT, MINUS, NULL_KW, PLUS};

    let tokens: Vec<_> = expr
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();

    if tokens.is_empty() {
        return None;
    }

    if tokens.iter().any(|t| t.kind() == IDENT) {
        return None;
    }

    if tokens.iter().any(|t| t.kind() == NULL_KW) {
        return Some("constant/NULL literal".to_string());
    }

    let has_literal = tokens.iter().any(|t| t.kind().is_literal());
    let all_literal_ish = tokens
        .iter()
        .all(|t| t.kind().is_literal() || matches!(t.kind(), MINUS | PLUS));

    if has_literal && all_literal_ish {
        return Some("constant/NULL literal".to_string());
    }

    None
}

/// Recursively check whether `expr` carries a column reference anywhere in
/// its structure. Used to decide, at each layer, which argument/operand is
/// "the column-bearing one" — never via text search, always via the typed
/// accessors.
fn expr_contains_column(expr: &Expr) -> bool {
    if expr.as_column_ref().is_some() {
        return true;
    }
    if let Some(bin) = expr.as_binary() {
        return bin.left().is_some_and(|e| expr_contains_column(&e))
            || bin.right().is_some_and(|e| expr_contains_column(&e));
    }
    if let Some(func) = expr.as_function_call() {
        return func.arguments().iter().any(expr_contains_column);
    }
    if let Some(cast) = expr.as_cast() {
        return cast.expression().is_some_and(|e| expr_contains_column(&e));
    }
    if let Some(extract) = expr.as_extract() {
        return extract
            .expression()
            .is_some_and(|e| expr_contains_column(&e));
    }
    if let Some(case) = expr.as_case() {
        let in_value = case.case_value().is_some_and(|e| expr_contains_column(&e));
        let in_whens = case.when_clauses().any(|w| {
            w.condition().is_some_and(|e| expr_contains_column(&e))
                || w.result().is_some_and(|e| expr_contains_column(&e))
        });
        let in_else = case.else_expr().is_some_and(|e| expr_contains_column(&e));
        return in_value || in_whens || in_else;
    }
    false
}

/// Classify a function-call layer. Whitelisted grid/truncation functions
/// recurse into their single column-bearing argument (weakening strictness);
/// `COALESCE(col, const)` is a static seed; everything else is a named
/// blacklist entry or an unknown function — both fail closed.
fn classify_function(func: &FunctionCall) -> Classification {
    let name = func.name().unwrap_or_default();
    let upper = name.to_uppercase();
    let args = func.arguments();

    match upper.as_str() {
        "DATE_TRUNC" | "DATE_BIN" | "TIME_BUCKET" => recurse_single_column_arg(&args, &upper, true),
        "FLOOR" => recurse_single_column_arg(&args, &upper, true),
        "COALESCE" => classify_coalesce(&args),
        "MOD" => Classification::NotTraceable("periodic function is not monotone".to_string()),
        "GREATEST" | "LEAST" => Classification::NotTraceable(
            "GREATEST/LEAST clamps to a plateau that can straddle a window boundary".to_string(),
        ),
        _ if classify_function_determinism(&upper) == FunctionDeterminism::RunDeterministic => {
            Classification::NotTraceable(
                "run-nondeterministic clock is not source-traceable".to_string(),
            )
        }
        _ => Classification::NotTraceable(format!(
            "unknown function {name}: monotonicity cannot be proven"
        )),
    }
}

/// Recurse into the single column-bearing argument among `args`, weakening
/// strictness (many-to-one truncation/grid function) if `weaken_strict`.
/// Zero or more-than-one column-bearing arguments fails closed.
fn recurse_single_column_arg(args: &[Expr], fn_label: &str, weaken_strict: bool) -> Classification {
    let column_bearing: Vec<&Expr> = args.iter().filter(|a| expr_contains_column(a)).collect();
    if column_bearing.len() != 1 {
        return Classification::NotTraceable(format!(
            "{fn_label}: expected exactly one column-bearing argument, found {}",
            column_bearing.len()
        ));
    }
    match classify(column_bearing[0]) {
        Classification::Trace(mut chain) => {
            if weaken_strict {
                chain.monotonicity.is_strict = false;
            }
            Classification::Trace(chain)
        }
        other => other,
    }
}

/// `COALESCE(col, const)` — exactly two args, exactly one column-bearing —
/// is a static seed (constant injected for NULL rows), not a monotone chain.
fn classify_coalesce(args: &[Expr]) -> Classification {
    if args.len() != 2 {
        return Classification::NotTraceable(format!(
            "COALESCE: expected exactly 2 arguments, found {}",
            args.len()
        ));
    }
    let column_bearing = args.iter().filter(|a| expr_contains_column(a)).count();
    if column_bearing == 1 {
        Classification::StaticSeed("COALESCE injects a constant for NULL rows".to_string())
    } else {
        Classification::NotTraceable(format!(
            "COALESCE: expected exactly one column-bearing argument, found {column_bearing}"
        ))
    }
}

/// `CAST(col AS <type>)` / `col::<type>` — recurse into the cast expression
/// iff the target type is temporal; any other target fails closed.
///
/// Judgment call: `CAST(... AS DATE)` sets `is_strict = false` (many-to-one,
/// same as `DATE_TRUNC`'s day-grid truncation). Other temporal targets
/// (`TIMESTAMP`/`TIMESTAMPTZ`/`DATETIME`) leave the child's `is_strict`
/// unchanged, since those casts are not lossy in the same way.
fn classify_cast(cast: &CastExpr) -> Classification {
    let type_name = cast
        .type_spec()
        .and_then(|t| t.type_name())
        .unwrap_or_default()
        .to_uppercase();

    if !matches!(
        type_name.as_str(),
        "DATE" | "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME"
    ) {
        return Classification::NotTraceable(format!(
            "CAST target {type_name} is not a temporal type"
        ));
    }

    let Some(inner) = cast.expression() else {
        return Classification::NotTraceable("CAST has no inner expression".to_string());
    };

    match classify(&inner) {
        Classification::Trace(mut chain) => {
            if type_name == "DATE" {
                chain.monotonicity.is_strict = false;
            }
            Classification::Trace(chain)
        }
        other => other,
    }
}

/// `col ± INTERVAL '<const>'` — exactly one operand column-bearing, the
/// other a parseable `INTERVAL` literal, operator `+` or `-`.
///
/// Sign convention: `Offset::Seconds` wraps an unsigned magnitude (`Seconds`
/// is `u64`), so both `col + INTERVAL '2 hours'` and `col - INTERVAL '2
/// hours'` fold to the same `Seconds(7200)` magnitude here — direction is
/// not separately tracked by this primitive (a future consumer that needs
/// signed offsets can read the original operator off the AST itself; this
/// phase only proves *that* the chain is a constant shift, not which way).
fn classify_binary(bin: &BinaryExpr) -> Classification {
    let Some(left) = bin.left() else {
        return Classification::NotTraceable(
            "binary expression is missing its left operand".to_string(),
        );
    };
    let Some(right) = bin.right() else {
        return Classification::NotTraceable(
            "binary expression is missing its right operand".to_string(),
        );
    };
    let op = bin.operator().unwrap_or_default();
    if op != "+" && op != "-" {
        return Classification::NotTraceable(format!(
            "binary operator {op} is not a column ± constant interval shift"
        ));
    }

    let left_has_col = expr_contains_column(&left);
    let right_has_col = expr_contains_column(&right);

    if left_has_col && right_has_col {
        return Classification::NotTraceable(
            "arithmetic on two columns is not monotone in either column alone".to_string(),
        );
    }
    if !left_has_col && !right_has_col {
        return Classification::NotTraceable(
            "binary +/- is not a column ± constant interval shift".to_string(),
        );
    }

    let (col_side, const_side) = if left_has_col {
        (left, right)
    } else {
        (right, left)
    };

    let Some(shift) = parse_interval_literal(&const_side) else {
        return Classification::NotTraceable(
            "binary +/- is not a column ± constant interval shift".to_string(),
        );
    };

    match classify(&col_side) {
        Classification::Trace(mut chain) => {
            chain.offset = combine_offset(chain.offset, shift);
            Classification::Trace(chain)
        }
        other => other,
    }
}

/// Parse an `INTERVAL '<value>'` literal expression (e.g. `INTERVAL '1
/// day'`) into an `Offset`. Note this parses the literal *string content of
/// a single INTERVAL token already located via the AST* — not a
/// text/substring search over the classifier's control flow.
fn parse_interval_literal(expr: &Expr) -> Option<Offset> {
    use smelt_parser::SyntaxKind::{IDENT, STRING};

    let tokens: Vec<_> = expr
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();

    let is_interval_keyword = tokens
        .iter()
        .any(|t| t.kind() == IDENT && t.text().eq_ignore_ascii_case("INTERVAL"));
    if !is_interval_keyword {
        return None;
    }

    let string_tok = tokens.iter().find(|t| t.kind() == STRING)?;
    let raw = string_tok.text();
    let value = raw.trim_matches(|c| c == '\'' || c == '"');
    source_bounds::parse_interval(value)
}

/// Fold a newly-parsed constant shift into the chain's running offset.
fn combine_offset(existing: Offset, shift: Offset) -> Offset {
    match (existing, shift) {
        (Offset::Seconds(a), Offset::Seconds(b)) => Offset::Seconds(Seconds(a.0 + b.0)),
        (Offset::Symbolic(a), Offset::Seconds(b)) => Offset::Symbolic(format!("{a} + {}s", b.0)),
        (Offset::Seconds(a), Offset::Symbolic(b)) if a.0 == 0 => Offset::Symbolic(b),
        (Offset::Seconds(a), Offset::Symbolic(b)) => Offset::Symbolic(format!("{}s + {b}", a.0)),
        (Offset::Symbolic(a), Offset::Symbolic(b)) => Offset::Symbolic(format!("{a} + {b}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `sql`, pull the first SELECT-list item's expression.
    fn first_select_expr(sql: &str) -> Expr {
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = smelt_parser::File::cast(root).expect("file cast");
        let select = file.select_stmt().expect("select stmt");
        let select_list = select.select_list().expect("select list");
        let item = select_list.items().next().expect("first select item");
        item.expression().expect("item expression")
    }

    fn events_ctx() -> BoundContext {
        BoundContext::new().with_source("events", "event_ts")
    }

    #[test]
    fn nondeterminism_predicate_classifies_row_nondeterministic() {
        for name in ["RANDOM", "RAND", "UUID", "GEN_RANDOM_UUID", "SETSEED"] {
            assert_eq!(
                classify_function_determinism(name),
                FunctionDeterminism::RowNondeterministic,
                "{name} should classify as row-nondeterministic"
            );
            // Case-insensitive.
            assert_eq!(
                classify_function_determinism(&name.to_ascii_lowercase()),
                FunctionDeterminism::RowNondeterministic
            );
        }
    }

    #[test]
    fn nondeterminism_predicate_classifies_run_deterministic() {
        for name in ["NOW", "CURRENT_TIMESTAMP", "CURRENT_DATE"] {
            assert_eq!(
                classify_function_determinism(name),
                FunctionDeterminism::RunDeterministic,
                "{name} should classify as run-deterministic"
            );
        }
    }

    #[test]
    fn nondeterminism_predicate_classifies_ordinary_function_as_neither() {
        for name in ["SUM", "COUNT", "DATE_TRUNC", "SOME_UNKNOWN_FN"] {
            assert_eq!(
                classify_function_determinism(name),
                FunctionDeterminism::Neither,
                "{name} should classify as neither"
            );
        }
    }

    #[test]
    fn trace_bare_column_traceable() {
        let expr = first_select_expr("SELECT event_ts AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source,
                source_column,
                offset,
                monotonicity,
            } => {
                assert_eq!(source, "events");
                assert_eq!(source_column, "event_ts");
                assert_eq!(offset, Offset::Seconds(Seconds::ZERO));
                assert!(monotonicity.is_strict);
                assert!(monotonicity.is_always_monotonic);
                assert!(monotonicity.is_monotonic);
                assert!(monotonicity.is_positive);
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_qualified_column_traceable() {
        let expr = first_select_expr("SELECT f.event_ts AS event_time FROM t f");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source,
                source_column,
                ..
            } => {
                assert_eq!(source, "events");
                assert_eq!(source_column, "event_ts");
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_date_trunc_traceable_weakly_monotonic() {
        let expr = first_select_expr("SELECT DATE_TRUNC('day', event_ts) AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source_column,
                monotonicity,
                ..
            } => {
                assert_eq!(source_column, "event_ts");
                assert!(!monotonicity.is_strict);
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_cast_to_date_traceable() {
        let expr = first_select_expr("SELECT CAST(event_ts AS DATE) AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable { source_column, .. } => {
                assert_eq!(source_column, "event_ts");
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_time_bucket_traceable() {
        let expr = first_select_expr("SELECT time_bucket('1 hour', event_ts) AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source_column,
                monotonicity,
                ..
            } => {
                assert_eq!(source_column, "event_ts");
                assert!(!monotonicity.is_strict);
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_interval_plus_offset_traceable() {
        let expr = first_select_expr("SELECT event_ts + INTERVAL '1 day' AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                offset,
                monotonicity,
                ..
            } => {
                assert_eq!(offset, Offset::Seconds(Seconds::days(1)));
                assert!(monotonicity.is_strict);
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_interval_minus_offset_traceable() {
        // Sign convention: Seconds is unsigned, so both `+` and `-` interval
        // shifts fold to the same positive magnitude; direction is not
        // separately tracked by this primitive (see `classify_binary` doc).
        let expr = first_select_expr("SELECT event_ts - INTERVAL '2 hours' AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable { offset, .. } => {
                assert_eq!(offset, Offset::Seconds(Seconds::hours(2)));
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_month_offset_is_symbolic() {
        let expr = first_select_expr("SELECT event_ts + INTERVAL '1 month' AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable { offset, .. } => {
                assert!(
                    matches!(offset, Offset::Symbolic(_)),
                    "expected Symbolic, got {offset:?}"
                );
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_three_layer_composition_traceable() {
        let expr = first_select_expr(
            "SELECT DATE_TRUNC('day', CAST(event_ts AS TIMESTAMP) + INTERVAL '2 hours') AS event_time FROM t",
        );
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source_column,
                monotonicity,
                ..
            } => {
                assert_eq!(source_column, "event_ts");
                assert!(
                    !monotonicity.is_strict,
                    "DATE_TRUNC weakens strictness at outermost layer"
                );
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    #[test]
    fn trace_static_seed_null_literal() {
        let expr = first_select_expr("SELECT NULL AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::StaticSeed { .. }
        ));
    }

    #[test]
    fn trace_static_seed_constant() {
        let expr = first_select_expr("SELECT 42 AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::StaticSeed { .. }
        ));
    }

    #[test]
    fn trace_static_seed_coalesce() {
        let expr =
            first_select_expr("SELECT COALESCE(event_ts, '2026-01-01') AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::StaticSeed { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_two_column_arithmetic() {
        let expr = first_select_expr("SELECT end_ts - start_ts AS event_time FROM t");
        let ctx = BoundContext::new()
            .with_source("events", "end_ts")
            .with_source("other", "start_ts");
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_extract() {
        let expr = first_select_expr("SELECT EXTRACT(HOUR FROM event_ts) AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_case() {
        let expr = first_select_expr(
            "SELECT CASE WHEN x THEN event_ts ELSE other_ts END AS event_time FROM t",
        );
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_greatest() {
        let expr =
            first_select_expr("SELECT GREATEST(event_ts, '2026-01-01') AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_unknown_udf() {
        let expr = first_select_expr("SELECT my_custom_fn(event_ts) AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_now() {
        let expr = first_select_expr("SELECT NOW() AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_cast_to_varchar() {
        let expr = first_select_expr("SELECT CAST(event_ts AS VARCHAR) AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn trace_not_traceable_unresolvable_leaf() {
        let expr = first_select_expr("SELECT unrelated_col AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time(&expr, &ctx),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    #[test]
    fn find_leaf_column_ref_qualified() {
        let expr = first_select_expr("SELECT f.event_ts AS event_time FROM t f");
        let col = find_leaf_column_ref(&expr).expect("expected a leaf column ref");
        assert_eq!(col.qualifier(), Some("f"));
        assert_eq!(col.name(), "event_ts");
    }

    #[test]
    fn find_leaf_column_ref_through_function_and_interval() {
        let expr = first_select_expr(
            "SELECT DATE_TRUNC('day', f.event_ts + INTERVAL '1 day') AS event_time FROM t f",
        );
        let col = find_leaf_column_ref(&expr).expect("expected a leaf column ref");
        assert_eq!(col.qualifier(), Some("f"));
        assert_eq!(col.name(), "event_ts");
    }

    #[test]
    fn find_leaf_column_ref_none_for_two_column_arithmetic() {
        let expr = first_select_expr("SELECT end_ts - start_ts AS event_time FROM t");
        assert!(find_leaf_column_ref(&expr).is_none());
    }

    #[test]
    fn unknown_head_fails_closed() {
        let expr = first_select_expr("SELECT ARRAY[1, 2, 3] AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::NotTraceable { .. } => {}
            other => panic!("expected NotTraceable (fail closed), got {other:?}"),
        }
    }
}
