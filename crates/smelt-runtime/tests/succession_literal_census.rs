//! Gap 4's fourth emission site, measured rather than assumed
//! (`docs/outcomes/20260913-trino-incremental/phases/06b-plan.md`): the
//! `driving_steps` call site in `execute/project/mod.rs`'s succession-patch
//! window-forward arm still passes `PartitionColumnType::Undeclared` (3f's
//! untouched residue). Unlike the driving-source pushdown filter and the
//! target-scan slice bound 3f fixed, this site's `column_type` is never read
//! on the way to a rendered predicate: `succession_window_predicate`
//! deliberately spells the window bound as an untyped string literal (a
//! measured GoogleSQL cross-dialect requirement —
//! `maintenance_sql_dialect_purity.rs::the_succession_window_predicate_uses_untyped_date_literals`
//! pins it). So `Undeclared` at the call site is inert residue by
//! construction, not a live gap 1/3b2 needs to close.

use smelt_core::config::Granularity;
use smelt_logical::maintenance::emit::PartitionColumnType;
use smelt_runtime::maintenance_driver::driving_steps;
use smelt_runtime::maintenance_driver::succession_window_predicate;

/// Source-scan census over `src/maintenance_driver/succession/`: no
/// `partition_literal(` call and no `PartitionColumnType` consumption
/// outside comments — `succession_window_predicate`'s deliberately untyped
/// spelling is this family's only window rendering. `tests.rs` is excluded,
/// matching `state_guard_census.rs`'s convention that a unit-test module
/// configures fixtures, it does not author production emission.
#[test]
fn succession_renders_its_window_only_through_the_untyped_predicate() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maintenance_driver/succession");
    let mut offending = Vec::new();
    for entry in std::fs::read_dir(&root).expect("read succession dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        if path.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read source");
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .display()
            .to_string();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            if line.contains("partition_literal(") || line.contains("PartitionColumnType") {
                offending.push(format!("{rel}:{}: {line}", i + 1));
            }
        }
    }
    assert!(
        offending.is_empty(),
        "succession must render its window only through succession_window_predicate's untyped \
         literal, never partition_literal/PartitionColumnType:\n  {}",
        offending.join("\n  ")
    );
}

/// `driving_steps(start, end, Day, Date)` and `driving_steps(start, end,
/// Day, Undeclared)` yield steps whose `partition_value`/`range.start`/
/// `range.end` are equal, and `succession_window_predicate` over each
/// yields the same string: the `Undeclared` at the call site changes no
/// emitted SQL, so it is residue by construction rather than by luck.
#[test]
fn succession_driving_steps_column_type_is_inert() {
    let dated = driving_steps(
        "2026-01-01",
        "2026-01-03",
        &Granularity::Day,
        PartitionColumnType::Date,
    )
    .expect("driving_steps (Date)");
    let undeclared = driving_steps(
        "2026-01-01",
        "2026-01-03",
        &Granularity::Day,
        PartitionColumnType::Undeclared,
    )
    .expect("driving_steps (Undeclared)");

    assert_eq!(dated.len(), undeclared.len());
    for (d, u) in dated.iter().zip(undeclared.iter()) {
        assert_eq!(d.partition_value, u.partition_value);
        assert_eq!(d.range.start, u.range.start);
        assert_eq!(d.range.end, u.range.end);

        let pred_d = succession_window_predicate("created_at", &d.range.start, &d.range.end);
        let pred_u = succession_window_predicate("created_at", &u.range.start, &u.range.end);
        assert_eq!(
            pred_d, pred_u,
            "the rendered window predicate must not depend on column_type"
        );
    }
}
