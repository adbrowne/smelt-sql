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

use crate::{parse_type, DataType, DialectId};
use smelt_parser::ast::{File as AstFile, Param as AstParam, SmeltDefine, SmeltExtern, TypeRef};
use smelt_parser::TextRange;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

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
///
/// **PartialEq design note (Phase E1):** `SmeltType` implements `PartialEq`
/// manually rather than via `derive` so that `Record` structural equality
/// ignores the optional `name` metadata field. Two `Record` values with
/// identical `fields` maps and differing `name` values (`Some("X")` vs `None`)
/// compare equal. This matches the spec's structural-equality rule (rule 4):
/// the `name` field is hover/attribution metadata, not a structural type
/// discriminant. All other variants delegate to field-by-field equality as
/// `derive` would produce.
#[derive(Debug, Clone)]
pub enum SmeltType {
    /// `Expr<T>` where T is a [`TypeConstraint`] — either a concrete
    /// [`DataType`] or one of the abstract constraints in
    /// [`TypeConstraint`].
    Expr(TypeConstraint),
    /// `List<T>` — a compile-time meta-list of elements typed `T` (Phase A,
    /// meta-language). `T` is itself a [`SmeltType`] (enabling `List<List<T>>`
    /// nesting). No `List<T>` value reaches the database engine.
    List(Box<SmeltType>),
    /// `Lambda<(S_1, …, S_k), U>` — a meta-language lambda value (Phase B/F).
    ///
    /// Constructed only at HOF positional-argument positions (e.g. the
    /// second argument of `map`, `filter`). **Meta-only** — not user-writable
    /// as a `smelt.define` parameter sort or return type. No `Lambda<…>`
    /// value reaches the database engine.
    ///
    /// Lambda is **invariant**: `is_subtype_of(Lambda<S1…Sk, T>, Lambda<S1'…Sk', T'>)`
    /// is `true` only when `k = k'`, each `S_i = S_i'`, and `T = T'` (byte-equal).
    ///
    /// The `Vec<SmeltType>` carries the parameter types in declaration order.
    /// For a single-parameter lambda (`fn x => body`) the vec has exactly one
    /// element. Zero-element vecs indicate an invalid lambda (`LambdaZeroParameters`).
    Lambda(Vec<SmeltType>, Box<SmeltType>),
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
    /// `smelt.columns_of` (meta-language reflection).
    ///
    /// Values of this type describe a single column from a `TableExpr`'s
    /// schema. The type has eight fields (see [`COLUMN_REF_FIELDS`]):
    ///   - `name: Text` — the column identifier (un-quoted, case-preserved)
    ///   - `type: DataType` (meta literal) — the column's smelt `DataType`
    ///   - `is_numeric: Boolean` — `TRUE` iff `type` is in the `Numeric` constraint set
    ///   - `is_decimal: Boolean` — `TRUE` iff head constructor is `Decimal`
    ///   - `is_string: Boolean` — `TRUE` iff head constructor is Text/Varchar/Char
    ///   - `is_temporal: Boolean` — `TRUE` iff head constructor is Date/Timestamp/Time (not Interval)
    ///   - `is_integer: Boolean` — `TRUE` iff head constructor is SmallInt/Integer/BigInt
    ///   - `is_boolean: Boolean` — `TRUE` iff head constructor is Boolean
    ///
    /// **Meta-only**: not user-writable as a `smelt.define` parameter or
    /// return type. Values originate only from `smelt.columns_of` and
    /// future reflection accessors. No `ColumnRef` value reaches the
    /// database engine.
    ///
    /// **Closed**: adding a field requires a spec edit and a compiler change.
    ColumnRef,
    /// `ModelRef` — a closed meta-only record type produced by wide-reflection
    /// accessors `smelt.models.with_tag` and `smelt.models.all` (Phase D,
    /// meta-language reflection).
    ///
    /// Values of this type describe a single model in the workspace. The type
    /// has exactly four fields (see [`MODEL_REF_FIELDS`]):
    ///   - `path: Text` — workspace-relative file path with `/` separators
    ///   - `name: Text` — the model's identifier (final path segment without `.sql`)
    ///   - `tags: List<Text>` — the model's merged tag set
    ///   - `columns: List<ColumnRef>` — the model's column list
    ///
    /// **Meta-only**: not user-writable as a `smelt.define` parameter or
    /// return type. Values originate only from `smelt.models.*` accessors.
    /// No `ModelRef` value reaches the database engine.
    ///
    /// **Closed**: the v1 field set is exactly `{path, name, tags, columns}`.
    ModelRef,
    /// `SourceRef` — a closed meta-only record type produced by wide-reflection
    /// accessors `smelt.sources.with_tag` and `smelt.sources.all` (Phase D,
    /// meta-language reflection).
    ///
    /// Values of this type describe a single source in the workspace. The type
    /// has exactly four fields (see [`SOURCE_REF_FIELDS`]):
    ///   - `path: Text` — workspace-relative file path of the source YAML
    ///   - `name: Text` — the source's identifier (final path segment without `.yml`)
    ///   - `tags: List<Text>` — the source's tag set as declared in the YAML
    ///   - `columns: List<ColumnRef>` — the source's column list
    ///
    /// **Meta-only**: not user-writable as a `smelt.define` parameter or
    /// return type. Values originate only from `smelt.sources.*` accessors.
    /// No `SourceRef` value reaches the database engine.
    ///
    /// **Closed**: the v1 field set is exactly `{path, name, tags, columns}`.
    SourceRef,
    /// `ModelDef` — the built-in closed user-constructible meta record type
    /// for multi-model generator files.
    ///
    /// Unlike `ColumnRef`, `ModelRef`, and `SourceRef` (which originate from
    /// reflection), `ModelDef` values are constructed via record literals inside
    /// a `generates: models` generator file body.
    ///
    /// The type has exactly five fields (see [`MODEL_DEF_FIELDS`]):
    ///   - `name: Text` — model identifier (`[A-Za-z0-9_]+`, non-empty)
    ///   - `body: TableExpr` — the model's SQL body (the single carve-out
    ///     admitting `TableExpr` in a record-like field position)
    ///   - `materialization: Text` — one of `view`, `table`, `incremental`
    ///   - `tags: List<Text>` — tag set (merges with workspace-level overlays)
    ///   - `description: Text` — human-readable description
    ///
    /// **Meta-only**: values never reach the database engine.
    ///
    /// **Closed**: the v1 field set is exactly `{name, body, materialization, tags, description}`.
    /// Adding a field requires a spec edit and a compiler change.
    ///
    /// **Not assignable to `Record`**: `ModelDef` is the only user-constructible
    /// closed meta record type. It is structurally distinguishable from any
    /// user-declared `smelt.record` type, even one with identical fields.
    ModelDef,
    /// `Record<{f1: T1, …}>` or a named record type `TypeName` (Phase E1,
    /// meta-language records).
    ///
    /// A user-writable meta-only record type introduced by `smelt.record`
    /// declarations (named) or inline brace notation at type-annotation
    /// positions (anonymous). No `Record` value reaches the database engine.
    ///
    /// **Structural equality ignores `name`:** two `Record` values with the
    /// same `fields` map compare equal regardless of their `name` metadata.
    /// This satisfies spec rule 4: the type system treats `{a: Text, b: Integer}`
    /// and a named `SourceEntry = {a: Text, b: Integer}` identically for
    /// subtyping and assignability. The `name` field is attribution-only
    /// (hover strings, goto-definition).
    ///
    /// **Width subtyping:** `{a: T, b: U} <: {a: T}` — a record with more
    /// fields is a subtype of a record with fewer fields, provided the shared
    /// fields have assignable types. See [`is_subtype_of`].
    Record {
        /// Canonical sorted field map. `BTreeMap` ensures iteration order is
        /// deterministic (lex on field name), so structural equality is
        /// insensitive to insertion order.
        fields: BTreeMap<String, SmeltType>,
        /// Optional name metadata from a `smelt.record TypeName = {…}`
        /// declaration. `None` for inline record types. This field is
        /// **excluded from `PartialEq`**; see the `SmeltType` doc comment.
        name: Option<String>,
    },
    /// `Map<K, V>` — a meta-only key-value collection (Phase E1).
    ///
    /// In v1 `K` is constrained to `Text`; a `Map<K, V>` with `K != Text`
    /// emits `MapKeyTypeNotText`. `Map<K, V>` is **invariant** in both axes:
    /// `Map<Text, Integer>` is not assignable to `Map<Text, Number>` even
    /// though `Integer <: Number`. No `Map` value reaches the database engine.
    Map {
        /// Key type (constrained to `Text` in v1).
        key: Box<SmeltType>,
        /// Value type (any meta-language type).
        value: Box<SmeltType>,
    },
}

/// Manual `PartialEq` for `SmeltType` — `Record` equality ignores the `name`
/// metadata field (structural equality rule, Phase E1 spec §4).
///
/// All other variants delegate to field-by-field equality as `derive` would.
impl PartialEq for SmeltType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SmeltType::Expr(a), SmeltType::Expr(b)) => a == b,
            (SmeltType::List(a), SmeltType::List(b)) => a == b,
            (SmeltType::Lambda(a_params, a_ret), SmeltType::Lambda(b_params, b_ret)) => {
                a_params == b_params && a_ret == b_ret
            }
            (SmeltType::Unknown, SmeltType::Unknown) => true,
            (SmeltType::TableExpr(a), SmeltType::TableExpr(b)) => a == b,
            (
                SmeltType::SelectItems {
                    kind: ka,
                    context: ca,
                },
                SmeltType::SelectItems {
                    kind: kb,
                    context: cb,
                },
            ) => ka == kb && ca == cb,
            (
                SmeltType::Struct {
                    fields: fa,
                    tail: ta,
                },
                SmeltType::Struct {
                    fields: fb,
                    tail: tb,
                },
            ) => fa == fb && ta == tb,
            (SmeltType::ColumnRef, SmeltType::ColumnRef) => true,
            (SmeltType::ModelRef, SmeltType::ModelRef) => true,
            (SmeltType::SourceRef, SmeltType::SourceRef) => true,
            (SmeltType::ModelDef, SmeltType::ModelDef) => true,
            // Record structural equality: `name` is deliberately excluded.
            // Two records with the same `fields` map compare equal regardless of
            // their `name` metadata (spec rule 4, Phase E1).
            (SmeltType::Record { fields: fa, .. }, SmeltType::Record { fields: fb, .. }) => {
                fa == fb
            }
            // Map equality: both `key` and `value` must be equal (invariance —
            // equality already implies invariance since there's no widening).
            (SmeltType::Map { key: ka, value: va }, SmeltType::Map { key: kb, value: vb }) => {
                ka == kb && va == vb
            }
            // Cross-variant: never equal.
            _ => false,
        }
    }
}

impl Eq for SmeltType {}

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
        // Lambda invariance: Lambda<S_1…S_k, T> <: Lambda<S_1'…S_k', T'> only when
        // k = k', each S_i = S_i', and T = T'.
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
        // Fragment-sort subtyping: ModelRef <: TableExpr and SourceRef <: TableExpr.
        //
        // A `ModelRef` or `SourceRef` value's `TableExpr` projection is the same
        // `TableExpr` that `smelt.<path>` resolves to for that model/source. The
        // subtyping rule is one-way: `ModelRef` and `SourceRef` lift to `TableExpr`
        // wherever a `TableExpr` is required (reducer-`union_all` arguments,
        // `smelt.columns_of` arguments, FROM-clause splice positions).
        //
        // The reverse direction (`TableExpr → ModelRef`) does not exist: only values
        // originating from `smelt.models.*` / `smelt.sources.*` are `ModelRef`/`SourceRef`-typed.
        //
        // List covariance (above) automatically lifts `List<ModelRef>` → `List<TableExpr>`
        // once this element rule is in place.
        (SmeltType::ModelRef, SmeltType::TableExpr(_)) => true,
        (SmeltType::SourceRef, SmeltType::TableExpr(_)) => true,
        // Record width subtyping (Phase E1, spec rule 8):
        // `sub <: sup` iff every field in `sup` exists in `sub` with an assignable type.
        // A record with MORE fields is the subtype (it satisfies every requirement of the
        // less-specific supertype). The `name` field is not consulted — subtyping is
        // purely structural over the `fields` map.
        (
            SmeltType::Record {
                fields: sub_fields, ..
            },
            SmeltType::Record {
                fields: sup_fields, ..
            },
        ) => sup_fields.iter().all(|(name, sup_ty)| {
            sub_fields
                .get(name)
                .is_some_and(|sub_ty| is_subtype_of(sub_ty, sup_ty))
        }),
        // Map invariance (Phase E1, spec §"Map invariants"):
        // `Map<K1, V1> <: Map<K2, V2>` iff `K1 == K2` AND `V1 == V2`.
        // The reflexivity arm above (`a == b`) already handles equal maps; any
        // unequal-by-PartialEq Map pair falls here and returns false, enforcing invariance.
        (SmeltType::Map { .. }, SmeltType::Map { .. }) => false,
        // All other cross-sort combinations are not subtypes.
        _ => false,
    }
}

/// `Display` for `SmeltType` (Phase E1).
///
/// - `Record { name: Some("TypeName"), .. }` renders as `<TypeName>`.
/// - `Record { name: None, fields }` renders as `Record<{f1: T1, f2: T2}>` with
///   fields in lex order (BTreeMap iteration order, which equals lex order).
/// - `Map { key, value }` renders as `Map<K, V>`.
/// - Other variants render as their type-annotation text.
impl std::fmt::Display for SmeltType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmeltType::Expr(tc) => match tc {
                TypeConstraint::Concrete(dt) => write!(f, "Expr<{dt}>"),
                TypeConstraint::Numeric => write!(f, "Expr<Numeric>"),
                TypeConstraint::Ordered => write!(f, "Expr<Ordered>"),
                TypeConstraint::Any => write!(f, "Expr<Any>"),
            },
            SmeltType::List(inner) => write!(f, "List<{inner}>"),
            SmeltType::Lambda(params, ret) => {
                if params.len() == 1 {
                    write!(f, "Lambda<{}, {}>", params[0], ret)
                } else {
                    let params_str = params
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, "Lambda<({params_str}), {ret}>")
                }
            }
            SmeltType::Unknown => write!(f, "Unknown"),
            SmeltType::TableExpr(_) => write!(f, "TableExpr"),
            SmeltType::SelectItems { kind, .. } => match kind {
                ExprKind::Scalar => write!(f, "SelectItems<Scalar>"),
                ExprKind::Agg => write!(f, "SelectItems<Agg>"),
                ExprKind::Window => write!(f, "SelectItems<Window>"),
            },
            SmeltType::Struct { .. } => write!(f, "Struct<{{…}}>"),
            SmeltType::ColumnRef => write!(f, "ColumnRef"),
            SmeltType::ModelRef => write!(f, "ModelRef"),
            SmeltType::SourceRef => write!(f, "SourceRef"),
            SmeltType::ModelDef => write!(f, "ModelDef"),
            SmeltType::Record { fields, name } => {
                if let Some(n) = name {
                    // Named declaration: display as the type name.
                    write!(f, "{n}")
                } else {
                    // Inline: display structural form in lex order.
                    let field_str: Vec<String> =
                        fields.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    write!(f, "Record<{{{}}}>", field_str.join(", "))
                }
            }
            SmeltType::Map { key, value } => write!(f, "Map<{key}, {value}>"),
        }
    }
}

// ============================================================================
// Map API registry (Phase E1)
// ============================================================================

/// Arity descriptor for Map API methods and future built-in registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arity {
    /// Exactly `n` positional arguments required (no variadic).
    Exact(usize),
}

/// Discriminates the dispatch behaviour of a Map API method.
///
/// Adding a new method to the registry requires choosing a `kind`, which
/// determines whether key-type validation and static-key resolution are
/// performed at the call site. This makes the registry the sole source of
/// truth: changing a method's kind changes its dispatch behaviour without
/// touching call-site code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapApiMethodKind {
    /// Zero-argument iteration method (`entries`, `keys`, `values`).
    /// No key argument — no key-type check, no static-key resolution.
    ZeroArg,
    /// One-argument lookup that resolves to the value type (`get`).
    /// Validates the key-argument type; resolves statically to the per-entry
    /// type when the key is a string literal and the map contents are known.
    KeyedGet,
    /// One-argument presence check that resolves to `Boolean` (`has`).
    /// Validates the key-argument type; resolves statically to `Bool(true)`
    /// or `Bool(false)` when the key is a string literal and map contents
    /// are known.
    KeyedHas,
}

/// A single entry in the closed Map API method registry.
///
/// The five entries are: `entries`, `keys`, `values`, `get`, `has`.
/// Named arguments are never supported on Map API methods.
pub struct MapApiMethod {
    /// The method name (e.g. `"entries"`).
    pub name: &'static str,
    /// Required positional argument count.
    pub arity: Arity,
    /// Dispatch kind — controls key-arg validation and static-resolution behaviour.
    pub kind: MapApiMethodKind,
    /// Whether named arguments are accepted (always `false` in v1).
    pub named_args_allowed: bool,
    /// Return type formula. Takes the receiver's `K` and `V` types and
    /// returns the synthesised result type. The formula uses owned values
    /// so the returned `SmeltType` is self-contained.
    pub return_type_formula: fn(&SmeltType, &SmeltType) -> SmeltType,
}

impl std::fmt::Debug for MapApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MapApiMethod {{ name: {:?}, arity: {:?}, kind: {:?} }}",
            self.name, self.arity, self.kind
        )
    }
}

/// Build the `List<{key: K, value: V}>` return type for `m.entries()`.
fn map_entries_return(k: &SmeltType, v: &SmeltType) -> SmeltType {
    let mut fields = BTreeMap::new();
    fields.insert("key".to_string(), k.clone());
    fields.insert("value".to_string(), v.clone());
    SmeltType::List(Box::new(SmeltType::Record { fields, name: None }))
}

/// Build `List<K>` for `m.keys()`.
fn map_keys_return(k: &SmeltType, _v: &SmeltType) -> SmeltType {
    SmeltType::List(Box::new(k.clone()))
}

/// Build `List<V>` for `m.values()`.
fn map_values_return(_k: &SmeltType, v: &SmeltType) -> SmeltType {
    SmeltType::List(Box::new(v.clone()))
}

/// Build `V` for `m.get(k)`.
fn map_get_return(_k: &SmeltType, v: &SmeltType) -> SmeltType {
    v.clone()
}

/// Build `Boolean` for `m.has(k)`.
fn map_has_return(_k: &SmeltType, _v: &SmeltType) -> SmeltType {
    SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Boolean))
}

/// Closed Map API method registry (Phase E1).
///
/// The five entries are the entire Map surface in v1.
/// `entries`, `keys`, `values` — arity 0 (no arguments).
/// `get`, `has` — arity 1 (one positional key argument).
///
/// Named arguments are not permitted on any Map API method.
pub static MAP_API_METHODS: &[MapApiMethod] = &[
    MapApiMethod {
        name: "entries",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_entries_return,
    },
    MapApiMethod {
        name: "keys",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_keys_return,
    },
    MapApiMethod {
        name: "values",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_values_return,
    },
    MapApiMethod {
        name: "get",
        arity: Arity::Exact(1),
        kind: MapApiMethodKind::KeyedGet,
        named_args_allowed: false,
        return_type_formula: map_get_return,
    },
    MapApiMethod {
        name: "has",
        arity: Arity::Exact(1),
        kind: MapApiMethodKind::KeyedHas,
        named_args_allowed: false,
        return_type_formula: map_has_return,
    },
];

/// Look up a Map API method by name. Returns `None` for any name outside
/// the closed set `{entries, keys, values, get, has}`.
pub fn lookup_map_api_method(name: &str) -> Option<&'static MapApiMethod> {
    MAP_API_METHODS.iter().find(|m| m.name == name)
}

// ============================================================================
// Record registry (Phase E1)
// ============================================================================

/// Diagnostic code for the record registry builder.
///
/// This is a local enum living in `smelt-types` so that `signatures.rs` and
/// `build_record_registry` can produce typed sentinels without depending on
/// `smelt-db::DiagnosticCode` (which would create a circular crate dependency).
/// The wiring layer in `smelt-db` translates these into `DiagnosticCode` values
/// for the LSP accumulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordRegistryCode {
    /// A second `smelt.record` declaration in the workspace shares an existing
    /// record's name. First-declaration-wins; the sentinel anchors at the
    /// second declaration's `name_span`.
    SmeltRecordRedefinition,
    /// A field's declared type contains a meta-only witness type that is not
    /// user-writable (`ColumnRef`, `ModelRef`, `SourceRef`, `Lambda`). Anchored
    /// at the offending field's type span.
    RecordFieldTypeForbidden,
    /// A record declaration references its own name directly or transitively
    /// through other record declarations, forming a cycle. v1 records must
    /// form a DAG. Anchored at the cycle-introducing field-type span.
    RecordCyclicDeclaration,
}

/// A diagnostic produced by the record registry builder.
///
/// Carries the typed [`RecordRegistryCode`] (for pattern-matching), the source
/// span (for diagnostic anchoring), and a pre-rendered message string.
#[derive(Debug, Clone)]
pub struct DiagnosticSentinel {
    /// The registry-layer diagnostic code.
    pub code: RecordRegistryCode,
    /// Source span of the offending token (e.g. the second declaration's name
    /// or the forbidden field-type expression). May be a zero-length span when
    /// the syntactic position was not tracked.
    pub span: smelt_parser::TextRange,
    /// Pre-rendered diagnostic message per the spec's message format.
    pub message: String,
}

/// A single `smelt.record` declaration parsed from source.
///
/// Phase E1: this struct carries the declaration's name, field list (with
/// per-field type and source span), the name-token span (for
/// `SmeltRecordRedefinition` anchoring), and the source-file path (included in
/// the redefinition message).
///
/// Pure — no Salsa dependency. Produced by the Phase 2 parser and consumed by
/// `build_record_registry`.
#[derive(Debug, Clone, PartialEq)]
pub struct SmeltRecordDeclaration {
    /// The declared record name (e.g. `"SourceEntry"`).
    pub name: String,
    /// Ordered field list: `(field_name, field_type, type_span)`.
    /// `type_span` anchors `RecordFieldTypeForbidden` and
    /// `RecordCyclicDeclaration` sentinels.
    pub fields: Vec<(String, SmeltType, smelt_parser::TextRange)>,
    /// Span of the name token in `smelt.record TypeName = {…}`.
    /// Used to anchor `SmeltRecordRedefinition` at the second declaration's
    /// name token.
    pub name_span: smelt_parser::TextRange,
    /// Workspace-relative source file path for the first-declaration message.
    pub source_path: Arc<str>,
}

/// Map from declared record name to its declaration. The authoritative
/// declaration for each name (first-wins on redefinition).
///
/// Phase E1: built by `build_record_registry` and passed into the inference
/// layer (`TypeContext`) in Phase 3/5.
#[derive(Debug)]
pub struct RecordRegistry {
    inner: HashMap<String, SmeltRecordDeclaration>,
}

impl RecordRegistry {
    /// Look up a record declaration by name.
    pub fn lookup(&self, name: &str) -> Option<&SmeltRecordDeclaration> {
        self.inner.get(name)
    }

    /// All declared record names (in unspecified order).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|s| s.as_str())
    }

    /// Create an empty registry (no declarations). Used as the default for
    /// pre-Phase-5 callers that have not wired the Salsa side yet.
    pub fn empty() -> Self {
        RecordRegistry {
            inner: HashMap::new(),
        }
    }
}

impl Default for RecordRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Returns `true` if the given `SmeltType` directly or transitively references
/// any of the record names in `declared_names`. Used by cycle detection to
/// identify field types that create edges in the record DAG.
fn field_type_references_record(ty: &SmeltType, declared_names: &HashSet<String>) -> Vec<String> {
    let mut refs = Vec::new();
    collect_record_references(ty, declared_names, &mut refs);
    refs
}

fn collect_record_references(
    ty: &SmeltType,
    declared_names: &HashSet<String>,
    out: &mut Vec<String>,
) {
    match ty {
        SmeltType::Record { name: Some(n), .. } if declared_names.contains(n) => {
            out.push(n.clone());
        }
        SmeltType::Record { fields, .. } => {
            for v in fields.values() {
                collect_record_references(v, declared_names, out);
            }
        }
        SmeltType::List(inner) => collect_record_references(inner, declared_names, out),
        SmeltType::Map { key, value } => {
            collect_record_references(key, declared_names, out);
            collect_record_references(value, declared_names, out);
        }
        _ => {}
    }
}

/// Build the workspace record registry from a list of parsed declarations.
///
/// **Algorithm:**
/// 1. Walk declarations in order. For each name:
///    - If already seen: emit `SmeltRecordRedefinition` at the second
///      declaration's `name_span`; skip the duplicate.
///    - Otherwise: record as authoritative.
/// 2. Validate each authoritative declaration's field types:
///    - Any field type containing `ColumnRef`, `ModelRef`, `SourceRef`, or
///      `Lambda` emits `RecordFieldTypeForbidden` at the field's type span.
/// 3. Cycle detection via DFS over the graph where nodes are declared record
///    names and edges are "field type references another declared record name":
///    - Any name reachable from itself (directly or via a chain) emits
///      `RecordCyclicDeclaration` at the introducing edge's field-type span.
///    - DFS is over the *directed* graph; back-edges (Gray → Gray in the DFS
///      coloring) detect cycles. We emit **one sentinel per cyclic target name**:
///      `cycle_emitted` is keyed on the back-edge's target, so the first
///      back-edge to a given record fires the sentinel and any subsequent
///      back-edges into the same target are suppressed. This means a single
///      record participating in several overlapping cycles (e.g. `A↔B` and
///      `A↔B↔C`) yields one sentinel per cyclic record rather than one per
///      distinct cycle path — sufficient to mark the offending records as
///      cyclic without flooding the user with overlapping reports.
///
/// **Returns:** `(RecordRegistry, Vec<DiagnosticSentinel>)`.
/// The registry contains only authoritative (first-wins) declarations.
/// The sentinel list carries redefinition, forbidden-type, and cycle errors.
///
/// Pure — no Salsa, no I/O.
pub fn build_record_registry(
    decls: &[SmeltRecordDeclaration],
) -> (RecordRegistry, Vec<DiagnosticSentinel>) {
    let mut sentinels: Vec<DiagnosticSentinel> = Vec::new();
    let mut registry_map: HashMap<String, SmeltRecordDeclaration> = HashMap::new();

    // Step 1: collect authoritative declarations (first-wins on redefinition).
    for decl in decls {
        if let Some(existing) = registry_map.get(&decl.name) {
            // Redefinition: emit sentinel anchored at the second declaration's name_span.
            sentinels.push(DiagnosticSentinel {
                code: RecordRegistryCode::SmeltRecordRedefinition,
                span: decl.name_span,
                message: format!(
                    "record `{}` is already declared in {}; record names must be unique workspace-wide",
                    decl.name,
                    existing.source_path,
                ),
            });
        } else {
            registry_map.insert(decl.name.clone(), decl.clone());
        }
    }

    // Step 2: validate field types for forbidden witnesses.
    for decl in registry_map.values() {
        for (_, field_ty, type_span) in &decl.fields {
            // Check if the field type itself is forbidden (not just recursively).
            // We check the immediate type and its components.
            if let Some(forbidden_name) = find_forbidden_type_name(field_ty) {
                sentinels.push(DiagnosticSentinel {
                    code: RecordRegistryCode::RecordFieldTypeForbidden,
                    span: *type_span,
                    message: format!(
                        "record field types may not reference {forbidden_name}; reflection witnesses are not user-writable"
                    ),
                });
            }
        }
    }

    // Step 3: cycle detection via DFS.
    // Build the adjacency graph: for each declared name, the set of other
    // declared names directly or transitively referenced in its field types.
    let declared_names: HashSet<String> = registry_map.keys().cloned().collect();

    // DFS cycle detection using iterative approach with explicit color tracking.
    // Nodes are record names (String). Colors: White=unvisited, Gray=in-stack, Black=done.
    //
    // We use String keys throughout to avoid lifetime complexity.
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<String, Color> = HashMap::new();
    let mut cycle_emitted: HashSet<String> = HashSet::new();

    // Iterate in deterministic (sorted) order.
    let mut sorted_names: Vec<String> = declared_names.iter().cloned().collect();
    sorted_names.sort();

    // Iterative DFS using an explicit call stack to avoid Rust recursive fn lifetime issues.
    // Each stack frame: (node_name, edge_list_index, edge_targets).
    // We collect edges lazily per frame.
    for start in sorted_names {
        if color.get(&start).copied().unwrap_or(Color::White) != Color::White {
            continue;
        }

        // DFS stack: each entry is (node, edge_list, current_edge_index).
        type DfsEdge = (String, smelt_parser::TextRange);
        type DfsFrame = (String, Vec<DfsEdge>, usize);
        let mut dfs_stack: Vec<DfsFrame> = Vec::new();

        color.insert(start.clone(), Color::Gray);

        // Build edges for start node.
        let start_edges = {
            let mut edges: Vec<(String, smelt_parser::TextRange)> = Vec::new();
            if let Some(decl) = registry_map.get(&start) {
                for (_, field_ty, span) in &decl.fields {
                    let refs = field_type_references_record(field_ty, &declared_names);
                    for r in refs {
                        edges.push((r, *span));
                    }
                }
            }
            edges.sort_by(|a, b| a.0.cmp(&b.0));
            edges
        };
        dfs_stack.push((start, start_edges, 0));

        'dfs: while let Some(frame) = dfs_stack.last_mut() {
            let (node, edges, idx) = frame;
            if *idx >= edges.len() {
                // All edges processed — mark Black.
                let node_done = node.clone();
                dfs_stack.pop();
                color.insert(node_done, Color::Black);
                continue 'dfs;
            }

            let (target, span) = edges[*idx].clone();
            *idx += 1;

            let target_color = color.get(&target).copied().unwrap_or(Color::White);
            match target_color {
                Color::White => {
                    // Push new frame.
                    color.insert(target.clone(), Color::Gray);
                    let target_edges = {
                        let mut edges: Vec<(String, smelt_parser::TextRange)> = Vec::new();
                        if let Some(decl) = registry_map.get(&target) {
                            for (_, field_ty, fspan) in &decl.fields {
                                let refs = field_type_references_record(field_ty, &declared_names);
                                for r in refs {
                                    edges.push((r, *fspan));
                                }
                            }
                        }
                        edges.sort_by(|a, b| a.0.cmp(&b.0));
                        edges
                    };
                    dfs_stack.push((target, target_edges, 0));
                }
                Color::Gray => {
                    // Back-edge → cycle detected.
                    if !cycle_emitted.contains(&target) {
                        cycle_emitted.insert(target.clone());
                        sentinels.push(DiagnosticSentinel {
                            code: RecordRegistryCode::RecordCyclicDeclaration,
                            span,
                            message: format!(
                                "record `{target}` forms a cycle; recursive record declarations are not supported in v1"
                            ),
                        });
                    }
                }
                Color::Black => {}
            }
        }
    }

    (
        RecordRegistry {
            inner: registry_map,
        },
        sentinels,
    )
}

/// Find the name of the first forbidden type in `ty`, if any.
/// Returns the type name (`"ColumnRef"`, `"ModelRef"`, `"SourceRef"`, `"Lambda"`)
/// or `None` if no forbidden type is present.
fn find_forbidden_type_name(ty: &SmeltType) -> Option<String> {
    match ty {
        SmeltType::ColumnRef => Some("ColumnRef".to_string()),
        SmeltType::ModelRef => Some("ModelRef".to_string()),
        SmeltType::SourceRef => Some("SourceRef".to_string()),
        SmeltType::Lambda(params, _) => {
            // Also check parameter types for forbidden type references.
            params
                .iter()
                .find_map(find_forbidden_type_name)
                .or(Some("Lambda".to_string()))
        }
        SmeltType::List(inner) => find_forbidden_type_name(inner),
        SmeltType::Record { fields, .. } => fields.values().find_map(find_forbidden_type_name),
        SmeltType::Map { key, value } => {
            find_forbidden_type_name(key).or_else(|| find_forbidden_type_name(value))
        }
        _ => None,
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
        if matches!(actual, DataType::Unknown(_) | DataType::Null) {
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
/// `required` is the ordered list of `(column_name, requirement, not_null)`
/// triples declared in the signature. `tail` is the trailing marker
/// decision: no tail, anonymous tail, or a named row variable.
///
/// The check at the call site is performed by
/// [`check_schema_requirement`] — a pure function that takes the
/// requirement, the caller's schema, and returns either a
/// [`RowVarBinding`] (the extras captured by a named tail, empty for
/// `None` / `Anon`) or a [`SchemaMismatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRequirement {
    /// The declared `(column_name, requirement, not_null)` triples in source order.
    /// `not_null` is `true` when the field was declared with a `NOT NULL` qualifier
    /// (Phase 5, nullability-soundness).
    pub required: Vec<(String, DataTypeReq, bool)>,
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
    check_schema_requirement_with_nullability(req, arg_schema, &[])
}

/// Extended variant that also checks nullability when the caller provides
/// per-column nullable flags (Phase 5, nullability-soundness).
///
/// `caller_nullability` maps column names to `true` (nullable) /
/// `false` (non-nullable). Columns not present in the map are treated as
/// nullable (conservative). When a required column is declared `NOT NULL`
/// and the caller's column is nullable, a `NullabilityMismatch` variant
/// (rendered as `TypeMismatch` in the diagnostic) is returned.
pub fn check_schema_requirement_with_nullability(
    req: &SchemaRequirement,
    arg_schema: &[(String, DataType)],
    caller_nullability: &[(String, bool)],
) -> Result<Option<RowVarBinding>, SchemaMismatch> {
    // 1. Every required column must be present and type-compatible.
    //    We report the first structural problem in declaration order.
    for (col_name, col_req, col_not_null) in &req.required {
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
        // Phase 5: check nullability when the column is declared NOT NULL.
        if *col_not_null {
            let caller_is_nullable = caller_nullability
                .iter()
                .find(|(n, _)| n == col_name)
                .map(|(_, nullable)| *nullable)
                .unwrap_or(true); // unknown → assume nullable (conservative)
            if caller_is_nullable {
                return Err(SchemaMismatch::TypeMismatch {
                    column: col_name.clone(),
                    required: col_req.clone(),
                    actual: format!("{} (nullable)", actual_dt),
                });
            }
        }
    }

    // 2. Extras handling. Compute the set of required column names
    //    once, then collect every caller column not in that set —
    //    this is the row-variable's extras list (or the
    //    "unexpected" list when `RowTail::None`).
    let required_names: std::collections::HashSet<&str> =
        req.required.iter().map(|(n, _, _)| n.as_str()).collect();
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
            Ok(SmeltType::Lambda(vec![t], Box::new(u)))
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
    pub name_range: Option<TextRange>,
    /// Raw text of the declared type, or `None` if unannotated.
    pub type_ref_text: Option<String>,
    /// Structured parse of `type_ref_text`, or `None` if unannotated.
    /// `Some(Err(...))` when an annotation was written but couldn't be parsed
    /// — these errors are surfaced as diagnostics by higher layers.
    pub type_ref: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the `TypeRef` node for this parameter, suitable for
    /// anchoring diagnostic spans. `None` when the parameter has no
    /// annotation at all.
    pub type_ref_range: Option<TextRange>,
    /// `true` when the parameter has a default value.
    pub has_default: bool,
    /// Pre-resolution context binding from `Expr<T, ctx>` / `AggExpr<T, ctx>`
    /// / `WindowExpr<T, ctx>` annotations (Phase 19). `None` when no context
    /// argument is present.
    pub context: Option<ContextRef>,
    /// `true` when the parameter carries a `NOT NULL` qualifier (Phase 5,
    /// nullability-soundness). A non-nullable parameter requires a non-nullable
    /// argument at the call site and binds the parameter as non-nullable in the
    /// function body. Bare annotations (without `NOT NULL`) are always nullable.
    pub not_null: bool,
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
    pub decl_range: Option<TextRange>,
    /// Range of the call-path span at this frame's call site (in the
    /// file that *contains* the call — distinct from `decl_path`, which
    /// points at where the callee is defined).
    pub call_site_range: Option<TextRange>,
    /// Identifier of the declaring function in the function registry.
    /// `None` for anonymous frames (e.g. HOF inline-expansion frames produced
    /// by `map`, `filter`, `reduce`). Named `smelt.define` frames carry `Some`.
    pub fn_id: Option<String>,
    /// Zero-based index into the source list literal at the HOF call site,
    /// identifying which element the expanded lambda body was operating on.
    /// `None` when the source list was not a literal or the information is
    /// not statically available (the common v1 case).
    pub element_index: Option<usize>,
    /// Phase C (meta-language) extension: the source span of the column's
    /// declaration in the upstream `ModelSchema`, when the HOF source list
    /// came from `smelt.columns_of(t)`. `None` for literal-sourced lists,
    /// unresolvable schemas, or non-`columns_of` HOF sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub column_origin: Option<smelt_parser::TextRange>,
    /// Wide-reflection extension: source-model provenance for HOF frames whose
    /// source list came from `smelt.models.*`. Carries the model's workspace-
    /// relative path and the frontmatter declaration span when statically
    /// traceable. `None` for all other HOF frame sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub model_origin: Option<ModelOrigin>,
    /// Wide-reflection extension: source-yaml provenance for HOF frames whose
    /// source list came from `smelt.sources.*`. Carries the source YAML's
    /// workspace-relative path and the YAML declaration span when statically
    /// traceable. `None` for all other HOF frame sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub source_origin: Option<SourceOrigin>,
}

/// Phase C (meta-language): concrete value produced by `smelt.columns_of`.
///
/// Each element of the `List<ColumnRef>` returned by `smelt.columns_of(t)`
/// is one `ColumnRefValue`. The list preserves the source schema's declared
/// column order (Phase C spec §"ColumnRef ordering").
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa query
/// `columns_of_for_table_expr`) and any future consumer (e.g. the planner)
/// can use it without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRefValue {
    /// The column's declared name (identifier text from the source schema).
    pub name: String,
    /// The column's declared data type (from `TypedColumn::data_type`).
    /// `None` when the column's type was not statically known (e.g. no
    /// type annotation and inference could not determine it).
    pub data_type: Option<crate::DataType>,
    /// Whether the column's type satisfies the `Numeric` constraint —
    /// `DataType::is_numeric()` per `types.md` §"Type constraints".
    /// `false` when `data_type` is `None`.
    pub is_numeric: bool,
    /// Source span of this column's declaration in the upstream `ModelSchema`
    /// (the `Column::range` field). `None` when the span is not statically
    /// resolvable (e.g. source came from an external YAML with no SQL range).
    pub source_span: Option<smelt_parser::TextRange>,
}

/// Source-model provenance attached to a wide-reflection HOF frame
/// (Phase D, `smelt.models.*`-sourced lists).
///
/// Captures the model's workspace-relative path and the frontmatter
/// declaration span (if statically resolvable). The producer stamps this
/// on each per-element anonymous HOF frame when the source list comes from
/// `smelt.models.with_tag` or `smelt.models.all`.
///
/// Producer-side only in v1 — the LSP renderer does not yet surface this
/// field (tracked as a Known Divergence in `expansion.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOrigin {
    /// Workspace-relative path of the model's source `.sql` file, with `/`
    /// separators. This is the same path used in `ModelRefValue::path`.
    pub path: String,
    /// Source span of the model's frontmatter block (if present and parseable).
    /// `None` when the model has no frontmatter or when the span is otherwise
    /// not statically resolvable.
    pub frontmatter_span: Option<smelt_parser::TextRange>,
}

/// Source-yaml provenance attached to a wide-reflection HOF frame
/// (Phase D, `smelt.sources.*`-sourced lists).
///
/// Captures the source YAML file's workspace-relative path and the
/// YAML declaration span (if statically resolvable). The producer stamps
/// this on each per-element anonymous HOF frame when the source list comes
/// from `smelt.sources.with_tag` or `smelt.sources.all`.
///
/// Producer-side only in v1 — the LSP renderer does not yet surface this
/// field (tracked as a Known Divergence in `expansion.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOrigin {
    /// Workspace-relative path of the source's YAML file, with `/` separators.
    /// This is the same path used in `SourceRefValue::path`.
    pub path: String,
    /// Source span of the YAML declaration (currently always `None` in v1
    /// since source YAMLs are not tracked by the Rowan CST — reserved for
    /// a future pass that parses span information from the YAML).
    pub declaration_span: Option<smelt_parser::TextRange>,
}

/// Concrete value produced by `smelt.models.with_tag` / `smelt.models.all`
/// at expansion time (Phase D, wide reflection).
///
/// Each element of the `List<ModelRef>` returned by those accessors is
/// one `ModelRefValue`. The list is sorted ascending by `path`
/// (byte-lexicographic on the workspace-relative path with `/` separators).
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa queries
/// `models_with_tag` / `models_all`) and any future consumer can use it
/// without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRefValue {
    /// Workspace-relative file path with `/` separators (e.g.
    /// `"models/orders.sql"`).
    pub path: String,
    /// Model name — the final path segment without the `.sql` extension
    /// (e.g. `"orders"`).
    pub name: String,
    /// Merged tag set: union of `smelt.yml` `models.<name>.tags` and SQL
    /// frontmatter `tags:`, deduplicated by `Config::get_tags`. In the
    /// order returned by that function (smelt.yml tags first, then
    /// frontmatter tags not already present).
    pub tags: Vec<String>,
    /// The model name used to route `m.columns` through `columns_of_for_table_expr`.
    /// This is the model's short name (same as `name`) — the Salsa query
    /// accepts this to resolve the column list at expansion time.
    pub model_name_for_columns: String,
}

/// Concrete value produced by `smelt.sources.with_tag` / `smelt.sources.all`
/// at expansion time (Phase D, wide reflection).
///
/// Each element of the `List<SourceRef>` returned by those accessors is
/// one `SourceRefValue`. The list is sorted ascending by `path`
/// (byte-lexicographic on the workspace-relative path with `/` separators).
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa queries
/// `sources_with_tag` / `sources_all`) and any future consumer can use it
/// without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefValue {
    /// Workspace-relative file path of the source YAML with `/` separators
    /// (e.g. `"models/sources/raw/users.yml"`).
    pub path: String,
    /// Source name — the final path segment without the `.yml` / `.yaml`
    /// extension (e.g. `"users"`).
    pub name: String,
    /// Tag set as declared in the source YAML's `tags:` list. No merge with
    /// a second source (source YAMLs are the single source of tag truth for
    /// sources).
    pub tags: Vec<String>,
    /// Address segments for routing `s.columns` through the source resolution
    /// machinery. These are the `address_segments` from `SourceInfo`.
    pub address_segments: Vec<String>,
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
    pub return_type_range: Option<TextRange>,
    /// Tier, derived from annotation completeness at parse time.
    pub tier: Tier,
    /// Byte-offset range of the function-name identifier, for diagnostics.
    pub name_range: TextRange,
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
    /// `true` when the declared return type carries a `NOT NULL` qualifier
    /// (Phase 5, nullability-soundness). The body must synthesise a
    /// non-nullable result when this flag is set.
    pub return_not_null: bool,
}

fn type_ref_range(type_ref: &TypeRef, _text: &str) -> TextRange {
    type_ref.syntax().text_range()
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

    // Phase 5 (nullability-soundness): detect `NOT NULL` qualifier from the
    // CST marker emitted by the parser. When present, strip "NOT NULL" from
    // the raw text before passing to the string-level `parse_smelt_type` so
    // the type parses correctly as if the qualifier were absent.
    let not_null = type_ref_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_type_ref_text: Option<String> = if not_null {
        type_ref_text.as_deref().map(strip_not_null_from_type_text)
    } else {
        type_ref_text.clone()
    };

    let mut type_ref: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_type_ref_text.as_deref().map(parse_smelt_type);

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
    let name_range = param.name_range();
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
        not_null,
    }
}

/// Strip the `NOT NULL` qualifier tokens from a raw type-ref text string
/// (Phase 5, nullability-soundness).
///
/// For `Expr<Integer NOT NULL>` the parser emits the tokens "Integer NOT NULL"
/// inside the angle brackets. This helper removes the trailing " NOT NULL"
/// (case-insensitive, with optional leading whitespace) from before the last
/// `>` so that `parse_smelt_type` can parse just the inner type.
///
/// The stripping is done by looking for the rightmost occurrence of " NOT NULL"
/// (case-insensitive) in the string and removing it along with any leading
/// whitespace between the type name and `NOT NULL`.
///
/// Examples:
/// - `"Expr<Integer NOT NULL>"` → `"Expr<Integer>"`
/// - `"Expr<Numeric NOT NULL>"` → `"Expr<Numeric>"`
///
/// Returns the original text unchanged if the pattern is not found.
fn strip_not_null_from_type_text(text: &str) -> String {
    // Find "NOT NULL" (case-insensitive) in the text.
    let upper = text.to_uppercase();
    if let Some(pos) = upper.rfind("NOT NULL") {
        // Strip from the whitespace before NOT back to the type text,
        // then re-attach everything after "NULL".
        let before = text[..pos].trim_end();
        let after = &text[pos + "NOT NULL".len()..];
        format!("{}{}", before, after)
    } else {
        text.to_string()
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

    // Build the ordered `(name, DataTypeReq, not_null)` list from the CST's
    // ROW_FIELD children.
    let mut required: Vec<(String, DataTypeReq, bool)> = Vec::new();
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
        // Phase 5: read the NOT NULL qualifier from the ROW_FIELD's
        // NOT_NULL_QUALIFIER child (placed as a sibling of TYPE_REF).
        let not_null = field.not_null();
        required.push((name, req, not_null));
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
/// Returns the `TextRange` of each struct field whose type text cannot be
/// parsed as a recognised concrete `DataType`. Used by the diagnostic layer to
/// emit `UnknownStructFieldType` at the individual field's span.
///
/// Returns an empty `Vec` when the `TypeRef` is not an `Expr<Struct<{…}>>` shape,
/// or when all field types are valid.
///
/// Pure — walks only the CST node, no Salsa dependency.
pub fn struct_field_unknown_ranges(tr: &TypeRef) -> Vec<TextRange> {
    use smelt_parser::ast::{StructType, TypeRefHead};
    if tr.kind() != TypeRefHead::Expr {
        return vec![];
    }
    let struct_node = match tr.syntax().descendants().find_map(StructType::cast) {
        Some(n) => n,
        None => return vec![],
    };
    let mut errors = Vec::new();
    for sf in struct_node.fields() {
        let type_ref_node = sf.type_ref();
        let inner_text = type_ref_node.as_ref().map(|t| t.text()).unwrap_or_default();
        if crate::parse_type(inner_text.trim()).is_err() {
            if let Some(field_tr) = type_ref_node {
                errors.push(field_tr.syntax().text_range());
            }
        }
    }
    errors
}

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
        let dt = crate::parse_type(inner_text.trim())
            .unwrap_or(DataType::Unknown(crate::UnknownReason::Dynamic));
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
    // A parameter counts as "annotated" only when its type_ref parsed
    // successfully (Some(Ok(_))).  A malformed annotation — one that
    // produces Some(Err(_)) — is treated as unannotated, demoting the
    // function to Tier 1 (spec: gradual_typing.md §"Tier dispatch is
    // implicit": "a malformed annotation (one that fails
    // InvalidFunctionTypeRef) is treated as unannotated").
    // Note: unannotated params (type_ref == None) also fall through to
    // Tier 1 via the `_ => false` arm below.
    let all_params_typed = params.iter().all(|p| matches!(&p.type_ref, Some(Ok(_))));
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
    let name_range = define.name_range()?;

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

    // Phase 5 (nullability-soundness): detect NOT NULL on return type.
    let return_not_null = return_type_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_return_type_text: Option<String> = if return_not_null {
        return_type_text
            .as_deref()
            .map(strip_not_null_from_type_text)
    } else {
        return_type_text.clone()
    };

    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_return_type_text.as_deref().map(parse_smelt_type);
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
    // Pass the cleaned text (without NOT NULL) to compute_tier so the tier is
    // derived from the presence of a return annotation, not the qualifier.
    let tier = compute_tier(&params, clean_return_type_text.as_deref());

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
        return_not_null,
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
    let name_range = ext.name_range()?;

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
    let return_not_null = return_type_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_return_type_text: Option<String> = if return_not_null {
        return_type_text
            .as_deref()
            .map(strip_not_null_from_type_text)
    } else {
        return_type_text.clone()
    };
    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_return_type_text.as_deref().map(parse_smelt_type);
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
    let tier = compute_tier(&params, clean_return_type_text.as_deref());

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
        return_not_null,
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
    pub engine_native: HashMap<DialectId, DataType>,
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
    /// Dialect-specific alternate names that resolve to this same entry
    /// (e.g. `IFNULL`'s `aliases` includes `NVL`; `JSON_OBJECT`'s includes
    /// `JSON_BUILD_OBJECT`). Empty for entries with no dialect alias.
    ///
    /// This is the single authoritative row per canonical function: an
    /// alias is a name, not a duplicated signature. [`BuiltinRegistry::resolve`]
    /// and [`BuiltinRegistry::canonical_name`] check this table (via the
    /// derived alias index) after a direct canonical-name match fails.
    pub aliases: &'static [&'static str],
    /// Nullability-propagation policy for this signature's result, layered
    /// on top of the generic "always nullable" default a registry-resolved
    /// call otherwise gets. See [`NullabilityPropagation`].
    pub nullability: NullabilityPropagation,
}

/// Nullability-propagation policy for a registry-resolved call's result,
/// consulted by the type-inference layer (`smelt-db`'s
/// `registry_result_nullable`) alongside the generic per-function default.
///
/// Registry data, not a hand-matched special case — per the function-registry
/// single-ownership rule (architecture.md §Constraints #14), a function's
/// nullability behaviour is declared once here rather than duplicated as a
/// name-matched arm in `smelt-db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NullabilityPropagation {
    /// No propagation tag — the result is nullable regardless of argument
    /// nullability or query shape. The default for every signature that
    /// doesn't opt into a more precise rule.
    #[default]
    None,
    /// Extremal-aggregate rule (`MIN`/`MAX`): a NOT NULL argument produces a
    /// NOT NULL result, but **only** under a `GROUP BY` — every group is
    /// guaranteed at least one row, so the fold can't collapse to NULL. An
    /// aggregate over possibly-empty ungrouped input stays nullable; that's
    /// a soundness boundary (an empty `SELECT MIN(x) FROM t` yields one NULL
    /// row), not a limitation the tag can lift.
    GroupedExtremal,
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
            aliases: &[],
            nullability: NullabilityPropagation::None,
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

    /// Attach dialect-specific alias names to this signature (Function-registry
    /// single ownership, architecture.md §Constraints #14).
    ///
    /// Builder-style — used by the registry seed to register alternate
    /// spellings (e.g. `NVL` for `IFNULL`) that resolve to this same entry
    /// without duplicating its signature.
    pub fn with_aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
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
    /// Calling this multiple times with different dialects builds up the full
    /// override table.
    pub fn with_engine_native(mut self, dialect: DialectId, dt: DataType) -> Self {
        self.engine_native.insert(dialect, dt);
        self
    }

    /// Attach a [`NullabilityPropagation`] tag to this signature.
    ///
    /// Builder-style — used by the registry seed to opt a function into a
    /// precise nullability rule (e.g. `MIN`/`MAX`'s grouped-extremal rule)
    /// instead of the generic "always nullable" default.
    pub fn with_nullability(mut self, rule: NullabilityPropagation) -> Self {
        self.nullability = rule;
        self
    }

    /// Does the signature require a CAST back to the canonical return
    /// type when executed on `dialect`? (§16 #9 / Phase 12, recording
    /// only — Step 7+ consumes this.)
    ///
    /// Returns `false` when no canonical type is declared (the common
    /// case — the signature's own [`Self::return_type`] is already
    /// canonical) or when the dialect's native type equals the canonical
    /// type. Returns `true` when the dialect is listed in
    /// [`Self::engine_native`] with a type that differs from
    /// [`Self::canonical_return`].
    pub fn needs_cast_for(&self, dialect: DialectId) -> bool {
        let Some(canonical) = &self.canonical_return else {
            return false;
        };
        match self.engine_native.get(&dialect) {
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
        TypeExpr::Concrete(TypeConstraint::Ordered) => {
            DataType::Unknown(crate::UnknownReason::Dynamic)
        }
        TypeExpr::Concrete(TypeConstraint::Any) => DataType::Unknown(crate::UnknownReason::Dynamic),
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
    use DataType::*;

    // Helper: lift an integer type to its Decimal equivalent per §15.
    // SmallInt → Decimal(5,0), Integer → Decimal(10,0), BigInt → Decimal(19,0).
    let lift_integer_to_decimal = |d: &DataType| -> Option<(u8, u8)> {
        match d {
            SmallInt => Some((5, 0)),
            Integer => Some((10, 0)),
            BigInt => Some((19, 0)),
            _ => None,
        }
    };

    // Apply the Decimal LUB formula (§15): given (p1,s1) and (p2,s2),
    // s' = max(s1,s2), p' = max(p1-s1, p2-s2) + s', saturated at 38.
    let decimal_lub = |p1: u8, s1: u8, p2: u8, s2: u8| -> DataType {
        let s = s1.max(s2) as u32;
        let int_digits1 = (p1 as u32).saturating_sub(s1 as u32);
        let int_digits2 = (p2 as u32).saturating_sub(s2 as u32);
        let p = int_digits1.max(int_digits2) + s;
        Decimal {
            precision: p.min(38) as u8,
            scale: s as u8,
        }
    };

    // Handle Decimal pairs (same or different params) using the formula.
    if let (
        Decimal {
            precision: p1,
            scale: s1,
        },
        Decimal {
            precision: p2,
            scale: s2,
        },
    ) = (a, b)
    {
        return decimal_lub(*p1, *s1, *p2, *s2);
    }

    // Handle Decimal + integer: lift the integer, then apply the formula.
    if let Decimal {
        precision: pd,
        scale: sd,
    } = a
    {
        if let Some((pi, si)) = lift_integer_to_decimal(b) {
            return decimal_lub(*pd, *sd, pi, si);
        }
    }
    if let Decimal {
        precision: pd,
        scale: sd,
    } = b
    {
        if let Some((pi, si)) = lift_integer_to_decimal(a) {
            return decimal_lub(pi, si, *pd, *sd);
        }
    }

    // For all other pairs, use the rank-based promotion chain (§16 #9).
    let rank = |d: &DataType| -> u8 {
        match d {
            SmallInt => 1,
            Integer => 2,
            BigInt => 3,
            Decimal { .. } => 4,
            Float => 5,
            Double => 6,
            _ => 0,
        }
    };
    let (ra, rb) = (rank(a), rank(b));
    if ra >= rb {
        a.clone()
    } else {
        b.clone()
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
    /// Checks the canonical name table first, then dialect aliases (e.g.
    /// `NVL` → `IFNULL`'s entry, `GET_JSON_OBJECT` → `JSON_EXTRACT_TEXT`'s
    /// entry). Returns `Some(&'static Signature)` when the name matches a
    /// registered entry or a registered alias of one, `None` otherwise.
    pub fn resolve(name: &str) -> Option<&'static Signature> {
        let upper = name.to_ascii_uppercase();
        if let Some(sig) = REGISTRY.get(&upper) {
            return Some(sig);
        }
        let canonical = ALIAS_MAP.get(&upper)?;
        REGISTRY.get(canonical)
    }

    /// Resolve a name (canonical or dialect alias) to its canonical
    /// (upper-cased) registry name, case-insensitively.
    ///
    /// This is the single alias-resolution entry point other crates use to
    /// map a dialect spelling back to the name `SqlFunction` recognises —
    /// keeping alias recognition registry-owned per architecture.md
    /// §Constraints #14.
    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::resolve(name).map(|sig| sig.name.as_str())
    }

    /// Iterator over all canonical (upper-cased) names in the registry.
    pub fn names() -> impl Iterator<Item = &'static str> {
        REGISTRY.keys().map(|s| s.as_str())
    }

    /// Iterator over every registered `(alias, canonical_name)` pair,
    /// upper-cased. Used by the registry-consistency gate to assert every
    /// alias is recognized and classified consistently with its canonical
    /// entry.
    pub fn aliases() -> impl Iterator<Item = (&'static str, &'static str)> {
        ALIAS_MAP
            .iter()
            .map(|(alias, canonical)| (alias.as_str(), canonical.as_str()))
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

/// The closed field set of [`SmeltType::ColumnRef`] (meta-language).
///
/// Invariant: exactly eight entries — `name`, `type`, `is_numeric`, `is_decimal`,
/// `is_string`, `is_temporal`, `is_integer`, `is_boolean` — in this order.
/// This is the single source of truth for the field set. Any future addition
/// requires a spec edit AND a change to this constant AND a bump of the field count
/// in all exhaustiveness checks.
///
/// Field types:
/// - `name`        → `Expr<Text>`    (the column identifier as plain Text)
/// - `type`        → `Unknown`       (represents the DataType meta-literal; the
///   concrete meta type is not yet in SmeltType v1)
/// - `is_numeric`  → `Expr<Boolean>` (TRUE iff `type` ∈ Numeric constraint set)
/// - `is_decimal`  → `Expr<Boolean>` (TRUE iff head constructor is `Decimal`)
/// - `is_string`   → `Expr<Boolean>` (TRUE iff head constructor is Text/Varchar/Char)
/// - `is_temporal` → `Expr<Boolean>` (TRUE iff head constructor is Date/Timestamp/Time — NOT Interval)
/// - `is_integer`  → `Expr<Boolean>` (TRUE iff head constructor is SmallInt/Integer/BigInt)
/// - `is_boolean`  → `Expr<Boolean>` (TRUE iff head constructor is Boolean)
pub const COLUMN_REF_FIELDS: &[(&str, ColumnRefFieldType)] = &[
    (
        "name",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    ),
    // `type` is a DataType meta-literal; maps to Unknown as a forward-compatibility
    // placeholder until a proper meta-DataType representation is introduced.
    ("type", SmeltType::Unknown),
    (
        "is_numeric",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
    (
        "is_decimal",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
    (
        "is_string",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
    (
        "is_temporal",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
    (
        "is_integer",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
    (
        "is_boolean",
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)),
    ),
];

/// Look up a [`ColumnRef`] field by name (case-sensitive, exact match).
///
/// Returns `Some(&SmeltType)` for `"name"`, `"type"`, `"is_numeric"`, `"is_decimal"`,
/// `"is_string"`, `"is_temporal"`, `"is_integer"`, or `"is_boolean"`;
/// `None` for any other identifier (the closed-field invariant).
///
/// Pure — no Salsa dependency.
pub fn column_ref_field(name: &str) -> Option<&'static ColumnRefFieldType> {
    COLUMN_REF_FIELDS
        .iter()
        .find_map(|(field_name, ty)| if *field_name == name { Some(ty) } else { None })
}

/// The closed field set of [`SmeltType::ModelRef`] (Phase D, meta-language).
///
/// Invariant: exactly four entries — `path`, `name`, `tags`, `columns` — in
/// this canonical order. This is the single source of truth for the v1 field
/// set. Any future addition requires a spec edit AND a change to this constant.
///
/// Field types:
/// - `path`    → `Expr<Text>`         — workspace-relative file path
/// - `name`    → `Expr<Text>`         — model identifier (path segment sans `.sql`)
/// - `tags`    → `List<Expr<Text>>`   — merged tag set
/// - `columns` → `List<ColumnRef>`    — model column list
///
/// Uses `LazyLock` because `SmeltType::List` contains a `Box`, which cannot
/// be constructed in `const` context. The logical invariants are still that this
/// is the single, immutable source of truth for the field set.
pub static MODEL_REF_FIELDS: LazyLock<Vec<(&'static str, SmeltType)>> = LazyLock::new(|| {
    vec![
        (
            "path",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "name",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "tags",
            SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text,
            )))),
        ),
        ("columns", SmeltType::List(Box::new(SmeltType::ColumnRef))),
    ]
});

/// The closed field set of [`SmeltType::SourceRef`] (Phase D, meta-language).
///
/// Invariant: exactly four entries — `path`, `name`, `tags`, `columns` — in
/// this canonical order, identical in shape to [`MODEL_REF_FIELDS`] (uniformity
/// invariant from the design rationale). The semantic meanings differ (source
/// YAML path vs model SQL path; source tags vs merged model tags; etc.) but the
/// structural types are the same.
pub static SOURCE_REF_FIELDS: LazyLock<Vec<(&'static str, SmeltType)>> = LazyLock::new(|| {
    vec![
        (
            "path",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "name",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "tags",
            SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text,
            )))),
        ),
        ("columns", SmeltType::List(Box::new(SmeltType::ColumnRef))),
    ]
});

/// Look up a [`ModelRef`] field by name (case-sensitive, exact match).
///
/// Returns `Some(&SmeltType)` for `"path"`, `"name"`, `"tags"`, or `"columns"`;
/// `None` for any other identifier (the closed-field invariant).
///
/// Pure — no Salsa dependency.
pub fn model_ref_field(name: &str) -> Option<&'static SmeltType> {
    MODEL_REF_FIELDS
        .iter()
        .find_map(|(field_name, ty)| if *field_name == name { Some(ty) } else { None })
}

/// Look up a [`SourceRef`] field by name (case-sensitive, exact match).
///
/// Returns `Some(&SmeltType)` for `"path"`, `"name"`, `"tags"`, or `"columns"`;
/// `None` for any other identifier (the closed-field invariant).
///
/// Pure — no Salsa dependency.
pub fn source_ref_field(name: &str) -> Option<&'static SmeltType> {
    SOURCE_REF_FIELDS
        .iter()
        .find_map(|(field_name, ty)| if *field_name == name { Some(ty) } else { None })
}

/// The closed field set of [`SmeltType::ModelDef`].
///
/// Invariant: exactly five entries — `name`, `body`, `materialization`, `tags`,
/// `description` — in this canonical order. This is the single source of truth
/// for the v1 field set. Any future addition requires a spec edit AND a change
/// to this constant.
///
/// Field types:
/// - `name`            → `Expr<Text>`       — model identifier (`[A-Za-z0-9_]+`, non-empty)
/// - `body`            → `TableExpr`        — the only carve-out admitting `TableExpr` in a record field
/// - `materialization` → `Expr<Text>`       — one of `view`, `table`, `incremental`
/// - `tags`            → `List<Expr<Text>>` — merged tag set
/// - `description`     → `Expr<Text>`       — human-readable description
///
/// Uses `LazyLock` because `SmeltType::List` and `SmeltType::TableExpr` contain
/// heap-allocated inner types that cannot be constructed in `const` context.
pub static MODEL_DEF_FIELDS: LazyLock<Vec<(&'static str, SmeltType)>> = LazyLock::new(|| {
    vec![
        (
            "name",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        // `body` is the single carve-out admitting `TableExpr` in a record-like
        // field position. User-defined `smelt.record` declarations remain
        // forbidden from declaring `TableExpr` fields.
        ("body", SmeltType::TableExpr(None)),
        (
            "materialization",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "tags",
            SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text,
            )))),
        ),
        (
            "description",
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
    ]
});

/// Look up a [`ModelDef`] field by name (case-sensitive, exact match).
///
/// Returns `Some(&SmeltType)` for `"name"`, `"body"`, `"materialization"`,
/// `"tags"`, or `"description"`; `None` for any other identifier (the
/// closed-field invariant).
///
/// Pure — no Salsa dependency.
pub fn model_def_field(name: &str) -> Option<&'static SmeltType> {
    MODEL_DEF_FIELDS
        .iter()
        .find_map(|(field_name, ty)| if *field_name == name { Some(ty) } else { None })
}

/// Returns `true` for every [`SmeltType`] that is meta-only — i.e., values of
/// this type never reach the database engine.
///
/// Meta-only types: `List<T>` (for any `T`), `Lambda<T,U>`, `TableExpr`,
/// `SelectItems`, `ColumnRef`, `ModelRef`, `SourceRef`, `ModelDef`,
/// `Record{…}`, `Map<K,V>`, and `Unknown`.
///
/// Data-world types: `Expr<T>` and `Struct{…}` (which represent SQL-level
/// typed expressions).
///
/// Pure — no Salsa dependency.
pub fn is_meta_only_type(ty: &SmeltType) -> bool {
    !is_data_world_type(ty)
}

/// Returns `true` for every [`SmeltType`] that represents a value in the
/// Data World (SQL execution layer).
///
/// Data-world types: `Expr<T>` and `Struct{…}`.
/// All other sorts are meta-only.
///
/// Pure — no Salsa dependency.
pub fn is_data_world_type(ty: &SmeltType) -> bool {
    matches!(ty, SmeltType::Expr(_) | SmeltType::Struct { .. })
}

/// The closed accessor set for the `smelt.models` namespace.
///
/// Returns `Some(&'static SmeltMetaSignature)` when `name` is a known accessor;
/// `None` when `name` is not in the closed set (trigger for
/// `WideReflectionUnknownAccessor`).
///
/// Pure — no Salsa dependency.
pub fn models_accessor(name: &str) -> Option<&'static SmeltMetaSignature> {
    MODELS_ACCESSORS.get(name)
}

/// The closed accessor set for the `smelt.sources` namespace.
///
/// Returns `Some(&'static SmeltMetaSignature)` when `name` is a known accessor;
/// `None` when `name` is not in the closed set (trigger for
/// `WideReflectionUnknownAccessor`).
///
/// Pure — no Salsa dependency.
pub fn sources_accessor(name: &str) -> Option<&'static SmeltMetaSignature> {
    SOURCES_ACCESSORS.get(name)
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

/// Closed accessor namespace for `smelt.models`.
///
/// Keys are the accessor names (lowercase); values are the signatures. The two
/// accessors are:
/// - `with_tag`: `(Text) -> List<ModelRef>` — one positional `Expr<Text>` param.
/// - `all`: `() -> List<ModelRef>` — zero parameters.
static MODELS_ACCESSORS: LazyLock<HashMap<&'static str, SmeltMetaSignature>> =
    LazyLock::new(|| {
        let mut m: HashMap<&'static str, SmeltMetaSignature> = HashMap::new();
        m.insert(
            "with_tag",
            SmeltMetaSignature {
                name: "smelt.models.with_tag",
                params: vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))],
                return_type: SmeltType::List(Box::new(SmeltType::ModelRef)),
            },
        );
        m.insert(
            "all",
            SmeltMetaSignature {
                name: "smelt.models.all",
                params: vec![],
                return_type: SmeltType::List(Box::new(SmeltType::ModelRef)),
            },
        );
        m
    });

/// Closed accessor namespace for `smelt.sources`.
///
/// Keys are the accessor names (lowercase); values are the signatures. The two
/// accessors are:
/// - `with_tag`: `(Text) -> List<SourceRef>` — one positional `Expr<Text>` param.
/// - `all`: `() -> List<SourceRef>` — zero parameters.
static SOURCES_ACCESSORS: LazyLock<HashMap<&'static str, SmeltMetaSignature>> =
    LazyLock::new(|| {
        let mut m: HashMap<&'static str, SmeltMetaSignature> = HashMap::new();
        m.insert(
            "with_tag",
            SmeltMetaSignature {
                name: "smelt.sources.with_tag",
                params: vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))],
                return_type: SmeltType::List(Box::new(SmeltType::SourceRef)),
            },
        );
        m.insert(
            "all",
            SmeltMetaSignature {
                name: "smelt.sources.all",
                params: vec![],
                return_type: SmeltType::List(Box::new(SmeltType::SourceRef)),
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
            DialectId::DuckDb,
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
        .with_kind(ExprKind::Agg)
        .with_nullability(NullabilityPropagation::GroupedExtremal),
    );
    insert(
        Signature::new(
            "MAX",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_nullability(NullabilityPropagation::GroupedExtremal),
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
    insert(
        Signature::new(
            "IFNULL",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), var("T")],
            TypeExpr::Var("T".into()),
        )
        // Null-handling alias (Oracle/Snowflake/DuckDB dialect).
        .with_aliases(&["NVL"]),
    );

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
        "MD5",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        // BigInt (not Integer) to match the hand-written arm — DuckDB returns
        // a 64-bit length and the migrated typing path must reproduce it.
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
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
        // BigInt to match the hand-written arm.
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
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
            with_timezone: true,
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
            with_timezone: true,
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
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown(
                crate::UnknownReason::Dynamic,
            ))),
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
    // arg_max(value, key) → value: return the value from the row with the maximum key.
    // Accepts any value type T and any key type K (must be orderable at runtime).
    // `MAX_BY` is DuckDB/Postgres's alias for the same order-monotone-overwrite
    // combiner (`incremental_shapes.md` §"The column-family catalogue").
    insert(
        Signature::new(
            "ARG_MAX",
            vec![tp("T", TypeConstraint::Any), tp("K", TypeConstraint::Any)],
            vec![var("T"), var("K")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_aliases(&["MAX_BY"]),
    );
    // arg_min(value, key) → value: the order-monotone-overwrite family's
    // minimum-ordering counterpart to `ARG_MAX`, aliased `MIN_BY`.
    insert(
        Signature::new(
            "ARG_MIN",
            vec![tp("T", TypeConstraint::Any), tp("K", TypeConstraint::Any)],
            vec![var("T"), var("K")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_aliases(&["MIN_BY"]),
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
        // SmallInt to match the hand-written arm (DuckDB `sign` returns a
        // small signed integer, not a float).
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::SmallInt)),
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
        // BigInt to match the hand-written arm (the date-part extraction
        // family — YEAR/MONTH/DAY/… — all return BigInt).
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
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
    insert(Signature::new(
        "TO_SECONDS",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Interval)),
    ));

    // ─── Function-registry consolidation: remaining recognised built-ins ─────
    //
    // Every name recognised by `SqlFunction::from_name` must resolve here so
    // the registry is the single authoritative home for recognition,
    // classification (`kind`), and — for migrated functions — typing. The
    // consistency gate `every_recognized_function_is_registry_backed`
    // (smelt-db integration tests) enforces this. Argument shapes here are
    // deliberately permissive (`Any`-variadic) for functions whose typing
    // still lives in the hand-written match; migrating a function tightens
    // both its parameter constraints and its return type to match the legacy
    // arm exactly.
    let any_args = || {
        vec![SigParam::Variadic(Box::new(SigParam::Concrete(
            TypeConstraint::Any,
        )))]
    };

    // Extended statistical / distribution aggregates → Double.
    for name in [
        "CORR",
        "COVAR_POP",
        "COVAR_SAMP",
        "REGR_SLOPE",
        "PERCENTILE_CONT",
        "PERCENTILE_DISC",
    ] {
        insert(
            Signature::new(
                name,
                vec![],
                any_args(),
                TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
            )
            .with_kind(ExprKind::Agg),
        );
    }
    // Boolean aggregate.
    insert(
        Signature::new(
            "EVERY",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg),
    );
    // Text-returning aggregate.
    insert(
        Signature::new(
            "GROUP_CONCAT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg),
    );
    // First-argument identity aggregates (typing stays in the exception list).
    for name in ["FIRST", "LAST", "MODE"] {
        insert(
            Signature::new(
                name,
                vec![tp("T", TypeConstraint::Any)],
                vec![var("T")],
                TypeExpr::Var("T".into()),
            )
            .with_kind(ExprKind::Agg),
        );
    }

    // Extended math / trig scalars → Double.
    for name in [
        "ACOS", "ASIN", "POW", "CEILING", "RANDOM", "TRUNC", "TRUNCATE",
    ] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        ));
    }
    // Extended text scalars → Text.
    for name in [
        "INITCAP",
        "QUOTE_IDENT",
        "QUOTE_LITERAL",
        "REVERSE",
        "TO_CHAR",
        "TRANSLATE",
    ] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        ));
    }
    // 1-based string search position → BigInt.
    insert(Signature::new(
        "POSITION",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    // Date-part extraction scalars → BigInt.
    for name in ["DAY", "DAYOFWEEK", "MONTH", "QUARTER", "YEAR"] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        ));
    }
    // Temporal constructors.
    insert(Signature::new(
        "MAKE_TIME",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Time)),
    ));
    insert(Signature::new(
        "MAKE_TIMESTAMPTZ",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: true,
        })),
    ));
    // JSON built-ins. Aliases per canonical name (dialect side-channel
    // consolidated into the registry per architecture.md §Constraints #14):
    // JSON_BUILD_OBJECT (Postgres) → JSON_OBJECT, JSON_BUILD_ARRAY (Postgres)
    // → JSON_ARRAY, TO_JSONB/ROW_TO_JSON (Postgres) → TO_JSON,
    // JSON_EXTRACT_PATH (Postgres) → JSON_EXTRACT, JSON_EXTRACT_STRING
    // (DuckDB) / JSON_EXTRACT_PATH_TEXT (Postgres) / GET_JSON_OBJECT (Spark
    // Hive) / JSON_VALUE (SQL-standard/Snowflake) → JSON_EXTRACT_TEXT.
    let json_text_aliases: &[(&str, &[&str])] = &[
        ("JSON_OBJECT", &["JSON_BUILD_OBJECT"]),
        ("JSON_ARRAY", &["JSON_BUILD_ARRAY"]),
        ("TO_JSON", &["TO_JSONB", "ROW_TO_JSON"]),
        ("JSON_EXTRACT", &["JSON_EXTRACT_PATH"]),
        (
            "JSON_EXTRACT_TEXT",
            &[
                "JSON_EXTRACT_STRING",
                "JSON_EXTRACT_PATH_TEXT",
                "GET_JSON_OBJECT",
                "JSON_VALUE",
            ],
        ),
    ];
    for (name, aliases) in json_text_aliases {
        insert(
            Signature::new(
                name,
                vec![],
                any_args(),
                TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
            )
            .with_aliases(aliases),
        );
    }
    insert(Signature::new(
        "JSON_ARRAY_LENGTH",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(
        Signature::new(
            "JSON_OBJECT_KEYS",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Array(Box::new(
                DataType::Text,
            )))),
        )
        // DuckDB alias.
        .with_aliases(&["JSON_KEYS"]),
    );
    insert(Signature::new(
        "JSON_CONTAINS",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
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
        "GLOB",
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
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown(
            crate::UnknownReason::Dynamic,
        ))),
    ));

    m
});

/// Alias index derived from [`REGISTRY`]: maps every dialect alias
/// (upper-cased) to its canonical (upper-cased) entry name. Built once from
/// each [`Signature::aliases`] table — the single authoritative source per
/// architecture.md §Constraints #14 — never populated by hand.
static ALIAS_MAP: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for sig in REGISTRY.values() {
        for alias in sig.aliases {
            m.insert(alias.to_ascii_uppercase(), sig.name.clone());
        }
    }
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
        SmeltType::Lambda(params, body_ty) => {
            let body_str = format_smelt_type_hover(body_ty);
            if params.len() == 1 {
                format!(
                    "Lambda<{}, {}>",
                    format_smelt_type_hover(&params[0]),
                    body_str
                )
            } else {
                let params_str = params
                    .iter()
                    .map(format_smelt_type_hover)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Lambda<({params_str}), {body_str}>")
            }
        }
        SmeltType::TableExpr(None) => "TableExpr".to_string(),
        SmeltType::TableExpr(Some(req)) => {
            let mut s = String::from("TableExpr<{");
            for (i, (col, col_req, _not_null)) in req.required.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(col);
                s.push_str(": ");
                s.push_str(&col_req.render());
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
        SmeltType::ModelRef => "ModelRef".to_string(),
        SmeltType::SourceRef => "SourceRef".to_string(),
        SmeltType::ModelDef => "ModelDef".to_string(),
        SmeltType::Record { fields, name } => {
            if let Some(n) = name {
                n.clone()
            } else {
                let field_str: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", format_smelt_type_hover(v)))
                    .collect();
                format!("Record<{{{}}}>", field_str.join(", "))
            }
        }
        SmeltType::Map { key, value } => {
            format!(
                "Map<{}, {}>",
                format_smelt_type_hover(key),
                format_smelt_type_hover(value)
            )
        }
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
        assert!(!c.satisfies(&DataType::Unknown(crate::UnknownReason::Dynamic)));
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
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt))
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
        // Decimal + integer applies the §15 LUB formula:
        // Integer → Decimal(10,0); s'=max(0,2)=2; p'=max(10,3)+2=12.
        assert_eq!(
            numeric_lub(
                &DataType::Integer,
                &DataType::Decimal {
                    precision: 5,
                    scale: 2,
                },
            ),
            DataType::Decimal {
                precision: 12,
                scale: 2,
            }
        );
    }

    // === Phase 4 TDD tests — Decimal LUB formula (§15) ===

    #[test]
    fn decimal_decimal_lub_coercion_formula() {
        // s' = max(2,3) = 3, p' = max(10-2, 8-3) + 3 = max(8,5) + 3 = 11
        assert_eq!(
            numeric_lub(
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                },
                &DataType::Decimal {
                    precision: 8,
                    scale: 3
                },
            ),
            DataType::Decimal {
                precision: 11,
                scale: 3
            }
        );
    }

    #[test]
    fn decimal_same_params_lub_unchanged() {
        // Same-params: returns unchanged
        assert_eq!(
            numeric_lub(
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                },
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                },
            ),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
    }

    #[test]
    fn integer_decimal_lub_lifting() {
        // Integer lifts to Decimal(10,0)
        // s' = max(0,2) = 2, p' = max(10-0, 10-2) + 2 = 10 + 2 = 12
        assert_eq!(
            numeric_lub(
                &DataType::Integer,
                &DataType::Decimal {
                    precision: 10,
                    scale: 2
                },
            ),
            DataType::Decimal {
                precision: 12,
                scale: 2
            }
        );
    }

    #[test]
    fn bigint_decimal_lub_lifting() {
        // BigInt lifts to Decimal(19,0)
        // s' = max(0,2) = 2, p' = max(19-0, 5-2) + 2 = 19 + 2 = 21
        assert_eq!(
            numeric_lub(
                &DataType::BigInt,
                &DataType::Decimal {
                    precision: 5,
                    scale: 2
                },
            ),
            DataType::Decimal {
                precision: 21,
                scale: 2
            }
        );
    }

    #[test]
    fn numeric_lub_chain_unaffected() {
        // Non-Decimal cases unchanged
        assert_eq!(
            numeric_lub(&DataType::Integer, &DataType::Double),
            DataType::Double
        );
    }

    // === Phase 12 TDD tests — multi-level frame rendering + CAST flag ===

    #[test]
    fn cast_flag_set_when_canonical_differs_from_engine() {
        // Phase 12 TDD test 3 (§16 #9): `SUM` is seeded with
        // canonical = BigInt and engine_native[DuckDb] = DECIMAL(38,0)
        // — the smelt stand-in for DuckDB's HUGEINT return. The
        // `needs_cast_for(DialectId::DuckDb)` hook must flag divergence so
        // Step 7+ can emit a CAST back to BigInt.
        let sum = BuiltinRegistry::resolve("SUM").expect("SUM seeded");
        assert_eq!(sum.canonical_return, Some(DataType::BigInt));
        assert_eq!(
            sum.engine_native.get(&DialectId::DuckDb),
            Some(&DataType::Decimal {
                precision: 38,
                scale: 0,
            })
        );
        assert!(
            sum.needs_cast_for(DialectId::DuckDb),
            "SUM on DuckDB returns HUGEINT (DECIMAL(38,0)) but canonical is BigInt \
             — needs_cast_for must flag the divergence"
        );
        // Dialects that aren't listed default to "native == canonical".
        assert!(
            !sum.needs_cast_for(DialectId::SparkSql),
            "No override for SparkSql → canonical matches native → no cast needed"
        );
    }

    #[test]
    fn cast_flag_false_for_canonical_less_signatures() {
        // The majority of signatures don't declare a canonical — their
        // native return IS their canonical. `needs_cast_for(...)` must
        // return `false` unconditionally for those.
        let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER seeded");
        assert!(lower.canonical_return.is_none());
        assert!(!lower.needs_cast_for(DialectId::DuckDb));
        assert!(!lower.needs_cast_for(DialectId::SparkSql));
        assert!(!lower.needs_cast_for(DialectId::PostgreSql));
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
        .with_engine_native(DialectId::DuckDb, DataType::Integer);
        assert!(!sig.needs_cast_for(DialectId::DuckDb));
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
            column_origin: None,
            model_origin: None,
            source_origin: None,
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
                    false,
                ),
                (
                    "cost".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                    false,
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
            required: vec![(
                "notes".to_string(),
                DataTypeReq::Concrete(DataType::Text),
                false,
            )],
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
                    false,
                ),
                (
                    "cost".to_string(),
                    DataTypeReq::Constraint(TypeConstraint::Numeric),
                    false,
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
            required: vec![(
                "id".to_string(),
                DataTypeReq::Concrete(DataType::BigInt),
                false,
            )],
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
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
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
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        let lambda_numeric_text = SmeltType::Lambda(
            vec![SmeltType::Expr(TypeConstraint::Numeric)],
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
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        assert!(
            is_subtype_of(&lambda, &lambda),
            "Lambda must be a subtype of itself (reflexivity)"
        );
        let lambda2 = SmeltType::Lambda(
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))),
        );
        assert!(
            !is_subtype_of(&lambda, &lambda2),
            "Lambda with different body type must NOT be a subtype"
        );
    }

    // === Phase F (meta-language) TDD tests: multi-arg Lambda ===

    /// Multi-arg lambda has distinct equality/arity from single-arg lambda.
    #[test]
    fn lambda_vec_arity() {
        let lambda_2arg = SmeltType::Lambda(
            vec![
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
            ],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        let lambda_1arg = SmeltType::Lambda(
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        // Different arities must NOT be equal.
        assert_ne!(
            lambda_2arg, lambda_1arg,
            "Lambda with 2 params must differ from Lambda with 1 param"
        );
        // Subtype must not hold either direction.
        assert!(
            !is_subtype_of(&lambda_2arg, &lambda_1arg),
            "Lambda<(Integer, Integer), Text> must NOT be a subtype of Lambda<Integer, Text>"
        );
        assert!(
            !is_subtype_of(&lambda_1arg, &lambda_2arg),
            "Lambda<Integer, Text> must NOT be a subtype of Lambda<(Integer, Integer), Text>"
        );
    }

    /// Multi-arg lambda Display renders with tuple syntax; single-arg renders without.
    #[test]
    fn lambda_vec_display() {
        let lambda_2arg = SmeltType::Lambda(
            vec![
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
            ],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        let lambda_1arg = SmeltType::Lambda(
            vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
            Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        );
        let display_2 = format!("{}", lambda_2arg);
        let display_1 = format!("{}", lambda_1arg);
        // Multi-arg uses tuple syntax.
        assert!(
            display_2.contains("(") && display_2.contains(")"),
            "Multi-arg lambda must render with tuple parens, got: {}",
            display_2
        );
        assert!(
            display_2.starts_with("Lambda<("),
            "Multi-arg lambda must render as Lambda<(...)>, got: {}",
            display_2
        );
        // Single-arg omits parens.
        assert!(
            display_1.starts_with("Lambda<Expr"),
            "Single-arg lambda must render without tuple parens, got: {}",
            display_1
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
        // COLUMN_REF_FIELDS must expose exactly the 8 closed fields and nothing else.
        let expected = [
            "name",
            "type",
            "is_numeric",
            "is_decimal",
            "is_string",
            "is_temporal",
            "is_integer",
            "is_boolean",
        ];
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
        // Exactly eight fields in the constant.
        assert_eq!(
            COLUMN_REF_FIELDS.len(),
            8,
            "COLUMN_REF_FIELDS must have exactly 8 entries"
        );
        // Verify key field types.
        let name_ty = column_ref_field("name").unwrap();
        assert!(
            matches!(
                name_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "name field must be Text (Expr<Text>), got: {name_ty:?}"
        );
        let type_ty = column_ref_field("type").unwrap();
        assert!(
            matches!(type_ty, SmeltType::Unknown),
            "c.type maps to SmeltType::Unknown as the forward-compatibility placeholder; got: {:?}",
            type_ty
        );
        // All is_* predicates must be Boolean.
        for pred in &[
            "is_numeric",
            "is_decimal",
            "is_string",
            "is_temporal",
            "is_integer",
            "is_boolean",
        ] {
            let ty = column_ref_field(pred).unwrap();
            assert!(
                matches!(
                    ty,
                    SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
                ),
                "{pred} field must be Boolean, got: {ty:?}"
            );
        }
    }

    // === Phase D (meta-language) TDD tests — ModelRef / SourceRef + wide reflection ===

    /// `smelt.models.with_tag` resolves to `(Text) -> List<ModelRef>` with one
    /// positional parameter; `smelt.models.all` resolves to `() -> List<ModelRef>` with
    /// zero parameters; analogous for `smelt.sources.*` returning `List<SourceRef>`.
    #[test]
    fn wide_reflection_accessor_signatures() {
        // smelt.models.with_tag: (Text) -> List<ModelRef>
        let with_tag_m =
            models_accessor("with_tag").expect("models_accessor(with_tag) must be registered");
        assert_eq!(
            with_tag_m.params.len(),
            1,
            "smelt.models.with_tag must have exactly one positional parameter"
        );
        assert!(
            matches!(
                &with_tag_m.params[0],
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "smelt.models.with_tag param must be Expr<Text>, got: {:?}",
            with_tag_m.params[0]
        );
        assert!(
            matches!(&with_tag_m.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ModelRef)),
            "smelt.models.with_tag must return List<ModelRef>, got: {:?}",
            with_tag_m.return_type
        );

        // smelt.models.all: () -> List<ModelRef>
        let all_m = models_accessor("all").expect("models_accessor(all) must be registered");
        assert_eq!(
            all_m.params.len(),
            0,
            "smelt.models.all must have zero parameters"
        );
        assert!(
            matches!(&all_m.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ModelRef)),
            "smelt.models.all must return List<ModelRef>, got: {:?}",
            all_m.return_type
        );

        // smelt.sources.with_tag: (Text) -> List<SourceRef>
        let with_tag_s =
            sources_accessor("with_tag").expect("sources_accessor(with_tag) must be registered");
        assert_eq!(
            with_tag_s.params.len(),
            1,
            "smelt.sources.with_tag must have exactly one positional parameter"
        );
        assert!(
            matches!(
                &with_tag_s.params[0],
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "smelt.sources.with_tag param must be Expr<Text>, got: {:?}",
            with_tag_s.params[0]
        );
        assert!(
            matches!(&with_tag_s.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::SourceRef)),
            "smelt.sources.with_tag must return List<SourceRef>, got: {:?}",
            with_tag_s.return_type
        );

        // smelt.sources.all: () -> List<SourceRef>
        let all_s = sources_accessor("all").expect("sources_accessor(all) must be registered");
        assert_eq!(
            all_s.params.len(),
            0,
            "smelt.sources.all must have zero parameters"
        );
        assert!(
            matches!(&all_s.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::SourceRef)),
            "smelt.sources.all must return List<SourceRef>, got: {:?}",
            all_s.return_type
        );
    }

    /// `MODEL_REF_FIELDS` exposes exactly `{path: Text, name: Text, tags: List<Text>,
    /// columns: List<ColumnRef>}` and no other field; same for `SOURCE_REF_FIELDS`.
    #[test]
    fn model_ref_field_set_is_closed() {
        let expected = ["path", "name", "tags", "columns"];
        for field in &expected {
            assert!(
                model_ref_field(field).is_some(),
                "MODEL_REF_FIELDS must contain field '{field}'"
            );
            assert!(
                source_ref_field(field).is_some(),
                "SOURCE_REF_FIELDS must contain field '{field}'"
            );
        }
        // Unknown fields must return None.
        assert!(
            model_ref_field("foo").is_none(),
            "MODEL_REF_FIELDS must not contain 'foo'"
        );
        assert!(
            model_ref_field("is_numeric").is_none(),
            "MODEL_REF_FIELDS must not contain 'is_numeric'"
        );
        assert!(
            source_ref_field("foo").is_none(),
            "SOURCE_REF_FIELDS must not contain 'foo'"
        );
        // Exactly four fields in each constant.
        assert_eq!(
            MODEL_REF_FIELDS.len(),
            4,
            "MODEL_REF_FIELDS must have exactly 4 entries"
        );
        assert_eq!(
            SOURCE_REF_FIELDS.len(),
            4,
            "SOURCE_REF_FIELDS must have exactly 4 entries"
        );
        // Verify types: path → Text, name → Text
        let path_ty = model_ref_field("path").unwrap();
        assert!(
            matches!(
                path_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "path field must be Expr<Text>, got: {path_ty:?}"
        );
        let name_ty = model_ref_field("name").unwrap();
        assert!(
            matches!(
                name_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "name field must be Expr<Text>, got: {name_ty:?}"
        );
        // tags → List<Expr<Text>>
        let tags_ty = model_ref_field("tags").unwrap();
        assert!(
            matches!(tags_ty, SmeltType::List(inner)
                if matches!(inner.as_ref(), SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)))),
            "tags field must be List<Expr<Text>>, got: {tags_ty:?}"
        );
        // columns → List<ColumnRef>
        let cols_ty = model_ref_field("columns").unwrap();
        assert!(
            matches!(cols_ty, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
            "columns field must be List<ColumnRef>, got: {cols_ty:?}"
        );

        // Same checks on source_ref_field
        let s_path_ty = source_ref_field("path").unwrap();
        assert!(
            matches!(
                s_path_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "SourceRef path field must be Expr<Text>, got: {s_path_ty:?}"
        );
        let s_cols_ty = source_ref_field("columns").unwrap();
        assert!(
            matches!(s_cols_ty, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
            "SourceRef columns field must be List<ColumnRef>, got: {s_cols_ty:?}"
        );
    }

    // === Phase D Phase 2 TDD tests — ModelRef/SourceRef subtype TableExpr ===

    /// `ModelRef <: TableExpr` — the subtyping rule fires in the forward direction.
    #[test]
    fn model_ref_is_subtype_of_table_expr() {
        assert!(
            is_subtype_of(&SmeltType::ModelRef, &SmeltType::TableExpr(None)),
            "ModelRef must be a subtype of TableExpr (forward direction)"
        );
    }

    /// `SourceRef <: TableExpr` — the subtyping rule fires in the forward direction.
    #[test]
    fn source_ref_is_subtype_of_table_expr() {
        assert!(
            is_subtype_of(&SmeltType::SourceRef, &SmeltType::TableExpr(None)),
            "SourceRef must be a subtype of TableExpr (forward direction)"
        );
    }

    /// `TableExpr <: ModelRef` does NOT hold — the rule is one-way.
    #[test]
    fn table_expr_not_subtype_of_model_ref() {
        assert!(
            !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::ModelRef),
            "TableExpr must NOT be a subtype of ModelRef (reverse direction forbidden)"
        );
        assert!(
            !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::SourceRef),
            "TableExpr must NOT be a subtype of SourceRef (reverse direction forbidden)"
        );
    }

    /// `List<ModelRef> <: List<TableExpr>` — List covariance lifts the element rule
    /// automatically.
    #[test]
    fn list_of_model_ref_is_subtype_of_list_of_table_expr() {
        let list_model_ref = SmeltType::List(Box::new(SmeltType::ModelRef));
        let list_table_expr = SmeltType::List(Box::new(SmeltType::TableExpr(None)));
        assert!(
            is_subtype_of(&list_model_ref, &list_table_expr),
            "List<ModelRef> must be a subtype of List<TableExpr> via List covariance"
        );
        // Reverse does not hold.
        assert!(
            !is_subtype_of(&list_table_expr, &list_model_ref),
            "List<TableExpr> must NOT be a subtype of List<ModelRef>"
        );

        let list_source_ref = SmeltType::List(Box::new(SmeltType::SourceRef));
        assert!(
            is_subtype_of(&list_source_ref, &list_table_expr),
            "List<SourceRef> must be a subtype of List<TableExpr> via List covariance"
        );
    }

    /// `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` have the same field names and
    /// types in the same order (uniformity invariant from the design rationale).
    #[test]
    fn model_ref_and_source_ref_field_sets_are_identical_shape() {
        assert_eq!(
            MODEL_REF_FIELDS.len(),
            SOURCE_REF_FIELDS.len(),
            "MODEL_REF_FIELDS and SOURCE_REF_FIELDS must have the same number of fields"
        );
        for (i, ((model_name, model_ty), (source_name, source_ty))) in MODEL_REF_FIELDS
            .iter()
            .zip(SOURCE_REF_FIELDS.iter())
            .enumerate()
        {
            assert_eq!(
                model_name, source_name,
                "field {i}: MODEL_REF_FIELDS name '{model_name}' != SOURCE_REF_FIELDS name '{source_name}'"
            );
            assert_eq!(
                model_ty, source_ty,
                "field {i} ({model_name}): MODEL_REF_FIELDS type does not match SOURCE_REF_FIELDS type"
            );
        }
    }

    // =========================================================================
    // Phase E1 TDD tests — Record, Map, MAP_API_METHODS, SmeltRecordRegistry
    // =========================================================================

    /// Helper: build a `SmeltRecordDeclaration` with no source span for testing.
    fn make_decl(name: &str, fields: Vec<(&str, SmeltType)>) -> SmeltRecordDeclaration {
        use smelt_parser::TextRange;
        SmeltRecordDeclaration {
            name: name.to_string(),
            fields: fields
                .into_iter()
                .map(|(f, ty)| (f.to_string(), ty, TextRange::new(0.into(), 0.into())))
                .collect(),
            name_span: TextRange::new(0.into(), 0.into()),
            source_path: Arc::from("models/test.sql"),
        }
    }

    /// Helper: build a `SmeltType::Record` from a slice of `(name, SmeltType)` pairs.
    fn record_type(fields: &[(&str, SmeltType)]) -> SmeltType {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), v.clone());
        }
        SmeltType::Record {
            fields: map,
            name: None,
        }
    }

    fn named_record_type(name: &str, fields: &[(&str, SmeltType)]) -> SmeltType {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), v.clone());
        }
        SmeltType::Record {
            fields: map,
            name: Some(name.to_string()),
        }
    }

    fn expr_text() -> SmeltType {
        SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Text))
    }

    fn expr_integer() -> SmeltType {
        SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Integer))
    }

    fn expr_number() -> SmeltType {
        SmeltType::Expr(TypeConstraint::Numeric)
    }

    fn map_text_integer() -> SmeltType {
        SmeltType::Map {
            key: Box::new(expr_text()),
            value: Box::new(expr_integer()),
        }
    }

    fn map_text_number() -> SmeltType {
        SmeltType::Map {
            key: Box::new(expr_text()),
            value: Box::new(expr_number()),
        }
    }

    /// Test 1: `record_type_round_trips_field_order_canonicalised`
    ///
    /// `SmeltType::Record { fields: BTreeMap, name: Some("SourceEntry") }` constructed
    /// twice with field-insertion in different orders compares equal under `==`.
    /// The `Display` impl renders fields in lex order when `name` is `None`;
    /// as the type name when `name` is `Some`.
    #[test]
    fn record_type_round_trips_field_order_canonicalised() {
        // Build in two different insertion orders.
        let mut fields_a = BTreeMap::new();
        fields_a.insert("b".to_string(), expr_integer());
        fields_a.insert("a".to_string(), expr_text());

        let mut fields_b = BTreeMap::new();
        fields_b.insert("a".to_string(), expr_text());
        fields_b.insert("b".to_string(), expr_integer());

        let rec_a = SmeltType::Record {
            fields: fields_a,
            name: Some("SourceEntry".to_string()),
        };
        let rec_b = SmeltType::Record {
            fields: fields_b,
            name: Some("SourceEntry".to_string()),
        };

        assert_eq!(
            rec_a, rec_b,
            "Records with same fields in different insertion orders must be equal"
        );

        // Display: named → renders as type name.
        let display_named = format!("{rec_a}");
        assert_eq!(
            display_named, "SourceEntry",
            "Named record Display must render as the type name"
        );

        // Display: unnamed → renders in lex order.
        let rec_unnamed = record_type(&[("b", expr_integer()), ("a", expr_text())]);
        let display_unnamed = format!("{rec_unnamed}");
        // Lex order: a, b.
        assert!(
            display_unnamed.starts_with("Record<{"),
            "Unnamed record Display must start with Record<{{"
        );
        assert!(
            display_unnamed.contains("a:"),
            "Unnamed record Display must include field 'a'"
        );
        assert!(
            display_unnamed.contains("b:"),
            "Unnamed record Display must include field 'b'"
        );
        // 'a' must appear before 'b' in lex order.
        let a_pos = display_unnamed.find("a:").unwrap();
        let b_pos = display_unnamed.find("b:").unwrap();
        assert!(
            a_pos < b_pos,
            "Unnamed record Display must render fields in lex order (a before b)"
        );
    }

    /// Test 2: `record_inline_and_named_with_same_field_set_are_structurally_equal`
    ///
    /// Inline and named records with the same fields are structurally equal.
    /// The `name` field is accessible and distinguishable for hover.
    #[test]
    fn record_inline_and_named_with_same_field_set_are_structurally_equal() {
        let inline = record_type(&[("a", expr_text())]);
        let named = named_record_type("X", &[("a", expr_text())]);

        // Structural equality: equal (name ignored).
        assert_eq!(
            inline, named,
            "Inline record and named record with same fields must be structurally equal"
        );

        // The `name` field is accessible and distinguishable.
        let name_val = match &named {
            SmeltType::Record { name, .. } => name.as_deref(),
            _ => panic!("expected Record"),
        };
        assert_eq!(
            name_val,
            Some("X"),
            "Named record must expose its name via the `name` field"
        );

        let inline_name = match &inline {
            SmeltType::Record { name, .. } => name.as_deref(),
            _ => panic!("expected Record"),
        };
        assert_eq!(inline_name, None, "Inline record must have name = None");
    }

    /// Test 3: `map_type_invariant_both_axes`
    ///
    /// `Map<K, V>` is invariant in both `K` and `V`.
    #[test]
    fn map_type_invariant_both_axes() {
        // Map<Text, Integer> is NOT a subtype of Map<Text, Number> (covariance forbidden).
        assert!(
            !is_subtype_of(&map_text_integer(), &map_text_number()),
            "Map<Text, Integer> must NOT be subtype of Map<Text, Number> (invariance)"
        );
        // Map<Text, Number> is NOT a subtype of Map<Text, Integer> (contravariance forbidden).
        assert!(
            !is_subtype_of(&map_text_number(), &map_text_integer()),
            "Map<Text, Number> must NOT be subtype of Map<Text, Integer> (invariance)"
        );
        // Map<Text, Integer> IS a subtype of Map<Text, Integer> (reflexivity).
        assert!(
            is_subtype_of(&map_text_integer(), &map_text_integer()),
            "Map<Text, Integer> must be subtype of Map<Text, Integer> (reflexivity)"
        );
    }

    /// Test 4: `map_api_methods_registry_is_closed_and_exact`
    ///
    /// `MAP_API_METHODS` exposes exactly the five names: `{entries, keys, values, get, has}`.
    #[test]
    fn map_api_methods_registry_is_closed_and_exact() {
        let expected_names = ["entries", "keys", "values", "get", "has"];
        let actual_names: Vec<&str> = MAP_API_METHODS.iter().map(|m| m.name).collect();

        // Exact five names.
        assert_eq!(
            actual_names.len(),
            5,
            "MAP_API_METHODS must have exactly 5 entries"
        );
        for name in &expected_names {
            assert!(
                actual_names.contains(name),
                "MAP_API_METHODS must contain '{name}'"
            );
        }

        // Lookup of any other identifier returns None.
        assert!(
            lookup_map_api_method("filter").is_none(),
            "lookup of 'filter' must return None"
        );
        assert!(
            lookup_map_api_method("").is_none(),
            "lookup of '' must return None"
        );
        assert!(
            lookup_map_api_method("ENTRIES").is_none(),
            "lookup is case-sensitive; 'ENTRIES' must return None"
        );

        // Arities: entries/keys/values → Exact(0), get/has → Exact(1).
        let entries = lookup_map_api_method("entries").expect("entries must be in MAP_API_METHODS");
        assert_eq!(
            entries.arity,
            Arity::Exact(0),
            "entries arity must be Exact(0)"
        );
        assert!(
            !entries.named_args_allowed,
            "entries must not allow named args"
        );

        let keys = lookup_map_api_method("keys").expect("keys must be in MAP_API_METHODS");
        assert_eq!(keys.arity, Arity::Exact(0), "keys arity must be Exact(0)");

        let values = lookup_map_api_method("values").expect("values must be in MAP_API_METHODS");
        assert_eq!(
            values.arity,
            Arity::Exact(0),
            "values arity must be Exact(0)"
        );

        let get = lookup_map_api_method("get").expect("get must be in MAP_API_METHODS");
        assert_eq!(get.arity, Arity::Exact(1), "get arity must be Exact(1)");

        let has = lookup_map_api_method("has").expect("has must be in MAP_API_METHODS");
        assert_eq!(has.arity, Arity::Exact(1), "has arity must be Exact(1)");

        // Return type of `entries`: List<Record<{key: K, value: V}>>.
        let k = expr_text();
        let v = expr_integer();
        let entries_result = (entries.return_type_formula)(&k, &v);
        match &entries_result {
            SmeltType::List(inner) => match inner.as_ref() {
                SmeltType::Record { fields, .. } => {
                    assert_eq!(fields.len(), 2, "entries result record must have 2 fields");
                    assert!(
                        fields.contains_key("key"),
                        "entries result must have 'key' field"
                    );
                    assert!(
                        fields.contains_key("value"),
                        "entries result must have 'value' field"
                    );
                    assert_eq!(fields["key"], k, "entries 'key' field must be K");
                    assert_eq!(fields["value"], v, "entries 'value' field must be V");
                }
                other => panic!("entries result inner must be Record, got: {other:?}"),
            },
            other => panic!("entries result must be List, got: {other:?}"),
        }
    }

    /// Test 5: `record_width_subtyping_rule`
    ///
    /// Width subtyping: `{a: Text, b: Integer} <: {a: Text}` but not the reverse.
    #[test]
    fn record_width_subtyping_rule() {
        let wide = record_type(&[("a", expr_text()), ("b", expr_integer())]);
        let narrow = record_type(&[("a", expr_text())]);
        let incompatible = record_type(&[("a", expr_integer())]);

        // Wide <: Narrow (width subtyping).
        assert!(
            is_subtype_of(&wide, &narrow),
            "Record with more fields must be a subtype of record with fewer fields"
        );
        // Narrow is NOT <: Wide (missing field `b`).
        assert!(
            !is_subtype_of(&narrow, &wide),
            "Record with fewer fields must NOT be a subtype of record with more fields"
        );
        // Type mismatch on shared field.
        assert!(
            !is_subtype_of(&narrow, &incompatible),
            "Record with wrong field type must NOT be a subtype"
        );
    }

    /// Test 6: `record_subtyping_through_nested_field`
    ///
    /// Width subtyping composes through nested record fields.
    #[test]
    fn record_subtyping_through_nested_field() {
        // sub: Record{a: Record{x: Text, y: Integer}}
        // sup: Record{a: Record{x: Text}}
        let inner_wide = record_type(&[("x", expr_text()), ("y", expr_integer())]);
        let inner_narrow = record_type(&[("x", expr_text())]);

        let sub = record_type(&[("a", inner_wide)]);
        let sup = record_type(&[("a", inner_narrow)]);

        assert!(
            is_subtype_of(&sub, &sup),
            "Width subtyping must compose through nested record fields"
        );
    }

    /// Test 7: `smelt_record_registry_builder_detects_redefinition`
    ///
    /// Two declarations with the same name produce a redefinition sentinel.
    #[test]
    fn smelt_record_registry_builder_detects_redefinition() {
        let decl1 = make_decl("Foo", vec![("x", expr_text())]);
        let decl2 = make_decl("Foo", vec![("y", expr_integer())]);

        let (registry, sentinels) = build_record_registry(&[decl1, decl2]);

        // One redefinition sentinel.
        let redef_sentinels: Vec<_> = sentinels
            .iter()
            .filter(|s| s.code == RecordRegistryCode::SmeltRecordRedefinition)
            .collect();
        assert_eq!(
            redef_sentinels.len(),
            1,
            "Expected exactly one SmeltRecordRedefinition sentinel; got: {redef_sentinels:?}"
        );

        // First declaration is authoritative.
        let decl = registry.lookup("Foo").expect("Foo must be in registry");
        assert_eq!(
            decl.fields.len(),
            1,
            "First declaration (x field) must be authoritative"
        );
        assert_eq!(decl.fields[0].0, "x", "First declaration field must be 'x'");
    }

    /// Test 8: `smelt_record_registry_builder_detects_cycle_self`
    ///
    /// A single self-referential declaration emits one `RecordCyclicDeclaration` sentinel.
    #[test]
    fn smelt_record_registry_builder_detects_cycle_self() {
        // Node = {child: Node} — self-referential.
        // We model the field type as a named Record with name "Node".
        let node_field_ty = named_record_type("Node", &[]);
        let decl = make_decl("Node", vec![("child", node_field_ty)]);

        let (_, sentinels) = build_record_registry(&[decl]);

        let cycle_sentinels: Vec<_> = sentinels
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordCyclicDeclaration)
            .collect();
        assert_eq!(
            cycle_sentinels.len(),
            1,
            "Expected exactly one RecordCyclicDeclaration sentinel for self-cycle; got {cycle_sentinels:?}"
        );
        assert!(
            cycle_sentinels[0].message.contains("Node"),
            "Cycle sentinel message must mention the cycle participant 'Node'"
        );
    }

    /// Test 9: `smelt_record_registry_builder_detects_cycle_mutual`
    ///
    /// Two mutually referential declarations emit exactly one `RecordCyclicDeclaration` sentinel.
    #[test]
    fn smelt_record_registry_builder_detects_cycle_mutual() {
        // A = {b: B}, B = {a: A}
        let b_ref = named_record_type("B", &[]);
        let a_ref = named_record_type("A", &[]);

        let decl_a = make_decl("A", vec![("b", b_ref)]);
        let decl_b = make_decl("B", vec![("a", a_ref)]);

        let (_, sentinels) = build_record_registry(&[decl_a, decl_b]);

        let cycle_sentinels: Vec<_> = sentinels
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordCyclicDeclaration)
            .collect();
        assert_eq!(
            cycle_sentinels.len(),
            1,
            "Expected exactly one RecordCyclicDeclaration sentinel for mutual cycle; got {cycle_sentinels:?}"
        );
    }

    /// Test 10: `smelt_record_registry_builder_rejects_reflection_witness_field_types`
    ///
    /// A declaration with `ModelRef`, `ColumnRef`, or `SourceRef` field types emits
    /// `RecordFieldTypeForbidden`.
    #[test]
    fn smelt_record_registry_builder_rejects_reflection_witness_field_types() {
        // Cohort = {model: ModelRef}
        let decl_model = make_decl("Cohort", vec![("model", SmeltType::ModelRef)]);
        let (_, sentinels) = build_record_registry(&[decl_model]);
        let forbidden: Vec<_> = sentinels
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
            .collect();
        assert_eq!(
            forbidden.len(),
            1,
            "Expected one RecordFieldTypeForbidden for ModelRef field; got {forbidden:?}"
        );
        assert!(
            forbidden[0].message.contains("ModelRef"),
            "Forbidden sentinel message must mention 'ModelRef'"
        );

        // Same for ColumnRef.
        let decl_col = make_decl("Cohort", vec![("col", SmeltType::ColumnRef)]);
        let (_, sentinels2) = build_record_registry(&[decl_col]);
        assert_eq!(
            sentinels2
                .iter()
                .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
                .count(),
            1,
            "Expected one RecordFieldTypeForbidden for ColumnRef field"
        );

        // Same for SourceRef.
        let decl_src = make_decl("Cohort", vec![("src", SmeltType::SourceRef)]);
        let (_, sentinels3) = build_record_registry(&[decl_src]);
        assert_eq!(
            sentinels3
                .iter()
                .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
                .count(),
            1,
            "Expected one RecordFieldTypeForbidden for SourceRef field"
        );

        // Lambda is also forbidden.
        let lambda_ty = SmeltType::Lambda(vec![expr_text()], Box::new(expr_text()));
        let decl_lambda = make_decl("Cohort", vec![("fn_field", lambda_ty)]);
        let (_, sentinels4) = build_record_registry(&[decl_lambda]);
        assert_eq!(
            sentinels4
                .iter()
                .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
                .count(),
            1,
            "Expected one RecordFieldTypeForbidden for Lambda field"
        );
    }

    // ── ModelDef type system tests ────────────────────────────────────────────

    /// `MODEL_DEF_FIELDS` exposes exactly five names and each entry's type
    /// matches the spec table.
    #[test]
    fn model_def_fields_registry_is_closed_and_exact() {
        // Exact five names in the spec-defined set.
        let spec_names = ["name", "body", "materialization", "tags", "description"];
        assert_eq!(
            MODEL_DEF_FIELDS.len(),
            5,
            "MODEL_DEF_FIELDS must have exactly 5 entries; got {}",
            MODEL_DEF_FIELDS.len()
        );
        for name in &spec_names {
            assert!(
                model_def_field(name).is_some(),
                "MODEL_DEF_FIELDS must contain field '{name}'"
            );
        }
        // Unknown identifiers return None (closed-field invariant).
        assert!(
            model_def_field("incremental").is_none(),
            "MODEL_DEF_FIELDS must NOT contain 'incremental'"
        );
        assert!(
            model_def_field("owner").is_none(),
            "MODEL_DEF_FIELDS must NOT contain 'owner'"
        );

        // name → Expr<Text>
        let name_ty = model_def_field("name").unwrap();
        assert!(
            matches!(
                name_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "name field must be Expr<Text>, got: {name_ty:?}"
        );
        // body → TableExpr (the single carve-out)
        let body_ty = model_def_field("body").unwrap();
        assert!(
            matches!(body_ty, SmeltType::TableExpr(None)),
            "body field must be TableExpr, got: {body_ty:?}"
        );
        // materialization → Expr<Text>
        let mat_ty = model_def_field("materialization").unwrap();
        assert!(
            matches!(
                mat_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "materialization field must be Expr<Text>, got: {mat_ty:?}"
        );
        // tags → List<Expr<Text>>
        let tags_ty = model_def_field("tags").unwrap();
        assert!(
            matches!(tags_ty, SmeltType::List(inner)
                if matches!(inner.as_ref(), SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)))),
            "tags field must be List<Expr<Text>>, got: {tags_ty:?}"
        );
        // description → Expr<Text>
        let desc_ty = model_def_field("description").unwrap();
        assert!(
            matches!(
                desc_ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "description field must be Expr<Text>, got: {desc_ty:?}"
        );
    }

    /// `SmeltType::ModelDef` exposes `field_type(name)` returning the
    /// spec-declared type for each of the five names; returns `None` for unknown.
    #[test]
    fn model_def_smelt_type_round_trips_field_access() {
        let ty = SmeltType::ModelDef;
        // All five spec fields resolve.
        for field in &["name", "body", "materialization", "tags", "description"] {
            let from_static = model_def_field(field);
            assert!(
                from_static.is_some(),
                "model_def_field must return Some for '{field}'"
            );
        }
        // Unknown field returns None.
        assert!(
            model_def_field("unknown_xyz").is_none(),
            "model_def_field must return None for unknown field"
        );
        // Verify ModelDef equality with itself.
        assert_eq!(ty, SmeltType::ModelDef, "ModelDef must equal itself");
    }

    /// `SmeltType::ModelDef` does not unify with a structurally-identical
    /// `SmeltType::Record` in either direction.
    #[test]
    fn model_def_is_assignment_isolated_from_record() {
        use std::collections::BTreeMap;
        // Build a Record with the same five field names and types as ModelDef.
        let mut fields = BTreeMap::new();
        fields.insert(
            "name".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        fields.insert("body".to_string(), SmeltType::TableExpr(None));
        fields.insert(
            "materialization".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        fields.insert(
            "tags".to_string(),
            SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text,
            )))),
        );
        fields.insert(
            "description".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        );
        let record_twin = SmeltType::Record { fields, name: None };

        // ModelDef != Record even with identical fields.
        assert_ne!(
            SmeltType::ModelDef,
            record_twin,
            "ModelDef must not equal a structurally-identical Record"
        );
        // Subtype checks in both directions must fail.
        assert!(
            !is_subtype_of(&SmeltType::ModelDef, &record_twin),
            "ModelDef must NOT be a subtype of a structurally-identical Record"
        );
        assert!(
            !is_subtype_of(&record_twin, &SmeltType::ModelDef),
            "Record must NOT be a subtype of ModelDef"
        );
    }

    /// The `body` field in `MODEL_DEF_FIELDS` is `TableExpr` — the only
    /// carve-out that admits `TableExpr` in a record-like field position.
    #[test]
    fn model_def_admits_table_expr_in_body_field() {
        let body_ty = model_def_field("body").unwrap();
        assert!(
            matches!(body_ty, SmeltType::TableExpr(None)),
            "body field in MODEL_DEF_FIELDS must be TableExpr(None); got: {body_ty:?}"
        );
    }

    /// `SmeltType::ModelDef` is meta-only and not a data-world type.
    #[test]
    fn model_def_is_meta_only_does_not_reach_data_world() {
        assert!(
            is_meta_only_type(&SmeltType::ModelDef),
            "ModelDef must be meta-only"
        );
        assert!(
            !is_data_world_type(&SmeltType::ModelDef),
            "ModelDef must NOT be a data-world type"
        );
    }

    // === C26 lock-in: signature nullability (bare = nullable, NOT NULL = opt-in) ===

    /// A bare type annotation (`Expr<Integer>`) produces `not_null = false` on the
    /// parameter — bare annotations are nullable, NOT NULL is the opt-in (C26, §11).
    #[test]
    fn bare_annotation_is_nullable() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert!(
            !sigs[0].params[0].not_null,
            "bare Expr<Integer> annotation must have not_null=false (nullable by default); got not_null=true"
        );
        assert!(
            !sigs[0].return_not_null,
            "bare Expr<Integer> return annotation must have return_not_null=false; got true"
        );
    }

    /// A `NOT NULL` qualifier on a parameter annotation sets `not_null = true`
    /// — the opt-in mechanism for non-nullable signatures (C26, §11).
    #[test]
    fn not_null_annotation_opts_in() {
        let (file, text) = parse_file(
            "smelt.define f(x: Expr<Integer NOT NULL>) -> Expr<Integer NOT NULL> AS (x + 1)",
        );
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert!(
            sigs[0].params[0].not_null,
            "Expr<Integer NOT NULL> annotation must have not_null=true; got false"
        );
        assert!(
            sigs[0].return_not_null,
            "Expr<Integer NOT NULL> return annotation must have return_not_null=true; got false"
        );
    }
}
