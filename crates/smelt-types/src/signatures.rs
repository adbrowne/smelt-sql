//! Function signature indexing for `smelt.define` declarations.
//!
//! This module defines the data shape used by the Salsa-backed function
//! registry (`smelt-db::functions_in_file`, `function_signature`, etc.) and
//! the pure extraction function that produces it from a parsed AST.
//!
//! Pure-function rule (CLAUDE.md): everything here is dependency-free
//! w.r.t. Salsa. Callers in `smelt-db` are responsible for wiring these
//! extractors into tracked queries.
//!
//! Phase 3 scope: raw `type_ref_text` only. Phase 4 adds structured `Expr<T>`
//! parsing into [`SmeltType`], alongside [`TypeConstraint`] for the `Numeric`
//! / `Any` constraints per §16 #9. Phase 7 adds the [`TypeConstraint::Ordered`]
//! member (§16 #13) and a monomorphic [`BuiltinRegistry`] skeleton seeded with
//! a handful of SQL built-ins. Phase 8 extends the registry with angle-bracket
//! generics and trailing variadic parameters (§16 #14 + #15), adds
//! [`unify_call`] for signature-driven type inference, and seeds ~30
//! commonly-used SQL built-ins. Non-`Expr` sorts (TableExpr, AggExpr, …) remain
//! deferred to later phases of the smelt-functions plan.

use crate::{parse_type, DataType};
use smelt_parser::ast::{
    File as AstFile, Param as AstParam, Range, SmeltDefine, SmeltExtern, TypeRef,
};
use smelt_parser::offset_to_position;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

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

/// Constraint placed on the type parameter of a fragment sort (e.g. `Expr<T>`).
///
/// `Concrete(dt)` pins the parameter to a single [`DataType`]. The abstract
/// constraints (`Numeric`, `Any`) are drawn from §16 #9 of the smelt-functions
/// research. Additional constraints (`Ordered`, …) land in later phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeConstraint {
    /// A single concrete [`DataType`] such as `Integer` or `Boolean`.
    Concrete(DataType),
    /// Any numeric type per §16 #9: `SmallInt`, `Integer`, `BigInt`, `Float`,
    /// `Double`, `Decimal`. Boolean is deliberately excluded.
    Numeric,
    /// Any type with a total order on every v1 backend, per §16 #13.
    ///
    /// Members: `Numeric` ∪ the string family (`Text`, `Varchar`, `Char`) ∪
    /// the temporal family (`Date`, `Time`, `Timestamp` with either tz, and
    /// `Interval`) ∪ `Boolean` ∪ `Blob`. Composite types (`Array`, `Struct`,
    /// `Map`) are excluded in v1 because their ordering semantics diverge
    /// across backends — `Null`/`Unknown` are likewise non-members.
    Ordered,
    /// Any type — effectively "no constraint". Reserved for parameters like
    /// `x: Expr<Any>`.
    Any,
}

impl TypeConstraint {
    /// Does the given [`DataType`] satisfy this constraint?
    ///
    /// Pure, deterministic, no Salsa dependency. Used both in signature checks
    /// and as the oracle for the registry-wide `numeric_constraint_*` tests.
    pub fn satisfies(&self, dt: &DataType) -> bool {
        match self {
            TypeConstraint::Concrete(expected) => expected == dt,
            // Centralise the numeric membership on `DataType::is_numeric()` so
            // there is a single source of truth matching §16 #9.
            TypeConstraint::Numeric => dt.is_numeric(),
            // §16 #13 members: numeric ∪ string family ∪ temporal family ∪
            // {Boolean, Blob}. Composite types, `Null`, and `Unknown` are
            // intentionally not members.
            TypeConstraint::Ordered => {
                dt.is_numeric()
                    || dt.is_string()
                    || dt.is_temporal()
                    || matches!(dt, DataType::Boolean | DataType::Blob)
            }
            TypeConstraint::Any => true,
        }
    }
}

/// Parsed smelt type reference.
///
/// Phase 4 only models the `Expr<T>` sort. Later phases will extend this with
/// `TableExpr`, `AggExpr`, etc. Unsupported sorts surface as
/// [`SmeltTypeParseError::UnsupportedSort`] rather than panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmeltType {
    /// `Expr<T>` where T is a [`TypeConstraint`] — either a concrete
    /// [`DataType`] or one of the abstract constraints in
    /// [`TypeConstraint`].
    Expr(TypeConstraint),
    /// `List<T>` — a compile-time meta-list of elements typed `T` (Phase A,
    /// meta-language). `T` is itself a [`SmeltType`] (enabling `List<List<T>>`
    /// nesting). No `List<T>` value reaches the database engine.
    List(Box<SmeltType>),
    /// `Lambda<T, U>` — a meta-language lambda value (Phase B).
    ///
    /// Constructed only at HOF positional-argument positions (e.g. the
    /// second argument of `map`, `filter`). **Meta-only** — not user-writable
    /// as a `smelt.define` parameter sort or return type. No `Lambda<T, U>`
    /// value reaches the database engine.
    ///
    /// Lambda is **invariant**: `is_subtype_of(Lambda<S1, T1>, Lambda<S2, T2>)`
    /// is `true` only when `S1 = S2` and `T1 = T2` (byte-equal).
    Lambda(Box<SmeltType>, Box<SmeltType>),
    /// Compiler's "already told you about this" type for list elements (Phase A).
    ///
    /// `Unknown` is produced when a list literal's element types cannot be
    /// unified (heterogeneous) or when an empty literal has no target context.
    /// It does NOT silently become `Any` — downstream consumers see it as a
    /// known-error marker per `gradual_typing.md` §"List<Unknown> widening".
    Unknown,
    /// `TableExpr` / `TableExpr<{…}>` — a row-polymorphic table parameter.
    ///
    /// - `TableExpr(None)` — bare `TableExpr` (Phase 15, §16 #7). The
    ///   caller's schema is accepted in full at call-site expansion,
    ///   no shape check runs.
    /// - `TableExpr(Some(req))` — `TableExpr<{col: Type, ..r}>`
    ///   (Phase 16). The caller's schema is verified against `req`
    ///   *before* the body is re-walked: missing columns and
    ///   constraint violations surface as
    ///   [`DiagnosticCode::RowRequirementUnsatisfied`] call-site
    ///   errors, and if `req.tail` is [`RowTail::Named`] the extras
    ///   are captured into the call's `row_var_env`.
    TableExpr(Option<SchemaRequirement>),
    /// `SelectItems<Kind>` / `SelectItems<Kind, ctx>` — a typed list of
    /// SELECT items (Phase 21).
    ///
    /// - `kind` is the required [`ExprKind`] ceiling for every item
    ///   in the list (e.g. `ExprKind::Agg` requires each item to be
    ///   at least aggregate-kind).
    /// - `context` names the [`ContextRef`] whose column schema the
    ///   items are validated against.
    SelectItems {
        kind: ExprKind,
        context: Option<ContextRef>,
    },
    /// `Expr<Struct<{field: Type, ..tail}>>` — a row-polymorphic struct
    /// parameter (Phase 35).
    ///
    /// This is the **type-level** struct descriptor for a *parameter*
    /// declared with `Struct<{...}>` inside an `Expr<...>` wrapper. It
    /// differs from [`crate::DataType::Struct`], which represents the
    /// *runtime value type* of a struct expression.
    ///
    /// - `fields` is the ordered list of `(field_name, required_type)`
    ///   pairs declared in the signature.
    /// - `tail` is the trailing row-variable marker: none, anonymous, or
    ///   named.
    Struct {
        fields: Vec<(String, DataType)>,
        tail: StructRowTail,
    },
    /// `ColumnRef` — a closed meta-only record type produced by
    /// `smelt.columns_of` (Phase C, meta-language reflection).
    ///
    /// Values of this type describe a single column from a `TableExpr`'s
    /// schema. The type has exactly three fields (see [`COLUMN_REF_FIELDS`]):
    ///   - `name: Text` — the column identifier (un-quoted, case-preserved)
    ///   - `type: DataType` (meta literal) — the column's smelt `DataType`
    ///   - `is_numeric: Boolean` — `TRUE` iff `type` is in the `Numeric` constraint set
    ///
    /// **Meta-only**: not user-writable as a `smelt.define` parameter or
    /// return type. Values originate only from `smelt.columns_of` (Phase C)
    /// and future reflection accessors (Phase D). No `ColumnRef` value
    /// reaches the database engine.
    ///
    /// **Closed**: the v1 field set is exactly `{name, type, is_numeric}`.
    /// Adding a field requires a spec edit and a compiler change.
    ColumnRef,
}

/// Subtype check for [`SmeltType`] (Phase A, meta-language).
///
/// Returns `true` iff `sub` is a subtype of `sup` under the following rules:
///
/// * Every type is a subtype of itself (reflexivity).
/// * `Expr<S> <: Expr<T>` iff the `ExprKind` ordering holds per
///   [`subkind_of`] — but we're checking data-type constraints here, so
///   `Expr<S> <: Expr<T>` only when `S == T` (concrete equality) or `T` is
///   an abstract constraint that `S` satisfies.
/// * `List<S> <: List<T>` iff `S <: T` — lists are **covariant** because
///   they are immutable compile-time values.
/// * All other combinations return `false`.
///
/// Pure function — no Salsa dependency.
pub fn is_subtype_of(sub: &SmeltType, sup: &SmeltType) -> bool {
    match (sub, sup) {
        // Reflexivity for identical variants.
        (a, b) if a == b => true,
        // List covariance: List<S> <: List<T> iff S <: T.
        (SmeltType::List(s_inner), SmeltType::List(t_inner)) => is_subtype_of(s_inner, t_inner),
        // Lambda invariance: Lambda<S1, T1> <: Lambda<S2, T2> only when S1 = S2 and T1 = T2.
        // The reflexivity arm above already handles the equal case (`a == b`), so
        // any non-equal Lambda pair falls through to the `_ => false` arm.
        // We add this arm for documentation clarity; it is unreachable in practice
        // because the reflexivity arm already fires for equal Lambdas.
        (SmeltType::Lambda(_, _), SmeltType::Lambda(_, _)) => false,
        // Expr<S> <: Expr<T> — the inner constraint determines compatibility.
        (SmeltType::Expr(s_tc), SmeltType::Expr(t_tc)) => {
            match (s_tc, t_tc) {
                // Concrete <: abstract constraint: the concrete type must
                // satisfy the abstract constraint.
                (TypeConstraint::Concrete(dt), TypeConstraint::Numeric) => {
                    TypeConstraint::Numeric.satisfies(dt)
                }
                (TypeConstraint::Concrete(dt), TypeConstraint::Ordered) => {
                    TypeConstraint::Ordered.satisfies(dt)
                }
                (TypeConstraint::Concrete(_), TypeConstraint::Any) => true,
                (TypeConstraint::Numeric, TypeConstraint::Any) => true,
                (TypeConstraint::Ordered, TypeConstraint::Any) => true,
                (TypeConstraint::Numeric, TypeConstraint::Ordered) => true,
                // Any other pair that isn't identical is not a subtype.
                _ => false,
            }
        }
        // All other cross-sort combinations are not subtypes.
        _ => false,
    }
}

/// Trailing row-polymorphism marker on a `Struct<{…}>` parameter type
/// (Phase 35).
///
/// Mirrors the shape of [`RowTail`] used by `TableExpr<{…}>` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructRowTail {
    /// No tail marker — the struct shape is fully concrete.
    None,
    /// `..` — anonymous row variable; extra fields accepted but not
    /// observable by name in the function body.
    Anon,
    /// `..<name>` — named row variable; extra fields are bound to this
    /// name and may be referenced / spread in the function body.
    Named(String),
}

/// Per-column requirement inside a [`SchemaRequirement`] (Phase 16).
///
/// Introduced by `TableExpr<{col: Type, ..}>` type annotations. Each
/// field can be:
///   - [`DataTypeReq::Concrete`] — a single concrete [`DataType`]
///     (e.g. `order_id: BigInt`). The caller's column must match
///     exactly via [`types_compatible_for_row_requirement`].
///   - [`DataTypeReq::Constraint`] — a constraint set from
///     [`TypeConstraint`] (e.g. `revenue: Numeric`). The caller's
///     column's data type must satisfy the constraint via
///     [`TypeConstraint::satisfies`].
///
/// The enum mirrors [`TypeConstraint`] / [`DataType`] so existing
/// constraint-satisfaction code applies unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataTypeReq {
    /// The caller's column must be (exactly) this [`DataType`].
    Concrete(DataType),
    /// The caller's column's data type must satisfy this
    /// [`TypeConstraint`] (e.g. `Numeric`, `Ordered`).
    Constraint(TypeConstraint),
}

impl DataTypeReq {
    /// Is the caller's [`DataType`] compatible with this requirement?
    ///
    /// - `Concrete(expected)`: matched against `actual` via the shared
    ///   row-requirement compatibility helper (equal, or a normalized
    ///   `Text ↔ Varchar` pair).
    /// - `Constraint(c)`: delegated to [`TypeConstraint::satisfies`].
    ///
    /// `Unknown` / `Null` actuals are always accepted — row-requirement
    /// checking is a best-effort pre-expansion guard; a truly unknown
    /// column type shouldn't generate a call-site error we can't act
    /// upon.
    pub fn is_satisfied_by(&self, actual: &DataType) -> bool {
        if matches!(actual, DataType::Unknown | DataType::Null) {
            return true;
        }
        match self {
            DataTypeReq::Concrete(expected) => {
                types_compatible_for_row_requirement(expected, actual)
            }
            DataTypeReq::Constraint(c) => c.satisfies(actual),
        }
    }

    /// Human-readable rendering for diagnostic messages. Mirrors the
    /// user-facing syntax: `Integer`, `Numeric`, `Any`, …
    pub fn render(&self) -> String {
        match self {
            DataTypeReq::Concrete(dt) => dt.to_string(),
            DataTypeReq::Constraint(TypeConstraint::Concrete(dt)) => dt.to_string(),
            DataTypeReq::Constraint(TypeConstraint::Numeric) => "Numeric".to_string(),
            DataTypeReq::Constraint(TypeConstraint::Ordered) => "Ordered".to_string(),
            DataTypeReq::Constraint(TypeConstraint::Any) => "Any".to_string(),
        }
    }
}

/// Structural check used by [`DataTypeReq::is_satisfied_by`] for the
/// `Concrete` branch. Callers should not use this directly; it is
/// published so row-requirement unit tests can mimic the canonical
/// compatibility rule.
///
/// Accepts:
///   - Exact equality of `DataType` (after normalization of the
///     `Text ↔ Varchar` family).
pub fn types_compatible_for_row_requirement(expected: &DataType, actual: &DataType) -> bool {
    if expected == actual {
        return true;
    }
    expected.normalize() == actual.normalize()
}

/// Trailing row-polymorphism marker on a [`SchemaRequirement`].
///
/// Introduced by the grammar `TableExpr<{col: Type, ..r}>` or
/// `TableExpr<{col: Type, ..}>`:
///   - [`RowTail::None`] — no trailing marker; the caller's schema
///     must exactly match `required` (no extras allowed).
///   - [`RowTail::Anon`] — `..`; extras are allowed but not observable.
///   - [`RowTail::Named(name)`] — `..<name>`; extras are bound to the
///     named row variable on the per-call `row_var_env`. In Phase 16
///     the binding exists but is not yet user-referenceable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTail {
    /// No tail marker — the schema must match exactly.
    None,
    /// `..` — extras accepted, not bound.
    Anon,
    /// `..<name>` — extras accepted and bound as a row variable.
    Named(String),
}

/// Structured row requirement on a `TableExpr<{…}>` parameter
/// (Phase 16).
///
/// `required` is the ordered list of `(column_name, requirement)`
/// pairs declared in the signature. `tail` is the trailing marker
/// decision: no tail, anonymous tail, or a named row variable.
///
/// The check at the call site is performed by
/// [`check_schema_requirement`] — a pure function that takes the
/// requirement, the caller's schema, and returns either a
/// [`RowVarBinding`] (the extras captured by a named tail, empty for
/// `None` / `Anon`) or a [`SchemaMismatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRequirement {
    /// The declared `(column_name, requirement)` pairs in source order.
    pub required: Vec<(String, DataTypeReq)>,
    /// Trailing row-variable behaviour.
    pub tail: RowTail,
}

/// Per-call binding produced by a successful [`check_schema_requirement`]
/// against a [`RowTail::Named`] tail (Phase 16).
///
/// `name` is the row variable's source name (e.g. `"r"`). `extras` is
/// the ordered list of caller columns not matched by any `required`
/// entry. For [`RowTail::None`] and [`RowTail::Anon`] the binding is
/// `None`; for [`RowTail::Named`] with no extras `extras` is the
/// empty vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowVarBinding {
    /// Source name of the row variable from the signature.
    pub name: String,
    /// Caller columns not covered by `required`, in caller-supplied
    /// order. Each element is `(column_name, column_data_type)`.
    pub extras: Vec<(String, DataType)>,
}

/// Structured failure describing why a [`check_schema_requirement`]
/// check failed (Phase 16).
///
/// The call-site checker converts this into a user-facing
/// [`DiagnosticCode::RowRequirementUnsatisfied`] diagnostic.
///
/// In Phase 16 we accept extras by default — i.e. even when the
/// requirement declared no explicit tail we behave as open-record
/// row polymorphism dictates (research §8). Future phases may opt in
/// to strict-schema enforcement; for now the row-requirement check
/// reports only `MissingColumn` and `TypeMismatch` failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaMismatch {
    /// The caller's schema does not contain a column declared in the
    /// requirement. `column` is the declared name; `required` is the
    /// declared constraint rendered for diagnostics.
    MissingColumn {
        column: String,
        required: DataTypeReq,
    },
    /// The caller's column has a type that does not satisfy the
    /// declared requirement. `actual` is the caller's reported
    /// [`DataType`] rendering.
    TypeMismatch {
        column: String,
        required: DataTypeReq,
        actual: String,
    },
}

/// Pure check: does the caller's schema satisfy the
/// [`SchemaRequirement`] declared on a `TableExpr<{…}>` parameter?
///
/// On success, returns the optional [`RowVarBinding`] that the tail's
/// named row variable — if any — should map to on the per-call
/// `row_var_env`. For [`RowTail::None`] / [`RowTail::Anon`] the
/// binding is `None`.
///
/// On failure, returns the first structural problem detected. We
/// report the first problem only (not a batch) because the
/// diagnostics layer generally benefits from one concrete, actionable
/// message per check; later phases can extend this to batched
/// reporting if needed.
///
/// Pure — no Salsa, no I/O.
pub fn check_schema_requirement(
    req: &SchemaRequirement,
    arg_schema: &[(String, DataType)],
) -> Result<Option<RowVarBinding>, SchemaMismatch> {
    // 1. Every required column must be present and type-compatible.
    //    We report the first structural problem in declaration order.
    for (col_name, col_req) in &req.required {
        let Some((_, actual_dt)) = arg_schema.iter().find(|(n, _)| n == col_name) else {
            return Err(SchemaMismatch::MissingColumn {
                column: col_name.clone(),
                required: col_req.clone(),
            });
        };
        if !col_req.is_satisfied_by(actual_dt) {
            return Err(SchemaMismatch::TypeMismatch {
                column: col_name.clone(),
                required: col_req.clone(),
                actual: actual_dt.to_string(),
            });
        }
    }

    // 2. Extras handling. Compute the set of required column names
    //    once, then collect every caller column not in that set —
    //    this is the row-variable's extras list (or the
    //    "unexpected" list when `RowTail::None`).
    let required_names: std::collections::HashSet<&str> =
        req.required.iter().map(|(n, _)| n.as_str()).collect();
    let extras: Vec<(String, DataType)> = arg_schema
        .iter()
        .filter(|(n, _)| !required_names.contains(n.as_str()))
        .cloned()
        .collect();

    match &req.tail {
        // Phase 16: extras are accepted regardless of tail — the
        // open-record semantics in research §8 treat missing /
        // wrong-type columns as the only structural failures.
        // `RowTail::None` and `RowTail::Anon` simply discard extras;
        // `RowTail::Named` captures them into the per-call
        // `row_var_env` for future reference by the body / return
        // type (Phases 17+ / 37).
        RowTail::None | RowTail::Anon => Ok(None),
        RowTail::Named(name) => Ok(Some(RowVarBinding {
            name: name.clone(),
            extras,
        })),
    }
}

/// Error produced when a type-reference annotation can't be resolved into a
/// structured [`SmeltType`].
///
/// Every variant carries the raw `span_text` (the original user-written
/// annotation) so diagnostic messages can quote what the user actually wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmeltTypeParseError {
    /// A recognised sort keyword other than `Expr` — e.g. `TableExpr<T>`,
    /// `AggExpr<T>`, `WindowExpr<T>`. Deferred to Step 3 of the plan.
    UnsupportedSort { sort: String, span_text: String },
    /// `Expr<Expr<Integer>>` — the inner parameter must itself be a concrete
    /// type or a constraint, never another sort.
    NestedExpr { span_text: String },
    /// The inner payload wasn't recognised as either a known concrete type or
    /// a known constraint (e.g. `Expr<FooBar>`).
    UnknownInner { inner: String, span_text: String },
    /// The annotation couldn't be lexically split into `Sort<Inner>` — missing
    /// angle brackets, empty, unbalanced, etc.
    Malformed { span_text: String },
}

impl std::fmt::Display for SmeltTypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmeltTypeParseError::UnsupportedSort { sort, span_text } => write!(
                f,
                "unsupported type sort `{sort}` in `{span_text}` (only `Expr<T>` is supported in Step 1)"
            ),
            SmeltTypeParseError::NestedExpr { span_text } => write!(
                f,
                "nested `Expr<...>` is not allowed in `{span_text}`; the inner must be a concrete type or constraint"
            ),
            SmeltTypeParseError::UnknownInner { inner, span_text } => write!(
                f,
                "unknown type `{inner}` in `{span_text}`"
            ),
            SmeltTypeParseError::Malformed { span_text } => {
                write!(f, "malformed type reference: `{span_text}`")
            }
        }
    }
}

impl std::error::Error for SmeltTypeParseError {}

/// Parse a type-reference annotation such as `Expr<Integer>` or
/// `Expr<Numeric>` into a structured [`SmeltType`].
///
/// This is deliberately a string-level parser — the Rowan CST's `TypeRef` is
/// a flat token run (see `parse_type_ref` in `smelt-parser`) and the grammar
/// we accept here is intentionally tiny:
///
/// ```text
/// TypeRef  := Sort '<' Inner '>'
/// Sort     := 'Expr'
/// Inner    := <concrete-DataType-name> | 'Numeric' | 'Any'
/// ```
///
/// Anything else is an error. In particular, the recognised non-`Expr` sorts
/// (`TableExpr`, `AggExpr`, `WindowExpr`, `SelectItems`, `OrderSpec`) deliver
/// [`SmeltTypeParseError::UnsupportedSort`] with a clear, actionable message
/// pointing the user at a future phase.
pub fn parse_smelt_type(text: &str) -> Result<SmeltType, SmeltTypeParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // Bare `TableExpr` (no `<...>`) is a valid row-polymorphic table
    // parameter (Phase 15). `TableExpr<{...}>` row requirements are
    // parsed by the signature extractor directly off the CST
    // (`extract_param_spec`) — the string-level parser here has no
    // structured grammar for row requirements, so it returns
    // `UnsupportedSort`, and the extractor overrides that with a
    // structured `SmeltType::TableExpr(Some(req))` when the CST
    // carries a `ROW_REQUIREMENT` (Phase 16).
    if trimmed == "TableExpr" {
        return Ok(SmeltType::TableExpr(None));
    }

    // Split on the first '<'. We intentionally require brackets — bare
    // `Integer` or similar is not a valid Step-1 type reference.
    let Some(lt_idx) = trimmed.find('<') else {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    };
    let Some(gt_idx) = trimmed.rfind('>') else {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    };
    if gt_idx <= lt_idx + 1 {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // Anything after the closing '>' must be whitespace only.
    let tail = trimmed[gt_idx + 1..].trim();
    if !tail.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    let sort = trimmed[..lt_idx].trim();
    let inner_raw = trimmed[lt_idx + 1..gt_idx].trim();
    if inner_raw.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // `<` in the inner means another generic — reject as nested Expr if the
    // inner sort is `Expr`, otherwise unsupported sort.
    if inner_raw.contains('<') {
        let inner_lt = inner_raw.find('<').unwrap();
        let inner_sort = inner_raw[..inner_lt].trim();
        // Also check for comma-prefixed sorts like ", Expr<...>" (second Lambda param).
        let actual_sort = inner_sort.trim_start_matches(',').trim();
        if sort == "Expr" && (actual_sort == "Expr") {
            return Err(SmeltTypeParseError::NestedExpr {
                span_text: text.to_string(),
            });
        }
        // `List<T>` and `Lambda<T, U>` allow nested generics — fall through to
        // sort dispatch so `List<Expr<Integer>>`, `Lambda<Expr<T>, Expr<U>>`, etc.
        // parse correctly.
        if sort == "List" || sort == "Lambda" || sort == "SelectItems" {
            // Fall through to the sort dispatch match below.
        } else if sort != "Expr" {
            // Other nested sort — surface the outer sort decision first.
            return Err(SmeltTypeParseError::UnsupportedSort {
                sort: sort.to_string(),
                span_text: text.to_string(),
            });
        } else {
            return Err(SmeltTypeParseError::UnknownInner {
                inner: inner_raw.to_string(),
                span_text: text.to_string(),
            });
        }
    }

    // Sort dispatch.
    match sort {
        "Expr" => {
            // Phase 19: strip optional `, ctx` context binding — the context
            // identifier is extracted from the CST in `extract_param_spec`.
            let type_part = inner_raw
                .find(',')
                .map(|i| inner_raw[..i].trim())
                .unwrap_or(inner_raw);
            let constraint = parse_inner_constraint(type_part).ok_or_else(|| {
                SmeltTypeParseError::UnknownInner {
                    inner: inner_raw.to_string(),
                    span_text: text.to_string(),
                }
            })?;
            Ok(SmeltType::Expr(constraint))
        }
        "List" => {
            // `List<T>` — recursive parse of the inner type.
            // `inner_raw` already has the leading `<` / trailing `>` stripped
            // (they were split off by the lt_idx / gt_idx logic above).
            let inner_ty =
                parse_smelt_type(inner_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                    inner: inner_raw.to_string(),
                    span_text: text.to_string(),
                })?;
            Ok(SmeltType::List(Box::new(inner_ty)))
        }
        "SelectItems" => {
            // `SelectItems<Kind>` or `SelectItems<Kind, ctx_name>` (Phase 21).
            let kind_part = inner_raw
                .find(',')
                .map(|i| inner_raw[..i].trim())
                .unwrap_or(inner_raw);
            let ctx_part = inner_raw
                .find(',')
                .map(|i| inner_raw[i + 1..].trim().to_string());
            let kind = match kind_part {
                "Agg" => ExprKind::Agg,
                "Scalar" => ExprKind::Scalar,
                "Window" => ExprKind::Window,
                _ => {
                    return Err(SmeltTypeParseError::UnknownInner {
                        inner: inner_raw.to_string(),
                        span_text: text.to_string(),
                    });
                }
            };
            let context = ctx_part.filter(|s| !s.is_empty()).map(ContextRef);
            Ok(SmeltType::SelectItems { kind, context })
        }
        "Lambda" => {
            // `Lambda<T, U>` — parse T and U as two comma-separated SmeltTypes.
            // `inner_raw` has outer `<>` stripped, e.g. "Expr<INTEGER>, Expr<TEXT>".
            // We need to split on the comma that separates the two type args,
            // respecting nested angle brackets.
            let split_pos =
                find_lambda_comma(inner_raw).ok_or_else(|| SmeltTypeParseError::Malformed {
                    span_text: text.to_string(),
                })?;
            let t_raw = inner_raw[..split_pos].trim();
            let u_raw = inner_raw[split_pos + 1..].trim();
            let t = parse_smelt_type(t_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                inner: t_raw.to_string(),
                span_text: text.to_string(),
            })?;
            let u = parse_smelt_type(u_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                inner: u_raw.to_string(),
                span_text: text.to_string(),
            })?;
            Ok(SmeltType::Lambda(Box::new(t), Box::new(u)))
        }
        "TableExpr" | "AggExpr" | "WindowExpr" | "OrderSpec" => {
            Err(SmeltTypeParseError::UnsupportedSort {
                sort: sort.to_string(),
                span_text: text.to_string(),
            })
        }
        // Unknown sort keyword — treat as malformed so we're not lenient about
        // typos. Callers show the span text so users can see what they wrote.
        other if !other.is_empty() => Err(SmeltTypeParseError::UnsupportedSort {
            sort: other.to_string(),
            span_text: text.to_string(),
        }),
        _ => Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        }),
    }
}

/// Find the position of the top-level comma separating the two type parameters
/// in a `Lambda<T, U>` inner string (after the outer `<>` are stripped).
///
/// Respects nested angle brackets so `Lambda<Expr<Integer>, Expr<Text>>` correctly
/// splits at the comma between the two top-level parameters, not at a nested one.
fn find_lambda_comma(inner: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Recognise the inner payload of an `Expr<...>`.
///
/// The order matters: constraint keywords (`Numeric`, `Any`) take priority over
/// concrete type names so a future `Ordered` constraint can live alongside
/// whatever `parse_type` might try to map to a concrete type.
fn parse_inner_constraint(inner: &str) -> Option<TypeConstraint> {
    match inner {
        "Numeric" => Some(TypeConstraint::Numeric),
        "Ordered" => Some(TypeConstraint::Ordered),
        "Any" => Some(TypeConstraint::Any),
        other => parse_type(other).ok().map(TypeConstraint::Concrete),
    }
}

/// Pre-resolution context binding attached to an `Expr<T, ctx>` /
/// `AggExpr<T, ctx>` / `WindowExpr<T, ctx>` parameter (Phase 19).
///
/// Stores the raw identifier written by the user (e.g. `"source"`). Phase 20
/// will add a post-resolution pointer to the actual parameter or CTE that
/// `ctx` names. For now this is just a newtype around `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRef(pub String);

impl ContextRef {
    /// The raw name written in the `Expr<T, ctx>` annotation.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// Description of a single parameter in a `smelt.define`.
///
/// `type_ref_text` is the raw source text of the `TypeRef` node (e.g.
/// `"Expr<Numeric>"`) or `None` when the parameter is unannotated. Phase 4
/// adds a parsed `type_ref` alongside it; callers that need the structured
/// form should prefer that field. `type_ref_text` stays in place (Option A
/// from the plan) so downstream integration tests that read the raw text
/// don't need to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The parameter's declared name.
    pub name: String,
    /// Source range of the parameter-name identifier, suitable for anchoring
    /// diagnostics (notably `DuplicateParameterName` in Phase 5). `None` only
    /// if the declaration was so malformed that no IDENT token was present in
    /// the PARAM node.
    pub name_range: Option<Range>,
    /// Raw text of the declared type, or `None` if unannotated.
    pub type_ref_text: Option<String>,
    /// Structured parse of `type_ref_text`, or `None` if unannotated.
    /// `Some(Err(...))` when an annotation was written but couldn't be parsed
    /// — these errors are surfaced as diagnostics by higher layers.
    pub type_ref: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the `TypeRef` node for this parameter, suitable for
    /// anchoring diagnostic spans. `None` when the parameter has no
    /// annotation at all.
    pub type_ref_range: Option<Range>,
    /// `true` when the parameter has a default value.
    pub has_default: bool,
    /// Pre-resolution context binding from `Expr<T, ctx>` / `AggExpr<T, ctx>`
    /// / `WindowExpr<T, ctx>` annotations (Phase 19). `None` when no context
    /// argument is present.
    pub context: Option<ContextRef>,
}

/// A single frame of expansion context attached to a body/call-site
/// diagnostic.
///
/// Phase 6 populated a 0-or-1-element `Vec<FrameInfo>` on every
/// `smelt.fn.*`-originated diagnostic. Phase 12 extends the checker to
/// stack frames at each level of recursive body expansion, producing
/// multi-level diagnostics:
///
/// - When the body's own type-check surfaces an error (no expansion
///   happened), the vec is empty.
/// - When the error fires *inside* an expanded body, the vec contains
///   one frame per nested expansion, **innermost-first →
///   outermost-last**. `frames.last()` is the outermost call site
///   (the one the user wrote in their source); `frames.first()` is the
///   deepest nested call. The renderer in `smelt-lsp` reverses this
///   iteration to present frames outer-to-inner in the message body.
///
/// `bound_type` is the concrete type that the parameter was bound to at the
/// call site, rendered via `DataType::to_string()` for display.
///
/// Phase 12 also adds decl-site and call-site location data so LSP
/// clients can surface each frame as a `DiagnosticRelatedInformation`
/// pointing at the declaring file / call-path. All three location fields
/// are `Option` — older frames (or sig-lookup misses during degraded
/// scenarios) simply carry `None` and the LSP renderer falls back to
/// inline-only messaging for those frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    /// Name of the function whose expansion is responsible for this frame
    /// (e.g. `"safe_divide"`).
    pub function: String,
    /// Name of the parameter inside that function whose binding produced
    /// the inner error (e.g. `"numerator"`).
    pub param: String,
    /// Textual rendering of the type that `param` was bound to at this
    /// call site (e.g. `"INTEGER"`, `"VARCHAR"`).
    pub bound_type: String,
    /// Path to the file that declares the function (`decl_path`). `None`
    /// when the declaration couldn't be located (e.g. sig-lookup miss
    /// during degraded scenarios).
    pub decl_path: Option<PathBuf>,
    /// Range of the `DEFINE_NAME` / `EXTERN_NAME` identifier in
    /// `decl_path`. Used by LSP clients to open the declaration site
    /// when a user clicks the frame's related-information link.
    pub decl_range: Option<Range>,
    /// Range of the call-path span at this frame's call site (in the
    /// file that *contains* the call — distinct from `decl_path`, which
    /// points at where the callee is defined).
    pub call_site_range: Option<Range>,
    /// Identifier of the declaring function in the function registry.
    /// `None` for anonymous frames (e.g. HOF inline-expansion frames produced
    /// by `map`, `filter`, `reduce`). Named `smelt.define` frames carry `Some`.
    pub fn_id: Option<String>,
    /// Zero-based index into the source list literal at the HOF call site,
    /// identifying which element the expanded lambda body was operating on.
    /// `None` when the source list was not a literal or the information is
    /// not statically available (the common v1 case).
    pub element_index: Option<usize>,
}

/// Tier of a function, derived from annotation completeness.
///
/// - `Tier::Three`: every parameter annotated AND return type annotated.
/// - `Tier::Two`: every parameter annotated, return type missing.
/// - `Tier::One`: at least one parameter unannotated (return-type state
///   irrelevant for this tier — unannotated inputs force Tier 1).
///
/// Behavioural differences between tiers (bidirectional checking, param
/// coercion, etc.) land in Steps 5+.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    One,
    Two,
    Three,
}

/// Origin of a user-declared function signature.
///
/// Phase 10 introduces `smelt.extern` declarations alongside the existing
/// `smelt.define`. Externs have no body (they bind a name to a
/// backend-provided function), but otherwise share the same indexing,
/// resolution, and type-check surface as user `smelt.define` declarations.
/// The two share `FunctionSig` with this discriminator so downstream lookups
/// stay uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigOrigin {
    /// A `smelt.define` declaration with a body.
    Define,
    /// A `smelt.extern` declaration — signature only.
    Extern,
}

/// The set of backends a function is declared (or inferred) to target.
///
/// `All` is the universal set — it accepts any backend. `Only(names)` is
/// a fixed, alphabetically-sortable list of backend names (e.g.
/// `["duckdb"]`, `["duckdb", "spark"]`). Backend names are case-sensitive
/// to keep string comparison simple; the canonical names are lowercase
/// (`duckdb`, `spark`, `databricks`).
///
/// Introduced in Phase 11 of the smelt-functions plan (§16 #23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSet {
    /// Unconstrained — valid on every backend.
    All,
    /// Restricted to the listed backends. The list is sorted & dedup'd
    /// when constructed via [`BackendSet::from_names`].
    Only(Vec<String>),
}

impl BackendSet {
    /// Canonicalise a raw list of backend names into a [`BackendSet::Only`].
    /// Normalises (trim + lowercase), deduplicates, and sorts so that
    /// two equivalent lists compare equal.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut v: Vec<String> = names
            .into_iter()
            .map(|s| s.as_ref().trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        v.sort();
        v.dedup();
        BackendSet::Only(v)
    }

    /// Is `self` a (non-strict) subset of `other`? `All` is only a
    /// subset of `All`; `Only(a)` is a subset of `All`; `Only(a)` is a
    /// subset of `Only(b)` iff every element of `a` is in `b`.
    pub fn is_subset_of(&self, other: &BackendSet) -> bool {
        match (self, other) {
            (BackendSet::All, BackendSet::All) => true,
            (BackendSet::All, BackendSet::Only(_)) => false,
            (BackendSet::Only(_), BackendSet::All) => true,
            (BackendSet::Only(a), BackendSet::Only(b)) => a.iter().all(|x| b.contains(x)),
        }
    }

    /// Intersection: backends present in both sets.
    pub fn intersect(&self, other: &BackendSet) -> BackendSet {
        match (self, other) {
            (BackendSet::All, other) => other.clone(),
            (self_, BackendSet::All) => self_.clone(),
            (BackendSet::Only(a), BackendSet::Only(b)) => {
                let mut v: Vec<String> = a.iter().filter(|x| b.contains(*x)).cloned().collect();
                v.sort();
                v.dedup();
                BackendSet::Only(v)
            }
        }
    }

    /// Human-readable rendering for diagnostics, e.g. `all`,
    /// `[duckdb]`, `[duckdb, spark]`.
    pub fn render(&self) -> String {
        match self {
            BackendSet::All => "all".to_string(),
            BackendSet::Only(v) => format!("[{}]", v.join(", ")),
        }
    }
}

/// Error produced by [`parse_frontmatter_backends`] when the YAML-ish
/// frontmatter text contains a malformed `backends:` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterParseError {
    pub message: String,
}

impl std::fmt::Display for FrontmatterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FrontmatterParseError {}

/// Parse a `backends:` declaration out of a frontmatter block body.
///
/// Accepted shapes (Phase 11, intentionally minimal — extensible):
///   * `backends: [duckdb, spark]`      — inline array of backend names
///   * `backends: [duckdb]`             — single-element inline array
///   * no `backends:` key               — returns `Ok(None)` (unconstrained)
///   * `backends: { duckdb: { emit: foo } }` — mapping form; only the
///     backend names are read (emit name is stored alongside). Parsed
///     minimally — a malformed nested shape returns an error.
///
/// The parser is hand-rolled: we don't want a full YAML dependency for
/// what's effectively a single key.
pub fn parse_frontmatter_backends(
    yaml_text: &str,
) -> Result<Option<BackendSet>, FrontmatterParseError> {
    for line in yaml_text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("backends:") {
            continue;
        }
        let value = trimmed["backends:".len()..].trim();
        if value.is_empty() {
            // `backends:` on its own line — not valid in this minimal parser.
            return Err(FrontmatterParseError {
                message: "backends: expects a value on the same line (e.g. `backends: [duckdb]`)"
                    .to_string(),
            });
        }
        // Inline-array form: `[a, b, c]`.
        if value.starts_with('[') && value.ends_with(']') {
            let inner = &value[1..value.len() - 1];
            let names: Vec<String> = inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Ok(Some(BackendSet::from_names(names)));
        }
        // Mapping form: `{ duckdb: { emit: foo } }`. We only extract the
        // top-level backend keys — enough for Phase 11 narrow-check.
        if value.starts_with('{') && value.ends_with('}') {
            let inner = &value[1..value.len() - 1];
            // Split on top-level commas. Nesting is shallow so this simple
            // depth-tracker suffices.
            let mut segments: Vec<String> = Vec::new();
            let mut depth = 0i32;
            let mut buf = String::new();
            for ch in inner.chars() {
                match ch {
                    '{' | '[' => {
                        depth += 1;
                        buf.push(ch);
                    }
                    '}' | ']' => {
                        depth -= 1;
                        buf.push(ch);
                    }
                    ',' if depth == 0 => {
                        segments.push(std::mem::take(&mut buf));
                    }
                    _ => buf.push(ch),
                }
            }
            if !buf.trim().is_empty() {
                segments.push(buf);
            }
            let mut names = Vec::new();
            for seg in segments {
                let seg = seg.trim();
                if seg.is_empty() {
                    continue;
                }
                // Take the backend name: everything up to the first `:`.
                let name = seg.split(':').next().unwrap_or("").trim().to_string();
                if name.is_empty() {
                    return Err(FrontmatterParseError {
                        message: "backends: map entry missing key".to_string(),
                    });
                }
                names.push(name);
            }
            return Ok(Some(BackendSet::from_names(names)));
        }
        return Err(FrontmatterParseError {
            message: format!("backends: expects `[...]` or `{{...}}`, got {:?}", value),
        });
    }
    Ok(None)
}

/// Signature of a single `smelt.define` or `smelt.extern` declaration.
///
/// The `name_range` field points at the `DEFINE_NAME` token in the source
/// file; diagnostic-emitting code (duplicate-definition detection, etc.)
/// uses it as the anchor for error spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSig {
    /// The function's declared name.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<ParamSpec>,
    /// Raw text of the declared return type, or `None` if no `-> Type` clause.
    pub return_type_text: Option<String>,
    /// Structured parse of `return_type_text`, or `None` if unannotated.
    /// `Some(Err(...))` indicates a malformed return annotation.
    pub return_type: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the return `TypeRef`, suitable for anchoring
    /// diagnostics. `None` when no return type is declared.
    pub return_type_range: Option<Range>,
    /// Tier, derived from annotation completeness at parse time.
    pub tier: Tier,
    /// Line/column range of the function-name identifier, for diagnostics.
    pub name_range: Range,
    /// Whether this signature comes from a `smelt.define` (has a body) or
    /// a `smelt.extern` (signature-only).
    pub origin: SigOrigin,
    /// Backend set declared in frontmatter (`backends: [...]`) OR implied
    /// by a backend-namespace sugar extern (e.g. `smelt.extern duckdb.foo`).
    /// `None` when no declaration is present — the checker then infers
    /// the set from the body (or defaults to [`BackendSet::All`] for
    /// body-less externs).
    ///
    /// Introduced in Phase 11.
    pub declared_backends: Option<BackendSet>,
    /// For externs only: the optional backend-specific emit name. When
    /// the extern is declared with the `duckdb.<name>` sugar, this is
    /// populated with `<name>` so the later SQL emitter knows which
    /// backend symbol to call. For `smelt.define`, always `None`.
    ///
    /// Introduced in Phase 11.
    pub emit_name: Option<String>,
    /// Malformed-frontmatter diagnostic, if any, anchored at the
    /// declaration's name range. `Some(msg)` indicates the frontmatter
    /// block attached to this decl could not be parsed. Checkers should
    /// surface this as a `DiagnosticCode::BackendsWideningNotAllowed`-
    /// adjacent error (for now we reuse the same code since only that
    /// check is wired in Phase 11).
    pub frontmatter_parse_error: Option<String>,
}

fn type_ref_range(type_ref: &TypeRef, text: &str) -> Range {
    let tr = type_ref.syntax().text_range();
    Range {
        start: offset_to_position(text, usize::from(tr.start())),
        end: offset_to_position(text, usize::from(tr.end())),
    }
}

/// Extract a `ParamSpec` from an AST `Param`.
///
/// Pure: does not consult Salsa. Takes only the AST node and the source text
/// (for the range conversion).
///
/// Phase 16: when the parameter's `TYPE_REF` is a `TableExpr` head
/// with a `ROW_REQUIREMENT` child, we bypass the string-level
/// `parse_smelt_type` (which has no structured row-requirement
/// grammar) and build a [`SmeltType::TableExpr(Some(req))`] directly
/// from the CST via [`row_requirement_from_type_ref`].
fn extract_param_spec(param: &AstParam, text: &str) -> ParamSpec {
    let type_ref_node = param.type_ref();
    let type_ref_text = type_ref_node.as_ref().map(|t| t.text());
    let mut type_ref: Option<Result<SmeltType, SmeltTypeParseError>> =
        type_ref_text.as_deref().map(parse_smelt_type);

    // Phase 16 override: replace the string-parser's best guess with
    // a CST-derived structured row-requirement when applicable.
    if let Some(tr) = &type_ref_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            type_ref = Some(Ok(structured));
        }
        // Phase 35 override: replace the string-parser error for
        // `Expr<Struct<{…}>>` with a CST-derived SmeltType::Struct.
        if let Some(structured) = struct_expr_type_from_cst(tr) {
            type_ref = Some(Ok(structured));
        }
    }

    let type_ref_range = type_ref_node.as_ref().map(|t| type_ref_range(t, text));
    let name_range = param.name_range().map(|r| Range {
        start: offset_to_position(text, usize::from(r.start())),
        end: offset_to_position(text, usize::from(r.end())),
    });
    // Phase 19: extract the optional context binding from `EXPR_CTX`.
    let context = type_ref_node
        .as_ref()
        .and_then(|tr| tr.expr_ctx())
        .map(ContextRef);
    ParamSpec {
        name: param.name().unwrap_or_default(),
        name_range,
        type_ref_text,
        type_ref,
        type_ref_range,
        has_default: param.default_value().is_some(),
        context,
    }
}

/// Phase 16 CST-aware extractor: if `type_ref` is a `TableExpr` head,
/// build a [`SmeltType::TableExpr`] that carries any structured
/// [`SchemaRequirement`] declared between its angle brackets.
///
/// Returns `None` when the head isn't `TableExpr` (so the string-level
/// `parse_smelt_type` wins), and when the head is `TableExpr` but
/// contains no `ROW_REQUIREMENT` (bare case — the string parser
/// already produces the same `SmeltType::TableExpr(None)`).
///
/// Pure — walks only the CST node.
fn tableexpr_type_from_cst(tr: &TypeRef) -> Option<SmeltType> {
    use smelt_parser::ast::{RowTail as AstRowTail, TypeRefHead};
    if tr.kind() != TypeRefHead::TableExpr {
        return None;
    }
    let Some(req_node) = tr.row_requirement() else {
        // Bare `TableExpr` — let the string-parser's
        // `SmeltType::TableExpr(None)` stand.
        return None;
    };

    // Build the ordered `(name, DataTypeReq)` list from the CST's
    // ROW_FIELD children.
    let mut required: Vec<(String, DataTypeReq)> = Vec::new();
    for field in req_node.fields() {
        let Some(name) = field.name() else { continue };
        // Inner type is a flat type reference (e.g. `Numeric`,
        // `Integer`, `Text`) — reuse `parse_inner_constraint` so
        // constraint keywords and concrete types go through the same
        // classifier as `Expr<T>`'s inner.
        let inner_text = field.type_ref().map(|t| t.text()).unwrap_or_default();
        let req = match parse_inner_constraint(inner_text.trim()) {
            Some(TypeConstraint::Concrete(dt)) => DataTypeReq::Concrete(dt),
            Some(c) => DataTypeReq::Constraint(c),
            None => {
                // Unknown inner type — fall back to `Any` so
                // downstream code doesn't panic, and the failure
                // surfaces as a match against `Any` (always accepts).
                // A later phase can lift this into a dedicated
                // `RowRequirementUnknownInner` diagnostic.
                DataTypeReq::Constraint(TypeConstraint::Any)
            }
        };
        required.push((name, req));
    }

    let tail = match req_node.tail() {
        AstRowTail::None => RowTail::None,
        AstRowTail::Anon => RowTail::Anon,
        AstRowTail::Named(n) => RowTail::Named(n),
    };

    Some(SmeltType::TableExpr(Some(SchemaRequirement {
        required,
        tail,
    })))
}

/// Phase 35 CST-aware extractor: if `type_ref` is an `Expr<Struct<{…}>>` head,
/// build a [`SmeltType::Struct`] that carries the declared field list and tail.
///
/// Returns `None` when the head isn't `Expr` wrapping a `STRUCT_TYPE` (so the
/// string-level `parse_smelt_type` wins or already succeeded). When a
/// `STRUCT_TYPE` is found, this always overrides the string-parser's error
/// (which reports `UnknownInner` because the string parser can't handle the
/// nested `Struct<{…}>` form).
///
/// Pure — walks only the CST node.
fn struct_expr_type_from_cst(tr: &TypeRef) -> Option<SmeltType> {
    use smelt_parser::ast::{StructType, TypeRefHead};

    if tr.kind() != TypeRefHead::Expr {
        return None;
    }
    // Look for a STRUCT_TYPE descendant inside the TYPE_REF.
    let struct_node = tr.syntax().descendants().find_map(StructType::cast)?;

    let mut fields: Vec<(String, DataType)> = Vec::new();
    for sf in struct_node.fields() {
        let Some(name) = sf.name() else { continue };
        let inner_text = sf.type_ref().map(|t| t.text()).unwrap_or_default();
        // Parse the field's type as a concrete DataType (struct fields must be
        // concrete in v1 — constraints like `Numeric` are not yet supported
        // inside struct field positions). Fall back to Unknown for unrecognised
        // names so diagnostics don't cascade.
        let dt = crate::parse_type(inner_text.trim()).unwrap_or(DataType::Unknown);
        fields.push((name, dt));
    }

    let tail = match struct_node.row_tail() {
        None => StructRowTail::None,
        Some(t) => match t.var_name() {
            None => StructRowTail::Anon,
            Some(n) => StructRowTail::Named(n),
        },
    };

    Some(SmeltType::Struct { fields, tail })
}

fn compute_tier(params: &[ParamSpec], return_type_text: Option<&str>) -> Tier {
    let all_params_typed = params.iter().all(|p| p.type_ref_text.is_some());
    if !all_params_typed {
        return Tier::One;
    }
    if return_type_text.is_some() {
        Tier::Three
    } else {
        Tier::Two
    }
}

/// Convert a `SmeltDefine` AST node into a `FunctionSig`.
///
/// Returns `None` if the declaration is missing a name (error recovery
/// produced a fragment without an identifier). File text is required so
/// the `name_range` can be rendered as a line/column range — `Range` is
/// the same type returned by other `smelt-db` queries.
///
/// Phase 11: `raw_text` is the pre-strip source text. It's used to
/// attach per-declaration frontmatter blocks. For the common "stripped
/// text == raw text" case (callers without raw access), pass the
/// stripped text — `attach_frontmatter_to_decls` simply finds no blocks
/// and `declared_backends` stays `None`.
pub fn extract_signature(define: &SmeltDefine, text: &str) -> Option<FunctionSig> {
    extract_signature_with_raw(define, text, text)
}

/// `extract_signature` plus raw-text access for per-declaration
/// frontmatter attachment (Phase 11).
pub fn extract_signature_with_raw(
    define: &SmeltDefine,
    stripped_text: &str,
    raw_text: &str,
) -> Option<FunctionSig> {
    let name = define.name()?;
    let name_text_range = define.name_range()?;
    let name_range = Range {
        start: offset_to_position(stripped_text, usize::from(name_text_range.start())),
        end: offset_to_position(stripped_text, usize::from(name_text_range.end())),
    };

    let params: Vec<ParamSpec> = define
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| extract_param_spec(&p, stripped_text))
                .collect()
        })
        .unwrap_or_default();

    let return_type_node = define.return_type();
    let return_type_text = return_type_node.as_ref().map(|t| t.text());
    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        return_type_text.as_deref().map(parse_smelt_type);
    if let Some(tr) = &return_type_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        } else if let Some(structured) = struct_expr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        }
    }
    let return_type_range = return_type_node
        .as_ref()
        .map(|t| type_ref_range(t, stripped_text));
    let tier = compute_tier(&params, return_type_text.as_deref());

    let (declared_backends, frontmatter_parse_error) =
        parse_frontmatter_on_define(define, raw_text);

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        return_type,
        return_type_range,
        tier,
        name_range,
        origin: SigOrigin::Define,
        declared_backends,
        emit_name: None,
        frontmatter_parse_error,
    })
}

/// Read the per-decl frontmatter attached to `define` and parse a
/// `backends:` directive if present.
fn parse_frontmatter_on_define(
    define: &SmeltDefine,
    raw_text: &str,
) -> (Option<BackendSet>, Option<String>) {
    let Some(fm) = define.frontmatter(raw_text) else {
        return (None, None);
    };
    match parse_frontmatter_backends(&fm) {
        Ok(backends) => (backends, None),
        Err(e) => (None, Some(e.message)),
    }
}

/// Convert a `SmeltExtern` AST node into a `FunctionSig`.
///
/// Externs have no body but share the same signature surface as defines.
/// Returns `None` when the declaration is missing a name.
pub fn extract_extern_signature(ext: &SmeltExtern, text: &str) -> Option<FunctionSig> {
    extract_extern_signature_with_raw(ext, text, text)
}

/// `extract_extern_signature` plus raw-text access for per-decl
/// frontmatter attachment (Phase 11). Also populates the backend
/// namespace when the extern uses the `duckdb.<name>` sugar.
pub fn extract_extern_signature_with_raw(
    ext: &SmeltExtern,
    stripped_text: &str,
    raw_text: &str,
) -> Option<FunctionSig> {
    let name = ext.name()?;
    let name_text_range = ext.name_range()?;
    let name_range = Range {
        start: offset_to_position(stripped_text, usize::from(name_text_range.start())),
        end: offset_to_position(stripped_text, usize::from(name_text_range.end())),
    };

    let params: Vec<ParamSpec> = ext
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| extract_param_spec(&p, stripped_text))
                .collect()
        })
        .unwrap_or_default();

    let return_type_node = ext.return_type();
    let return_type_text = return_type_node.as_ref().map(|t| t.text());
    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        return_type_text.as_deref().map(parse_smelt_type);
    if let Some(tr) = &return_type_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        } else if let Some(structured) = struct_expr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        }
    }
    let return_type_range = return_type_node
        .as_ref()
        .map(|t| type_ref_range(t, stripped_text));
    let tier = compute_tier(&params, return_type_text.as_deref());

    // Phase 11: frontmatter-declared backends take precedence, but the
    // backend-namespace sugar (`smelt.extern duckdb.foo`) implies a
    // [duckdb] constraint when no frontmatter is present. If both are
    // present and agree, they stack; disagreement lets the frontmatter
    // win (it is more explicit).
    let (mut declared_backends, frontmatter_parse_error) = {
        match ext.frontmatter(raw_text) {
            Some(fm) => match parse_frontmatter_backends(&fm) {
                Ok(b) => (b, None),
                Err(e) => (None, Some(e.message)),
            },
            None => (None, None),
        }
    };
    let mut emit_name: Option<String> = None;
    if let Some(backend) = ext.backend_namespace() {
        let sugar_set = BackendSet::from_names([backend.clone()]);
        // The sugar also implies the smelt-level name is the emit name.
        emit_name = Some(name.clone());
        declared_backends = match declared_backends {
            None => Some(sugar_set),
            Some(explicit) => Some(explicit),
        };
        let _ = backend; // keep for future use.
    }

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        return_type,
        return_type_range,
        tier,
        name_range,
        origin: SigOrigin::Extern,
        declared_backends,
        emit_name,
        frontmatter_parse_error,
    })
}

/// Extract all function signatures from a parsed file.
///
/// Pure function: takes an AST + source text and returns a freshly
/// allocated vector of signatures in declaration order. Includes both
/// `smelt.define` and `smelt.extern` declarations — callers inspect
/// `FunctionSig::origin` to distinguish them.
///
/// Order: externs interleave with defines in the order they appear in the
/// syntax tree, so downstream per-declaration diagnostics fire in source
/// order regardless of origin.
pub fn extract_function_signatures(file: &AstFile, text: &str) -> Vec<FunctionSig> {
    extract_function_signatures_with_raw(file, text, text)
}

/// `extract_function_signatures` variant that takes both the stripped
/// source (used for the parsed AST ranges) and the raw pre-strip source
/// (used to look up per-declaration frontmatter). Phase 11 call sites
/// pass the two explicitly; legacy callers using `extract_function_signatures`
/// get frontmatter-less signatures (`declared_backends = None`).
pub fn extract_function_signatures_with_raw(
    file: &AstFile,
    stripped_text: &str,
    raw_text: &str,
) -> Vec<FunctionSig> {
    let mut out: Vec<FunctionSig> = Vec::new();
    for child in file.syntax().children() {
        if let Some(d) = SmeltDefine::cast(child.clone()) {
            if let Some(sig) = extract_signature_with_raw(&d, stripped_text, raw_text) {
                out.push(sig);
            }
        } else if let Some(e) = SmeltExtern::cast(child) {
            if let Some(sig) = extract_extern_signature_with_raw(&e, stripped_text, raw_text) {
                out.push(sig);
            }
        }
    }
    out
}

/// Extract the signature for a single named define, if present.
///
/// Returns `None` when no declaration with the given name exists. If
/// multiple declarations share the name (which is a diagnostic), the
/// first in declaration order wins.
pub fn extract_function_signature_by_name(
    file: &AstFile,
    text: &str,
    name: &str,
) -> Option<FunctionSig> {
    file.defines()
        .filter_map(|d| extract_signature(&d, text))
        .find(|sig| sig.name == name)
}

/// One type parameter in a signature generic list (§16 #14).
///
/// Generics live on built-in signatures only; `smelt.define` stays
/// monomorphic in v1. The `constraint` narrows what concrete types may
/// bind: `TypeConstraint::Numeric` triggers the promotion-chain branch of
/// unification, everything else requires exact equality across positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParam {
    /// The variable's name as written in the signature (e.g. `"T"`).
    pub name: String,
    /// Narrowing constraint; `TypeConstraint::Any` means "no constraint".
    pub constraint: TypeConstraint,
}

/// A single parameter slot in a signature (§16 #14 + #15).
///
/// `Concrete(c)` demands a fixed `TypeConstraint` (usually
/// `Concrete(DataType::…)`). `Var(name)` refers to one of the signature's
/// [`TypeParam`]s by name. `Variadic(inner)` is only legal in the trailing
/// slot — enforced by [`Signature::new`]; see
/// [`SignatureBuildError::NonTrailingVariadic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigParam {
    /// A concrete constraint the argument must satisfy (e.g.
    /// `Concrete(Concrete(Text))` for a scalar Text parameter).
    Concrete(TypeConstraint),
    /// A reference to a generic type parameter by name.
    Var(String),
    /// Trailing zero-or-more parameter of the inner shape. The inner
    /// `SigParam` is itself a `Concrete(...)` or `Var(...)` (never a
    /// nested `Variadic`).
    Variadic(Box<SigParam>),
}

/// A signature's return type — same vocabulary as [`SigParam`] without
/// [`SigParam::Variadic`] (SQL functions return a single column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    /// A fixed `TypeConstraint` for the return type.
    Concrete(TypeConstraint),
    /// The return type is bound to a generic type variable of this name.
    Var(String),
}

/// Error produced at registry-construction time when a [`Signature`] is
/// shaped in a way the inference routine can't handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureBuildError {
    /// A non-trailing [`SigParam::Variadic`] appeared in the parameter list.
    NonTrailingVariadic { name: String, position: usize },
    /// A variadic contained another variadic — not expressible in v1.
    NestedVariadic { name: String },
    /// A [`TypeExpr::Var`] or [`SigParam::Var`] referenced a name that
    /// isn't declared in `type_params`.
    UndeclaredTypeVar { name: String, var_name: String },
}

impl std::fmt::Display for SignatureBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureBuildError::NonTrailingVariadic { name, position } => write!(
                f,
                "signature `{name}` declares a variadic at position {position} but variadics must be the final parameter"
            ),
            SignatureBuildError::NestedVariadic { name } => write!(
                f,
                "signature `{name}` nests a variadic inside a variadic"
            ),
            SignatureBuildError::UndeclaredTypeVar { name, var_name } => write!(
                f,
                "signature `{name}` references undeclared type variable `{var_name}`"
            ),
        }
    }
}

impl std::error::Error for SignatureBuildError {}

/// Polymorphic signature of a SQL built-in in the canonical registry.
///
/// Phase 7 was monomorphic (`params: Vec<TypeConstraint>`, same for return).
/// Phase 8 extends to full generics + trailing variadic per §16 #14/#15:
///
/// * [`Signature::type_params`] — generic list (empty for monomorphic
///   entries).
/// * [`Signature::params`] — positional [`SigParam`]s; at most one trailing
///   [`SigParam::Variadic`].
/// * [`Signature::return_type`] — concrete [`TypeConstraint`] or a reference
///   to one of the type parameters.
///
/// Construct entries via [`Signature::new`] so the well-formedness checks
/// run once at registry initialisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Canonical (upper-cased) function name.
    pub name: String,
    /// Declared generic type parameters, in declaration order. Empty when
    /// the signature is monomorphic.
    pub type_params: Vec<TypeParam>,
    /// Positional parameters, in declaration order. At most one
    /// [`SigParam::Variadic`], which must be last.
    pub params: Vec<SigParam>,
    /// Return type.
    pub return_type: TypeExpr,
    /// Canonical return type per §16 #9's widening chain. `Some(dt)` when
    /// the registry declares a canonical type that differs from what a
    /// given backend natively returns (e.g. `SUM(INTEGER)` canonical is
    /// `BigInt` even though DuckDB returns `HUGEINT`). `None` means
    /// "derive from [`Signature::return_type`] at call time" — the common
    /// case for monomorphic signatures.
    ///
    /// Phase 12: recording-only. Step 7+ will consume this via a CAST
    /// emitter when `needs_cast_for(engine)` returns `true`.
    pub canonical_return: Option<DataType>,
    /// Per-backend native return-type overrides. Keyed on lowercase
    /// backend id (e.g. `"duckdb"`, `"spark"`). An entry here means
    /// "this backend natively returns a type that differs from
    /// [`Self::canonical_return`]" — Step 7+ emits a CAST at emit-time
    /// to preserve the canonical type.
    ///
    /// Phase 12: recording-only. `HashMap::default()` on entries that
    /// need no override (the canonical type is also the native type on
    /// every backend).
    pub engine_native: HashMap<String, DataType>,
    /// Default [`ExprKind`] for a call to this signature when no `OVER (…)`
    /// clause is present (Phase 14, §16 #24).
    ///
    /// Aggregates (`SUM`, `AVG`, `MIN`, `MAX`, `COUNT`) seed `Agg`. Window
    /// functions that are *only* meaningful with an `OVER (…)` clause
    /// (`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`) seed `Window`.
    /// Everything else seeds `Scalar`.
    ///
    /// The type checker overrides this at the call site: an aggregate call
    /// with an attached `OVER (…)` clause is treated as `Window` regardless
    /// of the seeded kind (the canonical SQL dual-mode behaviour).
    pub kind: ExprKind,
}

impl Signature {
    /// Build a validated signature. Panics with a readable message when
    /// the signature violates a structural invariant — registry seeds are
    /// static data, so any error here is a programmer bug caught at first
    /// call (via [`std::sync::LazyLock`]).
    pub fn new(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<SigParam>,
        return_type: TypeExpr,
    ) -> Self {
        Self::try_new(name, type_params, params, return_type).expect("malformed built-in signature")
    }

    /// Non-panicking variant for tests / future `smelt.extern` use.
    pub fn try_new(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<SigParam>,
        return_type: TypeExpr,
    ) -> Result<Self, SignatureBuildError> {
        // Variadic must be trailing only.
        for (idx, p) in params.iter().enumerate() {
            if let SigParam::Variadic(inner) = p {
                if idx != params.len() - 1 {
                    return Err(SignatureBuildError::NonTrailingVariadic {
                        name: name.to_string(),
                        position: idx + 1,
                    });
                }
                if matches!(**inner, SigParam::Variadic(_)) {
                    return Err(SignatureBuildError::NestedVariadic {
                        name: name.to_string(),
                    });
                }
            }
        }
        // Every type-var reference must be declared.
        let declared: std::collections::HashSet<&str> =
            type_params.iter().map(|tp| tp.name.as_str()).collect();
        for p in &params {
            check_param_vars(name, p, &declared)?;
        }
        if let TypeExpr::Var(var_name) = &return_type {
            if !declared.contains(var_name.as_str()) {
                return Err(SignatureBuildError::UndeclaredTypeVar {
                    name: name.to_string(),
                    var_name: var_name.clone(),
                });
            }
        }
        Ok(Self {
            name: name.to_string(),
            type_params,
            params,
            return_type,
            canonical_return: None,
            engine_native: HashMap::new(),
            kind: ExprKind::Scalar,
        })
    }

    /// Attach an [`ExprKind`] to this signature (Phase 14).
    ///
    /// Builder-style — used by the registry seed to mark aggregates and
    /// window functions. Defaults to [`ExprKind::Scalar`] if never called.
    pub fn with_kind(mut self, kind: ExprKind) -> Self {
        self.kind = kind;
        self
    }

    /// Attach a canonical return type to this signature (Phase 12).
    ///
    /// Builder-style — intended for static registry initialisation. The
    /// canonical type is compared to each `engine_native` entry to decide
    /// whether a CAST should be emitted on that backend.
    pub fn with_canonical_return(mut self, dt: DataType) -> Self {
        self.canonical_return = Some(dt);
        self
    }

    /// Declare a per-backend native return-type override (Phase 12).
    ///
    /// The `engine` key is lowercased on insert. Calling this multiple
    /// times with different engines builds up the full override table.
    pub fn with_engine_native(mut self, engine: &str, dt: DataType) -> Self {
        self.engine_native
            .insert(engine.trim().to_ascii_lowercase(), dt);
        self
    }

    /// Does the signature require a CAST back to the canonical return
    /// type when executed on `engine`? (§16 #9 / Phase 12, recording
    /// only — Step 7+ consumes this.)
    ///
    /// Returns `false` when no canonical type is declared (the common
    /// case — the signature's own [`Self::return_type`] is already
    /// canonical) or when the engine's native type equals the canonical
    /// type. Returns `true` when the engine is listed in
    /// [`Self::engine_native`] with a type that differs from
    /// [`Self::canonical_return`].
    pub fn needs_cast_for(&self, engine: &str) -> bool {
        let Some(canonical) = &self.canonical_return else {
            return false;
        };
        let key = engine.trim().to_ascii_lowercase();
        match self.engine_native.get(&key) {
            Some(native) => native != canonical,
            None => false,
        }
    }

    /// Look up a declared type parameter by name.
    pub fn type_param(&self, var_name: &str) -> Option<&TypeParam> {
        self.type_params.iter().find(|tp| tp.name == var_name)
    }
}

fn check_param_vars(
    sig_name: &str,
    p: &SigParam,
    declared: &std::collections::HashSet<&str>,
) -> Result<(), SignatureBuildError> {
    match p {
        SigParam::Concrete(_) => Ok(()),
        SigParam::Var(v) => {
            if declared.contains(v.as_str()) {
                Ok(())
            } else {
                Err(SignatureBuildError::UndeclaredTypeVar {
                    name: sig_name.to_string(),
                    var_name: v.clone(),
                })
            }
        }
        SigParam::Variadic(inner) => check_param_vars(sig_name, inner, declared),
    }
}

/// Result of a successful [`unify_call`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifyResult {
    /// The concrete return type after resolving all type variables.
    pub return_type: DataType,
    /// Bindings collected for each declared type variable.
    pub bindings: HashMap<String, DataType>,
}

/// Error produced when call-site arguments don't match a signature (§16 #14/#15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnificationError {
    /// A concrete argument type didn't satisfy the parameter's constraint.
    /// `position` is 1-based.
    ConstraintViolation {
        position: usize,
        param_constraint: TypeConstraint,
        actual: DataType,
    },
    /// A type variable bound inconsistently across positions. `positions`
    /// holds every 1-based argument index where this variable appeared
    /// (plus the return position only if it participated — not used in v1).
    InconsistentBinding {
        var_name: String,
        positions: Vec<usize>,
        types: Vec<DataType>,
    },
    /// Not enough positional arguments supplied.
    MissingArgs { expected: usize, got: usize },
    /// Too many positional arguments — no variadic to absorb the overflow.
    TooManyArgs { expected: usize, got: usize },
    /// A variadic type variable with no supplied arguments and no return
    /// binding — can't determine what to bind it to (§16 #15 fallback 2).
    EmptyVariadicTypeVar { var_name: String },
}

impl std::fmt::Display for UnificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnificationError::ConstraintViolation {
                position,
                param_constraint,
                actual,
            } => write!(
                f,
                "argument at position {position} (type {actual}) does not satisfy {param_constraint:?}"
            ),
            UnificationError::InconsistentBinding {
                var_name,
                positions,
                types,
            } => {
                write!(f, "type variable `{var_name}` inferred inconsistently:")?;
                for (pos, ty) in positions.iter().zip(types.iter()) {
                    write!(f, " position {pos} = {ty};")?;
                }
                Ok(())
            }
            UnificationError::MissingArgs { expected, got } => {
                write!(f, "expected at least {expected} argument(s), got {got}")
            }
            UnificationError::TooManyArgs { expected, got } => {
                write!(f, "expected {expected} argument(s), got {got}")
            }
            UnificationError::EmptyVariadicTypeVar { var_name } => write!(
                f,
                "cannot infer type variable `{var_name}` — variadic position received no arguments and no return type is expected from context"
            ),
        }
    }
}

impl std::error::Error for UnificationError {}

/// Unify a signature against a list of concrete argument types, optionally
/// incorporating an expected return type for bidirectional inference (§16 #14,
/// Decision 14 — Phase 27).
///
/// When `expected_return` is `Some(dt)` and the signature's `return_type` is
/// [`TypeExpr::Var(name)`], `dt` is injected as an additional position at
/// **index 0** ("return context") before the binding-reduction step.  This
/// means the expected return participates in LUB (for `Numeric`-constrained
/// variables) or exact-equality checks (for `Ordered`/`Any`/`Concrete`
/// variables) alongside the argument-derived positions.
///
/// Concretely: `COALESCE(1, 2)` in a `Double` context has positions
/// `{(0, Double), (1, Integer), (2, Integer)}`; LUB under the Numeric chain
/// = `Double`, so the call successfully types as `Double`.
///
/// When `expected_return` is `None` this function is equivalent to
/// the plain [`unify_call`].
///
/// ### Position encoding
/// - Positions 1, 2, … are argument positions (1-based, as in v1).
/// - Position 0 is reserved for the "return context" when
///   `expected_return` is `Some(_)`.
///
/// The `lub` closure lives outside this crate because the real LUB
/// computation is in `smelt-db::type_inference::promote_types`, and
/// `smelt-types` must remain dependency-free.
pub fn unify_call_with_expected(
    sig: &Signature,
    args: &[DataType],
    expected_return: Option<&DataType>,
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
) -> Result<UnifyResult, UnificationError> {
    // Determine whether the return type is a naked type variable —
    // only then does `expected_return` contribute a position.
    let return_var: Option<&str> = match &sig.return_type {
        TypeExpr::Var(name) => Some(name.as_str()),
        _ => None,
    };

    // Split leading vs (optional) trailing variadic.
    let (leading, variadic) = match sig.params.last() {
        Some(SigParam::Variadic(inner)) => {
            let last_idx = sig.params.len() - 1;
            (&sig.params[..last_idx], Some(inner.as_ref()))
        }
        _ => (&sig.params[..], None),
    };

    // Arity checks.
    if variadic.is_none() {
        if args.len() < leading.len() {
            return Err(UnificationError::MissingArgs {
                expected: leading.len(),
                got: args.len(),
            });
        }
        if args.len() > leading.len() {
            return Err(UnificationError::TooManyArgs {
                expected: leading.len(),
                got: args.len(),
            });
        }
    } else if args.len() < leading.len() {
        return Err(UnificationError::MissingArgs {
            expected: leading.len(),
            got: args.len(),
        });
    }

    // Collect positions per type variable, in 1-based order.
    let mut var_positions: HashMap<String, Vec<(usize, DataType)>> = HashMap::new();
    for tp in &sig.type_params {
        var_positions.insert(tp.name.clone(), Vec::new());
    }

    // Inject expected_return at position 0 for the return type variable.
    if let (Some(var_name), Some(expected)) = (return_var, expected_return) {
        if let Some(positions) = var_positions.get_mut(var_name) {
            // Check that the expected return satisfies the constraint first.
            let tp = sig
                .type_param(var_name)
                .expect("validated in Signature::new");
            if !tp.constraint.satisfies(expected) {
                // ConstraintViolation at position 0 (return context).
                return Err(UnificationError::ConstraintViolation {
                    position: 0,
                    param_constraint: tp.constraint.clone(),
                    actual: expected.clone(),
                });
            }
            positions.push((0, expected.clone()));
        }
    }

    let check_concrete = |position: usize,
                          constraint: &TypeConstraint,
                          arg: &DataType|
     -> Result<(), UnificationError> {
        if constraint.satisfies(arg) {
            Ok(())
        } else {
            Err(UnificationError::ConstraintViolation {
                position,
                param_constraint: constraint.clone(),
                actual: arg.clone(),
            })
        }
    };

    // Leading params.
    for (idx, (param, arg)) in leading.iter().zip(args.iter()).enumerate() {
        let position = idx + 1;
        match param {
            SigParam::Concrete(c) => check_concrete(position, c, arg)?,
            SigParam::Var(var_name) => {
                let tp = sig
                    .type_param(var_name)
                    .expect("validated in Signature::new");
                check_concrete(position, &tp.constraint, arg)?;
                var_positions
                    .get_mut(var_name)
                    .expect("initialised above")
                    .push((position, arg.clone()));
            }
            SigParam::Variadic(_) => unreachable!("leading can't contain variadic"),
        }
    }

    // Variadic params.
    if let Some(inner) = variadic {
        for (rel, arg) in args[leading.len()..].iter().enumerate() {
            let position = leading.len() + rel + 1;
            match inner {
                SigParam::Concrete(c) => check_concrete(position, c, arg)?,
                SigParam::Var(var_name) => {
                    let tp = sig
                        .type_param(var_name)
                        .expect("validated in Signature::new");
                    check_concrete(position, &tp.constraint, arg)?;
                    var_positions
                        .get_mut(var_name)
                        .expect("initialised above")
                        .push((position, arg.clone()));
                }
                SigParam::Variadic(_) => {
                    unreachable!("nested variadic rejected in Signature::try_new")
                }
            }
        }
    }

    // Reduce per-var positions into a single binding.
    let mut bindings: HashMap<String, DataType> = HashMap::new();
    for tp in &sig.type_params {
        let positions = var_positions.remove(&tp.name).unwrap_or_default();
        if positions.is_empty() {
            return Err(UnificationError::EmptyVariadicTypeVar {
                var_name: tp.name.clone(),
            });
        }
        let binding = match tp.constraint {
            // Only Numeric has a declared promotion chain in v1 (§16 #9/#14).
            TypeConstraint::Numeric => {
                let mut iter = positions.iter();
                let (_, first) = iter.next().unwrap();
                let mut acc = first.clone();
                for (_, ty) in iter {
                    acc = lub(&acc, ty);
                }
                acc
            }
            // All non-Numeric constraints (Ordered, Any, Concrete): require
            // exact equality across positions.
            _ => {
                let first = &positions[0].1;
                let disagreements: Vec<(usize, DataType)> = positions
                    .iter()
                    .filter(|(_, ty)| ty != first)
                    .cloned()
                    .collect();
                if !disagreements.is_empty() {
                    let all_positions: Vec<usize> = positions.iter().map(|(p, _)| *p).collect();
                    let all_types: Vec<DataType> =
                        positions.iter().map(|(_, t)| t.clone()).collect();
                    return Err(UnificationError::InconsistentBinding {
                        var_name: tp.name.clone(),
                        positions: all_positions,
                        types: all_types,
                    });
                }
                first.clone()
            }
        };
        bindings.insert(tp.name.clone(), binding);
    }

    // Resolve the return type.
    let return_type = match &sig.return_type {
        TypeExpr::Concrete(TypeConstraint::Concrete(dt)) => dt.clone(),
        TypeExpr::Concrete(TypeConstraint::Numeric) => DataType::Double,
        TypeExpr::Concrete(TypeConstraint::Ordered) => DataType::Unknown,
        TypeExpr::Concrete(TypeConstraint::Any) => DataType::Unknown,
        TypeExpr::Var(var_name) => bindings
            .get(var_name)
            .cloned()
            .expect("type var validated at Signature::new"),
    };

    Ok(UnifyResult {
        return_type,
        bindings,
    })
}

/// Unify a signature against a list of concrete argument types.
///
/// For each signature, the checker collects every position where a type
/// variable appears (argument positions only in v1; bidirectional
/// checking is deferred to Step 5). Variables whose constraint is
/// [`TypeConstraint::Numeric`] reduce by LUB via the caller-supplied
/// `lub` closure (the only promotion-chain constraint in v1, §16 #14);
/// every other constraint requires exact equality across positions.
///
/// The `lub` closure lives outside this crate because the real LUB
/// computation is in `smelt-db::type_inference::promote_types`, and
/// `smelt-types` must remain dependency-free. Tests in this module use
/// a small inline LUB that matches §16 #9's promotion chain.
///
/// This is a thin wrapper around [`unify_call_with_expected`] with
/// `expected_return = None`. All existing callers continue to compile
/// unchanged.
pub fn unify_call(
    sig: &Signature,
    args: &[DataType],
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
) -> Result<UnifyResult, UnificationError> {
    unify_call_with_expected(sig, args, None, lub)
}

/// A minimal Numeric LUB matching §16 #9 — the only promotion chain in v1.
///
/// Lives in `smelt-types` so the signature-unification unit tests don't
/// depend on `smelt-db`. Production callers in Phase 9+ will pass
/// `smelt-db::promote_types` (or a thin adapter) to [`unify_call`] instead.
pub fn numeric_lub(a: &DataType, b: &DataType) -> DataType {
    // Exact-match early-out (including Decimal precision/scale — v1 keeps
    // Decimal opaque here; smelt-db's promote_types has full decimal rules).
    if std::mem::discriminant(a) == std::mem::discriminant(b) {
        return a.clone();
    }
    use DataType::*;
    let rank = |d: &DataType| -> u8 {
        match d {
            SmallInt => 1,
            Integer => 2,
            BigInt => 3,
            // Per §16 #9: Decimal sits between BigInt and Double; since v1
            // combines Decimal + integer by widening to DECIMAL(38,10),
            // rank Decimal at 4 and Float/Double at 5/6.
            Decimal { .. } => 4,
            Float => 5,
            Double => 6,
            _ => 0,
        }
    };
    match (a, b) {
        // Decimal combined with any integer type widens to DECIMAL(38,10)
        // (§16 #9 rule for precision/scale preservation).
        (Decimal { .. }, SmallInt | Integer | BigInt)
        | (SmallInt | Integer | BigInt, Decimal { .. }) => Decimal {
            precision: 38,
            scale: 10,
        },
        _ => {
            let (ra, rb) = (rank(a), rank(b));
            if ra >= rb {
                a.clone()
            } else {
                b.clone()
            }
        }
    }
}

/// Canonical registry of SQL built-in signatures (§16 #14/#15, Phase 8).
///
/// Phase 8 seeds the registry with ~30 of the most-commonly-used SQL
/// functions, spanning monomorphic, generic, and variadic shapes. The
/// registry is populated once via [`std::sync::LazyLock`] and stays
/// `'static`; [`BuiltinRegistry::resolve`] folds ASCII case at the
/// lookup boundary.
///
/// Known omissions (documented per Phase 8's scope):
/// * `IS NULL` / `IS NOT NULL` — unary predicates with dedicated SQL
///   syntax, not callable via the function-registry surface. A future
///   rewire may route them through a separate predicate resolver.
/// * `CAST(x AS T)` — also has dedicated SQL syntax; tracked separately
///   from the function registry.
///
/// The registry remains *data only* in Phase 8: inference is still
/// driven by the hand-written match in `smelt-db::type_inference`.
/// Phase 9 rewires `infer_function_type` through this registry.
pub struct BuiltinRegistry;

impl BuiltinRegistry {
    /// Resolve a built-in by name, case-insensitively (ASCII folding).
    ///
    /// Returns `Some(&'static Signature)` when the name matches a registered
    /// entry, `None` otherwise.
    pub fn resolve(name: &str) -> Option<&'static Signature> {
        REGISTRY.get(&name.to_ascii_uppercase())
    }

    /// Iterator over all canonical (upper-cased) names in the registry.
    pub fn names() -> impl Iterator<Item = &'static str> {
        REGISTRY.keys().map(|s| s.as_str())
    }

    /// Look up a smelt meta-builtin by its dotted path name (case-insensitive).
    ///
    /// These are smelt-specific meta-language builtins that operate on meta types
    /// (`SmeltType`) rather than SQL types. Examples: `smelt.columns_of`.
    ///
    /// Returns `Some(&'static SmeltMetaSignature)` when the name matches, `None`
    /// otherwise.
    pub fn lookup(name: &str) -> Option<&'static SmeltMetaSignature> {
        META_REGISTRY.get(&name.to_ascii_lowercase())
    }
}

/// The meta-type of a single field in a closed meta-record.
///
/// Used by [`COLUMN_REF_FIELDS`] to express the types of `ColumnRef`'s three
/// fields. Each field maps to a [`SmeltType`].
pub type ColumnRefFieldType = SmeltType;

/// The closed field set of [`SmeltType::ColumnRef`] (Phase C, meta-language).
///
/// Invariant: exactly three entries — `name`, `type`, `is_numeric` — in this order.
/// This is the single source of truth for the v1 field set. Any future addition
/// requires a spec edit AND a change to this constant AND a bump of the field count
/// in all exhaustiveness checks.
///
/// Field types:
/// - `name`       → `Expr<Text>`   (the column identifier as plain Text)
/// - `type`       → `Unknown`      (represents the DataType meta-literal; the
///   concrete meta type is not yet in SmeltType v1)
/// - `is_numeric` → `Expr<Boolean>` (TRUE iff `type` ∈ Numeric constraint set)
pub const COLUMN_REF_FIELDS: &[(&str, ColumnRefFieldType)] = &[
    (
        "name",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    ),
    // `type` is a DataType meta-literal; Phase C maps it to Unknown as a
    // forward-compatibility placeholder. Phase D will introduce a proper
    // meta-DataType representation.
    ("type", SmeltType::Unknown),
    (
        "is_numeric",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
];

/// Look up a [`ColumnRef`] field by name (case-sensitive, exact match).
///
/// Returns `Some(&SmeltType)` for `"name"`, `"type"`, or `"is_numeric"`;
/// `None` for any other identifier (the closed-field invariant).
///
/// Pure — no Salsa dependency.
pub fn column_ref_field(name: &str) -> Option<&'static ColumnRefFieldType> {
    COLUMN_REF_FIELDS
        .iter()
        .find_map(|(field_name, ty)| if *field_name == name { Some(ty) } else { None })
}

/// Signature for a smelt meta-language builtin (Phase C).
///
/// Unlike [`Signature`] (which uses SQL-world [`SigParam`] / [`TypeExpr`]),
/// `SmeltMetaSignature` uses [`SmeltType`] for both parameters and return type.
/// This is necessary because meta-builtins like `smelt.columns_of` operate on
/// meta sorts (`TableExpr`, `ColumnRef`, `List<ColumnRef>`) rather than scalar
/// SQL types.
///
/// Constraints:
/// - `params` are positional only (no variadics, no named args in Phase C).
/// - The registry stores `'static` instances via [`META_REGISTRY`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmeltMetaSignature {
    /// Dotted canonical name (lowercase), e.g. `"smelt.columns_of"`.
    pub name: &'static str,
    /// Positional parameter types in declaration order. Phase C has no variadics
    /// and no named-arg support for meta-builtins.
    pub params: Vec<SmeltType>,
    /// Return type.
    pub return_type: SmeltType,
}

static META_REGISTRY: LazyLock<HashMap<String, SmeltMetaSignature>> = LazyLock::new(|| {
    let mut m: HashMap<String, SmeltMetaSignature> = HashMap::new();

    // `smelt.columns_of(t: TableExpr) -> List<ColumnRef>`
    // One positional `TableExpr` parameter; no variadics; no named args.
    m.insert(
        "smelt.columns_of".to_string(),
        SmeltMetaSignature {
            name: "smelt.columns_of",
            params: vec![SmeltType::TableExpr(None)],
            return_type: SmeltType::List(Box::new(SmeltType::ColumnRef)),
        },
    );

    m
});

fn tp(name: &str, c: TypeConstraint) -> TypeParam {
    TypeParam {
        name: name.to_string(),
        constraint: c,
    }
}

fn concrete(dt: DataType) -> SigParam {
    SigParam::Concrete(TypeConstraint::Concrete(dt))
}

fn var(name: &str) -> SigParam {
    SigParam::Var(name.to_string())
}

fn variadic(inner: SigParam) -> SigParam {
    SigParam::Variadic(Box::new(inner))
}

static REGISTRY: LazyLock<HashMap<String, Signature>> = LazyLock::new(|| {
    let mut m: HashMap<String, Signature> = HashMap::new();
    let mut insert = |sig: Signature| {
        m.insert(sig.name.clone(), sig);
    };

    // ─── Aggregates (treated like scalars here; aggregate-ness is Phase 3+
    //     of the broader roadmap and has no bearing on unification).
    //
    // Phase 12: SUM is the canonical example of §16 #9 engine-divergence.
    // `SUM(INTEGER)` returns `BigInt` in the smelt type system (the
    // "canonical" widening), but DuckDB natively returns `HUGEINT` — a
    // 128-bit type smelt models as `Decimal(38, 0)` in v1 (no dedicated
    // `Hugeint` variant until we have a concrete consumer). The
    // divergence flag feeds Step 7+'s CAST emitter; Phase 12 records
    // only.
    insert(
        Signature::new(
            "SUM",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_canonical_return(DataType::BigInt)
        .with_engine_native(
            "duckdb",
            DataType::Decimal {
                precision: 38,
                scale: 0,
            },
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "AVG",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "MIN",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "MAX",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "COUNT",
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Any)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Agg),
    );

    // ─── Window-only built-ins (Phase 14, §16 #24).
    //
    // These are dispatched only at call sites that carry an `OVER (…)`
    // clause; calling them without `OVER` is a runtime error in every
    // backend. Phase 14 records the kind only — argument-list checks for
    // these signatures land in a later phase. The placeholder `Any` arg
    // lists keep the existing `unify_call` happy without imposing a
    // false constraint.
    insert(
        Signature::new(
            "ROW_NUMBER",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "DENSE_RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LAG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LEAD",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );

    // ─── Null / coalesce / comparison family.
    insert(Signature::new(
        "COALESCE",
        vec![tp("T", TypeConstraint::Any)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "GREATEST",
        vec![tp("T", TypeConstraint::Ordered)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "LEAST",
        vec![tp("T", TypeConstraint::Ordered)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "NULLIF",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T"), var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "IFNULL",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T"), var("T")],
        TypeExpr::Var("T".into()),
    ));

    // ─── Arithmetic / numeric scalars.
    insert(Signature::new(
        "ABS",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "POWER",
        vec![],
        vec![concrete(DataType::Double), concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "SQRT",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LOG",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ROUND",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "CEIL",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "FLOOR",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));

    // ─── Text / string scalars.
    insert(Signature::new(
        "LOWER",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "UPPER",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    ));
    insert(Signature::new(
        "SUBSTRING",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Integer),
            concrete(DataType::Integer),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "TRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "CONCAT",
        vec![],
        vec![variadic(concrete(DataType::Text))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));

    // ─── Date / time basics.
    insert(Signature::new(
        "DATE_TRUNC",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: false,
        })),
    ));
    insert(Signature::new(
        "EXTRACT",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    ));
    insert(Signature::new(
        "DATE",
        vec![],
        vec![concrete(DataType::Timestamp {
            with_timezone: false,
        })],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "NOW",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: false,
        })),
    ));
    insert(Signature::new(
        "CURRENT_DATE",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "CURRENT_TIMESTAMP",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: false,
        })),
    ));

    // ─── Phase 50: Extended aggregates ──────────────────────────────────────

    insert(
        Signature::new(
            "STRING_AGG",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "LISTAGG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "ARRAY_AGG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            // Array<T> cannot be expressed as a TypeExpr::Var directly; use Unknown for v1.
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "MEDIAN",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "STDDEV",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "STDDEV_POP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "STDDEV_SAMP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VARIANCE",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VAR_POP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VAR_SAMP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BOOL_AND",
            vec![],
            vec![concrete(DataType::Boolean)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BOOL_OR",
            vec![],
            vec![concrete(DataType::Boolean)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BIT_AND",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BIT_OR",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BIT_XOR",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "ANY_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "APPROX_COUNT_DISTINCT",
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Any)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Agg),
    );

    // ─── Phase 50: Extended window functions ─────────────────────────────────

    insert(
        Signature::new(
            "NTILE",
            vec![],
            vec![concrete(DataType::BigInt)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "FIRST_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LAST_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "NTH_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), concrete(DataType::BigInt)],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "CUME_DIST",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "PERCENT_RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Window),
    );

    // ─── Phase 50: Extended string scalars ───────────────────────────────────

    insert(Signature::new(
        "LTRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RTRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "CHAR_LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "CHARACTER_LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "REPLACE",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Text),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "LPAD",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::BigInt),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RPAD",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::BigInt),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "REPEAT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "SUBSTR",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "SPLIT_PART",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Text),
            concrete(DataType::BigInt),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "STRPOS",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "LEFT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RIGHT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));

    // ─── Phase 50: Extended math scalars ─────────────────────────────────────

    insert(Signature::new(
        "EXP",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LOG10",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LOG2",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "MOD",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![var("T"), var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "SIGN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "SIN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "COS",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "TAN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ATAN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ATAN2",
        vec![],
        vec![concrete(DataType::Double), concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "SINH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "COSH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "TANH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "PI",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));

    // ─── Phase 50: Extended temporal scalars ──────────────────────────────────

    insert(Signature::new(
        "DATE_PART",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "DATE_ADD",
        vec![],
        vec![concrete(DataType::Date), concrete(DataType::Interval)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "DATE_SUB",
        vec![],
        vec![concrete(DataType::Date), concrete(DataType::Interval)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "MAKE_DATE",
        vec![],
        vec![
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "MAKE_TIMESTAMP",
        vec![],
        vec![
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
            concrete(DataType::BigInt),
            concrete(DataType::Double),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: false,
        })),
    ));
    insert(Signature::new(
        "AGE",
        vec![],
        vec![
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Interval)),
    ));

    // ─── Phase 50: Operator stubs ────────────────────────────────────────────
    //
    // These are not dispatched through `infer_function_type`'s normal path
    // (they use dedicated SQL syntax), but having registry entries enables
    // hover, completion, and future lint rules.

    insert(Signature::new(
        "LIKE",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "ILIKE",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "IS_NULL",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "IS_NOT_NULL",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "BETWEEN",
        vec![tp("T", TypeConstraint::Ordered)],
        vec![var("T"), var("T"), var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "IN",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T"), var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "EXISTS",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
    ));
    insert(Signature::new(
        "CAST",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T")],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown)),
    ));

    m
});

/// Format a [`SmeltType`] as a concise hover string (Phase 18).
///
/// Used by the LSP hover handler to display parameter types in
/// `smelt.define` signatures. Examples:
///   - `Expr<Integer>` → `"Expr<Integer>"`
///   - `Expr<Numeric>` → `"Expr<Numeric>"`
///   - `TableExpr` → `"TableExpr"`
///   - `TableExpr<{revenue: Numeric, cost: Numeric}>` → `"TableExpr<{revenue: Numeric, cost: Numeric}>"`
///   - With named tail `..r` → `"TableExpr<{revenue: Numeric, ..r}>"`
pub fn format_smelt_type_hover(ty: &SmeltType) -> String {
    match ty {
        SmeltType::Expr(tc) => format!("Expr<{}>", format_type_constraint_hover(tc)),
        SmeltType::List(inner) => format!("List<{}>", format_smelt_type_hover(inner)),
        SmeltType::Lambda(param_ty, body_ty) => format!(
            "Lambda<{}, {}>",
            format_smelt_type_hover(param_ty),
            format_smelt_type_hover(body_ty)
        ),
        SmeltType::TableExpr(None) => "TableExpr".to_string(),
        SmeltType::TableExpr(Some(req)) => {
            let mut s = String::from("TableExpr<{");
            for (i, (col, req)) in req.required.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(col);
                s.push_str(": ");
                s.push_str(&req.render());
            }
            match &req.tail {
                RowTail::None => {}
                RowTail::Anon => {
                    if !req.required.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                }
                RowTail::Named(name) => {
                    if !req.required.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                    s.push_str(name);
                }
            }
            s.push_str("}>");
            s
        }
        SmeltType::SelectItems { kind, context } => {
            let kind_str = match kind {
                ExprKind::Scalar => "Scalar",
                ExprKind::Agg => "Agg",
                ExprKind::Window => "Window",
            };
            if let Some(ctx) = context {
                format!("SelectItems<{}, {}>", kind_str, ctx.name())
            } else {
                format!("SelectItems<{}>", kind_str)
            }
        }
        SmeltType::Struct { fields, tail } => {
            let mut s = String::from("Expr<Struct<{");
            for (i, (name, dt)) in fields.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(name);
                s.push_str(": ");
                s.push_str(&dt.to_string());
            }
            match tail {
                StructRowTail::None => {}
                StructRowTail::Anon => {
                    if !fields.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                }
                StructRowTail::Named(name) => {
                    if !fields.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                    s.push_str(name);
                }
            }
            s.push_str("}>");
            s.push('>');
            s
        }
        SmeltType::Unknown => "Unknown".to_string(),
        SmeltType::ColumnRef => "ColumnRef".to_string(),
    }
}

fn format_type_constraint_hover(tc: &TypeConstraint) -> String {
    match tc {
        TypeConstraint::Concrete(dt) => dt.to_sql().to_string(),
        TypeConstraint::Numeric => "Numeric".to_string(),
        TypeConstraint::Ordered => "Ordered".to_string(),
        TypeConstraint::Any => "Any".to_string(),
    }
}

/// A cache-key-safe wrapper around a list of parameter (name, DataType) bindings.
/// Used as a Salsa cache key for per-(callee, arg-types) expansion caching (Phase 26+).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DataTypeHash(pub Vec<(String, DataType)>);

impl DataTypeHash {
    pub fn new(bindings: Vec<(String, DataType)>) -> Self {
        Self(bindings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::{parse, File as AstFile};

    fn parse_file(text: &str) -> (AstFile, String) {
        let clean = smelt_parser::strip_frontmatter(text);
        let parse = parse(&clean);
        let ast = AstFile::cast(parse.syntax()).expect("FILE node");
        (ast, clean)
    }

    // === Phase 3 tests (still passing) ===

    #[test]
    fn extracts_minimal_signature() {
        let (file, text) = parse_file("smelt.define foo(x) AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params.len(), 1);
        assert_eq!(sigs[0].params[0].name, "x");
        assert!(sigs[0].params[0].type_ref_text.is_none());
        assert!(sigs[0].params[0].type_ref.is_none());
        assert!(sigs[0].return_type_text.is_none());
        assert!(sigs[0].return_type.is_none());
        assert_eq!(sigs[0].tier, Tier::One);
    }

    #[test]
    fn tier_two_when_params_annotated_return_missing() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].tier, Tier::Two);
    }

    #[test]
    fn tier_three_when_fully_annotated() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs[0].tier, Tier::Three);
        let ret = sigs[0].return_type_text.as_deref().unwrap();
        assert!(
            ret.contains("Expr<Integer>"),
            "expected return text to contain Expr<Integer>, got {ret:?}"
        );
    }

    #[test]
    fn default_value_flagged() {
        let (file, text) = parse_file("smelt.define f(x: Expr<Integer> = 0) AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert!(sigs[0].params[0].has_default);
    }

    #[test]
    fn lookup_by_name() {
        let (file, text) = parse_file("smelt.define a(x) AS (x)\nsmelt.define b(y) AS (y)\n");
        let sig = extract_function_signature_by_name(&file, &text, "b").unwrap();
        assert_eq!(sig.name, "b");
        assert!(extract_function_signature_by_name(&file, &text, "nope").is_none());
    }

    // === Phase 4 TDD tests ===

    #[test]
    fn parses_expr_of_concrete_type() {
        assert_eq!(
            parse_smelt_type("Expr<Integer>"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
        );
    }

    #[test]
    fn parses_expr_of_boolean_concrete_type() {
        // The plan explicitly calls out Boolean as a required concrete case.
        assert_eq!(
            parse_smelt_type("Expr<Boolean>"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)))
        );
    }

    #[test]
    fn parses_expr_of_numeric_constraint() {
        assert_eq!(
            parse_smelt_type("Expr<Numeric>"),
            Ok(SmeltType::Expr(TypeConstraint::Numeric))
        );
    }

    #[test]
    fn rejects_unknown_sort() {
        // `TableExpr<T>` — explicitly deferred to Step 3. We expect
        // `UnsupportedSort` with the sort keyword exposed to the user.
        match parse_smelt_type("TableExpr<T>") {
            Err(SmeltTypeParseError::UnsupportedSort { sort, span_text }) => {
                assert_eq!(sort, "TableExpr");
                assert_eq!(span_text, "TableExpr<T>");
            }
            other => panic!("expected UnsupportedSort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_nested_expr() {
        match parse_smelt_type("Expr<Expr<Integer>>") {
            Err(SmeltTypeParseError::NestedExpr { span_text }) => {
                assert_eq!(span_text, "Expr<Expr<Integer>>");
            }
            other => panic!("expected NestedExpr, got {other:?}"),
        }
    }

    #[test]
    fn numeric_constraint_accepts_integer() {
        let c = TypeConstraint::Numeric;
        // Full membership of §16 #9.
        assert!(c.satisfies(&DataType::SmallInt));
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::BigInt));
        assert!(c.satisfies(&DataType::Float));
        assert!(c.satisfies(&DataType::Double));
        assert!(c.satisfies(&DataType::Decimal {
            precision: 10,
            scale: 2,
        }));
    }

    #[test]
    fn numeric_constraint_rejects_text() {
        let c = TypeConstraint::Numeric;
        assert!(!c.satisfies(&DataType::Text));
        assert!(!c.satisfies(&DataType::Boolean));
        assert!(!c.satisfies(&DataType::Date));
    }

    #[test]
    fn any_constraint_accepts_everything() {
        let c = TypeConstraint::Any;
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::Text));
        assert!(c.satisfies(&DataType::Boolean));
    }

    #[test]
    fn concrete_constraint_is_exact() {
        let c = TypeConstraint::Concrete(DataType::Integer);
        assert!(c.satisfies(&DataType::Integer));
        assert!(!c.satisfies(&DataType::BigInt));
        assert!(!c.satisfies(&DataType::Text));
    }

    #[test]
    fn parses_expr_with_whitespace() {
        assert_eq!(
            parse_smelt_type("Expr< Integer >"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
        );
    }

    #[test]
    fn malformed_missing_angle_brackets() {
        assert!(matches!(
            parse_smelt_type("Expr"),
            Err(SmeltTypeParseError::Malformed { .. })
        ));
    }

    #[test]
    fn malformed_empty_inner() {
        assert!(matches!(
            parse_smelt_type("Expr<>"),
            Err(SmeltTypeParseError::Malformed { .. })
        ));
    }

    #[test]
    fn unknown_inner_type() {
        match parse_smelt_type("Expr<FooBar>") {
            Err(SmeltTypeParseError::UnknownInner { inner, .. }) => {
                assert_eq!(inner, "FooBar");
            }
            other => panic!("expected UnknownInner, got {other:?}"),
        }
    }

    // === FunctionSig / ParamSpec wiring ===

    #[test]
    fn function_sig_exposes_parsed_param_types() {
        let (file, text) = parse_file(
            "smelt.define f(x: Expr<Integer>, y: Expr<Numeric>) -> Expr<Double> AS (x + y)",
        );
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let sig = &sigs[0];
        assert_eq!(
            sig.params[0].type_ref.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))
        );
        assert_eq!(
            sig.params[1].type_ref.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Numeric)
        );
        assert_eq!(
            sig.return_type.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Concrete(DataType::Double))
        );

        // Ranges populated on annotated params and return.
        assert!(sig.params[0].type_ref_range.is_some());
        assert!(sig.params[1].type_ref_range.is_some());
        assert!(sig.return_type_range.is_some());
    }

    #[test]
    fn function_sig_surfaces_bad_annotation_as_error() {
        // `TableExpr<T>` should be parsed into an `Err(UnsupportedSort)` so
        // higher layers can emit a diagnostic. Until the Phase 6 unified
        // harness arrives this is the targeted unit test called out in the
        // plan.
        let (file, text) = parse_file("smelt.define bad(x: TableExpr<T>) AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let param = &sigs[0].params[0];
        let err = param
            .type_ref
            .as_ref()
            .expect("annotation present")
            .as_ref()
            .expect_err("should be an error");
        match err {
            SmeltTypeParseError::UnsupportedSort { sort, .. } => {
                assert_eq!(sort, "TableExpr");
            }
            other => panic!("expected UnsupportedSort, got {other:?}"),
        }
        assert!(param.type_ref_range.is_some());
    }

    #[test]
    fn function_sig_surfaces_bad_return_annotation() {
        let (file, text) = parse_file("smelt.define bad(x: Expr<Integer>) -> TableExpr<T> AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let err = sigs[0]
            .return_type
            .as_ref()
            .expect("annotation present")
            .as_ref()
            .expect_err("should be an error");
        assert!(matches!(err, SmeltTypeParseError::UnsupportedSort { .. }));
        assert!(sigs[0].return_type_range.is_some());
    }

    // === Phase 7 TDD tests — Ordered constraint + registry skeleton ===

    #[test]
    fn ordered_members_match_decision_13() {
        // §16 #13: Numeric ∪ {Text family, temporal family, Boolean, Interval,
        // Blob}. This test enumerates every member exhaustively.
        let c = TypeConstraint::Ordered;

        // Numeric members (also covered by numeric_is_subset_of_ordered, but
        // the research text explicitly enumerates them here).
        assert!(c.satisfies(&DataType::SmallInt));
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::BigInt));
        assert!(c.satisfies(&DataType::Float));
        assert!(c.satisfies(&DataType::Double));
        assert!(c.satisfies(&DataType::Decimal {
            precision: 10,
            scale: 2,
        }));

        // String family.
        assert!(c.satisfies(&DataType::Text));
        assert!(c.satisfies(&DataType::Varchar { max_length: None }));
        assert!(c.satisfies(&DataType::Varchar {
            max_length: Some(10)
        }));
        assert!(c.satisfies(&DataType::Char { length: 1 }));

        // Temporal family, including both Timestamp tz variants, plus
        // Interval.
        assert!(c.satisfies(&DataType::Date));
        assert!(c.satisfies(&DataType::Time));
        assert!(c.satisfies(&DataType::Timestamp {
            with_timezone: false
        }));
        assert!(c.satisfies(&DataType::Timestamp {
            with_timezone: true
        }));
        assert!(c.satisfies(&DataType::Interval));

        // Remaining singletons.
        assert!(c.satisfies(&DataType::Boolean));
        // "Binary" in §16 #13 is spelt Blob here.
        assert!(c.satisfies(&DataType::Blob));
    }

    #[test]
    fn ordered_excludes_composites() {
        let c = TypeConstraint::Ordered;
        assert!(!c.satisfies(&DataType::Array(Box::new(DataType::Integer))));
        assert!(!c.satisfies(&DataType::Struct(vec![(
            "a".to_string(),
            DataType::Integer,
        )])));
        assert!(!c.satisfies(&DataType::Map(
            Box::new(DataType::Text),
            Box::new(DataType::Integer),
        )));
        // Null and Unknown are explicitly not Ordered members.
        assert!(!c.satisfies(&DataType::Null));
        assert!(!c.satisfies(&DataType::Unknown));
    }

    #[test]
    fn numeric_is_subset_of_ordered() {
        // Every type the Numeric constraint accepts must also satisfy the
        // Ordered constraint (§16 #13: Numeric ⊂ Ordered).
        let numerics = [
            DataType::SmallInt,
            DataType::Integer,
            DataType::BigInt,
            DataType::Float,
            DataType::Double,
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        ];
        for dt in &numerics {
            assert!(
                TypeConstraint::Numeric.satisfies(dt),
                "expected Numeric to accept {dt:?}"
            );
            assert!(
                TypeConstraint::Ordered.satisfies(dt),
                "expected Ordered to accept numeric {dt:?}"
            );
        }
    }

    #[test]
    fn registry_lookup_by_name() {
        // Phase 8 migrated these entries to the new shape. LOWER/UPPER/LENGTH
        // are still monomorphic (no type params, concrete params + return);
        // ABS moved to `ABS<T: Numeric>(T) → T` per the plan.
        let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
        assert_eq!(lower.name, "LOWER");
        assert!(lower.type_params.is_empty());
        assert_eq!(
            lower.params,
            vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
        );
        assert_eq!(
            lower.return_type,
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text))
        );

        let upper = BuiltinRegistry::resolve("UPPER").expect("UPPER present");
        assert_eq!(
            upper.params,
            vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
        );
        assert_eq!(
            upper.return_type,
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text))
        );

        let length = BuiltinRegistry::resolve("LENGTH").expect("LENGTH present");
        assert_eq!(
            length.params,
            vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
        );
        assert_eq!(
            length.return_type,
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer))
        );

        let abs = BuiltinRegistry::resolve("ABS").expect("ABS present");
        assert_eq!(abs.type_params.len(), 1);
        assert_eq!(abs.type_params[0].name, "T");
        assert_eq!(abs.type_params[0].constraint, TypeConstraint::Numeric);
        assert_eq!(abs.params, vec![SigParam::Var("T".into())]);
        assert_eq!(abs.return_type, TypeExpr::Var("T".into()));
    }

    #[test]
    fn registry_lookup_case_insensitive() {
        let canonical = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
        let lowercase = BuiltinRegistry::resolve("lower").expect("lower present");
        let titlecase = BuiltinRegistry::resolve("Lower").expect("Lower present");
        let mixed = BuiltinRegistry::resolve("LoWeR").expect("LoWeR present");

        // All four lookups must resolve to the same `&'static Signature` —
        // ASCII case folding happens at the lookup boundary, not by inserting
        // multiple entries.
        assert!(std::ptr::eq(canonical, lowercase));
        assert!(std::ptr::eq(canonical, titlecase));
        assert!(std::ptr::eq(canonical, mixed));
    }

    // === Phase 8 TDD tests — generics + variadics (§16 #14, #15) ===

    #[test]
    fn min_generic_preserves_input_type() {
        // `MIN<T: Ordered>(T) → T` with Integer must return Integer — the
        // canonical type-preserving case (§16 #14).
        let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
        let res = unify_call(sig, &[DataType::Integer], &numeric_lub).expect("unification ok");
        assert_eq!(res.return_type, DataType::Integer);
        assert_eq!(res.bindings.get("T"), Some(&DataType::Integer));
    }

    #[test]
    fn coalesce_lub_of_numeric_args() {
        // COALESCE is `<T: Any>(T...) → T`. Any has no promotion chain, so
        // mixing Integer/BigInt/Double would normally fail unification.
        // For the LUB test, we exercise the Numeric-chain path via a bespoke
        // signature (the core behaviour under test is that a Numeric-constrained
        // type variable reduces by LUB across all positions).
        let sig = Signature::new(
            "numeric_coalesce",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![variadic(var("T"))],
            TypeExpr::Var("T".into()),
        );
        let res = unify_call(
            &sig,
            &[DataType::Integer, DataType::BigInt, DataType::Double],
            &numeric_lub,
        )
        .expect("LUB should succeed under Numeric constraint");
        assert_eq!(res.return_type, DataType::Double);
    }

    #[test]
    fn coalesce_text_int_rejects() {
        // COALESCE has an Any-constrained type var. Any has no promotion
        // chain, so mixing Text/Integer must fail with an InconsistentBinding
        // citing position 2 (the Integer that conflicts with T=Text
        // established at position 1).
        let sig = BuiltinRegistry::resolve("COALESCE").expect("COALESCE present");
        let err = unify_call(sig, &[DataType::Text, DataType::Integer], &numeric_lub)
            .expect_err("Text/Integer must not unify under `Any`");
        match err {
            UnificationError::InconsistentBinding {
                var_name,
                positions,
                types,
            } => {
                assert_eq!(var_name, "T");
                // Both positions are cited; position 2 (the mismatch) must
                // appear so the user can pinpoint the offender.
                assert!(
                    positions.contains(&2),
                    "expected position 2 to be cited, got {positions:?}"
                );
                assert!(
                    positions.contains(&1),
                    "expected position 1 (the establishing Text) to be cited, got {positions:?}"
                );
                assert!(types.contains(&DataType::Text));
                assert!(types.contains(&DataType::Integer));
            }
            other => panic!("expected InconsistentBinding, got {other:?}"),
        }
    }

    #[test]
    fn greatest_variadic_allows_single_arg() {
        // `GREATEST<T: Ordered>(T...) → T` must accept exactly one arg.
        let sig = BuiltinRegistry::resolve("GREATEST").expect("GREATEST present");
        let res = unify_call(sig, &[DataType::Integer], &numeric_lub)
            .expect("GREATEST should accept a single Integer");
        assert_eq!(res.return_type, DataType::Integer);
    }

    #[test]
    fn concat_zero_args_returns_text() {
        // CONCAT has a concrete Text variadic and a concrete Text return —
        // no type vars to infer. Zero args therefore types cleanly.
        let sig = BuiltinRegistry::resolve("CONCAT").expect("CONCAT present");
        let res = unify_call(sig, &[], &numeric_lub).expect("zero-arity CONCAT ok");
        assert_eq!(res.return_type, DataType::Text);
    }

    #[test]
    fn generic_inference_error_cites_positions() {
        // §16 #14's error-surface contract: messages must cite the positions
        // that forced the inconsistent binding.
        let sig = BuiltinRegistry::resolve("COALESCE").expect("COALESCE present");
        let err = unify_call(
            sig,
            &[DataType::Text, DataType::Integer, DataType::Text],
            &numeric_lub,
        )
        .expect_err("Text/Integer/Text must not unify under Any");
        match &err {
            UnificationError::InconsistentBinding { positions, .. } => {
                // All three positions are cited in the error payload.
                assert_eq!(positions.len(), 3);
                assert!(positions.contains(&1));
                assert!(positions.contains(&2));
                assert!(positions.contains(&3));
            }
            other => panic!("expected InconsistentBinding, got {other:?}"),
        }
        // The Display impl mentions "position N" so users can read the error.
        let rendered = format!("{err}");
        assert!(
            rendered.contains("position 1"),
            "error message should mention position 1, got {rendered:?}"
        );
        assert!(
            rendered.contains("position 2"),
            "error message should mention position 2, got {rendered:?}"
        );
    }

    // === Phase 8 supplementary tests — signature construction invariants ===

    #[test]
    fn non_trailing_variadic_rejected() {
        let err = Signature::try_new(
            "bad",
            vec![],
            vec![
                SigParam::Variadic(Box::new(SigParam::Concrete(TypeConstraint::Concrete(
                    DataType::Text,
                )))),
                SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .expect_err("variadic in non-trailing position must be rejected");
        assert!(matches!(
            err,
            SignatureBuildError::NonTrailingVariadic { .. }
        ));
    }

    #[test]
    fn undeclared_type_var_rejected() {
        let err = Signature::try_new(
            "bad",
            vec![],
            vec![SigParam::Var("T".into())],
            TypeExpr::Var("T".into()),
        )
        .expect_err("undeclared type var must be rejected");
        assert!(matches!(err, SignatureBuildError::UndeclaredTypeVar { .. }));
    }

    #[test]
    fn too_many_args_for_fixed_arity_signature() {
        let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
        let err = unify_call(sig, &[DataType::Text, DataType::Text], &numeric_lub)
            .expect_err("LOWER takes exactly one arg");
        assert!(matches!(err, UnificationError::TooManyArgs { .. }));
    }

    #[test]
    fn missing_args_for_leading_positions() {
        // SUBSTRING takes three args; supplying one triggers MissingArgs.
        let sig = BuiltinRegistry::resolve("SUBSTRING").expect("SUBSTRING present");
        let err =
            unify_call(sig, &[DataType::Text], &numeric_lub).expect_err("SUBSTRING needs 3 args");
        assert!(matches!(
            err,
            UnificationError::MissingArgs {
                expected: 3,
                got: 1
            }
        ));
    }

    #[test]
    fn constraint_violation_for_wrong_type() {
        // LENGTH(Text) → Integer — passing Integer must violate the Text
        // constraint at position 1.
        let sig = BuiltinRegistry::resolve("LENGTH").expect("LENGTH present");
        let err = unify_call(sig, &[DataType::Integer], &numeric_lub)
            .expect_err("LENGTH(Integer) rejects");
        match err {
            UnificationError::ConstraintViolation {
                position, actual, ..
            } => {
                assert_eq!(position, 1);
                assert_eq!(actual, DataType::Integer);
            }
            other => panic!("expected ConstraintViolation, got {other:?}"),
        }
    }

    #[test]
    fn count_accepts_any_returns_bigint() {
        // COUNT(Any) → BigInt is the monomorphic shape that accepts any
        // concrete type without introducing a type variable.
        let sig = BuiltinRegistry::resolve("COUNT").expect("COUNT present");
        for dt in [
            DataType::Integer,
            DataType::Text,
            DataType::Boolean,
            DataType::Date,
        ] {
            let res = unify_call(sig, std::slice::from_ref(&dt), &numeric_lub)
                .unwrap_or_else(|e| panic!("COUNT({dt:?}) should succeed: {e}"));
            assert_eq!(res.return_type, DataType::BigInt);
        }
    }

    #[test]
    fn numeric_lub_matches_promotion_chain() {
        // Spot-check the helper LUB against §16 #9.
        assert_eq!(
            numeric_lub(&DataType::Integer, &DataType::BigInt),
            DataType::BigInt
        );
        assert_eq!(
            numeric_lub(&DataType::Integer, &DataType::Double),
            DataType::Double
        );
        assert_eq!(
            numeric_lub(&DataType::Integer, &DataType::SmallInt),
            DataType::Integer
        );
        // Decimal + integer widens to DECIMAL(38,10).
        assert_eq!(
            numeric_lub(
                &DataType::Integer,
                &DataType::Decimal {
                    precision: 5,
                    scale: 2,
                },
            ),
            DataType::Decimal {
                precision: 38,
                scale: 10,
            }
        );
    }

    // === Phase 12 TDD tests — multi-level frame rendering + CAST flag ===

    #[test]
    fn cast_flag_set_when_canonical_differs_from_engine() {
        // Phase 12 TDD test 3 (§16 #9): `SUM` is seeded with
        // canonical = BigInt and engine_native[duckdb] = DECIMAL(38,0)
        // — the smelt stand-in for DuckDB's HUGEINT return. The
        // `needs_cast_for("duckdb")` hook must flag divergence so
        // Step 7+ can emit a CAST back to BigInt.
        let sum = BuiltinRegistry::resolve("SUM").expect("SUM seeded");
        assert_eq!(sum.canonical_return, Some(DataType::BigInt));
        assert_eq!(
            sum.engine_native.get("duckdb"),
            Some(&DataType::Decimal {
                precision: 38,
                scale: 0,
            })
        );
        assert!(
            sum.needs_cast_for("duckdb"),
            "SUM on DuckDB returns HUGEINT (DECIMAL(38,0)) but canonical is BigInt \
             — needs_cast_for must flag the divergence"
        );
        // Engines that aren't listed default to "native == canonical".
        assert!(
            !sum.needs_cast_for("spark"),
            "No override for spark → canonical matches native → no cast needed"
        );
        // Key lookup is case-insensitive / trimmed.
        assert!(sum.needs_cast_for("  DuckDB  "));
    }

    #[test]
    fn cast_flag_false_for_canonical_less_signatures() {
        // The majority of signatures don't declare a canonical — their
        // native return IS their canonical. `needs_cast_for(...)` must
        // return `false` unconditionally for those.
        let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER seeded");
        assert!(lower.canonical_return.is_none());
        assert!(!lower.needs_cast_for("duckdb"));
        assert!(!lower.needs_cast_for("spark"));
        assert!(!lower.needs_cast_for(""));
    }

    #[test]
    fn cast_flag_false_when_native_matches_canonical() {
        // Explicit divergence-negation: a signature that declares a
        // canonical AND a matching native override still reports false.
        let sig = Signature::new(
            "SAME",
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Concrete(
                DataType::Integer,
            ))],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        )
        .with_canonical_return(DataType::Integer)
        .with_engine_native("duckdb", DataType::Integer);
        assert!(!sig.needs_cast_for("duckdb"));
    }

    #[test]
    fn frame_info_location_fields_default_none() {
        // Phase 12 added `decl_path`, `decl_range`, `call_site_range`
        // to `FrameInfo`. Constructors that don't populate them must
        // default to `None` so legacy callers (tests, mock harnesses)
        // continue to compile and behave identically.
        let frame = FrameInfo {
            function: "f".into(),
            param: "x".into(),
            bound_type: "INTEGER".into(),
            decl_path: None,
            decl_range: None,
            call_site_range: None,
            fn_id: None,
            element_index: None,
        };
        assert!(frame.decl_path.is_none());
        assert!(frame.decl_range.is_none());
        assert!(frame.call_site_range.is_none());
    }

    // === Phase 14 TDD tests — ExprKind helpers + registry kind seeding ===

    /// `subkind_of` realises the linear `Scalar <= Agg <= Window` chain
    /// (§16 #24). Every kind is its own subkind; non-comparable pairs in
    /// the *reverse* direction return `false`.
    #[test]
    fn kind_subtype_chain() {
        // Reflexive.
        assert!(subkind_of(ExprKind::Scalar, ExprKind::Scalar));
        assert!(subkind_of(ExprKind::Agg, ExprKind::Agg));
        assert!(subkind_of(ExprKind::Window, ExprKind::Window));

        // Forward chain: Scalar <= Agg <= Window.
        assert!(subkind_of(ExprKind::Scalar, ExprKind::Agg));
        assert!(subkind_of(ExprKind::Scalar, ExprKind::Window));
        assert!(subkind_of(ExprKind::Agg, ExprKind::Window));

        // Reverse direction is disallowed — Window does NOT fit a Scalar
        // splice point and Agg does NOT fit a Scalar splice point.
        assert!(!subkind_of(ExprKind::Window, ExprKind::Scalar));
        assert!(!subkind_of(ExprKind::Window, ExprKind::Agg));
        assert!(!subkind_of(ExprKind::Agg, ExprKind::Scalar));
    }

    /// `kind_ceiling` returns the maximum kind in the slice (§16 #24).
    /// Empty slice degrades to `Scalar` per the documented invariant.
    #[test]
    fn selectitems_kind_ceiling() {
        // Empty: Scalar by convention.
        assert_eq!(kind_ceiling(&[]), ExprKind::Scalar);

        // [Scalar] → Scalar.
        assert_eq!(kind_ceiling(&[ExprKind::Scalar]), ExprKind::Scalar);

        // [user_id, COUNT(*)] → Agg (one Agg item lifts the whole list).
        assert_eq!(
            kind_ceiling(&[ExprKind::Scalar, ExprKind::Agg]),
            ExprKind::Agg
        );

        // [COUNT(*) OVER (...)] → Window.
        assert_eq!(kind_ceiling(&[ExprKind::Window]), ExprKind::Window);

        // Window dominates Agg in mixed lists.
        assert_eq!(
            kind_ceiling(&[ExprKind::Agg, ExprKind::Window, ExprKind::Scalar]),
            ExprKind::Window
        );
    }

    /// Registry seed: aggregates carry [`ExprKind::Agg`].
    #[test]
    fn registry_aggregates_seeded_with_agg_kind() {
        for name in ["SUM", "AVG", "MIN", "MAX", "COUNT"] {
            let sig = BuiltinRegistry::resolve(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            assert_eq!(
                sig.kind,
                ExprKind::Agg,
                "{name} should be seeded with ExprKind::Agg"
            );
        }
    }

    /// Registry seed: window-only built-ins carry [`ExprKind::Window`].
    #[test]
    fn registry_window_funcs_seeded_with_window_kind() {
        for name in ["ROW_NUMBER", "RANK", "DENSE_RANK", "LAG", "LEAD"] {
            let sig = BuiltinRegistry::resolve(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            assert_eq!(
                sig.kind,
                ExprKind::Window,
                "{name} should be seeded with ExprKind::Window"
            );
        }
    }

    /// Registry seed: plain scalar built-ins default to [`ExprKind::Scalar`].
    #[test]
    fn registry_scalar_defaults() {
        for name in ["LOWER", "UPPER", "ABS", "CONCAT", "POWER", "NOW"] {
            let sig = BuiltinRegistry::resolve(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            assert_eq!(
                sig.kind,
                ExprKind::Scalar,
                "{name} should default to ExprKind::Scalar"
            );
        }
    }

    // =================================================================
    // Phase 16 — SchemaRequirement / check_schema_requirement tests
    // =================================================================

    fn req_numeric_rev_cost(tail: RowTail) -> SchemaRequirement {
        SchemaRequirement {
            required: vec![
                (
                    "revenue".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                ),
                (
                    "cost".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                ),
            ],
            tail,
        }
    }

    #[test]
    fn schema_requirement_happy_path_matches_required_columns_exactly() {
        let req = req_numeric_rev_cost(RowTail::None);
        let schema = vec![
            ("revenue".to_string(), DataType::Double),
            (
                "cost".to_string(),
                DataType::Decimal {
                    precision: 18,
                    scale: 2,
                },
            ),
        ];
        let out = check_schema_requirement(&req, &schema).expect("match");
        // No tail → no binding.
        assert!(out.is_none());
    }

    #[test]
    fn schema_requirement_missing_column_returns_structured_error() {
        // `cost` is absent from the caller's schema.
        let req = req_numeric_rev_cost(RowTail::None);
        let schema = vec![("revenue".to_string(), DataType::Double)];
        let err = check_schema_requirement(&req, &schema).unwrap_err();
        match err {
            SchemaMismatch::MissingColumn { column, required } => {
                assert_eq!(column, "cost");
                assert!(matches!(
                    required,
                    DataTypeReq::Constraint(TypeConstraint::Numeric)
                ));
            }
            other => panic!("expected MissingColumn, got {other:?}"),
        }
    }

    #[test]
    fn schema_requirement_type_mismatch_returns_structured_error() {
        // `revenue` is Text — not numeric.
        let req = req_numeric_rev_cost(RowTail::None);
        let schema = vec![
            ("revenue".to_string(), DataType::Text),
            ("cost".to_string(), DataType::Double),
        ];
        let err = check_schema_requirement(&req, &schema).unwrap_err();
        match err {
            SchemaMismatch::TypeMismatch {
                column,
                required,
                actual,
            } => {
                assert_eq!(column, "revenue");
                assert!(matches!(
                    required,
                    DataTypeReq::Constraint(TypeConstraint::Numeric)
                ));
                assert!(actual.contains("TEXT") || actual.contains("Text"));
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn schema_requirement_tail_none_accepts_extras_without_binding() {
        // Phase 16 accepts extras by default — `RowTail::None` still
        // succeeds on a superset schema; the open-record semantics
        // from research §8 mean only `MissingColumn` / `TypeMismatch`
        // produce structural failures. No binding is recorded because
        // the tail is not named.
        let req = req_numeric_rev_cost(RowTail::None);
        let schema = vec![
            ("revenue".to_string(), DataType::Double),
            ("cost".to_string(), DataType::Double),
            ("extra".to_string(), DataType::Text),
        ];
        let out = check_schema_requirement(&req, &schema).expect("extras accepted");
        assert!(
            out.is_none(),
            "tail `None` does not bind extras; got {out:?}"
        );
    }

    #[test]
    fn schema_requirement_tail_anon_accepts_extras_without_binding() {
        let req = req_numeric_rev_cost(RowTail::Anon);
        let schema = vec![
            ("revenue".to_string(), DataType::Double),
            ("cost".to_string(), DataType::Double),
            ("extra".to_string(), DataType::Text),
        ];
        let out = check_schema_requirement(&req, &schema).expect("accept");
        assert!(out.is_none(), "anon tail does not bind; got {out:?}");
    }

    #[test]
    fn schema_requirement_named_tail_binds_extras_in_caller_order() {
        let req = req_numeric_rev_cost(RowTail::Named("r".to_string()));
        let schema = vec![
            ("revenue".to_string(), DataType::Double),
            ("cost".to_string(), DataType::Double),
            ("notes".to_string(), DataType::Text),
            ("extra".to_string(), DataType::BigInt),
        ];
        let binding = check_schema_requirement(&req, &schema)
            .expect("match")
            .expect("named tail produces binding");
        assert_eq!(binding.name, "r");
        assert_eq!(
            binding.extras,
            vec![
                ("notes".to_string(), DataType::Text),
                ("extra".to_string(), DataType::BigInt),
            ]
        );
    }

    #[test]
    fn schema_requirement_concrete_match_accepts_text_varchar() {
        // `notes: Text` required, caller supplies canonical `Varchar`
        // — same family, compatible under our row-requirement rule
        // (Text normalizes to Varchar { max_length: None }).
        let req = SchemaRequirement {
            required: vec![("notes".to_string(), DataTypeReq::Concrete(DataType::Text))],
            tail: RowTail::Anon,
        };
        let schema = vec![("notes".to_string(), DataType::Varchar { max_length: None })];
        assert!(check_schema_requirement(&req, &schema).is_ok());
    }

    // Phase 18 TDD tests — hover formatter

    #[test]
    fn lsp_hover_tableexpr_shows_schema() {
        let req = SchemaRequirement {
            required: vec![
                (
                    "revenue".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                ),
                (
                    "cost".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                ),
            ],
            tail: RowTail::None,
        };
        let ty = SmeltType::TableExpr(Some(req));
        let hover = format_smelt_type_hover(&ty);
        assert!(hover.contains("revenue"), "missing 'revenue' in: {hover}");
        assert!(hover.contains("cost"), "missing 'cost' in: {hover}");
        assert!(hover.contains("Numeric"), "missing 'Numeric' in: {hover}");
        assert!(
            hover.starts_with("TableExpr<{"),
            "expected TableExpr<{{..}}>: {hover}"
        );
    }

    #[test]
    fn lsp_hover_bare_tableexpr_shows_type() {
        assert_eq!(
            format_smelt_type_hover(&SmeltType::TableExpr(None)),
            "TableExpr"
        );
    }

    #[test]
    fn lsp_hover_tableexpr_named_tail() {
        let req = SchemaRequirement {
            required: vec![("id".to_string(), DataTypeReq::Concrete(DataType::BigInt))],
            tail: RowTail::Named("r".to_string()),
        };
        let hover = format_smelt_type_hover(&SmeltType::TableExpr(Some(req)));
        assert!(hover.contains("..r"), "expected ..r in: {hover}");
    }

    #[test]
    fn lsp_hover_expr_numeric() {
        let hover = format_smelt_type_hover(&SmeltType::Expr(TypeConstraint::Numeric));
        assert_eq!(hover, "Expr<Numeric>");
    }

    #[test]
    fn lsp_hover_expr_concrete() {
        let hover = format_smelt_type_hover(&SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Integer,
        )));
        assert_eq!(hover, "Expr<INTEGER>");
    }

    // === Phase 27 TDD tests — bidirectional generics (§16 #14, Decision 14) ===

    #[test]
    fn coalesce_expected_double_literals_widen() {
        // Decision 14: when context expects Double and the call has Integer
        // args, the expected return type is an additional position for `T`
        // under the Numeric chain.  LUB({Integer, Integer, Double}) = Double.
        let sig = Signature::new(
            "numeric_coalesce",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![variadic(var("T"))],
            TypeExpr::Var("T".into()),
        );
        let res = unify_call_with_expected(
            &sig,
            &[DataType::Integer, DataType::Integer],
            Some(&DataType::Double),
            &numeric_lub,
        )
        .expect("unification ok");
        assert_eq!(res.return_type, DataType::Double);
    }

    #[test]
    fn no_expected_return_positions_unchanged() {
        // Without an expected return, LUB({Integer, Integer}) = Integer.
        let sig = Signature::new(
            "numeric_coalesce",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![variadic(var("T"))],
            TypeExpr::Var("T".into()),
        );
        let res = unify_call_with_expected(
            &sig,
            &[DataType::Integer, DataType::Integer],
            None,
            &numeric_lub,
        )
        .expect("unification ok");
        assert_eq!(res.return_type, DataType::Integer);
    }

    #[test]
    fn expected_return_conflict_local_error() {
        // MIN<T: Ordered>(T) → T with arg=BigInt; expected return=Integer
        // conflicts (Ordered uses exact equality, not LUB).
        // The error must cite both positions: argument position 1 AND return context (0).
        let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
        let err = unify_call_with_expected(
            sig,
            &[DataType::BigInt],
            Some(&DataType::Integer),
            &numeric_lub,
        )
        .expect_err("BigInt arg vs Integer expected-return must conflict");
        match err {
            UnificationError::InconsistentBinding {
                var_name,
                positions,
                types,
            } => {
                assert_eq!(var_name, "T");
                // Position 0 = return context; position 1 = first argument.
                assert!(
                    positions.contains(&0),
                    "return context (pos 0) must be cited, got {positions:?}"
                );
                assert!(
                    positions.contains(&1),
                    "argument position 1 must be cited, got {positions:?}"
                );
                assert!(types.contains(&DataType::BigInt));
                assert!(types.contains(&DataType::Integer));
            }
            other => panic!("expected InconsistentBinding, got {other:?}"),
        }
    }

    #[test]
    fn generics_within_tier2_body() {
        // MIN<T: Ordered>(T) → T with Decimal arg and no expected return
        // must preserve the Decimal type.
        let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
        let dt = DataType::Decimal {
            precision: 18,
            scale: 6,
        };
        let res = unify_call_with_expected(sig, std::slice::from_ref(&dt), None, &numeric_lub)
            .expect("unification ok");
        assert_eq!(res.return_type, dt);
    }

    // === Phase A (meta-language) TDD tests: SmeltType::List ===

    /// `List<Expr<Integer>>` round-trips through `parse_smelt_type` and
    /// `format_smelt_type_hover`.
    #[test]
    fn list_type_round_trip() {
        let ty = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Integer,
        ))));
        // format_smelt_type_hover produces "List<Expr<Integer>>"
        let rendered = format_smelt_type_hover(&ty);
        assert_eq!(rendered, "List<Expr<INTEGER>>");
        // parse_smelt_type parses it back.
        let parsed = parse_smelt_type(&rendered).expect("List<Expr<Integer>> should parse");
        assert_eq!(parsed, ty);
    }

    /// `List<List<Expr<Varchar>>>` round-trips.
    ///
    /// Note: `DataType::Text` renders as `"TEXT"` via `to_sql()` but `parse_type("TEXT")`
    /// returns `Varchar { max_length: None }`, so we use `Varchar` directly for a clean
    /// round-trip. The types.md annotation surface uses `Varchar` / `TEXT` interchangeably,
    /// and `DataType::Text` normalises to `Varchar`.
    #[test]
    fn list_type_nested() {
        let inner = SmeltType::Expr(TypeConstraint::Concrete(DataType::Varchar {
            max_length: None,
        }));
        let middle = SmeltType::List(Box::new(inner));
        let outer = SmeltType::List(Box::new(middle));
        let rendered = format_smelt_type_hover(&outer);
        assert_eq!(rendered, "List<List<Expr<VARCHAR>>>");
        let parsed = parse_smelt_type(&rendered).expect("List<List<Expr<Varchar>>> should parse");
        assert_eq!(parsed, outer);
    }

    /// Covariance: `List<Expr<Integer>> <: List<Expr<Numeric>>` (Integer satisfies Numeric).
    /// Anti-covariance: `List<Expr<Numeric>> <: List<Expr<Integer>>` is false.
    #[test]
    fn list_subtype_covariant() {
        let list_int = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Integer,
        ))));
        let list_numeric = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Numeric)));

        // List<Expr<Integer>> <: List<Expr<Numeric>> — Integer satisfies Numeric.
        assert!(
            is_subtype_of(&list_int, &list_numeric),
            "List<Expr<Integer>> must be a subtype of List<Expr<Numeric>>"
        );
        // List<Expr<Numeric>> is NOT <: List<Expr<Integer>>.
        assert!(
            !is_subtype_of(&list_numeric, &list_int),
            "List<Expr<Numeric>> must NOT be a subtype of List<Expr<Integer>>"
        );
    }

    /// Unrelated element sorts: `List<TableExpr>` is not a subtype of `List<Expr<Numeric>>`.
    #[test]
    fn list_subtype_invariant_when_element_unrelated() {
        let list_table = SmeltType::List(Box::new(SmeltType::TableExpr(None)));
        let list_numeric = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Numeric)));

        assert!(
            !is_subtype_of(&list_table, &list_numeric),
            "List<TableExpr> must NOT be a subtype of List<Expr<Numeric>>"
        );
        assert!(
            !is_subtype_of(&list_numeric, &list_table),
            "List<Expr<Numeric>> must NOT be a subtype of List<TableExpr>"
        );
    }

    // === Phase B (meta-language) TDD tests: SmeltType::Lambda ===

    /// `Lambda<Expr<Integer>, Expr<Text>>` round-trips through
    /// `format_smelt_type_hover` and `parse_smelt_type`.
    ///
    /// Note: `DataType::Text` renders as `"TEXT"` via `to_sql()` but
    /// `parse_type("TEXT")` returns `Varchar { max_length: None }`. We use
    /// `Varchar` directly for a clean round-trip, consistent with `list_type_nested`.
    #[test]
    fn lambda_type_round_trip() {
        let ty = SmeltType::Lambda(
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Varchar { max_length: None },
            ))),
        );
        let rendered = format_smelt_type_hover(&ty);
        assert_eq!(rendered, "Lambda<Expr<INTEGER>, Expr<VARCHAR>>");
        let parsed =
            parse_smelt_type(&rendered).expect("Lambda<Expr<INTEGER>, Expr<VARCHAR>> should parse");
        assert_eq!(parsed, ty);
    }

    /// Lambda is invariant: `Lambda<Expr<Integer>, Expr<Text>>` is NOT a subtype of
    /// `Lambda<Expr<Numeric>, Expr<Text>>` even though `Expr<Integer> <: Expr<Numeric>`.
    #[test]
    fn lambda_type_invariant() {
        let lambda_int_text = SmeltType::Lambda(
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        let lambda_numeric_text = SmeltType::Lambda(
            Box::new(SmeltType::Expr(TypeConstraint::Numeric)),
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        // Lambda is invariant — Integer does NOT widen to Numeric for subtyping.
        assert!(
            !is_subtype_of(&lambda_int_text, &lambda_numeric_text),
            "Lambda<Expr<Integer>, Expr<Text>> must NOT be a subtype of Lambda<Expr<Numeric>, Expr<Text>> (invariant)"
        );
        assert!(
            !is_subtype_of(&lambda_numeric_text, &lambda_int_text),
            "Lambda<Expr<Numeric>, Expr<Text>> must NOT be a subtype of Lambda<Expr<Integer>, Expr<Text>> (invariant)"
        );
    }

    /// `is_subtype_of(L, L) == true` only for byte-equal `L` (reflexivity).
    #[test]
    fn lambda_type_equality_only_when_exact() {
        let lambda = SmeltType::Lambda(
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        assert!(
            is_subtype_of(&lambda, &lambda),
            "Lambda must be a subtype of itself (reflexivity)"
        );
        let lambda2 = SmeltType::Lambda(
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))),
        );
        assert!(
            !is_subtype_of(&lambda, &lambda2),
            "Lambda with different body type must NOT be a subtype"
        );
    }

    // === Phase C (meta-language) TDD tests — ColumnRef witness + smelt.columns_of ===

    #[test]
    fn columns_of_signature_returns_list_of_column_ref() {
        // BuiltinRegistry::lookup("smelt.columns_of") must return a SmeltMetaSignature
        // with one positional TableExpr parameter and List<ColumnRef> return.
        let sig = BuiltinRegistry::lookup("smelt.columns_of")
            .expect("smelt.columns_of must be in the smelt meta registry");
        assert_eq!(
            sig.params.len(),
            1,
            "smelt.columns_of takes exactly one param"
        );
        assert!(
            matches!(&sig.params[0], SmeltType::TableExpr(None)),
            "smelt.columns_of param must be TableExpr, got: {:?}",
            sig.params[0]
        );
        assert!(
            matches!(&sig.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
            "smelt.columns_of must return List<ColumnRef>, got: {:?}",
            sig.return_type
        );
    }

    #[test]
    fn column_ref_field_set_is_closed() {
        // COLUMN_REF_FIELDS must expose exactly {name: Text, type: DataType, is_numeric: Boolean}
        // and nothing else.
        let expected = ["name", "type", "is_numeric"];
        for field in &expected {
            assert!(
                column_ref_field(field).is_some(),
                "COLUMN_REF_FIELDS must contain field '{field}'"
            );
        }
        // Any other identifier must return None.
        assert!(
            column_ref_field("foo").is_none(),
            "COLUMN_REF_FIELDS must not contain 'foo'"
        );
        assert!(
            column_ref_field("column_name").is_none(),
            "COLUMN_REF_FIELDS must not contain 'column_name'"
        );
        // Exactly three fields in the constant.
        assert_eq!(
            COLUMN_REF_FIELDS.len(),
            3,
            "COLUMN_REF_FIELDS must have exactly 3 entries"
        );
        // Verify field types.
        let name_ty = column_ref_field("name").unwrap();
        assert!(
            matches!(
                name_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "name field must be Text (Expr<Text>), got: {name_ty:?}"
        );
        let type_ty = column_ref_field("type").unwrap();
        // c.type maps to SmeltType::Unknown as the Phase C sentinel for "DataType (meta literal)".
        // Phase D will introduce a proper meta-DataType representation; for now Unknown
        // is the documented placeholder per the Phase C plan.
        assert!(
            matches!(type_ty, SmeltType::Unknown),
            "c.type maps to SmeltType::Unknown as the Phase C sentinel for DataType (meta literal); \
             Phase D will introduce a proper meta-DataType representation; got: {:?}",
            type_ty
        );
        let is_numeric_ty = column_ref_field("is_numeric").unwrap();
        assert!(
            matches!(
                is_numeric_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
            ),
            "is_numeric field must be Boolean, got: {is_numeric_ty:?}"
        );
    }
}
