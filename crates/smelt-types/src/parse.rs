//! Type string parsing
//!
//! Parses SQL type strings (e.g., "VARCHAR(255)", "DECIMAL(10,2)") into DataType.
//! Handles various SQL dialects and common aliases.

use crate::DataType;
use thiserror::Error;

/// Error parsing a type string
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypeParseError {
    #[error("empty type string")]
    EmptyString,
    #[error("unknown type: {0}")]
    UnknownType(String),
    #[error("invalid precision/scale for DECIMAL: {0}")]
    InvalidDecimal(String),
    #[error("invalid length for {type_name}: {value}")]
    InvalidLength { type_name: String, value: String },
    #[error("missing closing parenthesis")]
    MissingCloseParen,
}

/// Parse a SQL type string into a DataType
///
/// Supports common SQL type names and aliases:
/// - Numeric: INT, INTEGER, BIGINT, SMALLINT, FLOAT, DOUBLE, REAL, DECIMAL, NUMERIC
/// - String: VARCHAR, CHAR, TEXT, STRING
/// - Boolean: BOOLEAN, BOOL
/// - Date/Time: DATE, TIME, TIMESTAMP, TIMESTAMPTZ, INTERVAL
/// - Binary: BLOB, BYTEA, BINARY
///
/// # Examples
/// ```
/// use smelt_types::parse_type;
///
/// let ty = parse_type("INTEGER").unwrap();
/// let ty = parse_type("VARCHAR(255)").unwrap();
/// let ty = parse_type("DECIMAL(10,2)").unwrap();
/// let ty = parse_type("TIMESTAMP WITH TIME ZONE").unwrap();
/// ```
pub fn parse_type(type_str: &str) -> Result<DataType, TypeParseError> {
    let type_str = type_str.trim();
    if type_str.is_empty() {
        return Err(TypeParseError::EmptyString);
    }

    // Normalize to uppercase for matching
    let upper = type_str.to_uppercase();

    // Handle parameterized types first (those with parentheses)
    if let Some(paren_pos) = upper.find('(') {
        return parse_parameterized_type(&upper, paren_pos);
    }

    // Handle multi-word types
    if upper.starts_with("TIMESTAMP") {
        return parse_timestamp_type(&upper);
    }

    // Simple types without parameters
    match upper.as_str() {
        // Boolean
        "BOOLEAN" | "BOOL" => Ok(DataType::Boolean),

        // Integer types
        "TINYINT" | "INT1" => Ok(DataType::SmallInt), // Map to SmallInt
        "SMALLINT" | "INT2" => Ok(DataType::SmallInt),
        "INT" | "INTEGER" | "INT4" => Ok(DataType::Integer),
        "BIGINT" | "INT8" | "LONG" => Ok(DataType::BigInt),
        "HUGEINT" | "INT16" => Ok(DataType::BigInt), // DuckDB's 128-bit int -> BigInt

        // Floating point
        "REAL" | "FLOAT4" | "FLOAT" => Ok(DataType::Float),
        "DOUBLE" | "FLOAT8" | "DOUBLE PRECISION" => Ok(DataType::Double),

        // String types (without length)
        "VARCHAR" | "STRING" | "TEXT" => Ok(DataType::Varchar { max_length: None }),
        "CHAR" | "CHARACTER" => Ok(DataType::Char { length: 1 }),

        // Date/Time
        "DATE" => Ok(DataType::Date),
        "TIME" => Ok(DataType::Time),
        "TIMESTAMP" => Ok(DataType::Timestamp {
            with_timezone: false,
        }),
        "TIMESTAMPTZ" => Ok(DataType::Timestamp {
            with_timezone: true,
        }),
        "INTERVAL" => Ok(DataType::Interval),

        // Binary
        "BLOB" | "BYTEA" | "BINARY" | "VARBINARY" => Ok(DataType::Blob),

        // Numeric without precision defaults to DECIMAL(18,0)
        "NUMERIC" | "DECIMAL" => Ok(DataType::Decimal {
            precision: 18,
            scale: 0,
        }),

        _ => Err(TypeParseError::UnknownType(type_str.to_string())),
    }
}

fn parse_parameterized_type(upper: &str, paren_pos: usize) -> Result<DataType, TypeParseError> {
    let type_name = upper[..paren_pos].trim();
    let params_str = &upper[paren_pos + 1..];

    // Find closing paren
    let close_pos = params_str
        .find(')')
        .ok_or(TypeParseError::MissingCloseParen)?;
    let params = &params_str[..close_pos];

    match type_name {
        "VARCHAR" | "VARYING" | "CHARACTER VARYING" | "STRING" => {
            let length = parse_single_number(params, "VARCHAR")?;
            Ok(DataType::Varchar {
                max_length: Some(length),
            })
        }
        "CHAR" | "CHARACTER" => {
            let length = parse_single_number(params, "CHAR")?;
            Ok(DataType::Char { length })
        }
        "DECIMAL" | "NUMERIC" | "DEC" => parse_decimal_params(params),
        "FLOAT" => {
            // FLOAT(n) - if n <= 24, use Float; otherwise Double
            let precision = parse_single_number(params, "FLOAT")?;
            if precision <= 24 {
                Ok(DataType::Float)
            } else {
                Ok(DataType::Double)
            }
        }
        "TIME" => {
            // TIME(precision) - we ignore precision for now
            Ok(DataType::Time)
        }
        "TIMESTAMP" => {
            // TIMESTAMP(precision) - we ignore precision for now
            // Check for WITH TIME ZONE suffix after the closing paren
            let suffix = &params_str[close_pos + 1..].trim();
            let with_tz =
                suffix.starts_with("WITH TIME ZONE") || suffix.starts_with("WITH TIMEZONE");
            Ok(DataType::Timestamp {
                with_timezone: with_tz,
            })
        }
        _ => Err(TypeParseError::UnknownType(type_name.to_string())),
    }
}

fn parse_timestamp_type(upper: &str) -> Result<DataType, TypeParseError> {
    // Handle: TIMESTAMP, TIMESTAMPTZ, TIMESTAMP WITH TIME ZONE, TIMESTAMP WITHOUT TIME ZONE
    let with_tz = upper.contains("WITH TIME ZONE")
        || upper.contains("WITH TIMEZONE")
        || upper == "TIMESTAMPTZ";
    Ok(DataType::Timestamp {
        with_timezone: with_tz,
    })
}

fn parse_single_number(params: &str, type_name: &str) -> Result<u32, TypeParseError> {
    params
        .trim()
        .parse::<u32>()
        .map_err(|_| TypeParseError::InvalidLength {
            type_name: type_name.to_string(),
            value: params.to_string(),
        })
}

fn parse_decimal_params(params: &str) -> Result<DataType, TypeParseError> {
    let parts: Vec<&str> = params.split(',').map(|s| s.trim()).collect();

    match parts.len() {
        1 => {
            // DECIMAL(precision)
            let precision = parts[0]
                .parse::<u8>()
                .map_err(|_| TypeParseError::InvalidDecimal(params.to_string()))?;
            Ok(DataType::Decimal {
                precision,
                scale: 0,
            })
        }
        2 => {
            // DECIMAL(precision, scale)
            let precision = parts[0]
                .parse::<u8>()
                .map_err(|_| TypeParseError::InvalidDecimal(params.to_string()))?;
            let scale = parts[1]
                .parse::<u8>()
                .map_err(|_| TypeParseError::InvalidDecimal(params.to_string()))?;
            Ok(DataType::Decimal { precision, scale })
        }
        _ => Err(TypeParseError::InvalidDecimal(params.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_types() {
        assert_eq!(parse_type("INTEGER").unwrap(), DataType::Integer);
        assert_eq!(parse_type("int").unwrap(), DataType::Integer);
        assert_eq!(parse_type("BIGINT").unwrap(), DataType::BigInt);
        assert_eq!(parse_type("BOOLEAN").unwrap(), DataType::Boolean);
        assert_eq!(parse_type("bool").unwrap(), DataType::Boolean);
        assert_eq!(parse_type("DATE").unwrap(), DataType::Date);
        assert_eq!(
            parse_type("VARCHAR").unwrap(),
            DataType::Varchar { max_length: None }
        );
    }

    #[test]
    fn test_parse_varchar_with_length() {
        assert_eq!(
            parse_type("VARCHAR(255)").unwrap(),
            DataType::Varchar {
                max_length: Some(255)
            }
        );
        assert_eq!(
            parse_type("varchar(100)").unwrap(),
            DataType::Varchar {
                max_length: Some(100)
            }
        );
    }

    #[test]
    fn test_parse_char_with_length() {
        assert_eq!(
            parse_type("CHAR(10)").unwrap(),
            DataType::Char { length: 10 }
        );
        assert_eq!(parse_type("CHAR").unwrap(), DataType::Char { length: 1 });
    }

    #[test]
    fn test_parse_decimal() {
        assert_eq!(
            parse_type("DECIMAL(10,2)").unwrap(),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
        assert_eq!(
            parse_type("DECIMAL(18)").unwrap(),
            DataType::Decimal {
                precision: 18,
                scale: 0
            }
        );
        assert_eq!(
            parse_type("NUMERIC(5, 3)").unwrap(),
            DataType::Decimal {
                precision: 5,
                scale: 3
            }
        );
        // Without parameters
        assert_eq!(
            parse_type("DECIMAL").unwrap(),
            DataType::Decimal {
                precision: 18,
                scale: 0
            }
        );
    }

    #[test]
    fn test_parse_timestamp() {
        assert_eq!(
            parse_type("TIMESTAMP").unwrap(),
            DataType::Timestamp {
                with_timezone: false
            }
        );
        assert_eq!(
            parse_type("TIMESTAMP WITH TIME ZONE").unwrap(),
            DataType::Timestamp {
                with_timezone: true
            }
        );
        assert_eq!(
            parse_type("TIMESTAMPTZ").unwrap(),
            DataType::Timestamp {
                with_timezone: true
            }
        );
    }

    #[test]
    fn test_parse_float_precision() {
        assert_eq!(parse_type("FLOAT").unwrap(), DataType::Float);
        assert_eq!(parse_type("FLOAT(24)").unwrap(), DataType::Float);
        assert_eq!(parse_type("FLOAT(53)").unwrap(), DataType::Double);
    }

    #[test]
    fn test_parse_aliases() {
        assert_eq!(parse_type("INT").unwrap(), DataType::Integer);
        assert_eq!(parse_type("INT4").unwrap(), DataType::Integer);
        assert_eq!(parse_type("INT8").unwrap(), DataType::BigInt);
        assert_eq!(parse_type("REAL").unwrap(), DataType::Float);
        assert_eq!(parse_type("DOUBLE PRECISION").unwrap(), DataType::Double);
        assert_eq!(
            parse_type("TEXT").unwrap(),
            DataType::Varchar { max_length: None }
        );
        assert_eq!(
            parse_type("STRING").unwrap(),
            DataType::Varchar { max_length: None }
        );
    }

    #[test]
    fn test_parse_errors() {
        assert!(matches!(parse_type(""), Err(TypeParseError::EmptyString)));
        assert!(matches!(
            parse_type("FOOBAR"),
            Err(TypeParseError::UnknownType(_))
        ));
        assert!(matches!(
            parse_type("VARCHAR(abc)"),
            Err(TypeParseError::InvalidLength { .. })
        ));
        assert!(matches!(
            parse_type("DECIMAL(a,b)"),
            Err(TypeParseError::InvalidDecimal(_))
        ));
    }

    #[test]
    fn test_case_insensitivity() {
        assert_eq!(parse_type("integer").unwrap(), DataType::Integer);
        assert_eq!(parse_type("INTEGER").unwrap(), DataType::Integer);
        assert_eq!(parse_type("Integer").unwrap(), DataType::Integer);
        assert_eq!(
            parse_type("varchar(100)").unwrap(),
            DataType::Varchar {
                max_length: Some(100)
            }
        );
    }

    #[test]
    fn test_whitespace_handling() {
        assert_eq!(parse_type("  INTEGER  ").unwrap(), DataType::Integer);
        assert_eq!(
            parse_type("DECIMAL( 10 , 2 )").unwrap(),
            DataType::Decimal {
                precision: 10,
                scale: 2
            }
        );
    }
}
