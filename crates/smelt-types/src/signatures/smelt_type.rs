use super::*;
use crate::DataType;
use std::collections::BTreeMap;

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
    /// The type has exactly seven fields (see [`MODEL_DEF_FIELDS`]):
    ///   - `name: Text` — model identifier (`[A-Za-z0-9_]+`, non-empty)
    ///   - `body: TableExpr` — the model's SQL body (the single carve-out
    ///     admitting `TableExpr` in a record-like field position)
    ///   - `materialization: Text` — one of `view`, `table`, `incremental`
    ///   - `tags: List<Text>` — tag set (merges with workspace-level overlays)
    ///   - `description: Text` — human-readable description
    ///   - `timeseries: Record{…}` — optional per-emission override of the
    ///     generator's file-wide `timeseries:` block; incremental-only
    ///   - `safety_overrides: Record{…}` — optional per-emission override of the
    ///     generator's file-wide `safety_overrides:` block; incremental-only
    ///
    /// **Meta-only**: values never reach the database engine.
    ///
    /// **Closed**: the v1 field set is exactly `{name, body, materialization, tags,
    /// description, timeseries, safety_overrides}`. Adding a field requires a
    /// spec edit and a compiler change.
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
