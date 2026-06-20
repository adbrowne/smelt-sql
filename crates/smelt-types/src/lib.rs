//! Type system definitions for smelt
//!
//! This crate provides the core type representations used throughout smelt:
//! - `DataType`: SQL data types (INTEGER, VARCHAR, DECIMAL, etc.)
//! - `TypedColumn`: Column with type and nullability
//!
//! These types are used by:
//! - smelt-db for type checking and schema inference
//! - smelt-cli for source configuration
//! - smelt-lsp for type-aware editor features

mod functions;
mod parse;
pub mod signatures;

pub use functions::{FunctionCategory, SqlFunction};
pub use parse::{parse_type, TypeParseError};
pub use signatures::{
    column_ref_field, extract_extern_signature, extract_extern_signature_with_raw,
    extract_function_signature_by_name, extract_function_signatures,
    extract_function_signatures_with_raw, extract_signature, extract_signature_with_raw,
    format_smelt_type_hover, kind_ceiling, numeric_lub, parse_frontmatter_backends,
    parse_smelt_type, subkind_of, unify_call, unify_call_with_expected, BackendSet,
    BuiltinRegistry, ColumnRefFieldType, ColumnRefValue, ContextRef, DataTypeHash, DataTypeReq,
    ExprKind, FrameInfo, FrontmatterParseError, FunctionSig, ModelOrigin, ModelRefValue, ParamSpec,
    RowTail, RowVarBinding, SchemaRequirement, SigOrigin, SigParam, Signature, SignatureBuildError,
    SmeltMetaSignature, SmeltType, SmeltTypeParseError, SourceOrigin, SourceRefValue,
    StructRowTail, Tier, TypeConstraint, TypeExpr, TypeParam, UnificationError, UnifyResult,
    COLUMN_REF_FIELDS,
};

/// Reason a type resolved to `Unknown`.
///
/// `PartialEq`/`Eq`/`Hash` treat all reasons as equal so that two `Unknown`s
/// with different reasons compare and hash identically — the reason is
/// diagnostic metadata, not part of the type lattice's bottom identity.
#[derive(Debug, Clone, Copy)]
pub enum UnknownReason {
    /// Compiler-resolvable gap: a diagnostic fires at the origin.
    Unresolved,
    /// Legitimately unknowable (e.g. `Expr<Any>` return). No diagnostic.
    Dynamic,
    /// Unknown only because an upstream value was already Unknown.
    /// Reporting is origin-only; no re-emission.
    Propagated,
}

impl PartialEq for UnknownReason {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for UnknownReason {}
impl std::hash::Hash for UnknownReason {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {}
}

/// SQL data types supported by smelt
///
/// This enum represents the logical SQL types. Backend-specific variations
/// (e.g., DuckDB's HUGEINT) are mapped to these canonical types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataType {
    // Numeric types
    /// Boolean (TRUE/FALSE)
    Boolean,
    /// Small integer (2 bytes, -32768 to 32767)
    SmallInt,
    /// Integer (4 bytes)
    Integer,
    /// Big integer (8 bytes)
    BigInt,
    /// Single-precision floating point
    Float,
    /// Double-precision floating point
    Double,
    /// Exact decimal with precision and scale
    Decimal { precision: u8, scale: u8 },

    // String types
    /// Variable-length string with optional max length
    Varchar { max_length: Option<u32> },
    /// Fixed-length string
    Char { length: u32 },
    /// Unbounded text
    Text,

    // Binary types
    /// Binary large object
    Blob,

    // Date/Time types
    /// Calendar date (year, month, day)
    Date,
    /// Time of day
    Time,
    /// Timestamp (date + time)
    Timestamp { with_timezone: bool },
    /// Time interval
    Interval,

    // Complex types
    /// Array of elements
    Array(Box<DataType>),
    /// Struct with named fields: STRUCT(a INTEGER, b VARCHAR)
    Struct(Vec<(String, DataType)>),
    /// Map from key type to value type: MAP(VARCHAR, INTEGER)
    Map(Box<DataType>, Box<DataType>),

    // Special types
    /// NULL literal type
    Null,
    /// Type could not be determined; reason encodes whether a diagnostic fires.
    Unknown(UnknownReason),
}

impl DataType {
    /// Check if this type is numeric (supports arithmetic operations)
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            DataType::SmallInt
                | DataType::Integer
                | DataType::BigInt
                | DataType::Float
                | DataType::Double
                | DataType::Decimal { .. }
        )
    }

    /// Check if this type is a string type
    pub fn is_string(&self) -> bool {
        matches!(
            self,
            DataType::Varchar { .. } | DataType::Char { .. } | DataType::Text
        )
    }

    /// Check if this type is a `Decimal` type (any precision/scale).
    pub fn is_decimal(&self) -> bool {
        matches!(self, DataType::Decimal { .. })
    }

    /// Check if this type is an integer type (`SmallInt`, `Integer`, `BigInt`).
    ///
    /// `Float`, `Double`, and `Decimal` are excluded — those belong to the broader
    /// `is_numeric()` family.
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            DataType::SmallInt | DataType::Integer | DataType::BigInt
        )
    }

    /// Check if this type is `Boolean`.
    pub fn is_boolean(&self) -> bool {
        matches!(self, DataType::Boolean)
    }

    /// Check if this type is a complex/nested type (Array, Struct, Map)
    pub fn is_complex(&self) -> bool {
        matches!(
            self,
            DataType::Array(_) | DataType::Struct(_) | DataType::Map(_, _)
        )
    }

    /// Check if this type is a date/time type
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            DataType::Date | DataType::Time | DataType::Timestamp { .. } | DataType::Interval
        )
    }

    /// Check if this type is any variant of `Unknown`.
    pub fn is_unknown(&self) -> bool {
        matches!(self, DataType::Unknown(_))
    }

    /// Return the `UnknownReason` if this type is `Unknown`, otherwise `None`.
    pub fn unknown_reason(&self) -> Option<UnknownReason> {
        if let DataType::Unknown(r) = self {
            Some(*r)
        } else {
            None
        }
    }

    /// Construct an `Unknown` with reason `Unresolved` (diagnostic must fire at origin).
    pub fn unknown_unresolved() -> Self {
        DataType::Unknown(UnknownReason::Unresolved)
    }

    /// Construct an `Unknown` with reason `Dynamic` (legitimately unknowable, no diagnostic).
    pub fn unknown_dynamic() -> Self {
        DataType::Unknown(UnknownReason::Dynamic)
    }

    /// Construct an `Unknown` with reason `Propagated` (upstream was Unknown; no re-emission).
    pub fn unknown_propagated() -> Self {
        DataType::Unknown(UnknownReason::Propagated)
    }

    /// Normalize this type to its canonical form for comparison.
    ///
    /// - `Text`, `Char(_)`, `Varchar(_)` → `Varchar { max_length: None }` (canonical string type)
    ///   Length annotations and the varchar/char distinction are discarded for equality (§4 types.md).
    /// - Recursively normalizes Array elements, Struct fields, Map key/value
    /// - All other types are returned as-is
    pub fn normalize(&self) -> DataType {
        match self {
            DataType::Text | DataType::Char { .. } | DataType::Varchar { .. } => {
                DataType::Varchar { max_length: None }
            }
            DataType::Array(inner) => DataType::Array(Box::new(inner.normalize())),
            DataType::Struct(fields) => DataType::Struct(
                fields
                    .iter()
                    .map(|(name, dt)| (name.clone(), dt.normalize()))
                    .collect(),
            ),
            DataType::Map(k, v) => DataType::Map(Box::new(k.normalize()), Box::new(v.normalize())),
            other => other.clone(),
        }
    }

    /// Format as SQL type string for backend compilation.
    ///
    /// Translates smelt-internal types to what backends actually support:
    /// - `Text` → `"VARCHAR"` (backends don't distinguish Text from VARCHAR)
    pub fn to_backend_sql(&self) -> String {
        match self {
            DataType::Text => "VARCHAR".to_string(),
            other => other.to_sql(),
        }
    }

    /// Format as SQL type string for the default dialect
    pub fn to_sql(&self) -> String {
        match self {
            DataType::Boolean => "BOOLEAN".to_string(),
            DataType::SmallInt => "SMALLINT".to_string(),
            DataType::Integer => "INTEGER".to_string(),
            DataType::BigInt => "BIGINT".to_string(),
            DataType::Float => "FLOAT".to_string(),
            DataType::Double => "DOUBLE".to_string(),
            DataType::Decimal { precision, scale } => {
                if *scale == 0 {
                    format!("DECIMAL({precision})")
                } else {
                    format!("DECIMAL({precision},{scale})")
                }
            }
            DataType::Varchar { max_length: None } => "VARCHAR".to_string(),
            DataType::Varchar {
                max_length: Some(len),
            } => format!("VARCHAR({len})"),
            DataType::Char { length } => format!("CHAR({length})"),
            DataType::Text => "TEXT".to_string(),
            DataType::Blob => "BLOB".to_string(),
            DataType::Date => "DATE".to_string(),
            DataType::Time => "TIME".to_string(),
            DataType::Timestamp { with_timezone } => {
                if *with_timezone {
                    "TIMESTAMP WITH TIME ZONE".to_string()
                } else {
                    "TIMESTAMP".to_string()
                }
            }
            DataType::Interval => "INTERVAL".to_string(),
            DataType::Array(inner) => format!("{}[]", inner.to_sql()),
            DataType::Struct(fields) => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(name, dt)| format!("{} {}", name, dt.to_sql()))
                    .collect();
                format!("STRUCT({})", field_strs.join(", "))
            }
            DataType::Map(key, value) => {
                format!("MAP({}, {})", key.to_sql(), value.to_sql())
            }
            DataType::Null => "NULL".to_string(),
            DataType::Unknown(_) => "UNKNOWN".to_string(),
        }
    }
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_sql())
    }
}

/// A column with its data type and nullability
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedColumn {
    /// The SQL data type
    pub data_type: DataType,
    /// Whether the column can contain NULL values
    pub nullable: bool,
}

impl TypedColumn {
    /// Create a new typed column
    pub fn new(data_type: DataType, nullable: bool) -> Self {
        Self {
            data_type,
            nullable,
        }
    }

    /// Create a nullable column
    pub fn nullable(data_type: DataType) -> Self {
        Self::new(data_type, true)
    }

    /// Create a non-nullable column
    pub fn not_null(data_type: DataType) -> Self {
        Self::new(data_type, false)
    }

    /// Create an unknown type (for error recovery)
    pub fn unknown() -> Self {
        Self::nullable(DataType::Unknown(UnknownReason::Dynamic))
    }
}

impl std::fmt::Display for TypedColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.data_type)?;
        if !self.nullable {
            write!(f, " NOT NULL")?;
        }
        Ok(())
    }
}

/// Render a `TypedColumn` for display in hover text and diagnostic messages.
///
/// Non-nullable columns show as `T NOT NULL`; nullable columns show as bare `T`.
/// This is the single canonical renderer — route all hover and diagnostic
/// column-type display through this function so tracked axes are never silently
/// dropped from user-facing output.
///
/// The `NOT NULL` suffix matches the writable annotation syntax accepted by the
/// signature parser (`Expr<Integer NOT NULL>`).
pub fn format_typed_column_display(tc: &TypedColumn) -> String {
    tc.to_string()
}

/// Returns true iff `Decimal(p2, s2)` can losslessly hold a value from `Decimal(p1, s1)`.
///
/// Safe iff:
///   - `s2 >= s1`       — fractional-digit capacity doesn't shrink
///   - `(p2 - s2) >= (p1 - s1)` — integer-digit capacity doesn't shrink
///
/// Callers must ensure `p >= s` for both arguments (invariant of valid Decimal types).
pub fn decimal_widening_is_safe(p1: u8, s1: u8, p2: u8, s2: u8) -> bool {
    s2 >= s1 && (p2 - s2) >= (p1 - s1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_identity_is_reason_agnostic() {
        use std::collections::HashSet;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let a = DataType::Unknown(UnknownReason::Unresolved);
        let b = DataType::Unknown(UnknownReason::Dynamic);
        let c = DataType::Unknown(UnknownReason::Propagated);

        // All Unknown variants are equal regardless of reason
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a, c);

        // All Unknown variants hash identically (insert 3 differently-reasoned, get set of size 1)
        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        set.insert(c.clone());
        assert_eq!(set.len(), 1, "differently-reasoned Unknown values must deduplicate");

        // Also verify hash values are equal
        let hash_of = |dt: &DataType| -> u64 {
            let mut h = DefaultHasher::new();
            dt.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_of(&a), hash_of(&b));
        assert_eq!(hash_of(&b), hash_of(&c));
    }

    #[test]
    fn unknown_reason_is_readable() {
        let u = DataType::Unknown(UnknownReason::Unresolved);
        let d = DataType::Unknown(UnknownReason::Dynamic);
        let p = DataType::Unknown(UnknownReason::Propagated);

        assert_eq!(u.unknown_reason(), Some(UnknownReason::Unresolved));
        assert_eq!(d.unknown_reason(), Some(UnknownReason::Dynamic));
        assert_eq!(p.unknown_reason(), Some(UnknownReason::Propagated));
        assert_eq!(DataType::Integer.unknown_reason(), None);
    }

    #[test]
    fn lub_and_dedup_unaffected_by_reason() {
        // Two differently-reasoned Unknowns normalize to one in a set
        use std::collections::HashSet;
        let types: HashSet<DataType> = [
            DataType::Unknown(UnknownReason::Unresolved),
            DataType::Unknown(UnknownReason::Dynamic),
        ]
        .into();
        assert_eq!(types.len(), 1);
    }

    #[test]
    fn is_unknown_matches_any_reason() {
        assert!(DataType::Unknown(UnknownReason::Unresolved).is_unknown());
        assert!(DataType::Unknown(UnknownReason::Dynamic).is_unknown());
        assert!(DataType::Unknown(UnknownReason::Propagated).is_unknown());
        assert!(!DataType::Integer.is_unknown());
        assert!(!DataType::Null.is_unknown());
    }

    #[test]
    fn test_data_type_display() {
        assert_eq!(DataType::Integer.to_string(), "INTEGER");
        assert_eq!(
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
            .to_string(),
            "DECIMAL(10,2)"
        );
        assert_eq!(
            DataType::Varchar { max_length: None }.to_string(),
            "VARCHAR"
        );
        assert_eq!(
            DataType::Varchar {
                max_length: Some(255)
            }
            .to_string(),
            "VARCHAR(255)"
        );
        assert_eq!(
            DataType::Timestamp {
                with_timezone: true
            }
            .to_string(),
            "TIMESTAMP WITH TIME ZONE"
        );
        assert_eq!(
            DataType::Array(Box::new(DataType::Integer)).to_string(),
            "INTEGER[]"
        );
    }

    #[test]
    fn test_to_backend_sql_text_becomes_varchar() {
        assert_eq!(DataType::Text.to_backend_sql(), "VARCHAR");
        assert_eq!(DataType::Integer.to_backend_sql(), "INTEGER");
        assert_eq!(
            DataType::Varchar { max_length: None }.to_backend_sql(),
            "VARCHAR"
        );
    }

    #[test]
    fn test_is_numeric() {
        assert!(DataType::Integer.is_numeric());
        assert!(DataType::BigInt.is_numeric());
        assert!(DataType::Double.is_numeric());
        assert!(DataType::Decimal {
            precision: 10,
            scale: 2
        }
        .is_numeric());
        assert!(!DataType::Varchar { max_length: None }.is_numeric());
        assert!(!DataType::Date.is_numeric());
    }

    #[test]
    fn test_is_complex() {
        assert!(DataType::Array(Box::new(DataType::Integer)).is_complex());
        assert!(DataType::Struct(vec![("a".to_string(), DataType::Integer)]).is_complex());
        assert!(DataType::Map(
            Box::new(DataType::Varchar { max_length: None }),
            Box::new(DataType::Integer)
        )
        .is_complex());
        assert!(!DataType::Integer.is_complex());
        assert!(!DataType::Varchar { max_length: None }.is_complex());
        assert!(!DataType::Boolean.is_complex());
    }

    #[test]
    fn test_map_to_sql() {
        assert_eq!(
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Integer)
            )
            .to_sql(),
            "MAP(VARCHAR, INTEGER)"
        );
    }

    // === normalize() tests ===

    #[test]
    fn test_normalize_text_to_varchar() {
        assert_eq!(
            DataType::Text.normalize(),
            DataType::Varchar { max_length: None }
        );
    }

    #[test]
    fn test_normalize_scalar_unchanged() {
        assert_eq!(DataType::Integer.normalize(), DataType::Integer);
        assert_eq!(DataType::BigInt.normalize(), DataType::BigInt);
        assert_eq!(DataType::Boolean.normalize(), DataType::Boolean);
        assert_eq!(
            DataType::Varchar { max_length: None }.normalize(),
            DataType::Varchar { max_length: None }
        );
        assert_eq!(
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
            .normalize(),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
    }

    #[test]
    fn test_normalize_array_recursive() {
        // Array(Text) → Array(Varchar)
        let arr = DataType::Array(Box::new(DataType::Text));
        assert_eq!(
            arr.normalize(),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );

        // Array(Integer) unchanged
        let arr = DataType::Array(Box::new(DataType::Integer));
        assert_eq!(
            arr.normalize(),
            DataType::Array(Box::new(DataType::Integer))
        );
    }

    #[test]
    fn test_normalize_struct_recursive() {
        let s = DataType::Struct(vec![
            ("a".to_string(), DataType::Text),
            ("b".to_string(), DataType::Integer),
        ]);
        assert_eq!(
            s.normalize(),
            DataType::Struct(vec![
                ("a".to_string(), DataType::Varchar { max_length: None }),
                ("b".to_string(), DataType::Integer),
            ])
        );
    }

    #[test]
    fn test_normalize_map_recursive() {
        let m = DataType::Map(Box::new(DataType::Text), Box::new(DataType::Text));
        assert_eq!(
            m.normalize(),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Varchar { max_length: None })
            )
        );
    }

    #[test]
    fn test_normalize_deeply_nested() {
        // STRUCT(a STRUCT(x Text)) → STRUCT(a STRUCT(x Varchar))
        let s = DataType::Struct(vec![(
            "a".to_string(),
            DataType::Struct(vec![("x".to_string(), DataType::Text)]),
        )]);
        assert_eq!(
            s.normalize(),
            DataType::Struct(vec![(
                "a".to_string(),
                DataType::Struct(vec![(
                    "x".to_string(),
                    DataType::Varchar { max_length: None }
                )]),
            )])
        );
    }

    #[test]
    fn test_typed_column_display() {
        let col = TypedColumn::not_null(DataType::Integer);
        assert_eq!(col.to_string(), "INTEGER NOT NULL");

        let col = TypedColumn::nullable(DataType::Varchar {
            max_length: Some(100),
        });
        assert_eq!(col.to_string(), "VARCHAR(100)");
    }

    #[test]
    fn decimal_widening_safe_and_unsafe() {
        // Decimal(10,2) can hold Decimal(5,2): s2=2>=s1=2, (10-2)=8>=(5-2)=3 ✓
        assert!(decimal_widening_is_safe(5, 2, 10, 2));
        // Identity: same type holds itself
        assert!(decimal_widening_is_safe(5, 2, 5, 2));
        // Decimal(5,4) cannot hold Decimal(5,2): (5-4)=1 < (5-2)=3 — integer digits shrink
        assert!(!decimal_widening_is_safe(5, 2, 5, 4));
        // Scale shrinks: Decimal(10,1) cannot hold Decimal(10,2): s2=1 < s1=2
        assert!(!decimal_widening_is_safe(10, 2, 10, 1));
        // Both scale and integer digits increase: Decimal(15,3) can hold Decimal(5,2)
        assert!(decimal_widening_is_safe(5, 2, 15, 3));
    }

    #[test]
    fn char_normalizes_to_string_family() {
        let canonical = DataType::Varchar { max_length: None };
        assert_eq!(DataType::Char { length: 5 }.normalize(), canonical);
        assert_eq!(DataType::Text.normalize(), canonical);
        assert_eq!(
            DataType::Varchar { max_length: None }.normalize(),
            canonical
        );
        assert_eq!(
            DataType::Char { length: 5 }.normalize(),
            DataType::Text.normalize()
        );
        assert_eq!(
            DataType::Char { length: 5 }.normalize(),
            DataType::Varchar {
                max_length: Some(10)
            }
            .normalize()
        );
    }
}
