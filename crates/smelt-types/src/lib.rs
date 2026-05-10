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
    ExprKind, FrameInfo, FrontmatterParseError, FunctionSig, ParamSpec, RowTail, RowVarBinding,
    SchemaRequirement, SigOrigin, SigParam, Signature, SignatureBuildError, SmeltMetaSignature,
    SmeltType, SmeltTypeParseError, StructRowTail, Tier, TypeConstraint, TypeExpr, TypeParam,
    UnificationError, UnifyResult, COLUMN_REF_FIELDS,
};

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
    /// Type could not be determined (error recovery)
    Unknown,
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

    /// Normalize this type to its canonical form for comparison.
    ///
    /// - `Text` → `Varchar { max_length: None }` (canonical string type)
    /// - Recursively normalizes Array elements, Struct fields, Map key/value
    /// - All other types are returned as-is
    pub fn normalize(&self) -> DataType {
        match self {
            DataType::Text => DataType::Varchar { max_length: None },
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
            DataType::Unknown => "UNKNOWN".to_string(),
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
        Self::nullable(DataType::Unknown)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
