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
use smelt_core::Granularity;
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
    /// The truncation/grid unit of the outermost many-to-one transform in
    /// the chain, if any (e.g. `DATE_TRUNC('day', …)` → `Some(Granularity::Day)`,
    /// `CAST(… AS DATE)` → `Some(Granularity::Day)`). `None` when the chain
    /// has no grid-truncating layer (a bare column, an offset-only shift) or
    /// the layer's unit couldn't be resolved to a `Granularity` (e.g. an
    /// unrecognised `DATE_TRUNC` unit string) — undecided, not a positive
    /// disproof. Set by the outermost grid-truncating layer, overwriting any
    /// inner value, since the outermost transform governs the expression's
    /// final partition grid. See `docs/specs/incremental_models.md`
    /// §"Run window vs partition granularity".
    pub grid_unit: Option<Granularity>,
}

/// Whether a `NotTraceable` verdict is a positive structural disproof or
/// merely an unrecognised (undecidable) shape.
///
/// This is the axis the declared-monotonicity escape hatch
/// (`TimeseriesConfig::assert_monotonic`) reads: a declaration may widen
/// `Undecidable` (the classifier has no rule for the shape — an opaque
/// UDF/function it cannot reason about) but must never widen `Disproven`
/// (a shape the classifier positively knows is not a monotone chain — a
/// periodic function, a piecewise `CASE`, a run-nondeterministic clock, a
/// row-nondeterministic function, two-column arithmetic, an ambiguous or
/// unresolvable leaf, …). See `docs/specs/model_properties.md` §Constraints
/// "Declared escape hatches may only widen".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NotTraceableKind {
    /// The classifier positively knows this shape is not a monotone chain
    /// (or cannot be resolved to a single source). Never widened.
    Disproven,
    /// The classifier has no rule for this shape (an opaque/unknown
    /// function). The only kind `assert_monotonic` is permitted to widen.
    Undecidable,
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
    NotTraceable {
        reason: String,
        kind: NotTraceableKind,
    },
}

/// Entry point: trace `event_time_expr` back to a source partition column,
/// classifying the shape of the chain along the way. `ctx` supplies the
/// source → partition-column mapping used to resolve the traced leaf column.
///
/// Resolution is deliberately name-based and conservative: no FROM-clause /
/// alias-resolution machinery exists at this layer yet. If the leaf column's
/// name (ignoring qualifier) matches zero or more-than-one source's
/// partition column in `ctx`, the result is `NotTraceable` (fail closed).
///
/// Equivalent to `trace_event_time_declared(event_time_expr, ctx, false)` —
/// the un-declared, static-only trace.
pub fn trace_event_time(event_time_expr: &Expr, ctx: &BoundContext) -> EventTimeTrace {
    trace_event_time_declared(event_time_expr, ctx, false)
}

/// Like [`trace_event_time`], but when `declared_monotonic` is `true` and the
/// static classifier's *only* obstacle is an unrecognised (undecidable)
/// function call, admits the pushdown by tracing into that function's single
/// column-bearing argument (weakening strictness, since the opaque function's
/// own shape is unproven) — the declared-monotonicity escape hatch
/// (`model_properties.md` §"Model-scoped declarations").
///
/// The widen is verdict-scoped, not blanket: a `Disproven` `NotTraceable`
/// (periodic function, piecewise `CASE`, run-/row-nondeterministic function,
/// two-column arithmetic, ambiguous/unresolvable leaf, non-temporal cast, …)
/// and every `StaticSeed` are returned unchanged regardless of
/// `declared_monotonic` — the declaration can only widen the *undecidable*
/// verdict, never substitute for a positive disproof.
pub fn trace_event_time_declared(
    event_time_expr: &Expr,
    ctx: &BoundContext,
    declared_monotonic: bool,
) -> EventTimeTrace {
    match classify(event_time_expr, declared_monotonic) {
        Classification::Trace(chain) => resolve_against_ctx(chain, ctx),
        Classification::StaticSeed(reason) => EventTimeTrace::StaticSeed { reason },
        Classification::NotTraceable(reason, kind) => EventTimeTrace::NotTraceable { reason, kind },
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
    NotTraceable(String, NotTraceableKind),
}

impl Classification {
    fn disproven(reason: impl Into<String>) -> Classification {
        Classification::NotTraceable(reason.into(), NotTraceableKind::Disproven)
    }

    fn undecidable(reason: impl Into<String>) -> Classification {
        Classification::NotTraceable(reason.into(), NotTraceableKind::Undecidable)
    }
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
            kind: NotTraceableKind::Disproven,
        },
        _ => EventTimeTrace::NotTraceable {
            reason: format!(
                "leaf column '{}' matches more than one source's partition column (ambiguous)",
                chain.source_column
            ),
            kind: NotTraceableKind::Disproven,
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
        grid_unit: None,
    }
}

/// Structural, top-down classification of a single expression layer.
/// NEVER falls back to text/substring matching — every branch below uses
/// typed AST accessors. An expression shape with no explicit arm always
/// resolves to `NotTraceable`, never `Traceable` (fail closed).
///
/// `declared_monotonic` threads the declared-monotonicity escape hatch down
/// to the one arm permitted to read it — the unrecognised-function-name arm
/// of [`classify_function`]. Every other arm below is a *positive* disproof
/// and ignores the flag entirely.
fn classify(expr: &Expr, declared_monotonic: bool) -> Classification {
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
        return classify_function(&func, declared_monotonic);
    }

    if expr.as_extract().is_some() {
        return Classification::disproven("periodic function is not monotone");
    }

    if let Some(cast) = expr.as_cast() {
        return classify_cast(&cast, declared_monotonic);
    }

    if let Some(bin) = expr.as_binary() {
        return classify_binary(&bin, declared_monotonic);
    }

    if expr.as_case().is_some() {
        return Classification::disproven("CASE expression is piecewise, not monotone");
    }

    Classification::disproven(format!(
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
/// `COALESCE(col, const)` is a static seed; a handful of named functions are
/// positively disproven; an unrecognised function name is `Undecidable` — and
/// only that arm reads `declared_monotonic`, recursing into the function's
/// single column-bearing argument (weakened strictness) when the declaration
/// is present and exactly one such argument exists; otherwise it stays
/// `NotTraceable` even when declared (fail closed — the declaration cannot
/// license a shape it cannot resolve to a single leaf).
fn classify_function(func: &FunctionCall, declared_monotonic: bool) -> Classification {
    let name = func.name().unwrap_or_default();
    let upper = name.to_uppercase();
    let args = func.arguments();

    match upper.as_str() {
        "DATE_TRUNC" | "DATE_BIN" | "TIME_BUCKET" => {
            let grid_unit = args
                .iter()
                .find_map(extract_string_literal_text)
                .and_then(|s| unit_text_to_granularity(&s));
            match recurse_single_column_arg(&args, &upper, true, declared_monotonic) {
                Classification::Trace(mut chain) => {
                    chain.monotonicity.grid_unit = grid_unit;
                    Classification::Trace(chain)
                }
                other => other,
            }
        }
        "FLOOR" => recurse_single_column_arg(&args, &upper, true, declared_monotonic),
        "COALESCE" => classify_coalesce(&args),
        "MOD" => Classification::disproven("periodic function is not monotone"),
        "GREATEST" | "LEAST" => Classification::disproven(
            "GREATEST/LEAST clamps to a plateau that can straddle a window boundary",
        ),
        _ if classify_function_determinism(&upper) == FunctionDeterminism::RunDeterministic => {
            Classification::disproven("run-nondeterministic clock is not source-traceable")
        }
        // Row-nondeterministic functions (RANDOM/UUID/…) must never be
        // widened by the declaration even though they'd otherwise fall into
        // the unrecognised-function arm below — a row-nondeterministic value
        // in the event-time/skeleton position is a positive hazard, not an
        // undecidable shape (model_properties.md §Constraints).
        _ if classify_function_determinism(&upper) == FunctionDeterminism::RowNondeterministic => {
            Classification::disproven(format!(
                "{name} is row-nondeterministic and cannot occupy an event-time position"
            ))
        }
        _ if declared_monotonic => {
            match recurse_single_column_arg(&args, &upper, true, declared_monotonic) {
                Classification::Trace(chain) => Classification::Trace(chain),
                _ => Classification::undecidable(format!(
                    "unknown function {name}: monotonicity cannot be proven, and the \
                     declared-monotonicity escape hatch could not resolve a single traced \
                     leaf column among its arguments"
                )),
            }
        }
        _ => Classification::undecidable(format!(
            "unknown function {name}: monotonicity cannot be proven"
        )),
    }
}

/// Recurse into the single column-bearing argument among `args`, weakening
/// strictness (many-to-one truncation/grid function) if `weaken_strict`.
/// Zero or more-than-one column-bearing arguments fails closed.
fn recurse_single_column_arg(
    args: &[Expr],
    fn_label: &str,
    weaken_strict: bool,
    declared_monotonic: bool,
) -> Classification {
    let column_bearing: Vec<&Expr> = args.iter().filter(|a| expr_contains_column(a)).collect();
    if column_bearing.len() != 1 {
        return Classification::disproven(format!(
            "{fn_label}: expected exactly one column-bearing argument, found {}",
            column_bearing.len()
        ));
    }
    match classify(column_bearing[0], declared_monotonic) {
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
        return Classification::disproven(format!(
            "COALESCE: expected exactly 2 arguments, found {}",
            args.len()
        ));
    }
    let column_bearing = args.iter().filter(|a| expr_contains_column(a)).count();
    if column_bearing == 1 {
        Classification::StaticSeed("COALESCE injects a constant for NULL rows".to_string())
    } else {
        Classification::disproven(format!(
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
fn classify_cast(cast: &CastExpr, declared_monotonic: bool) -> Classification {
    let type_name = cast
        .type_spec()
        .and_then(|t| t.type_name())
        .unwrap_or_default()
        .to_uppercase();

    if !matches!(
        type_name.as_str(),
        "DATE" | "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME"
    ) {
        return Classification::disproven(format!(
            "CAST target {type_name} is not a temporal type"
        ));
    }

    let Some(inner) = cast.expression() else {
        return Classification::disproven("CAST has no inner expression");
    };

    match classify(&inner, declared_monotonic) {
        Classification::Trace(mut chain) => {
            if type_name == "DATE" {
                chain.monotonicity.is_strict = false;
                chain.monotonicity.grid_unit = Some(Granularity::Day);
            }
            Classification::Trace(chain)
        }
        other => other,
    }
}

/// Extract the text of a single `STRING` literal token from `expr`'s
/// immediate token children, if any (e.g. the `'day'` in `DATE_TRUNC('day',
/// col)`, or the `'1 day'` in `DATE_BIN(INTERVAL '1 day', col)`). Structural
/// AST-token lookup, not a text/substring search over the SQL.
fn extract_string_literal_text(expr: &Expr) -> Option<String> {
    use smelt_parser::SyntaxKind::STRING;

    let token = expr
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == STRING)?;
    Some(
        token
            .text()
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string(),
    )
}

/// Map a truncation/grid unit string (e.g. `"day"`, `"1 day"`, `"hours"`) to
/// the closed `Granularity` enum. Takes the last whitespace-separated token
/// (so an `INTERVAL`-shaped bucket width like `"1 day"` resolves the same as
/// a bare unit string), lower-cases, and strips a trailing plural `s`.
/// Returns `None` for an unrecognised unit — undecided, not a positive
/// disproof; callers fall back to skipping the `g_run >= g_part` comparison.
fn unit_text_to_granularity(text: &str) -> Option<Granularity> {
    let last_word = text.split_whitespace().next_back()?.to_ascii_lowercase();
    let singular = last_word.strip_suffix('s').unwrap_or(&last_word);
    match singular {
        "hour" => Some(Granularity::Hour),
        "day" => Some(Granularity::Day),
        "week" => Some(Granularity::Week),
        "month" => Some(Granularity::Month),
        "quarter" => Some(Granularity::Quarter),
        "year" => Some(Granularity::Year),
        _ => None,
    }
}

/// Structural classification entry point exposed to callers that need only
/// the derived truncation/grid unit of a projected expression (`g_part`),
/// without a resolvable `BoundContext` (e.g. `smelt_cli::temporal`'s run-window
/// validation, which has the model's own SQL but no upstream-source bound
/// map). Reuses the same `classify` walk `trace_event_time` uses — not a
/// second independent parser — so `grid_unit` is always the walk's
/// outermost-truncation verdict. Returns `None` when the expression doesn't
/// classify to a traceable chain at all, or when it does but no layer in the
/// chain resolved a grid unit (fail-open: caller treats this as
/// undecidable and skips the `g_run >= g_part` comparison, matching the
/// classifier's existing `Undecidable` posture elsewhere).
pub fn classify_truncation_grid_unit(expr: &Expr) -> Option<Granularity> {
    match classify(expr, false) {
        Classification::Trace(chain) => chain.monotonicity.grid_unit,
        _ => None,
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
fn classify_binary(bin: &BinaryExpr, declared_monotonic: bool) -> Classification {
    let Some(left) = bin.left() else {
        return Classification::disproven("binary expression is missing its left operand");
    };
    let Some(right) = bin.right() else {
        return Classification::disproven("binary expression is missing its right operand");
    };
    let op = bin.operator().unwrap_or_default();
    if op != "+" && op != "-" {
        // Name the construct explicitly for the two non-monotone integer
        // transforms this whitelist must positively reject (fail-closed):
        // `batch_id % n` (modulo — periodic, not monotone) and
        // `batch_id * n` (multiplication — not a constant *shift*, and
        // negative multipliers reverse direction). Any other non-`+`/`-`
        // operator gets the generic message.
        let construct = match op.as_str() {
            "%" => "modulo".to_string(),
            "*" => "multiplication".to_string(),
            other => format!("operator {other}"),
        };
        return Classification::disproven(format!(
            "binary {construct} is not a monotone constant shift"
        ));
    }

    let left_has_col = expr_contains_column(&left);
    let right_has_col = expr_contains_column(&right);

    if left_has_col && right_has_col {
        return Classification::disproven(
            "arithmetic on two columns is not monotone in either column alone",
        );
    }
    if !left_has_col && !right_has_col {
        return Classification::disproven("binary +/- is not a column ± constant interval shift");
    }

    // Subtraction does not commute: `col - const` is a monotone constant
    // shift (direction preserved), but `const - col` reverses direction as
    // `col` increases (e.g. `5 - batch_id` is strictly *decreasing* in
    // `batch_id`, not "batch_id shifted by a constant"). Only admit the
    // column-on-the-left form for `-`; addition is commutative so both
    // `col + const` and `const + col` remain fine below.
    if op == "-" && !left_has_col {
        return Classification::disproven(
            "constant minus column reverses direction and is not a monotone shift",
        );
    }

    let (col_side, const_side) = if left_has_col {
        (left, right)
    } else {
        (right, left)
    };

    if let Some(shift) = parse_interval_literal(&const_side) {
        return match classify(&col_side, declared_monotonic) {
            Classification::Trace(mut chain) => {
                chain.offset = combine_offset(chain.offset, shift);
                Classification::Trace(chain)
            }
            other => other,
        };
    }

    // Non-`INTERVAL` bare integer constant shift — admits monotone integer
    // partition keys (sequence id / offset / watermark), e.g. `batch_id + 1`
    // or `batch_id - 1`. The column side must still independently classify
    // as a monotone chain (`classify` below); only the constant shift itself
    // is being admitted here, mirroring the INTERVAL case above. See
    // `docs/specs/incremental_models.md` §Surface "`partition_column` must be
    // monotone".
    if let Some(n) = parse_bare_integer_literal(&const_side) {
        let signed = if op == "-" { -n } else { n };
        return match classify(&col_side, declared_monotonic) {
            Classification::Trace(mut chain) => {
                chain.offset = combine_offset(chain.offset, Offset::Integer(signed));
                Classification::Trace(chain)
            }
            other => other,
        };
    }

    Classification::disproven("binary +/- is not a column ± constant interval shift")
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

/// Parse a bare (non-`INTERVAL`) signed integer literal expression (e.g. `5`
/// or `-5`) — the non-temporal sibling of [`parse_interval_literal`], for
/// monotone integer partition keys (sequence id / offset / watermark).
/// Returns `None` for anything that is not a pure sign + `NUMBER` token run
/// (an identifier, a string literal, a function call, two number tokens, …)
/// — fail-closed, consistent with [`parse_interval_literal`] returning
/// `None` for anything that isn't a recognised `INTERVAL '<value>'` shape.
fn parse_bare_integer_literal(expr: &Expr) -> Option<i64> {
    use smelt_parser::SyntaxKind::{IDENT, MINUS, NUMBER, PLUS, STRING};

    let tokens: Vec<_> = expr
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();

    if tokens.is_empty() {
        return None;
    }
    // An IDENT (e.g. the `INTERVAL` keyword, a bare identifier, a function
    // name) or a STRING literal disqualifies this as a bare integer.
    if tokens.iter().any(|t| matches!(t.kind(), IDENT | STRING)) {
        return None;
    }

    let mut sign: i64 = 1;
    let mut number: Option<i64> = None;
    for t in &tokens {
        match t.kind() {
            MINUS => sign = -sign,
            PLUS => {}
            NUMBER => {
                if number.is_some() {
                    // More than one NUMBER token — not a single literal.
                    return None;
                }
                number = Some(t.text().parse().ok()?);
            }
            _ => return None,
        }
    }
    number.map(|n| n * sign)
}

/// Fold a newly-parsed constant shift into the chain's running offset.
fn combine_offset(existing: Offset, shift: Offset) -> Offset {
    match (existing, shift) {
        (Offset::Seconds(a), Offset::Seconds(b)) => Offset::Seconds(Seconds(a.0 + b.0)),
        (Offset::Symbolic(a), Offset::Seconds(b)) => Offset::Symbolic(format!("{a} + {}s", b.0)),
        (Offset::Seconds(a), Offset::Symbolic(b)) if a.0 == 0 => Offset::Symbolic(b),
        (Offset::Seconds(a), Offset::Symbolic(b)) => Offset::Symbolic(format!("{}s + {b}", a.0)),
        (Offset::Symbolic(a), Offset::Symbolic(b)) => Offset::Symbolic(format!("{a} + {b}")),

        // Integer-domain combos (monotone non-temporal partition keys). The
        // base case (a bare column reference) seeds `chain.offset` at
        // `Offset::Seconds(Seconds::ZERO)` regardless of the column's own
        // domain (see `classify`'s column-ref arm) — so the first integer
        // shift folded onto that identity zero must convert the running
        // offset to `Integer`, not stay `Seconds`.
        (Offset::Integer(a), Offset::Integer(b)) => Offset::Integer(a + b),
        (Offset::Seconds(a), Offset::Integer(b)) if a.0 == 0 => Offset::Integer(b),
        (Offset::Integer(a), Offset::Seconds(b)) if b.0 == 0 => Offset::Integer(a),
        // A genuinely mixed chain (a real nonzero `Seconds` interval shift
        // composed with a bare-integer shift on the same expression) is
        // exotic and not a supported shape — fold to a descriptive
        // `Symbolic` string rather than silently discarding either
        // magnitude (fail-loud: the value is visibly not a clean bound
        // rather than looking like one).
        (Offset::Seconds(a), Offset::Integer(b)) => Offset::Symbolic(format!("{}s + {b}", a.0)),
        (Offset::Integer(a), Offset::Seconds(b)) => Offset::Symbolic(format!("{a} + {}s", b.0)),
        (Offset::Symbolic(a), Offset::Integer(b)) => Offset::Symbolic(format!("{a} + {b}")),
        (Offset::Integer(a), Offset::Symbolic(b)) => Offset::Symbolic(format!("{a} + {b}")),
    }
}

// ===== Relational composition over the property walk =====

/// Whole-model composed event-time trace for one target output column
/// (`model_properties.md` §"The composition walk"): the expression-level
/// trace lifted through the model's relational operators — a rename/monotone
/// re-projection through a CTE or derived table reduces to the source leaf
/// (offsets add, strictness meets, the outermost grid wins), and a set
/// operation carries a per-branch vector instead of a collapsed verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposedTrace {
    /// The model's root is a single SELECT scope: one verdict, reduced
    /// through any CTE / derived-table projection layers beneath it.
    Single(EventTimeTrace),
    /// The model's root is a set operation: one verdict per arm, in source
    /// order. Branches may anchor to different sources with different
    /// offsets — no single-verdict reduction exists in general, so the
    /// vector is the honest type; a `StaticSeed`/`NotTraceable` arm keeps
    /// its own refusal.
    Branches(Vec<EventTimeTrace>),
}

/// Per-column event-time trace of one projected output column of a walk
/// scope — the value a consuming (outer) scope reduces its own leaf-column
/// reference against.
#[derive(Debug, Clone)]
pub(crate) struct ColumnTrace {
    pub output: String,
    pub trace: EventTimeTrace,
}

/// The trace transfer function's per-node verdict over the composition walk.
#[derive(Debug, Clone)]
pub(crate) enum TraceNode {
    /// A single SELECT scope: the trace of each projected output column, in
    /// projection order.
    Scope(Vec<ColumnTrace>),
    /// A set operation: the per-arm verdicts, in source order (CTE-definition
    /// children excluded — each reference site carries the definition's
    /// verdict already).
    Branches(Vec<TraceNode>),
    /// A construct the walk cannot model — the entry point rejects the whole
    /// tree fail-closed before walking, so this arm is a defensive mirror of
    /// `OpNode::Unsupported`.
    Unsupported,
}

/// The event-time trace transfer function (`model_properties.md` §"The
/// composition walk"): leaf-level classification is the unchanged pure
/// expression fold (`classify`); the operator rule is projection re-mapping —
/// a scope's leaf column that resolves (via the walk's alias map) to a CTE or
/// derived-table input reduces against that input's own column trace
/// (offsets add, strictness meets, outermost grid wins; `StaticSeed` and
/// `NotTraceable` are absorbing, kind preserved). Every shape the reduction
/// does not positively resolve falls back to the existing name-based
/// `resolve_against_ctx` — the widening is exactly trace-through-projection,
/// never a new expression admission.
pub(crate) struct TraceTransfer<'a> {
    pub ctx: &'a BoundContext,
    pub declared_monotonic: bool,
}

impl TraceTransfer<'_> {
    /// Trace every projected column of one SELECT scope. Wildcard items keep
    /// a placeholder entry (so ordinal correspondence across set-op arms is
    /// preserved) that no outer scope can reduce through.
    fn scope_columns(
        &self,
        sn: &crate::analysis::walk::SelectNode,
        cx: &crate::analysis::walk::NodeCx,
        input_traces: &std::collections::BTreeMap<String, &TraceNode>,
    ) -> Vec<ColumnTrace> {
        let Some(select_list) = sn.select.select_list() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in select_list.items() {
            if item.is_wildcard() {
                out.push(ColumnTrace {
                    output: "*".to_string(),
                    trace: EventTimeTrace::NotTraceable {
                        reason: "wildcard projection is not a single traced column".to_string(),
                        kind: NotTraceableKind::Undecidable,
                    },
                });
                continue;
            }
            let Some(expr) = item.expression() else {
                continue;
            };
            let output = item
                .column_name()
                .unwrap_or_else(|| expr.text().trim().to_string());
            let trace = self.trace_item(&expr, cx, input_traces);
            out.push(ColumnTrace { output, trace });
        }
        out
    }

    /// Trace one projected expression: the pure expression fold, then leaf
    /// resolution through the scope's inputs where that positively resolves,
    /// name-based `resolve_against_ctx` otherwise (unchanged behaviour).
    fn trace_item(
        &self,
        expr: &Expr,
        cx: &crate::analysis::walk::NodeCx,
        input_traces: &std::collections::BTreeMap<String, &TraceNode>,
    ) -> EventTimeTrace {
        use crate::analysis::walk::RelationSource;

        let chain = match classify(expr, self.declared_monotonic) {
            Classification::Trace(chain) => chain,
            Classification::StaticSeed(reason) => return EventTimeTrace::StaticSeed { reason },
            Classification::NotTraceable(reason, kind) => {
                return EventTimeTrace::NotTraceable { reason, kind }
            }
        };

        // Which FROM input does the traced leaf column read from? A
        // qualifier names it; unqualified is unambiguous only with a single
        // input. Anything unresolvable keeps the name-based resolution.
        let qualifier =
            find_leaf_column_ref(expr).and_then(|col| col.qualifier().map(|q| q.to_string()));
        let alias_key = match qualifier {
            Some(q) => Some(q.to_ascii_lowercase()),
            None if cx.aliases.len() == 1 => cx.aliases.keys().next().cloned(),
            None => None,
        };
        let Some(key) = alias_key else {
            return resolve_against_ctx(chain, self.ctx);
        };

        match cx.aliases.get(&key) {
            Some(RelationSource::Table(table)) => {
                // Alias-scoped confirmation against the context (both the
                // segment-path and `smelt.`-prefixed key forms, matching the
                // reach transfer). Confirmation only ever admits; every
                // non-confirming case keeps the name-based verdict.
                let entry = self
                    .ctx
                    .source_partition_cols
                    .get_key_value(table.as_str())
                    .or_else(|| {
                        self.ctx
                            .source_partition_cols
                            .get_key_value(&format!("smelt.{table}"))
                    });
                if let Some((ctx_key, partition_col)) = entry {
                    if partition_col.as_str() == chain.source_column.as_str() {
                        return EventTimeTrace::Traceable {
                            source: ctx_key.clone(),
                            source_column: chain.source_column,
                            offset: chain.offset,
                            monotonicity: chain.monotonicity,
                        };
                    }
                }
                resolve_against_ctx(chain, self.ctx)
            }
            Some(RelationSource::Cte(_)) | Some(RelationSource::DerivedTable(_)) => {
                let inner = input_traces.get(&key).and_then(|node| match node {
                    TraceNode::Scope(cols) => cols
                        .iter()
                        .find(|c| c.output.eq_ignore_ascii_case(&chain.source_column)),
                    // A set-op or unmodellable input is not a simple
                    // projection — no reduction; keep the current verdict.
                    TraceNode::Branches(_) | TraceNode::Unsupported => None,
                });
                match inner {
                    Some(col) => compose_projection(&col.trace, chain),
                    None => resolve_against_ctx(chain, self.ctx),
                }
            }
            None => resolve_against_ctx(chain, self.ctx),
        }
    }
}

impl crate::analysis::walk::Transfer for TraceTransfer<'_> {
    type Verdict = TraceNode;

    fn leaf(
        &self,
        _leaf: &crate::analysis::walk::LeafInput<'_>,
        _cx: &crate::analysis::walk::NodeCx,
    ) -> TraceNode {
        // A base relation projects no expressions of its own; the consuming
        // scope resolves its columns directly against the context.
        TraceNode::Scope(Vec::new())
    }

    fn operator(
        &self,
        op: &crate::analysis::walk::OpNode<'_>,
        children: &[TraceNode],
        cx: &crate::analysis::walk::NodeCx,
    ) -> TraceNode {
        use crate::analysis::walk::{InputItem, OpNode};
        match op {
            OpNode::Unsupported { .. } => TraceNode::Unsupported,
            OpNode::SetOp(so) => TraceNode::Branches(children[so.ctes.len()..].to_vec()),
            OpNode::Select(sn) => {
                // Input verdicts keyed like the walk's alias map, for the
                // projection re-mapping (CTE references and aliased derived
                // tables only — base tables resolve via the context).
                let mut input_traces = std::collections::BTreeMap::new();
                for (input, child) in sn.inputs.iter().zip(&children[sn.ctes.len()..]) {
                    match input {
                        InputItem::CteRef { name, alias } => {
                            let key = alias.as_deref().unwrap_or(name).to_ascii_lowercase();
                            input_traces.insert(key, child);
                        }
                        InputItem::Derived {
                            alias: Some(alias), ..
                        } => {
                            input_traces.insert(alias.to_ascii_lowercase(), child);
                        }
                        _ => {}
                    }
                }
                TraceNode::Scope(self.scope_columns(sn, cx, &input_traces))
            }
        }
    }
}

/// Reduce an outer projection layer (`outer`, the scope's own §expression
/// chain over an input's output column) onto that column's inner verdict:
/// offsets add ([`combine_offset`]), strictness meets, the outermost grid
/// wins. `StaticSeed` and `NotTraceable` are absorbing — a projection layer
/// over them keeps the refusal, including its `Disproven`/`Undecidable` kind.
fn compose_projection(inner: &EventTimeTrace, outer: Chain) -> EventTimeTrace {
    match inner {
        EventTimeTrace::Traceable {
            source,
            source_column,
            offset,
            monotonicity,
        } => EventTimeTrace::Traceable {
            source: source.clone(),
            source_column: source_column.clone(),
            offset: combine_offset(offset.clone(), outer.offset),
            monotonicity: meet_monotonicity(*monotonicity, outer.monotonicity),
        },
        other => other.clone(),
    }
}

/// The monotonicity meet across two stacked layers: booleans conjoin (the
/// stack is monotone/strict only when every layer is), and the grid is
/// last-writer-wins — the outer layer's grid when it truncates, the inner
/// layer's surviving grid otherwise.
fn meet_monotonicity(inner: Monotonicity, outer: Monotonicity) -> Monotonicity {
    Monotonicity {
        is_monotonic: inner.is_monotonic && outer.is_monotonic,
        is_positive: inner.is_positive && outer.is_positive,
        is_always_monotonic: inner.is_always_monotonic && outer.is_always_monotonic,
        is_strict: inner.is_strict && outer.is_strict,
        grid_unit: outer.monotonicity_grid_or(inner.grid_unit),
    }
}

impl Monotonicity {
    /// `self.grid_unit` when set, else `inner` — the outermost
    /// grid-truncating layer governs the final partition grid.
    fn monotonicity_grid_or(&self, inner: Option<Granularity>) -> Option<Granularity> {
        self.grid_unit.or(inner)
    }
}

/// Trace `target_col` (the projected event-time column of the model's
/// outermost scope) through the model's relational composition.
///
/// Pure and fail-closed. Returns `None` when the text has no SELECT, when
/// the walk tree contains a construct the normalization cannot model
/// (`has_unsupported` — the caller keeps its existing non-walk behaviour,
/// mirroring the bound-derivation fallback discipline), or when the root
/// scope does not project `target_col`. For a set-operation root, arms after
/// the first are matched by output name when they alias the same column,
/// falling back to the first arm's ordinal position (SQL takes a compound
/// query's output names from its first arm).
pub fn trace_event_time_composed(
    sql: &str,
    target_col: &str,
    ctx: &BoundContext,
    declared_monotonic: bool,
) -> Option<ComposedTrace> {
    use crate::analysis::walk::QueryTree;

    let tree = QueryTree::from_sql(sql)?;
    trace_event_time_composed_for_tree(&tree, target_col, ctx, declared_monotonic)
}

/// Like [`trace_event_time_composed`], but from an already-parsed outermost
/// `SelectStmt` rather than raw SQL text — avoids a redundant re-parse for
/// callers (e.g. `rules::incremental::trace_union_branches`) that already
/// hold the parsed statement. `select` must be the *whole* set-operation
/// chain's head (the statement `.union_select()`/`.set_operation_select()`
/// walks from) — passing a single arm mid-chain would re-flatten the
/// remaining arms as its own branches.
pub(crate) fn trace_event_time_composed_from_select(
    select: &smelt_parser::SelectStmt,
    target_col: &str,
    ctx: &BoundContext,
    declared_monotonic: bool,
) -> Option<ComposedTrace> {
    use crate::analysis::walk::QueryTree;

    let tree = QueryTree::from_select(select);
    trace_event_time_composed_for_tree(&tree, target_col, ctx, declared_monotonic)
}

/// Shared reduction of an already-built [`QueryTree`] to the composed trace
/// of `target_col` — the common tail of [`trace_event_time_composed`] and
/// [`trace_event_time_composed_from_select`].
fn trace_event_time_composed_for_tree(
    tree: &crate::analysis::walk::QueryTree,
    target_col: &str,
    ctx: &BoundContext,
    declared_monotonic: bool,
) -> Option<ComposedTrace> {
    use crate::analysis::walk::walk;

    if tree.root.has_unsupported() {
        return None;
    }
    let verdict = walk(
        tree,
        &TraceTransfer {
            ctx,
            declared_monotonic,
        },
    );
    match verdict {
        TraceNode::Scope(cols) => cols
            .iter()
            .find(|c| c.output.eq_ignore_ascii_case(target_col))
            .map(|c| ComposedTrace::Single(c.trace.clone())),
        TraceNode::Branches(branches) => {
            let TraceNode::Scope(first) = branches.first()? else {
                return None;
            };
            let position = first
                .iter()
                .position(|c| c.output.eq_ignore_ascii_case(target_col))?;
            let mut out = Vec::with_capacity(branches.len());
            for branch in &branches {
                let TraceNode::Scope(cols) = branch else {
                    return None;
                };
                let col = cols
                    .iter()
                    .find(|c| c.output.eq_ignore_ascii_case(target_col))
                    .or_else(|| cols.get(position))?;
                out.push(col.trace.clone());
            }
            Some(ComposedTrace::Branches(out))
        }
        TraceNode::Unsupported => None,
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

    /// Fail-closed reject: `INTERVAL '2 hours' - event_ts` (constant minus
    /// column, same structural gap as the bare-integer case above but on
    /// the pre-existing `INTERVAL` branch) reverses direction and must not
    /// be admitted as a monotone shift.
    #[test]
    fn trace_interval_minus_column_not_traceable_reverses_direction() {
        let expr = first_select_expr("SELECT INTERVAL '2 hours' - event_ts AS event_time FROM t");
        let ctx = events_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::NotTraceable { reason, kind } => {
                assert_eq!(kind, NotTraceableKind::Disproven);
                assert!(
                    reason.to_lowercase().contains("reverses direction"),
                    "reason should explain the direction reversal, got: {reason}"
                );
            }
            other => panic!("expected NotTraceable (fail closed), got {other:?}"),
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
            EventTimeTrace::NotTraceable {
                kind: NotTraceableKind::Undecidable,
                ..
            }
        ));
    }

    // --- Declared-monotonicity escape hatch (DC1) ---

    #[test]
    fn declared_monotonic_widens_unknown_udf() {
        // Without the declaration: undecidable, stays NotTraceable.
        let expr = first_select_expr("SELECT my_custom_fn(event_ts) AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time_declared(&expr, &ctx, false),
            EventTimeTrace::NotTraceable {
                kind: NotTraceableKind::Undecidable,
                ..
            }
        ));

        // With the declaration: the opaque function's single column-bearing
        // argument is traced through, admitting the pushdown.
        match trace_event_time_declared(&expr, &ctx, true) {
            EventTimeTrace::Traceable {
                source,
                source_column,
                monotonicity,
                ..
            } => {
                assert_eq!(source, "events");
                assert_eq!(source_column, "event_ts");
                assert!(
                    !monotonicity.is_strict,
                    "opaque function weakens strictness"
                );
            }
            other => panic!("expected Traceable under declared_monotonic, got {other:?}"),
        }
    }

    #[test]
    fn declared_monotonic_does_not_widen_static_seed() {
        // A StaticSeed (constant/NULL event-time) is a positive disproof —
        // the declaration must not override it.
        let expr = first_select_expr("SELECT NULL AS event_time FROM t");
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time_declared(&expr, &ctx, true),
            EventTimeTrace::StaticSeed { .. }
        ));
    }

    #[test]
    fn declared_monotonic_does_not_widen_row_nondeterministic_function() {
        // RANDOM()/UUID() etc. in the event-time position is a positive
        // hazard, not an undecidable shape — the declaration cannot widen it
        // even though it would otherwise fall into the "unknown function" arm.
        for func in ["RANDOM()", "UUID()", "GEN_RANDOM_UUID()"] {
            let sql = format!("SELECT {func} AS event_time FROM t");
            let expr = first_select_expr(&sql);
            let ctx = events_ctx();
            assert!(
                matches!(
                    trace_event_time_declared(&expr, &ctx, true),
                    EventTimeTrace::NotTraceable {
                        kind: NotTraceableKind::Disproven,
                        ..
                    }
                ),
                "{func} declared monotonic must still be refused (Disproven), not widened"
            );
        }
    }

    #[test]
    fn declared_monotonic_does_not_widen_case_or_greatest() {
        // Structurally-proven-not-monotone shapes stay Disproven under the
        // declaration too — the escape hatch only ever widens Undecidable.
        let case_expr = first_select_expr(
            "SELECT CASE WHEN x THEN event_ts ELSE other_ts END AS event_time FROM t",
        );
        let ctx = events_ctx();
        assert!(matches!(
            trace_event_time_declared(&case_expr, &ctx, true),
            EventTimeTrace::NotTraceable {
                kind: NotTraceableKind::Disproven,
                ..
            }
        ));

        let greatest_expr =
            first_select_expr("SELECT GREATEST(event_ts, '2026-01-01') AS event_time FROM t");
        assert!(matches!(
            trace_event_time_declared(&greatest_expr, &ctx, true),
            EventTimeTrace::NotTraceable {
                kind: NotTraceableKind::Disproven,
                ..
            }
        ));
    }

    #[test]
    fn declared_monotonic_still_fails_closed_on_ambiguous_leaf() {
        // An opaque function with zero or multiple column-bearing arguments
        // still can't be resolved to a single leaf even when declared.
        let expr = first_select_expr("SELECT my_custom_fn(a, b) AS event_time FROM t");
        let ctx = BoundContext::new()
            .with_source("events", "a")
            .with_source("other", "b");
        assert!(matches!(
            trace_event_time_declared(&expr, &ctx, true),
            EventTimeTrace::NotTraceable {
                kind: NotTraceableKind::Undecidable,
                ..
            }
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

    // ---- BL6: monotone-integer partition keys ----

    fn batch_ctx() -> BoundContext {
        BoundContext::new().with_source("events", "batch_id")
    }

    /// `batch_id + 1` (bare integer, no `INTERVAL` keyword) admits a
    /// monotone integer partition key as `Traceable`, carrying the
    /// magnitude as `Offset::Integer`.
    #[test]
    fn trace_bare_integer_plus_offset_traceable() {
        let expr = first_select_expr("SELECT batch_id + 1 AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable {
                source,
                source_column,
                offset,
                monotonicity,
            } => {
                assert_eq!(source, "events");
                assert_eq!(source_column, "batch_id");
                assert_eq!(offset, Offset::Integer(1));
                assert!(monotonicity.is_strict);
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    /// `batch_id - 5` (bare integer subtraction) is also a constant shift —
    /// same admission as the `+` case, sign preserved.
    #[test]
    fn trace_bare_integer_minus_offset_traceable() {
        let expr = first_select_expr("SELECT batch_id - 5 AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable { offset, .. } => {
                assert_eq!(offset, Offset::Integer(-5));
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    /// Fail-closed reject: `5 - batch_id` (constant minus column) reverses
    /// direction — as `batch_id` increases, the expression *decreases* — so
    /// it must NOT be admitted as a monotone constant shift, even though
    /// `batch_id - 5` (column minus constant) is fine. Regression test for
    /// the bug where `classify_binary` picked `col_side`/`const_side` purely
    /// by which operand contained a column, ignoring operand order under
    /// `-`, and produced a strictly-increasing certification for a
    /// strictly-decreasing expression.
    #[test]
    fn trace_const_minus_column_not_traceable_reverses_direction() {
        let expr = first_select_expr("SELECT 5 - batch_id AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::NotTraceable { reason, kind } => {
                assert_eq!(kind, NotTraceableKind::Disproven);
                assert!(
                    reason.to_lowercase().contains("reverses direction"),
                    "reason should explain the direction reversal, got: {reason}"
                );
            }
            other => panic!("expected NotTraceable (fail closed), got {other:?}"),
        }
    }

    /// Fail-closed reject: `batch_id + 1` addition still commutes, we must
    /// not have accidentally broken the `const + col` order while fixing
    /// the `-` case.
    #[test]
    fn trace_const_plus_column_still_traceable() {
        let expr = first_select_expr("SELECT 1 + batch_id AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::Traceable { offset, .. } => {
                assert_eq!(offset, Offset::Integer(1));
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    /// Fail-closed reject: `batch_id % 3` (modulo) is periodic, not
    /// monotone — `NotTraceable`, reason names "modulo".
    #[test]
    fn trace_integer_modulo_not_traceable_names_modulo() {
        let expr = first_select_expr("SELECT batch_id % 3 AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::NotTraceable { reason, kind } => {
                assert_eq!(kind, NotTraceableKind::Disproven);
                assert!(
                    reason.to_lowercase().contains("modulo"),
                    "reason should name modulo, got: {reason}"
                );
            }
            other => panic!("expected NotTraceable, got {other:?}"),
        }
    }

    /// Fail-closed reject: `batch_id * -1` (multiplication) is not a
    /// constant *shift* — `NotTraceable`, reason names "multiplication".
    #[test]
    fn trace_integer_multiplication_not_traceable_names_multiplication() {
        let expr = first_select_expr("SELECT batch_id * -1 AS partition_key FROM t");
        let ctx = batch_ctx();
        match trace_event_time(&expr, &ctx) {
            EventTimeTrace::NotTraceable { reason, kind } => {
                assert_eq!(kind, NotTraceableKind::Disproven);
                assert!(
                    reason.to_lowercase().contains("multiplication"),
                    "reason should name multiplication, got: {reason}"
                );
            }
            other => panic!("expected NotTraceable, got {other:?}"),
        }
    }

    // ---- Relational composition (the composition walk) ----

    /// A trace that fails ONLY because the event-time column was renamed
    /// through a CTE projection reduces to the source leaf: name-based
    /// expression-level matching alone sees `event_ts`, which is no source's
    /// partition column.
    #[test]
    fn trace_reduces_through_renaming_cte() {
        let sql = "WITH b AS (SELECT ts AS event_ts FROM src) \
                   SELECT event_ts AS event_time FROM b";
        let ctx = BoundContext::new().with_source("src", "ts");
        match trace_event_time_composed(sql, "event_time", &ctx, false) {
            Some(ComposedTrace::Single(EventTimeTrace::Traceable {
                source,
                source_column,
                offset,
                monotonicity,
            })) => {
                assert_eq!(source, "src");
                assert_eq!(source_column, "ts");
                assert_eq!(offset, Offset::Seconds(Seconds::ZERO));
                assert!(monotonicity.is_strict);
                assert!(monotonicity.is_monotonic);
            }
            other => panic!("expected Traceable src.ts through the renaming CTE, got {other:?}"),
        }
    }

    /// Offsets ADD across projection layers and strictness MEETS: a CTE
    /// shifting `+ INTERVAL '1 day'` under an outer `+ INTERVAL '2 hours'`
    /// folds to one 26h offset; a `DATE_TRUNC` in the inner layer weakens
    /// `is_strict` for the whole stack (and its grid survives an outer
    /// shift-only layer).
    #[test]
    fn offsets_add_and_strictness_meets_across_layers() {
        let ctx = BoundContext::new().with_source("src", "ts");

        let sql = "WITH b AS (SELECT ts + INTERVAL '1 day' AS event_ts FROM src) \
                   SELECT event_ts + INTERVAL '2 hours' AS event_time FROM b";
        match trace_event_time_composed(sql, "event_time", &ctx, false) {
            Some(ComposedTrace::Single(EventTimeTrace::Traceable {
                source,
                source_column,
                offset,
                monotonicity,
            })) => {
                assert_eq!(source, "src");
                assert_eq!(source_column, "ts");
                assert_eq!(
                    offset,
                    Offset::Seconds(Seconds(Seconds::days(1).0 + Seconds::hours(2).0)),
                    "layer offsets must fold additively"
                );
                assert!(monotonicity.is_strict, "constant shifts keep strictness");
            }
            other => panic!("expected folded offset across layers, got {other:?}"),
        }

        let sql = "WITH b AS (SELECT DATE_TRUNC('day', ts) AS event_ts FROM src) \
                   SELECT event_ts + INTERVAL '2 hours' AS event_time FROM b";
        match trace_event_time_composed(sql, "event_time", &ctx, false) {
            Some(ComposedTrace::Single(EventTimeTrace::Traceable { monotonicity, .. })) => {
                assert!(
                    !monotonicity.is_strict,
                    "an inner truncation weakens strictness for the whole stack"
                );
                assert_eq!(
                    monotonicity.grid_unit,
                    Some(Granularity::Day),
                    "the inner grid survives an outer shift-only layer"
                );
            }
            other => panic!("expected weakened strictness through the CTE, got {other:?}"),
        }
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
