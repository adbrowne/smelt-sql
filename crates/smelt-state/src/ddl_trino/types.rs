//! Trino SQL spellings: types, identifier quoting, and dotted-path addressing.
//!
//! Every spelling here is a fact measured against a live coordinator by
//! `scripts/trino-probe-ddl.sh` (see the crate module header for how to run
//! it), not a reading of Trino's or Iceberg's documentation.

use smelt_types::DataType;

/// Quote one identifier for Trino: double quotes, embedded `"` doubled.
///
/// Standard SQL identifier quoting, which Trino follows — the same form
/// `smelt-backend-trino`'s (private) `quote_identifier` uses; this module
/// cannot depend on that crate (`smelt-state` sits below the backend
/// crates), so the same one-line rule is restated here.
pub(super) fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Build a catalog-qualified, double-quoted table name: `"cat"."sch"."tbl"`.
pub(super) fn qualified(catalog: &str, schema: &str, table: &str) -> String {
    format!(
        "{}.{}.{}",
        quote_ident(catalog),
        quote_ident(schema),
        quote_ident(table)
    )
}

/// Build a dotted column path for `ADD COLUMN`/`DROP COLUMN`/`ALTER COLUMN
/// … SET DATA TYPE`, quoting each segment independently.
///
/// Measured: `ALTER TABLE t ADD COLUMN "meta"."b" t`,
/// `ALTER TABLE t DROP COLUMN "meta"."b"`, and
/// `ALTER TABLE t ALTER COLUMN "items"."element"."a" SET DATA TYPE t` all
/// execute against a `ROW`/`ARRAY(ROW(..))` column — the literal path
/// segment `element` addresses an array's element type, exactly as it does
/// on `ADD COLUMN`.
pub(super) fn dotted_path(column: &str, path: &[String], leaf: Option<&str>) -> String {
    let mut parts = vec![quote_ident(column)];
    parts.extend(path.iter().map(|p| quote_ident(p)));
    if let Some(l) = leaf {
        parts.push(quote_ident(l));
    }
    parts.join(".")
}

/// Render a `DataType` as a Trino/Iceberg type name.
///
/// `Err` carries the reason the type has no Trino/Iceberg spelling, for a
/// refusal message — it is never a silent fallback.
pub fn trino_type_sql(dt: &DataType) -> Result<String, String> {
    Ok(match dt {
        DataType::Boolean => "BOOLEAN".to_string(),
        DataType::SmallInt => "SMALLINT".to_string(),
        DataType::Integer => "INTEGER".to_string(),
        DataType::BigInt => "BIGINT".to_string(),
        // Trino spells single precision `REAL`, not `FLOAT` — measured:
        // `CREATE TABLE (c REAL)` executes, `FLOAT` does not appear in any
        // accepted case in the probe.
        DataType::Float => "REAL".to_string(),
        DataType::Double => "DOUBLE".to_string(),
        DataType::Decimal { precision, scale } => format!("DECIMAL({},{})", precision, scale),
        // Bare, unbounded `VARCHAR` — measured accepted with no length.
        DataType::Varchar { max_length: None } => "VARCHAR".to_string(),
        DataType::Varchar {
            max_length: Some(n),
        } => format!("VARCHAR({})", n),
        DataType::Char { length } => format!("CHAR({})", length),
        // Trino has no separate unbounded-text type; unbounded `VARCHAR` is
        // the same type smelt's own `Text` collapses to elsewhere.
        DataType::Text => "VARCHAR".to_string(),
        DataType::Blob => "VARBINARY".to_string(),
        DataType::Date => "DATE".to_string(),
        DataType::Time => "TIME".to_string(),
        // Trino's default `TIMESTAMP` precision is 3; the generator always
        // spells `(6)` so a value smelt computes at microsecond precision
        // round-trips without silent truncation — measured accepted, both
        // with and without `WITH TIME ZONE`.
        DataType::Timestamp {
            with_timezone: false,
        } => "TIMESTAMP(6)".to_string(),
        DataType::Timestamp {
            with_timezone: true,
        } => "TIMESTAMP(6) WITH TIME ZONE".to_string(),
        // Measured: `CREATE TABLE (c INTERVAL DAY TO SECOND)` is refused
        // outright — `Type not supported for Iceberg: interval day to
        // second`. Iceberg has no interval type at all.
        DataType::Interval => return Err("Iceberg has no INTERVAL type".to_string()),
        // Parenthesised, not angle-bracketed — measured accepted.
        DataType::Array(inner) => format!("ARRAY({})", trino_type_sql(inner)?),
        DataType::Struct(fields) => {
            let rendered: Result<Vec<String>, String> = fields
                .iter()
                .map(|(name, ty)| Ok(format!("{} {}", name, trino_type_sql(ty)?)))
                .collect();
            format!("ROW({})", rendered?.join(", "))
        }
        DataType::Map(key, value) => {
            format!("MAP({}, {})", trino_type_sql(key)?, trino_type_sql(value)?)
        }
        DataType::Null => return Err("Trino has no NULL column type".to_string()),
        DataType::Unknown(reason) => {
            return Err(format!("type could not be inferred ({:?})", reason))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_are_the_measured_trino_spellings() {
        let cases = [
            (DataType::Boolean, "BOOLEAN"),
            (DataType::SmallInt, "SMALLINT"),
            (DataType::Integer, "INTEGER"),
            (DataType::BigInt, "BIGINT"),
            (DataType::Float, "REAL"),
            (DataType::Double, "DOUBLE"),
            (DataType::Text, "VARCHAR"),
            (DataType::Varchar { max_length: None }, "VARCHAR"),
            (
                DataType::Varchar {
                    max_length: Some(50),
                },
                "VARCHAR(50)",
            ),
            (DataType::Char { length: 3 }, "CHAR(3)"),
            (DataType::Blob, "VARBINARY"),
            (DataType::Date, "DATE"),
            (DataType::Time, "TIME"),
            (
                DataType::Timestamp {
                    with_timezone: false,
                },
                "TIMESTAMP(6)",
            ),
            (
                DataType::Timestamp {
                    with_timezone: true,
                },
                "TIMESTAMP(6) WITH TIME ZONE",
            ),
            (
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
                "DECIMAL(10,2)",
            ),
            (
                DataType::Array(Box::new(DataType::Integer)),
                "ARRAY(INTEGER)",
            ),
            (
                DataType::Struct(vec![("a".to_string(), DataType::Text)]),
                "ROW(a VARCHAR)",
            ),
            (
                DataType::Map(Box::new(DataType::Text), Box::new(DataType::Integer)),
                "MAP(VARCHAR, INTEGER)",
            ),
        ];
        for (dt, expected) in cases {
            assert_eq!(
                trino_type_sql(&dt).unwrap(),
                expected,
                "wrong Trino spelling for {:?}",
                dt
            );
        }
    }

    #[test]
    fn interval_has_no_trino_type() {
        let err = trino_type_sql(&DataType::Interval).unwrap_err();
        assert!(err.contains("INTERVAL"), "{err}");
    }

    #[test]
    fn quote_ident_doubles_an_embedded_quote() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn qualified_quotes_each_part_separately() {
        assert_eq!(
            qualified("iceberg", "smelt_dev", "t"),
            "\"iceberg\".\"smelt_dev\".\"t\""
        );
    }

    #[test]
    fn dotted_path_quotes_every_segment() {
        assert_eq!(
            dotted_path("meta", &["a".to_string()], Some("b")),
            "\"meta\".\"a\".\"b\""
        );
        assert_eq!(dotted_path("meta", &[], Some("b")), "\"meta\".\"b\"");
        assert_eq!(dotted_path("meta", &[], None), "\"meta\"");
    }
}
