use super::*;

#[test]
fn key_grain_declaration_is_refused() {
    let err = validate_frozen_horizon(GrainLabel::Key).unwrap_err();
    assert!(
        err.contains("key"),
        "error must name the offending grain, got: {err}"
    );
}

#[test]
fn partition_grain_declaration_is_admitted() {
    assert!(validate_frozen_horizon(GrainLabel::Partition).is_ok());
}

#[test]
fn frozen_horizon_refused_on_a_succession_model() {
    let err = validate_frozen_horizon(GrainLabel::Succession).unwrap_err();
    assert!(
        err.contains("succession"),
        "error must name the succession grain, not a Key fallback, got: {err}"
    );
}

#[test]
fn posture_refuses_mutable_snapshot() {
    let err = validate_frozen_horizon_posture(
        "orders",
        Some(smelt_core::sources::MutationProfile::Mutable),
    )
    .unwrap_err();
    assert!(err.contains("orders"), "{err}");
    assert!(err.contains("Mutable"), "{err}");
}

#[test]
fn posture_refuses_change_feed() {
    let err = validate_frozen_horizon_posture(
        "orders",
        Some(smelt_core::sources::MutationProfile::ChangeFeed),
    )
    .unwrap_err();
    assert!(err.contains("orders"), "{err}");
    assert!(err.contains("ChangeFeed"), "{err}");
}

#[test]
fn posture_admits_append_only() {
    assert!(validate_frozen_horizon_posture(
        "orders",
        Some(smelt_core::sources::MutationProfile::AppendOnly)
    )
    .is_ok());
}

#[test]
fn posture_admits_undeclared_profile() {
    assert!(validate_frozen_horizon_posture("orders", None).is_ok());
}

#[test]
fn clamp_narrows_start_to_end_minus_h() {
    // A 400-day run range with H = 90 days floors at end - 90d.
    let start = 0;
    let end = 400;
    let h = 90;
    assert_eq!(clamp_write_range(start, end, h), 310);
}

#[test]
fn clamp_never_widens() {
    // A run range shorter than H is returned unchanged.
    let start = 350;
    let end = 400;
    let h = 90;
    assert_eq!(clamp_write_range(start, end, h), start);
}

#[test]
fn frozen_band_before_is_end_minus_h() {
    assert_eq!(frozen_band_before(400, 90), 310);
    // Shares its derivation with `clamp_write_range`'s own floor.
    assert_eq!(frozen_band_before(400, 90), clamp_write_range(0, 400, 90));
}

fn pc(partition_value: &str, row_count: i64) -> PartitionCount {
    PartitionCount {
        partition_value: partition_value.to_string(),
        row_count,
    }
}

#[test]
fn late_arrivals_flags_count_increase_in_frozen_band() {
    let baseline = vec![pc("2026-01-01", 100)];
    let current = vec![pc("2026-01-01", 120)];
    let arrivals = late_arrivals(&baseline, &current, "2026-02-01");
    assert_eq!(
        arrivals,
        vec![LateArrival {
            partition: "2026-01-01".to_string(),
            added_rows: 20,
        }]
    );
}

#[test]
fn late_arrivals_flags_partition_absent_from_baseline() {
    let baseline = vec![];
    let current = vec![pc("2026-01-01", 5)];
    let arrivals = late_arrivals(&baseline, &current, "2026-02-01");
    assert_eq!(
        arrivals,
        vec![LateArrival {
            partition: "2026-01-01".to_string(),
            added_rows: 5,
        }]
    );
}

#[test]
fn late_arrivals_ignores_partitions_inside_horizon() {
    let baseline = vec![pc("2026-02-05", 100)];
    // 2026-02-05 is not strictly before the frozen_before boundary
    // 2026-02-01, so a change there is ordinary maintenance.
    let current = vec![pc("2026-02-05", 200)];
    let arrivals = late_arrivals(&baseline, &current, "2026-02-01");
    assert!(arrivals.is_empty());
}

#[test]
fn late_arrivals_ignores_count_decrease() {
    let baseline = vec![pc("2026-01-01", 100)];
    let current = vec![pc("2026-01-01", 90)];
    let arrivals = late_arrivals(&baseline, &current, "2026-02-01");
    assert!(arrivals.is_empty());
}

#[test]
fn emit_frozen_band_snapshot_counts_per_partition() {
    for dialect in [MaintenanceDialect::DuckDb, MaintenanceDialect::Spark] {
        let stmt = emit_frozen_band_snapshot("raw.events", "event_date", "2026-02-01", dialect);
        assert!(stmt.sql.contains("GROUP BY event_date"), "{}", stmt.sql);
        assert!(stmt.sql.contains("< '2026-02-01'"), "{}", stmt.sql);
        assert!(stmt.sql.contains("COUNT(*) AS row_count"), "{}", stmt.sql);
    }
}
