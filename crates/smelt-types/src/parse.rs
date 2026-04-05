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
    #[error("invalid STRUCT: {0}")]
    InvalidStruct(String),
    #[error("invalid MAP: {0}")]
    InvalidMap(String),
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

    let upper = type_str.to_uppercase();
    parse_type_inner(&upper)
}

/// Recursive inner parser that handles complex types
fn parse_type_inner(upper: &str) -> Result<DataType, TypeParseError> {
    let upper = upper.trim();
    if upper.is_empty() {
        return Err(TypeParseError::EmptyString);
    }

    // Check for [] suffix (array bracket notation) — peel from right
    if upper.ends_with("[]") {
        let inner = upper[..upper.len() - 2].trim();
        let inner_type = parse_type_inner(inner)?;
        return Ok(DataType::Array(Box::new(inner_type)));
    }

    // Check for " ARRAY" suffix (SQL standard notation)
    // Must not match "ARRAY" by itself or "ARRAY(...)"
    if upper.ends_with(" ARRAY") {
        let inner = upper[..upper.len() - 6].trim();
        if !inner.is_empty() {
            let inner_type = parse_type_inner(inner)?;
            return Ok(DataType::Array(Box::new(inner_type)));
        }
    }

    // Handle STRUCT(...) — must use matching-paren logic
    if upper.starts_with("STRUCT(") || upper.starts_with("STRUCT (") {
        let open = upper.find('(').unwrap();
        let close = find_matching_paren(upper, open).ok_or(TypeParseError::MissingCloseParen)?;
        // There should be nothing after the closing paren ([] was already handled above)
        let trailing = upper[close + 1..].trim();
        if !trailing.is_empty() {
            return Err(TypeParseError::InvalidStruct(format!(
                "unexpected trailing characters: {trailing}"
            )));
        }
        let fields_str = &upper[open + 1..close];
        return parse_struct_fields(fields_str);
    }

    // Handle MAP(...)
    if upper.starts_with("MAP(") || upper.starts_with("MAP (") {
        let open = upper.find('(').unwrap();
        let close = find_matching_paren(upper, open).ok_or(TypeParseError::MissingCloseParen)?;
        let trailing = upper[close + 1..].trim();
        if !trailing.is_empty() {
            return Err(TypeParseError::InvalidMap(format!(
                "unexpected trailing characters: {trailing}"
            )));
        }
        let params_str = &upper[open + 1..close];
        return parse_map_params(params_str);
    }

    // Handle ARRAY(...) prefix notation (Spark style)
    if upper.starts_with("ARRAY(") || upper.starts_with("ARRAY (") {
        let open = upper.find('(').unwrap();
        let close = find_matching_paren(upper, open).ok_or(TypeParseError::MissingCloseParen)?;
        let trailing = upper[close + 1..].trim();
        if !trailing.is_empty() {
            return Err(TypeParseError::UnknownType(upper.to_string()));
        }
        let inner_str = &upper[open + 1..close];
        let inner_type = parse_type_inner(inner_str)?;
        return Ok(DataType::Array(Box::new(inner_type)));
    }

    // Handle parameterized scalar types (VARCHAR(...), DECIMAL(...), etc.)
    if let Some(paren_pos) = upper.find('(') {
        return parse_parameterized_type(upper, paren_pos);
    }

    // Handle multi-word types
    if upper.starts_with("TIMESTAMP") {
        return parse_timestamp_type(upper);
    }

    // Simple types without parameters
    parse_simple_type(upper)
}

/// Parse simple (non-parameterized) types
fn parse_simple_type(upper: &str) -> Result<DataType, TypeParseError> {
    match upper {
        // Boolean
        "BOOLEAN" | "BOOL" => Ok(DataType::Boolean),

        // Integer types
        "TINYINT" | "INT1" => Ok(DataType::SmallInt),
        "SMALLINT" | "INT2" => Ok(DataType::SmallInt),
        "INT" | "INTEGER" | "INT4" => Ok(DataType::Integer),
        "BIGINT" | "INT8" | "LONG" => Ok(DataType::BigInt),
        "HUGEINT" | "INT16" => Ok(DataType::BigInt),

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

        _ => Err(TypeParseError::UnknownType(upper.to_string())),
    }
}

/// Find the matching closing parenthesis for the opening paren at `open_pos`
fn find_matching_paren(s: &str, open_pos: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s[open_pos..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_pos + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a string by commas at the top level (not inside nested parentheses or brackets)
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// Parse STRUCT field list: "a INTEGER, b VARCHAR" → vec of (name, DataType)
fn parse_struct_fields(fields_str: &str) -> Result<DataType, TypeParseError> {
    let fields_str = fields_str.trim();
    if fields_str.is_empty() {
        return Err(TypeParseError::InvalidStruct(
            "empty field list".to_string(),
        ));
    }

    let parts = split_top_level_commas(fields_str);
    let mut fields = Vec::new();

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            return Err(TypeParseError::InvalidStruct("empty field".to_string()));
        }

        // Split into name and type: first whitespace-delimited token is the name,
        // everything after is the type string
        let first_space = part.find(|c: char| c.is_whitespace());
        match first_space {
            Some(pos) => {
                let name = part[..pos].trim().to_lowercase();
                let type_str = part[pos..].trim();
                let dt = parse_type_inner(type_str)?;
                fields.push((name, dt));
            }
            None => {
                return Err(TypeParseError::InvalidStruct(format!(
                    "field '{}' is missing a type",
                    part
                )));
            }
        }
    }

    Ok(DataType::Struct(fields))
}

/// Parse MAP parameters: "KEY_TYPE, VALUE_TYPE"
fn parse_map_params(params_str: &str) -> Result<DataType, TypeParseError> {
    let params_str = params_str.trim();
    if params_str.is_empty() {
        return Err(TypeParseError::InvalidMap(
            "empty parameter list".to_string(),
        ));
    }

    let parts = split_top_level_commas(params_str);
    if parts.len() != 2 {
        return Err(TypeParseError::InvalidMap(format!(
            "expected 2 type parameters, got {}",
            parts.len()
        )));
    }

    let key_type = parse_type_inner(parts[0].trim())?;
    let value_type = parse_type_inner(parts[1].trim())?;

    Ok(DataType::Map(Box::new(key_type), Box::new(value_type)))
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

    // === Complex type parsing tests ===

    #[test]
    fn test_parse_array_bracket_notation() {
        assert_eq!(
            parse_type("INTEGER[]").unwrap(),
            DataType::Array(Box::new(DataType::Integer))
        );
        assert_eq!(
            parse_type("VARCHAR[]").unwrap(),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );
        assert_eq!(
            parse_type("BOOLEAN[]").unwrap(),
            DataType::Array(Box::new(DataType::Boolean))
        );
    }

    #[test]
    fn test_parse_array_suffix_notation() {
        assert_eq!(
            parse_type("INTEGER ARRAY").unwrap(),
            DataType::Array(Box::new(DataType::Integer))
        );
        assert_eq!(
            parse_type("VARCHAR ARRAY").unwrap(),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );
    }

    #[test]
    fn test_parse_array_prefix_notation() {
        assert_eq!(
            parse_type("ARRAY(INTEGER)").unwrap(),
            DataType::Array(Box::new(DataType::Integer))
        );
        assert_eq!(
            parse_type("ARRAY(VARCHAR)").unwrap(),
            DataType::Array(Box::new(DataType::Varchar { max_length: None }))
        );
    }

    #[test]
    fn test_parse_nested_arrays() {
        assert_eq!(
            parse_type("BIGINT[][]").unwrap(),
            DataType::Array(Box::new(DataType::Array(Box::new(DataType::BigInt))))
        );
    }

    #[test]
    fn test_parse_struct_simple() {
        assert_eq!(
            parse_type("STRUCT(a INTEGER, b VARCHAR)").unwrap(),
            DataType::Struct(vec![
                ("a".to_string(), DataType::Integer),
                ("b".to_string(), DataType::Varchar { max_length: None }),
            ])
        );
    }

    #[test]
    fn test_parse_struct_aliases() {
        // INT should resolve to INTEGER, BOOL to BOOLEAN
        assert_eq!(
            parse_type("STRUCT(a INT, b BOOL)").unwrap(),
            DataType::Struct(vec![
                ("a".to_string(), DataType::Integer),
                ("b".to_string(), DataType::Boolean),
            ])
        );
    }

    #[test]
    fn test_parse_struct_with_array_field() {
        assert_eq!(
            parse_type("STRUCT(a INTEGER[])").unwrap(),
            DataType::Struct(vec![(
                "a".to_string(),
                DataType::Array(Box::new(DataType::Integer))
            ),])
        );
    }

    #[test]
    fn test_parse_nested_struct() {
        assert_eq!(
            parse_type("STRUCT(a STRUCT(x INTEGER))").unwrap(),
            DataType::Struct(vec![(
                "a".to_string(),
                DataType::Struct(vec![("x".to_string(), DataType::Integer),])
            ),])
        );
    }

    #[test]
    fn test_parse_struct_array() {
        // Array of structs
        assert_eq!(
            parse_type("STRUCT(a INTEGER, b VARCHAR)[]").unwrap(),
            DataType::Array(Box::new(DataType::Struct(vec![
                ("a".to_string(), DataType::Integer),
                ("b".to_string(), DataType::Varchar { max_length: None }),
            ])))
        );
    }

    #[test]
    fn test_parse_map_simple() {
        assert_eq!(
            parse_type("MAP(VARCHAR, INTEGER)").unwrap(),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Integer)
            )
        );
    }

    #[test]
    fn test_parse_map_with_complex_value() {
        assert_eq!(
            parse_type("MAP(VARCHAR, STRUCT(a INTEGER))").unwrap(),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Struct(
                    vec![("a".to_string(), DataType::Integer),]
                ))
            )
        );
    }

    #[test]
    fn test_parse_struct_with_map_field() {
        assert_eq!(
            parse_type("STRUCT(a INTEGER[], b MAP(VARCHAR, INTEGER))").unwrap(),
            DataType::Struct(vec![
                (
                    "a".to_string(),
                    DataType::Array(Box::new(DataType::Integer))
                ),
                (
                    "b".to_string(),
                    DataType::Map(
                        Box::new(DataType::Varchar { max_length: None }),
                        Box::new(DataType::Integer)
                    )
                ),
            ])
        );
    }

    #[test]
    fn test_parse_deeply_nested() {
        // STRUCT(a STRUCT(x INTEGER, y VARCHAR), b BIGINT)
        assert_eq!(
            parse_type("STRUCT(a STRUCT(x INTEGER, y VARCHAR), b BIGINT)").unwrap(),
            DataType::Struct(vec![
                (
                    "a".to_string(),
                    DataType::Struct(vec![
                        ("x".to_string(), DataType::Integer),
                        ("y".to_string(), DataType::Varchar { max_length: None }),
                    ])
                ),
                ("b".to_string(), DataType::BigInt),
            ])
        );
    }

    #[test]
    fn test_parse_struct_with_decimal_field() {
        // DECIMAL(10,2) has commas inside parens — must not split on them
        assert_eq!(
            parse_type("STRUCT(a DECIMAL(10,2), b INTEGER)").unwrap(),
            DataType::Struct(vec![
                (
                    "a".to_string(),
                    DataType::Decimal {
                        precision: 10,
                        scale: 2
                    }
                ),
                ("b".to_string(), DataType::Integer),
            ])
        );
    }

    #[test]
    fn test_parse_complex_type_errors() {
        assert!(parse_type("STRUCT()").is_err());
        assert!(parse_type("STRUCT(a)").is_err()); // missing type
        assert!(parse_type("MAP(VARCHAR)").is_err()); // missing value type
        assert!(parse_type("MAP()").is_err());
    }

    #[test]
    fn test_round_trip_all_types() {
        // Verify: DataType → to_sql() → parse_type() → DataType for all canonical types.
        // NOTE: DataType::Text is NOT round-trip safe (Text.to_sql() = "TEXT",
        // parse_type("TEXT") = Varchar). Use normalize() before round-trip testing.
        let types = vec![
            DataType::Boolean,
            DataType::SmallInt,
            DataType::Integer,
            DataType::BigInt,
            DataType::Float,
            DataType::Double,
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            DataType::Decimal {
                precision: 18,
                scale: 0,
            },
            DataType::Varchar { max_length: None },
            DataType::Varchar {
                max_length: Some(255),
            },
            DataType::Char { length: 10 },
            DataType::Date,
            DataType::Time,
            DataType::Timestamp {
                with_timezone: false,
            },
            DataType::Timestamp {
                with_timezone: true,
            },
            DataType::Interval,
            DataType::Blob,
            // Complex types
            DataType::Array(Box::new(DataType::Integer)),
            DataType::Array(Box::new(DataType::Array(Box::new(DataType::BigInt)))),
            DataType::Struct(vec![
                ("a".to_string(), DataType::Integer),
                ("b".to_string(), DataType::Varchar { max_length: None }),
            ]),
            DataType::Struct(vec![(
                "nested".to_string(),
                DataType::Struct(vec![("x".to_string(), DataType::BigInt)]),
            )]),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Integer),
            ),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Struct(vec![("a".to_string(), DataType::Integer)])),
            ),
            // Array of struct
            DataType::Array(Box::new(DataType::Struct(vec![
                ("id".to_string(), DataType::Integer),
                ("name".to_string(), DataType::Varchar { max_length: None }),
            ]))),
        ];

        for dt in &types {
            let sql = dt.to_sql();
            let parsed = parse_type(&sql).unwrap_or_else(|e| {
                panic!(
                    "Failed to parse to_sql() output '{}' for {:?}: {}",
                    sql, dt, e
                )
            });
            assert_eq!(
                dt, &parsed,
                "Round-trip failed for {:?}: to_sql()='{}', parsed back={:?}",
                dt, sql, parsed
            );
        }
    }

    #[test]
    fn test_round_trip_normalized_text() {
        // Text normalizes to Varchar, which does round-trip
        let dt = DataType::Text.normalize();
        let sql = dt.to_sql();
        let parsed = parse_type(&sql).unwrap();
        assert_eq!(dt, parsed);
    }

    #[test]
    fn test_parse_complex_case_insensitive() {
        assert_eq!(
            parse_type("struct(a integer)").unwrap(),
            DataType::Struct(vec![("a".to_string(), DataType::Integer),])
        );
        assert_eq!(
            parse_type("map(varchar, integer)").unwrap(),
            DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Integer)
            )
        );
    }
}
