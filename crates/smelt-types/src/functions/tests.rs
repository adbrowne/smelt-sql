use super::*;

#[test]
fn round_trip_all_variants() {
    for f in SqlFunction::all() {
        assert_eq!(
            SqlFunction::from_name(f.name()),
            Some(f),
            "round-trip failed for {:?} (name={})",
            f,
            f.name()
        );
    }
}

#[test]
fn from_name_case_insensitive() {
    assert_eq!(SqlFunction::from_name("count"), Some(SqlFunction::Count));
    assert_eq!(SqlFunction::from_name("Count"), Some(SqlFunction::Count));
    assert_eq!(SqlFunction::from_name("COUNT"), Some(SqlFunction::Count));
}

#[test]
fn from_name_unknown_returns_none() {
    assert_eq!(SqlFunction::from_name("not_a_function"), None);
}

#[test]
fn aggregate_classification() {
    assert!(SqlFunction::Count.is_aggregate());
    assert!(SqlFunction::Sum.is_aggregate());
    assert!(SqlFunction::Stddev.is_aggregate());
    assert!(!SqlFunction::RowNumber.is_aggregate());
    assert!(!SqlFunction::Upper.is_aggregate());
}

#[test]
fn window_classification() {
    assert!(SqlFunction::RowNumber.is_window());
    assert!(SqlFunction::Lag.is_window());
    assert!(SqlFunction::CumeDist.is_window());
    assert!(!SqlFunction::Count.is_window());
}

#[test]
fn all_variants_have_category() {
    // Ensures the category match is exhaustive at compile time
    // (it already is, but this test exercises the runtime path)
    for f in SqlFunction::all() {
        let _ = f.category();
    }
}

#[test]
fn json_dialect_aliases() {
    // PostgreSQL names
    assert_eq!(
        SqlFunction::from_name("json_build_object"),
        Some(SqlFunction::JsonObject)
    );
    assert_eq!(
        SqlFunction::from_name("json_build_array"),
        Some(SqlFunction::JsonArray)
    );
    assert_eq!(
        SqlFunction::from_name("to_jsonb"),
        Some(SqlFunction::ToJson)
    );
    assert_eq!(
        SqlFunction::from_name("row_to_json"),
        Some(SqlFunction::ToJson)
    );
    assert_eq!(
        SqlFunction::from_name("json_extract_path_text"),
        Some(SqlFunction::JsonExtractText)
    );

    // DuckDB names
    assert_eq!(
        SqlFunction::from_name("json_extract_string"),
        Some(SqlFunction::JsonExtractText)
    );
    assert_eq!(
        SqlFunction::from_name("json_keys"),
        Some(SqlFunction::JsonObjectKeys)
    );

    // Spark names
    assert_eq!(
        SqlFunction::from_name("get_json_object"),
        Some(SqlFunction::JsonExtractText)
    );

    // Canonical names
    assert_eq!(
        SqlFunction::from_name("json_object"),
        Some(SqlFunction::JsonObject)
    );
    assert_eq!(
        SqlFunction::from_name("json_extract"),
        Some(SqlFunction::JsonExtract)
    );
    assert_eq!(
        SqlFunction::from_name("json_contains"),
        Some(SqlFunction::JsonContains)
    );
}
