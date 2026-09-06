use super::*;
use crate::DataType;
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

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
/// Invariant: exactly seven entries — `name`, `body`, `materialization`, `tags`,
/// `description`, `timeseries`, `safety_overrides` — in this canonical order.
/// This is the single source of truth for the v1 field set. Any future
/// addition requires a spec edit AND a change to this constant.
///
/// Field types:
/// - `name`             → `Expr<Text>`       — model identifier (`[A-Za-z0-9_]+`, non-empty)
/// - `body`             → `TableExpr`        — the only carve-out admitting `TableExpr` in a record field
/// - `materialization`  → `Expr<Text>`       — one of `view`, `table`, `incremental`
/// - `tags`              → `List<Expr<Text>>` — merged tag set
/// - `description`       → `Expr<Text>`       — human-readable description
/// - `timeseries`        → `Record{…}`        — per-emission override of the generator's
///   file-wide `timeseries:` frontmatter block; mirrors `TimeseriesConfig`. Whole-block
///   replacement, honoured only when `materialization == 'incremental'`.
/// - `safety_overrides`  → `Record{…}`        — per-emission override of the generator's
///   file-wide `safety_overrides:` block; mirrors `PartitionGrainSafetyOverrides`. Whole-block
///   replacement, honoured only when `materialization == 'incremental'`.
///
/// `timeseries` and `safety_overrides` are validated with bespoke required/optional
/// sub-field rules in `smelt-db`'s `type_inference::multi_model` (not through the
/// generic `check_record_literal` path, which treats every declared field as
/// required) — see `MODELDEF_TIMESERIES_OVERRIDE_FIELDS` /
/// `MODELDEF_SAFETY_OVERRIDES_OVERRIDE_FIELDS` there.
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
        (
            "timeseries",
            SmeltType::Record {
                fields: BTreeMap::new(),
                name: Some("ModelDef.timeseries".to_string()),
            },
        ),
        (
            "safety_overrides",
            SmeltType::Record {
                fields: BTreeMap::new(),
                name: Some("ModelDef.safety_overrides".to_string()),
            },
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

pub(super) static META_REGISTRY: LazyLock<HashMap<String, SmeltMetaSignature>> =
    LazyLock::new(|| {
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
