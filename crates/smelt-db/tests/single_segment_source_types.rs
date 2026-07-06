//! Regression test for design fork F4
//! (`docs/research/20260705-refresh-as-maintenance-plan/03-design-forks.md`,
//! ledger cell G-01 aside): `add_source_info_to_type_context` silently
//! skipped every source whose `address_segments` had fewer than two entries.
//! A source YAML at scan root gets a **single-segment** address by
//! construction (`smelt-core/src/sources.rs` derives segments from the file
//! stem), so all of its declared column types were dropped from the
//! `TypeContext`; `SUM(val)` over a declared `DOUBLE` then fell through to
//! the historical `BigInt` default and `wrap_with_type_casts` emitted
//! `CAST(total AS BIGINT)` — silently truncating fractional aggregates.
//!
//! This is the uncovered variant of regression-triage bug #3 (the *empty*
//! TypeContext, pinned by `tests/proptests/aggregate_widening.rs`): here the
//! context is populated but one source's rows were dropped by an arity check.

use std::path::PathBuf;

use smelt_core::sources::{SourceColumn, SourceInfo};
use smelt_db::type_inference::TypeContext;
use smelt_db::{add_source_info_to_type_context, infer_select_column_types};
use smelt_parser::ast::File;
use smelt_types::DataType;

fn source_info(segments: &[&str], columns: &[(&str, DataType)]) -> SourceInfo {
    SourceInfo {
        path: PathBuf::from("/tmp/test.yml"),
        address_segments: segments.iter().map(|s| s.to_string()).collect(),
        columns: columns
            .iter()
            .map(|(name, dt)| SourceColumn {
                name: name.to_string(),
                data_type: dt.clone(),
                nullable: true,
                description: None,
            })
            .collect(),
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: None,
        mutation_profile: None,
        source_lateness: None,
    }
}

fn infer(sql: &str, ctx: &TypeContext) -> Vec<smelt_types::TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, ctx)
}

/// The F4 red test: a single-segment source (scan-root YAML) with a DOUBLE
/// column must type-resolve — `SUM(val)` stays Double-family, never the
/// BigInt fallback.
#[test]
fn single_segment_source_columns_reach_the_type_context() {
    let src = source_info(&["payments"], &[("val", DataType::Double)]);
    let mut ctx = TypeContext::new();
    add_source_info_to_type_context(&[src], &mut ctx);

    let types = infer("SELECT SUM(val) AS total FROM payments", &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "SUM over a single-segment source's DOUBLE column must stay Double — \
         the BigInt fallback means the declared columns were dropped (F4)"
    );
}

/// The two-segment path keeps working exactly as before.
#[test]
fn two_segment_source_columns_still_resolve() {
    let src = source_info(&["raw", "payments"], &[("val", DataType::Double)]);
    let mut ctx = TypeContext::new();
    add_source_info_to_type_context(&[src], &mut ctx);

    let qualified = infer("SELECT SUM(val) AS total FROM raw.payments", &ctx);
    assert_eq!(qualified[0].data_type, DataType::Double);
    let simple = infer("SELECT SUM(val) AS total FROM payments", &ctx);
    assert_eq!(simple[0].data_type, DataType::Double);
}

/// Deeper addresses resolve by their last two segments (unchanged).
#[test]
fn three_segment_source_resolves_by_last_two_segments() {
    let src = source_info(
        &["sources", "raw", "payments"],
        &[("val", DataType::Double)],
    );
    let mut ctx = TypeContext::new();
    add_source_info_to_type_context(&[src], &mut ctx);

    let types = infer("SELECT SUM(val) AS total FROM raw.payments", &ctx);
    assert_eq!(types[0].data_type, DataType::Double);
}
