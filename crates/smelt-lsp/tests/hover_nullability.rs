//! Tests for hover rendering of nullability (Phase 6).
//!
//! Spec §11 Surface §Hover:
//!   Non-nullable columns display as `T NOT NULL`; nullable columns display
//!   as bare `T`. One shared canonical renderer is used for both hover and
//!   diagnostic messages so tracked axes are never silently dropped.

use smelt_lsp::hover::hover_text_for_column_reference;
use smelt_types::{DataType, TypedColumn};

// ─── hover_text_for_column_reference ────────────────────────────────────────

#[test]
fn hover_shows_not_null_for_non_nullable_column() {
    let tc = TypedColumn {
        data_type: DataType::Integer,
        nullable: false,
    };
    let text = hover_text_for_column_reference("id", Some(&tc), None);
    assert!(
        text.contains("INTEGER NOT NULL"),
        "non-nullable column should show 'INTEGER NOT NULL'; got: {text}"
    );
    assert!(
        !text.contains("INTEGER?"),
        "should not contain '?' suffix; got: {text}"
    );
}

#[test]
fn hover_bare_type_for_nullable_column() {
    let tc = TypedColumn {
        data_type: DataType::Text,
        nullable: true,
    };
    let text = hover_text_for_column_reference("name", Some(&tc), None);
    assert!(
        text.contains("`TEXT`"),
        "nullable column should show bare 'TEXT'; got: {text}"
    );
    assert!(
        !text.contains("NOT NULL"),
        "nullable column should not show 'NOT NULL'; got: {text}"
    );
    assert!(
        !text.contains("TEXT?"),
        "should not contain '?' suffix; got: {text}"
    );
}

#[test]
fn hover_left_join_column_drops_not_null() {
    // A column forced nullable by a LEFT JOIN (nullable: true) should hover
    // as bare `T`, not `T NOT NULL`.
    let tc = TypedColumn {
        data_type: DataType::Integer,
        nullable: true, // forced by LEFT JOIN
    };
    let text = hover_text_for_column_reference("event_id", Some(&tc), None);
    assert!(
        !text.contains("NOT NULL"),
        "left-join nullable column should not show 'NOT NULL'; got: {text}"
    );
}

// ─── format_typed_column_display ────────────────────────────────────────────

#[test]
fn renderer_not_null_suffix_matches_writable_syntax() {
    // The canonical renderer must produce exactly "INTEGER NOT NULL" for
    // non-nullable Integer — matching the writable signature syntax from Phase 5.
    let tc = TypedColumn {
        data_type: DataType::Integer,
        nullable: false,
    };
    assert_eq!(
        smelt_types::format_typed_column_display(&tc),
        "INTEGER NOT NULL"
    );

    // Nullable: bare type, no suffix.
    let tc2 = TypedColumn {
        data_type: DataType::Text,
        nullable: true,
    };
    assert_eq!(smelt_types::format_typed_column_display(&tc2), "TEXT");
}

#[test]
fn renderer_bigint_not_null() {
    let tc = TypedColumn::not_null(DataType::BigInt);
    assert_eq!(
        smelt_types::format_typed_column_display(&tc),
        "BIGINT NOT NULL"
    );
}

#[test]
fn renderer_varchar_nullable() {
    let tc = TypedColumn::nullable(DataType::Varchar { max_length: None });
    assert_eq!(smelt_types::format_typed_column_display(&tc), "VARCHAR");
}
