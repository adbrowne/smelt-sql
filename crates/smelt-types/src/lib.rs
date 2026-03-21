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

pub use functions::{FunctionCategory, SqlFunction};
pub use parse::{parse_type, TypeParseError};

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

    // Complex types (future expansion)
    /// Array of elements
    Array(Box<DataType>),

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

    /// Check if this type is a date/time type
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            DataType::Date | DataType::Time | DataType::Timestamp { .. } | DataType::Interval
        )
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
    fn test_typed_column_display() {
        let col = TypedColumn::not_null(DataType::Integer);
        assert_eq!(col.to_string(), "INTEGER NOT NULL");

        let col = TypedColumn::nullable(DataType::Varchar {
            max_length: Some(100),
        });
        assert_eq!(col.to_string(), "VARCHAR(100)");
    }
}
