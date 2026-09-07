use crate::support::*;
use crate::support_ext::*;

#[test]
fn timeseries_no_diagnostics() {
    check_workspace_no_diagnostics("examples/timeseries");
}

#[test]
fn retail_analytics_no_diagnostics() {
    check_workspace_no_diagnostics("examples/retail_analytics");
}

#[test]
fn test_workspace_no_diagnostics() {
    check_workspace_no_diagnostics("examples/test_workspace");
}

#[test]
fn ephemeral_demo_no_diagnostics() {
    check_workspace_no_diagnostics("examples/ephemeral_demo");
}

#[test]
fn multi_engine_no_diagnostics() {
    check_workspace_no_diagnostics("examples/multi_engine");
}

#[test]
fn ecommerce_no_diagnostics() {
    check_workspace_no_diagnostics("examples/ecommerce");
}

#[test]
fn functions_demo_no_diagnostics() {
    check_workspace_no_diagnostics("examples/functions_demo");
}

#[test]
fn web_analytics_no_diagnostics() {
    check_workspace_no_diagnostics("examples/web_analytics");
}

#[test]
fn fn_tableexpr_star_no_diagnostics() {
    check_workspace_no_diagnostics("examples/fn_tableexpr_star");
}

/// The succession grain's worked example (`docs/specs/incremental_shapes.md`
/// §"The succession grain"): `customer_history` recognised as keyed
/// succession over an arrival-partitioned append-only source, with no
/// declared `grain:`, `unique_key:`, or `timeseries:` on the model itself.
#[test]
fn scd2_succession_no_diagnostics() {
    check_workspace_no_diagnostics("examples/scd2_succession");
}

/// D1 phase: functions × incremental × timeseries happy-path fixture.
/// Verifies that a workspace mixing smelt.define predicates (called in WHERE)
/// and partition-aligned window-function helpers inside incremental models
/// loads without any diagnostics.
#[test]
fn fn_incremental_ts_no_diagnostics() {
    check_workspace_no_diagnostics("examples/fn_incremental_ts");
}

/// `columns.<c>.contract: plausible` opt-in fixture: an incremental model
/// stamping every row with `NOW()` into a listed payload column. Verifies
/// the workspace loads without any diagnostics (the non-determinism
/// flow/taint check runs at build time in `smelt-logical::rules::incremental`,
/// not through `file_diagnostics`, but the workspace itself must still be
/// diagnostic-clean).
#[test]
fn incremental_nondeterministic_columns_no_diagnostics() {
    check_workspace_no_diagnostics("examples/incremental_nondeterministic_columns");
}

/// `timeseries.assert_monotonic` declared-monotonicity escape hatch fixture
/// (DC1): a join whose driving-fact partition column is projected through an
/// opaque scalar function. Verifies the workspace loads without any
/// diagnostics (the widened join driving-fact resolution runs at build time
/// in `smelt-logical::rules::incremental`, not through `file_diagnostics`,
/// but the workspace itself must still be diagnostic-clean).
#[test]
fn incremental_declared_monotonic_no_diagnostics() {
    check_workspace_no_diagnostics("examples/incremental_declared_monotonic");
}

/// `functional_dependencies` declaration fixture (DC2): a plain pass-through
/// column asserted to be a per-key constant. Verifies the workspace loads
/// without any diagnostics (structural validation of the declaration passes;
/// the widening/guard proof it feeds is exercised by
/// `smelt-logical::analysis::functional_dependency` unit tests, not through
/// `file_diagnostics`).
#[test]
fn functional_dependency_declared_no_diagnostics() {
    check_workspace_no_diagnostics("examples/functional_dependency_declared");
}

/// `bounded_domain` declaration fixture: an exact `MEDIAN` aggregate over a
/// column asserted to have a bounded active domain (an explicit
/// `max_cardinality` cap). Verifies the workspace loads without any
/// diagnostics (structural validation of the declaration passes; the
/// widening/guard proof it feeds is exercised by
/// `smelt-logical::analysis::bounded_domain` unit tests, not through
/// `file_diagnostics`).
#[test]
fn bounded_domain_declared_no_diagnostics() {
    check_workspace_no_diagnostics("examples/bounded_domain_declared");
}

/// `horizon_ceiling` declaration fixture (DC4): a downstream model's 2-hour
/// `RANGE BETWEEN INTERVAL` lookback derives a horizon comfortably inside
/// the declared 30-day ceiling. Verifies the workspace loads without any
/// diagnostics (the warning this declaration licenses is a compile-time
/// `tracing::warn!`, not a Salsa `Diagnostic` — exercised by
/// `crates/smelt-runtime/tests/horizon_ceiling_warning.rs`, not through
/// `file_diagnostics`).
#[test]
fn horizon_ceiling_comfortable_no_diagnostics() {
    check_workspace_no_diagnostics("examples/horizon_ceiling_comfortable");
}

/// Source-side `mutation_profile` + `source_lateness` declaration fixture
/// (DC5): a source declaring `mutation_profile: change_feed` and a `2 hours`
/// `source_lateness` margin. Verifies the workspace loads without any
/// diagnostics — the declaration is structural YAML validation
/// (`sources.md`), not a Salsa `Diagnostic`; `SourceShape::from_source_info`'s
/// read of the declared profile is exercised by
/// `crates/smelt-logical/src/analysis/input_delta.rs` unit tests.
#[test]
fn source_mutation_profile_declared_no_diagnostics() {
    check_workspace_no_diagnostics("examples/source_mutation_profile_declared");
}

/// D-01/D-05: domain-grouped layout where models live under `billing/` and
/// `finance/` rather than a top-level `models/` directory. Verifies that
/// project-wide discovery (no scan-root gate) finds both models and that
/// `paths:` acts as a strip-list only, producing addresses `orders` and `revenue`.
#[test]
fn architecture_domain_layout_no_diagnostics() {
    check_workspace_no_diagnostics("examples/architecture_domain_layout");
}

/// Test 4 (TDD): All example SQL files must use the unified `smelt.<path>`
/// syntax.  This test FAILS until the migration tool has been run on all
/// example workspaces.
#[test]
fn all_examples_use_path_syntax() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples");
    let mut legacy_usages: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(&examples_dir) {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for (line_no, line) in content.lines().enumerate() {
            // Skip comment lines
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue;
            }
            for pattern in &["smelt.ref(", "smelt.source(", "smelt.fn."] {
                if line.contains(pattern) {
                    legacy_usages.push(format!(
                        "{}:{}: {}",
                        entry.path().display(),
                        line_no + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        legacy_usages.is_empty(),
        "Found legacy smelt syntax in examples (must be migrated to smelt.<path>):\n{}",
        legacy_usages.join("\n")
    );
}

/// Phase 4 TDD: After legacy `smelt.ref()` and `smelt.source()` deletion, all
/// known-good example workspaces must still produce zero diagnostics.  This is
/// the named TDD gate for Phase 4 of the smelt-path migration plan.
///
/// This test will pass only when:
///   1. All example SQL has been migrated from `smelt.ref()`/`smelt.source()` to
///      `smelt.<path>` form (covered by `all_examples_use_path_syntax`), AND
///   2. The parser correctly handles the new path form without introducing
///      spurious parse errors.
#[test]
fn all_examples_clean_after_legacy_removal() {
    for workspace in &[
        "examples/timeseries",
        "examples/retail_analytics",
        "examples/test_workspace",
        "examples/ephemeral_demo",
        "examples/multi_engine",
        "examples/ecommerce",
        "examples/functions_demo",
        "examples/web_analytics",
    ] {
        check_workspace_no_diagnostics(workspace);
    }
}

/// Test 5 (TDD): All known-good example workspaces must produce zero LSP
/// diagnostics after migration.  This re-runs every non-broken workspace in
/// one sweep so a migration regression is caught quickly.
///
/// The per-workspace `*_no_diagnostics` tests above also cover this — this
/// test is a belt-and-suspenders sweep that makes the intent explicit.
#[test]
fn all_examples_have_zero_lsp_diagnostics_after_migration() {
    // This serves as a combined check; the individual per-workspace tests
    // above cover the same workspaces individually for better error messages.
    for workspace in &[
        "examples/timeseries",
        "examples/retail_analytics",
        "examples/test_workspace",
        "examples/ephemeral_demo",
        "examples/multi_engine",
        "examples/ecommerce",
        "examples/functions_demo",
        "examples/web_analytics",
    ] {
        check_workspace_no_diagnostics(workspace);
    }
}
