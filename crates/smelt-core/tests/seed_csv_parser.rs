//! Phase 4 TDD — strict CSV parser tests.
//!
//! These tests verify `smelt_core::seeds::csv::read_csv` behaviour:
//! - Strict defaults (comma delimiter, double-quote quoting, header required)
//! - UTF-8 BOM consumed silently
//! - Mixed CRLF/LF line endings
//! - Tab-delimited file → hard error
//! - Empty cells → NULL

use smelt_core::seeds::csv::{read_csv, SeedError};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_tmp(content: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content).unwrap();
    f.flush().unwrap();
    f
}

// ---------------------------------------------------------------------------
// strict_defaults
// ---------------------------------------------------------------------------

/// Consolidated test for the three strict-defaults sub-cases from `seeds.md`:
///
/// 1. UTF-8 BOM is consumed silently (first header is "id", not "\u{feff}id")
/// 2. LF and CRLF line endings are both accepted (mixed in same file)
/// 3. Tab-delimited file → hard `WrongDelimiter` parse error
#[test]
fn strict_defaults() {
    // Sub-case 1: BOM consumed silently
    {
        let content = b"\xEF\xBB\xBFid,name\n1,Alice\n";
        let f = write_tmp(content);
        let (headers, rows_iter) = read_csv(f.path()).unwrap();
        let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(headers[0], "id", "BOM must be stripped from first header");
        assert_eq!(rows.len(), 1);
    }

    // Sub-case 2: mixed CRLF/LF accepted
    {
        let content = b"id,name\r\n1,Alice\r\n2,Bob\n";
        let f = write_tmp(content);
        let (headers, rows_iter) = read_csv(f.path()).unwrap();
        let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(headers, vec!["id", "name"]);
        assert_eq!(rows.len(), 2);
    }

    // Sub-case 3: tab delimiter → hard error
    {
        let content = b"id\tname\n1\tAlice\n";
        let f = write_tmp(content);
        let result = read_csv(f.path());
        match result {
            Err(SeedError::WrongDelimiter { .. }) => {} // expected
            Ok((headers, _)) => panic!("expected WrongDelimiter error, got headers={:?}", headers),
            Err(other) => panic!("expected WrongDelimiter, got {:?}", other),
        }
    }
}

#[test]
fn bom_consumed_silently() {
    // UTF-8 BOM (\xEF\xBB\xBF) at start of file is stripped; first header
    // must be "id" not "\u{feff}id".
    let content = b"\xEF\xBB\xBFid,name\n1,Alice\n";
    let f = write_tmp(content);
    let (headers, rows_iter) = read_csv(f.path()).unwrap();
    let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(headers[0], "id", "BOM must be stripped from first header");
    assert_eq!(rows.len(), 1);
}

#[test]
fn mixed_crlf_lf_accepted() {
    // CRLF on first data row, LF on second — both must parse correctly.
    let content = b"id,name\r\n1,Alice\r\n2,Bob\n";
    let f = write_tmp(content);
    let (headers, rows_iter) = read_csv(f.path()).unwrap();
    let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(headers, vec!["id", "name"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].fields[0], Some("1".to_string()));
    assert_eq!(rows[1].fields[0], Some("2".to_string()));
}

#[test]
fn tab_delimiter_is_hard_error() {
    // Tab-delimited file → `SeedError::WrongDelimiter` (or similar hard error).
    // The test checks that `read_csv` returns an error, not that it panics.
    //
    // A tab-delimited file with `\t` between fields will be parsed by the
    // strict comma reader as a single-column file (the \t is part of the
    // value). That means two "columns" end up in the first "column" separated
    // by \t. We detect this by checking that the header count is 1 while the
    // raw line clearly has a tab.
    let content = b"id\tname\n1\tAlice\n";
    let f = write_tmp(content);
    let result = read_csv(f.path());
    match result {
        Err(SeedError::WrongDelimiter { .. }) => {} // expected
        Ok((headers, _)) => panic!("expected WrongDelimiter error, got headers={:?}", headers),
        Err(other) => panic!("expected WrongDelimiter, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// empty_cell_is_null
// ---------------------------------------------------------------------------

#[test]
fn empty_cell_is_null() {
    // Empty cells (nothing between commas, or trailing comma) produce None.
    // Test with an integer column, a varchar column, and a date column.
    let content = b"int_col,text_col,date_col\n,hello,\n42,,2025-01-01\n";
    let f = write_tmp(content);
    let (_headers, rows_iter) = read_csv(f.path()).unwrap();
    let rows: Vec<_> = rows_iter.collect::<Result<Vec<_>, _>>().unwrap();

    assert_eq!(rows.len(), 2);

    // Row 0: int_col is empty (NULL), text_col="hello", date_col is empty (NULL)
    assert_eq!(rows[0].fields[0], None, "empty int cell must be None");
    assert_eq!(
        rows[0].fields[1],
        Some("hello".to_string()),
        "non-empty text cell must be Some"
    );
    assert_eq!(rows[0].fields[2], None, "empty date cell must be None");

    // Row 1: int_col="42", text_col is empty (NULL), date_col="2025-01-01"
    assert_eq!(rows[1].fields[0], Some("42".to_string()));
    assert_eq!(rows[1].fields[1], None, "empty text cell must be None");
    assert_eq!(rows[1].fields[2], Some("2025-01-01".to_string()));
}
