use super::*;
use crate::DataType;

/// Linear subtyping rank of an expression-typed AST node (Phase 14, §16 #24).
///
/// Every typed node synthesised by the checker carries one of these alongside
/// its [`DataType`]. The ordering `Scalar < Agg < Window` captures SQL's
/// linear "where can this expression appear" rule:
///
/// * `Scalar` — a plain expression (literal, column, arithmetic, scalar
///   function). Acceptable in every splice point.
/// * `Agg` — an aggregate call (`SUM(x)`, `COUNT(*)`, …). Acceptable in
///   `SELECT`, `HAVING`, `ORDER BY`, but not in `WHERE` / `GROUP BY` / `ON`.
/// * `Window` — an aggregate or window function with an `OVER (...)` clause
///   (`ROW_NUMBER() OVER (…)`, `SUM(x) OVER (…)`). Acceptable only in `SELECT`
///   and `QUALIFY`; rejected in `WHERE`, `GROUP BY`, `ON`, etc.
///
/// The check at every splice point is `subkind_of(found, expected)` — O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExprKind {
    /// Plain scalar expression — acceptable in every splice point.
    Scalar,
    /// Aggregate call — `SUM(x)`, `COUNT(*)`, etc.
    Agg,
    /// Aggregate / window call carrying an `OVER (…)` clause.
    Window,
}

impl ExprKind {
    /// Linear rank: `Scalar` = 0, `Agg` = 1, `Window` = 2.
    fn rank(self) -> u8 {
        match self {
            ExprKind::Scalar => 0,
            ExprKind::Agg => 1,
            ExprKind::Window => 2,
        }
    }
}

/// Linear subkind check (§16 #24).
///
/// Returns `true` iff `found` may appear in a context that expects `expected`.
/// The chain is `Scalar <= Agg <= Window`, so a context that accepts `Window`
/// accepts everything; a context that accepts `Scalar` rejects both `Agg`
/// and `Window`.
pub fn subkind_of(found: ExprKind, expected: ExprKind) -> bool {
    found.rank() <= expected.rank()
}

/// Compute the kind ceiling of a list of items (§16 #24, `SelectItems<K>`).
///
/// Returns the maximum kind in the slice. An empty slice is by convention
/// `Scalar` — this matches the empty-default for an empty `SelectItems<K>`
/// value (which only arises from error recovery; well-formed SELECT lists
/// have at least one item).
pub fn kind_ceiling(items: &[ExprKind]) -> ExprKind {
    let mut max = ExprKind::Scalar;
    for &k in items {
        if k.rank() > max.rank() {
            max = k;
        }
    }
    max
}

/// How a built-in is spelled at a call site. Required so operators can be
/// registry entries at all, and what lets the audit harness derive a probe from
/// a signature instead of a hand-written table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SyntaxForm {
    /// `NAME(a, b)` — the default, and the only form on the callable-function surface.
    #[default]
    Call,
    /// `a OP b` — `%`, `^`, `**`, `||`, `//`, `LIKE`, `ILIKE`, `GLOB`.
    Infix,
    /// `a OP` — `IS NULL`, `IS NOT NULL`.
    Postfix,
    /// `FROM UNNEST(a)` — a table function; not a scalar call position.
    TableFn,
    /// Dedicated syntax with no uniform shape — `CAST(x AS T)`, `a BETWEEN b AND c`,
    /// `a IN (…)`, `EXISTS (…)`, interval add/sub.
    Special,
}

/// The per-dialect verdict for one registry entry: how the built-in must be
/// spelled so the backend computes what smelt's semantics say it computes.
///
/// `Native` is a **claim, not an assumption**. The audit's value leg exists to
/// test it, and an untested `Native` is reported as *unverified* rather than as
/// *passing*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// Same spelling, same semantics.
    Native,
    /// Same call shape, different name.
    Rename(&'static str),
    /// The target spells this built-in as a fixed shape over the call's own
    /// positional arguments (`docs/specs/multi_backend.md` §"Template
    /// emission"). The `&'static str` is target-dialect text in which `{n}`
    /// names the call's zero-based `n`-th positional argument — an infix
    /// `BINARY_EXPR` supplies exactly two, `{0}` the left operand and `{1}`
    /// the right. A placeholder may reference an argument any number of
    /// times; every fixed parameter must be referenced at least once.
    /// Validated once at registry construction by [`validate_template`]:
    /// every placeholder index within the signature's fixed arity, balanced
    /// parentheses, no unreferenced fixed parameter, no template on a
    /// variadic signature, and no non-call-shaped template stated at a
    /// window position. Interpreted by one generic printer routine that
    /// substitutes each placeholder with the argument's own printed text,
    /// parenthesising a compound argument (`BINARY_EXPR`, `CASE_EXPR`,
    /// `CAST_EXPR` — comparisons and unary forms reuse `BINARY_EXPR`) and
    /// leaving an atom (literal, identifier, call) bare; a template whose
    /// outermost shape is not itself a single call is wrapped in parentheses
    /// so its result composes safely in the surrounding expression. The
    /// printer holds no function names and no per-template special case.
    Template(&'static str),
    /// Structural rewrite: the printer owns the code, the registry owns the claim.
    Rewrite(RewriteId),
    /// Statement-level restructure: the backend offers this built-in only in
    /// the opposite position from the one the call occupies, so no
    /// expression-level substitution exists — the whole query block is
    /// restructured around a synthesised CTE (`docs/specs/multi_backend.md`
    /// §"Statement-level lowering"). The registry states *that* the position
    /// needs restructuring and *which shape*; a pure planner (never the
    /// printer) turns the claim into data, and admissibility over the
    /// enclosing query block decides whether the restructure can be taken at
    /// all.
    Restructure(RestructureId),
    /// The backend cannot express this. A diagnostic, never a silent pass-through.
    Unsupported { reason: &'static str },
    /// The verdict depends on the call's own arity and/or operand types
    /// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
    /// An ordered list of arms; the first arm whose guard the call satisfies
    /// wins. Must end in an `otherwise` arm (arity `None`, classes `&[]`) —
    /// validated once at registry construction by [`validate_conditional`].
    /// Resolved to a [`SettledEmission`] on the compile path by
    /// [`Signature::settle_at`]; the printer never inspects this variant.
    Conditional(&'static [ConditionalArm]),
}

/// An argument's class for an [`Emission::Conditional`] guard — a pure, total
/// function of its inferred [`DataType`]
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
///
/// A guard names classes, never concrete types: the engine behaviours this
/// axis exists for split along exactly these lines, and a finer key would
/// multiply arms without buying a different answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandClass {
    /// The integer widths (`SmallInt`, `Integer`, `BigInt`).
    Integral,
    /// `Decimal { .. }`.
    Decimal,
    /// `Float`, `Double`.
    Floating,
    /// `Varchar`, `Char`, `Text`.
    String,
    /// `Boolean`.
    Boolean,
    /// `Date`, `Time`, and the timestamps.
    Temporal,
    /// `Interval`.
    Interval,
    /// `Array`, `Struct`, `Map`.
    Composite,
    /// `Blob`.
    Binary,
    /// The argument's type could not be resolved — `DataType::Unknown` (type
    /// inference gave up) or `DataType::Null` (a NULL literal discriminates
    /// nothing). This is the fail-safe direction the cost rule in
    /// `docs/specs/multi_backend.md` §"Operand-conditional verdicts" demands:
    /// an unresolved operand always lands on the `otherwise` arm.
    Unresolved,
}

impl OperandClass {
    /// Classify a [`DataType`] into its [`OperandClass`]. Total — a new
    /// `DataType` variant is a compile error here, never a silent
    /// misclassification via a wildcard arm.
    pub fn of(dt: &DataType) -> OperandClass {
        match dt {
            DataType::Boolean => OperandClass::Boolean,
            DataType::SmallInt | DataType::Integer | DataType::BigInt => OperandClass::Integral,
            DataType::Float | DataType::Double => OperandClass::Floating,
            DataType::Decimal { .. } => OperandClass::Decimal,
            DataType::Varchar { .. } | DataType::Char { .. } | DataType::Text => {
                OperandClass::String
            }
            DataType::Blob => OperandClass::Binary,
            DataType::Date | DataType::Time | DataType::Timestamp { .. } => OperandClass::Temporal,
            DataType::Interval => OperandClass::Interval,
            DataType::Array(_) | DataType::Struct(_) | DataType::Map(_, _) => {
                OperandClass::Composite
            }
            DataType::Null | DataType::Unknown(_) => OperandClass::Unresolved,
        }
    }
}

/// A settled [`Emission`] verdict — the six ordinary verdicts an
/// [`Emission::Conditional`] arm may carry, and what [`Signature::settle_at`]
/// always returns. Unlike [`Emission`], this type has no `Conditional`
/// variant: a nested conditional is unrepresentable by construction, not
/// merely rejected at validation time. This is what the printer consumes —
/// it holds no type context and cannot itself resolve an arm
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettledEmission {
    /// Same spelling, same semantics.
    Native,
    /// Same call shape, different name.
    Rename(&'static str),
    /// A fixed shape over the call's own positional arguments. See
    /// [`Emission::Template`].
    Template(&'static str),
    /// Structural rewrite. See [`Emission::Rewrite`].
    Rewrite(RewriteId),
    /// Statement-level restructure. See [`Emission::Restructure`].
    Restructure(RestructureId),
    /// The backend cannot express this. See [`Emission::Unsupported`].
    Unsupported { reason: &'static str },
}

/// One arm of an [`Emission::Conditional`] entry.
///
/// `arity`: `Some(n)` guards on the call having exactly `n` arguments;
/// `None` matches any arity. `classes`: `(argument_index, required_class)`
/// pairs, all of which must match for the arm to apply; `&[]` matches any
/// operand shape. The mandatory final `otherwise` arm has `arity: None` and
/// `classes: &[]`, so it always matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionalArm {
    pub arity: Option<usize>,
    pub classes: &'static [(usize, OperandClass)],
    pub verdict: SettledEmission,
}

impl ConditionalArm {
    /// Does `facts` satisfy this arm's guard?
    pub(super) fn matches(&self, facts: &CallFacts) -> bool {
        if let Some(arity) = self.arity {
            if facts.arity != arity {
                return false;
            }
        }
        self.classes
            .iter()
            .all(|(idx, class)| facts.class_at(*idx) == Some(*class))
    }

    /// Is this arm unconditional (the required shape of the mandatory
    /// trailing `otherwise` arm)?
    fn is_otherwise(&self) -> bool {
        self.arity.is_none() && self.classes.is_empty()
    }
}

/// The call-site facts an [`Emission::Conditional`] entry is resolved
/// against: the call's own arity, and each argument's [`OperandClass`] by
/// position. Built once on the compile path from the source CST and the
/// same type inference that derives the model's projection; the printer
/// never constructs one from a `DataType` — see
/// [`CallFacts::unresolved`] for the printer's own lookup-miss fallback,
/// which needs only arity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts {
    arity: usize,
    classes: Vec<OperandClass>,
}

impl CallFacts {
    /// Build call facts from a resolved class per positional argument.
    pub fn new(classes: Vec<OperandClass>) -> Self {
        Self {
            arity: classes.len(),
            classes,
        }
    }

    /// A call whose arity is known but whose operand types are not — every
    /// class is [`OperandClass::Unresolved`], so any class-guarded arm is
    /// skipped and resolution lands on `otherwise`.
    pub fn unresolved(arity: usize) -> Self {
        Self {
            arity,
            classes: vec![OperandClass::Unresolved; arity],
        }
    }

    /// The class of the argument at `index`, or `None` if `index` is beyond
    /// this call's arity.
    fn class_at(&self, index: usize) -> Option<OperandClass> {
        self.classes.get(index).copied()
    }
}

/// Why an [`Emission::Conditional`] entry failed registry-construction
/// validation (`docs/specs/multi_backend.md` §"Operand-conditional
/// verdicts"). Every variant names the offending signature, mirroring
/// [`TemplateError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalError {
    /// The arm list has no trailing `otherwise` arm (`arity: None, classes:
    /// &[]`), or the `otherwise` arm is not last.
    MissingOtherwise { signature: String },
    /// An arm's `arity` guard names a call shape the signature itself does
    /// not admit (fewer than the signature's non-variadic arity, or more
    /// than it admits when the signature has no variadic tail).
    ArityNotAdmitted { signature: String, arity: usize },
    /// An arm's class guard names an argument index at or beyond the arity
    /// it guards on (or, for an arity-less arm, beyond the signature's own
    /// fixed arity).
    ArgumentIndexOutOfRange { signature: String, index: usize },
}

impl std::fmt::Display for ConditionalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConditionalError::MissingOtherwise { signature } => write!(
                f,
                "conditional emission for `{signature}` has no trailing `otherwise` arm (arity \
                 `None`, classes `&[]`)"
            ),
            ConditionalError::ArityNotAdmitted { signature, arity } => write!(
                f,
                "conditional emission for `{signature}` has an arm guarding on arity {arity}, \
                 which the signature does not admit"
            ),
            ConditionalError::ArgumentIndexOutOfRange { signature, index } => write!(
                f,
                "conditional emission for `{signature}` has an arm naming argument index \
                 {index}, beyond the arity it guards on"
            ),
        }
    }
}

impl std::error::Error for ConditionalError {}

/// Validate an [`Emission::Conditional`] entry at registry-construction time.
/// Pure — the registry seed converts an `Err` into a build-time panic, the
/// same discipline as [`validate_template`].
pub fn validate_conditional(
    arms: &'static [ConditionalArm],
    sig: &Signature,
) -> Result<(), ConditionalError> {
    let variadic = matches!(sig.params.last(), Some(SigParam::Variadic(_)));
    let fixed_arity = sig.params.len();

    match arms.last() {
        Some(last) if last.is_otherwise() => {}
        _ => {
            return Err(ConditionalError::MissingOtherwise {
                signature: sig.name.clone(),
            })
        }
    }
    // No arm before the last may itself be an `otherwise` arm — the ordered
    // list would then have unreachable arms after it, and validation must
    // catch a malformed list, not merely "there exists an otherwise arm".
    if arms[..arms.len() - 1]
        .iter()
        .any(ConditionalArm::is_otherwise)
    {
        return Err(ConditionalError::MissingOtherwise {
            signature: sig.name.clone(),
        });
    }

    for arm in arms {
        if let Some(arity) = arm.arity {
            let admitted = if variadic {
                arity + 1 >= fixed_arity
            } else {
                arity == fixed_arity
            };
            if !admitted {
                return Err(ConditionalError::ArityNotAdmitted {
                    signature: sig.name.clone(),
                    arity,
                });
            }
        }
        let bound = arm.arity.unwrap_or(fixed_arity);
        for (idx, _) in arm.classes {
            if *idx >= bound {
                return Err(ConditionalError::ArgumentIndexOutOfRange {
                    signature: sig.name.clone(),
                    index: *idx,
                });
            }
        }
    }
    Ok(())
}

/// The SQL call-site context an emission verdict is stated for.
///
/// A built-in's support on a backend routinely differs between the positions
/// it can appear in — GoogleSQL refuses `PERCENTILE_CONT` under a `GROUP BY`
/// but accepts it with an `OVER` clause, while `MAX_BY` is the exact reverse
/// — so a verdict is looked up by `(dialect, position)`, never by dialect
/// alone. `Any` is a lookup wildcard for an entry whose verdict does not vary
/// by position; it is never returned by a classifier that decides a call's
/// actual position from its source CST — such a classifier always resolves
/// to one of the other four variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Position {
    /// Lookup wildcard, matching any call position. Never returned by a
    /// position classifier — only ever used as a stated verdict key.
    Any,
    /// A row-wise expression: no `OVER` clause, and not itself an aggregate
    /// call. A scalar call under a `GROUP BY` (e.g. applied to a grouping
    /// key or in a `WHERE` clause) is still `Scalar` — the enclosing
    /// statement's `GROUP BY` does not change a call's own position.
    Scalar,
    /// The call is itself an aggregate call, with no `OVER` clause.
    Aggregate,
    /// An `OVER` clause whose window covers the call's whole partition —
    /// after resolving any named-window reference, no window `ORDER BY` and
    /// no frame clause, or an explicit `BETWEEN UNBOUNDED PRECEDING AND
    /// UNBOUNDED FOLLOWING` frame with no `EXCLUDE` clause.
    WholePartitionWindow,
    /// An `OVER` clause whose window is narrower than its whole partition —
    /// includes the common `ORDER BY` with no explicit frame (whose SQL
    /// default frame is `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT
    /// ROW`), any frame carrying `EXCLUDE`, and an unresolvable named-window
    /// reference (refusing is the safe direction: it costs a diagnostic,
    /// where guessing costs a wrong number).
    Window,
}

/// A structural rewrite the printer implements. Enumerable by construction, so
/// the set of rewrites is knowable without reading the printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewriteId {
    /// `MEDIAN(x)` → `PERCENTILE_CONT(x, 0.5)` in window position, an
    /// `ARRAY_AGG`-indexing `CASE` in aggregate position. Position-dependent;
    /// the registry says *that* it needs rewriting, the printer says *how*.
    ///
    /// Not a template: the output shape itself differs by call position (a
    /// single substitution in window position, a multi-statement `CASE` over
    /// an `ARRAY_AGG` in aggregate position) — a `{n}` placeholder names an
    /// argument, not a choice of output shape.
    BigQueryMedian,
    /// `PERCENTILE_CONT(f) WITHIN GROUP (ORDER BY x)` → `PERCENTILE_CONT(x,
    /// f)` at a whole-partition window position — GoogleSQL's two-argument
    /// analytic spelling, since `WITHIN GROUP` under an `OVER` clause is a
    /// syntax error there (measured live 2026-08-27). The window itself is
    /// left as-is; only the call's own spelling changes. A `DESC` sort key
    /// inverts the fraction argument; a `NULLS FIRST`/`LAST` modifier the
    /// analytic form cannot express is refused upstream by
    /// `emission_check`, never reaching the printer
    /// (`restructure::within_group_sort_key` is the shared reader for both
    /// this rewrite and `RestructureId::AnalyticToCte`).
    ///
    /// Not a template: the sort key and its direction come from the call's
    /// own `WITHIN GROUP (ORDER BY …)` clause, a construct a positional `{n}`
    /// placeholder cannot address — the rewrite reads that clause with
    /// `within_group_sort_key` rather than substituting a positional argument.
    WithinGroupToAnalytic,
}

/// A statement-level restructure shape. Enumerable by construction, mirroring
/// [`RewriteId`] — the set of shapes is knowable without reading the planner.
///
/// Correctness oracle: `docs/specs/multi_backend.md` §"Statement-level
/// lowering".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RestructureId {
    /// An aggregate-only built-in reached with an `OVER` clause (GoogleSQL's
    /// `MAX_BY`/`MIN_BY`/`APPROX_COUNT_DISTINCT`; DuckDB's and Spark's
    /// ordered-set `PERCENTILE_CONT`/`PERCENTILE_DISC`). The source is bound
    /// once, grouped by the call's partition keys, and joined back —
    /// admissible only at `Position::WholePartitionWindow`.
    WindowToCte,
    /// An analytic-only built-in reached under `GROUP BY` (GoogleSQL's
    /// `PERCENTILE_CONT`/`PERCENTILE_DISC`, which require an `OVER` clause
    /// and reject `WITHIN GROUP` outright). The query's `FROM`/`WHERE` move
    /// into a CTE that adds the value as an analytic column over the
    /// grouping keys, read back through `ANY_VALUE`.
    AnalyticToCte,
}

/// Why an [`Emission::Template`] row failed registry-construction validation
/// (`docs/specs/multi_backend.md` §"Template emission"). Every variant names
/// the malformed template and the signature it was declared against, so a bad
/// registry seed fails loudly with enough context to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// A placeholder `{n}` named an index at or beyond the signature's fixed
    /// (non-variadic) arity.
    IndexOutOfRange {
        signature: String,
        index: usize,
        arity: usize,
    },
    /// A fixed parameter had no placeholder referencing it — the template
    /// would silently drop an argument the call supplies.
    ArgumentUnreferenced { signature: String, index: usize },
    /// The template's literal parentheses (outside `{n}` placeholders) do not
    /// balance.
    UnbalancedParens { signature: String },
    /// A template stated at `Window`/`WholePartitionWindow`, or at `Any` for
    /// a signature whose `kind` is `Agg`/`Window`, is not call-shaped —
    /// `(expr) OVER (…)` is not valid SQL, so the substituted form must be a
    /// single call.
    NonCallAtWindowPosition { signature: String },
    /// A template was declared on a signature with a trailing variadic
    /// parameter — a fixed `{n}` placeholder cannot name a variadic tail.
    VariadicSignature { signature: String },
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::IndexOutOfRange {
                signature,
                index,
                arity,
            } => write!(
                f,
                "template for `{signature}` references index {{{index}}}, but the signature's fixed arity is {arity}"
            ),
            TemplateError::ArgumentUnreferenced { signature, index } => write!(
                f,
                "template for `{signature}` never references fixed parameter {index} — it would silently drop an argument"
            ),
            TemplateError::UnbalancedParens { signature } => write!(
                f,
                "template for `{signature}` has unbalanced parentheses"
            ),
            TemplateError::NonCallAtWindowPosition { signature } => write!(
                f,
                "template for `{signature}` is stated at a window position but is not call-shaped — `(expr) OVER (…)` is not valid SQL"
            ),
            TemplateError::VariadicSignature { signature } => write!(
                f,
                "template for `{signature}` is declared on a variadic signature — a placeholder cannot name a variadic tail"
            ),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Structural (never textual) test for "the template's outermost form is a
/// single call": an identifier followed by a parenthesised group that closes
/// exactly at the end of the string — `MOD({0}, {1})`, not `{0} - {1}` or
/// `DAYOFWEEK({0}) - 1`.
///
/// Shared by [`validate_template`] (a non-call template is refused at a
/// window position) and the printer's interpreter (a non-call template's
/// substituted output is wrapped in parentheses so it composes correctly in
/// the surrounding expression; a call-shaped one never needs argument-level
/// wrapping, since a function call's comma-separated arguments are already
/// unambiguously delimited).
pub fn is_call_shaped_template(template: &str) -> bool {
    let t = template.trim();
    let Some(open) = t.find('(') else {
        return false;
    };
    if !t.ends_with(')') || open == t.len() - 1 {
        return false;
    }
    let name = &t[..open];
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }
    // The opening paren at `open` must close exactly at the template's last
    // byte — a trailing operator after a balanced call, e.g. `MOD(a, b) + 1`,
    // is not call-shaped even though it ends in `)`.
    let mut depth = 0i32;
    for (i, c) in t.char_indices().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i == t.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Validate an [`Emission::Template`] row at registry-construction time
/// (`docs/specs/multi_backend.md` §"Template emission"). Pure — no I/O, no
/// panics; the registry seed converts an `Err` into a build-time panic so a
/// malformed template is a compile-adjacent failure, never a runtime one.
pub fn validate_template(
    template: &'static str,
    sig: &Signature,
    position: Position,
) -> Result<(), TemplateError> {
    let variadic = matches!(sig.params.last(), Some(SigParam::Variadic(_)));
    if variadic {
        return Err(TemplateError::VariadicSignature {
            signature: sig.name.clone(),
        });
    }
    let arity = sig.params.len();

    let mut depth = 0i32;
    let mut referenced = vec![false; arity];
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(TemplateError::UnbalancedParens {
                        signature: sig.name.clone(),
                    });
                }
                i += 1;
            }
            b'{' => {
                let Some(rel_end) = template[i..].find('}') else {
                    return Err(TemplateError::UnbalancedParens {
                        signature: sig.name.clone(),
                    });
                };
                let end = i + rel_end;
                let idx: usize =
                    template[i + 1..end]
                        .parse()
                        .map_err(|_| TemplateError::IndexOutOfRange {
                            signature: sig.name.clone(),
                            index: 0,
                            arity,
                        })?;
                if idx >= arity {
                    return Err(TemplateError::IndexOutOfRange {
                        signature: sig.name.clone(),
                        index: idx,
                        arity,
                    });
                }
                referenced[idx] = true;
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 {
        return Err(TemplateError::UnbalancedParens {
            signature: sig.name.clone(),
        });
    }
    if let Some(index) = referenced.iter().position(|seen| !seen) {
        return Err(TemplateError::ArgumentUnreferenced {
            signature: sig.name.clone(),
            index,
        });
    }

    let call_shaped = is_call_shaped_template(template);
    let window_position = matches!(position, Position::Window | Position::WholePartitionWindow)
        || (position == Position::Any && matches!(sig.kind, ExprKind::Agg | ExprKind::Window));
    if window_position && !call_shaped {
        return Err(TemplateError::NonCallAtWindowPosition {
            signature: sig.name.clone(),
        });
    }

    Ok(())
}
