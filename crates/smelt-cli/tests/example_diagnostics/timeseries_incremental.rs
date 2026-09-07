use crate::support::*;
use crate::support_ext::*;

/// Timeseries TDD: `examples/timeseries_broken_incremental_without_timeseries/` produces
/// exactly one `TimeseriesRequiredForPartitionGrain` diagnostic anchored at
/// `models/incremental_without_timeseries.sql`.
///
/// This test verifies that `validate_timeseries` is wired into the production
/// diagnostics pipeline (not just callable in unit tests).
#[test]
fn timeseries_broken_incremental_without_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_incremental_without_timeseries",
        "models/incremental_without_timeseries.sql",
        smelt_db::DiagnosticCode::TimeseriesRequiredForPartitionGrain,
    );
}
