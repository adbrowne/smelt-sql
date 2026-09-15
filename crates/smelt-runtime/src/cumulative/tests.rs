use super::*;
use smelt_core::config::TimeseriesConfig;
use smelt_planner::{AggregatorColumn, CrossPartitionCombiner, DrivingSource};

fn dummy_ts() -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: "event_date".to_string(),
        partition_column: "event_date".to_string(),
        granularity: smelt_core::config::Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

/// The plain unconditional matched arm — the pre-Phase-C6 default the
/// existing byte-identity tests below still exercise, so the
/// `emit_keyed_fold` dispatch path stays unchanged.
fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test exercises the unconditional dispatch path directly".to_string(),
    }
}

/// `build_cumulative_merge_sql` is a thin wrapper over the single-owner
/// `emit_keyed_fold` emitter (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)"): this test asserts its output
/// is byte-identical to a direct emitter call over the same rendered
/// combiner expressions, not merely emitter-*shaped* (contains checks
/// alone would pass even if a stray character crept into the wrapper's
/// own formatting).
#[test]
fn test_build_cumulative_merge_sql() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string(), "user_id".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "event_count".to_string(),
                per_partition_agg: "COUNT".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
                state: None,
            },
            AggregatorColumn {
                output_name: "first_seen".to_string(),
                per_partition_agg: "MIN".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Min,
                state: None,
            },
            AggregatorColumn {
                output_name: "last_seen".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, user_id, COUNT(*) AS event_count, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen FROM events GROUP BY 1, 2";
    let sql = build_cumulative_merge_sql(
        "main",
        "device_user_edges",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    assert!(sql.contains("MERGE INTO main.device_user_edges"));
    assert!(sql.contains("target.device_id = delta.device_id"));
    assert!(sql.contains("target.user_id = delta.user_id"));
    assert!(sql.contains("event_count = target.event_count + delta.event_count"));
    assert!(sql.contains("first_seen = LEAST(target.first_seen, delta.first_seen)"));
    assert!(sql.contains("last_seen = GREATEST(target.last_seen, delta.last_seen)"));
    assert!(sql.contains("WHEN NOT MATCHED THEN INSERT *"));

    let expected = emit_keyed_fold(
        "main.device_user_edges",
        &classification.unique_key,
        &[
            (
                "event_count".to_string(),
                "target.event_count + delta.event_count".to_string(),
            ),
            (
                "first_seen".to_string(),
                "LEAST(target.first_seen, delta.first_seen)".to_string(),
            ),
            (
                "last_seen".to_string(),
                "GREATEST(target.last_seen, delta.last_seen)".to_string(),
            ),
        ],
        delta_sql,
        None,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        sql, expected.statements[0].sql,
        "build_cumulative_merge_sql must be byte-identical to a direct emitter call"
    );
}

/// `build_cumulative_merge_sql` must honour the caller-supplied target
/// dialect rather than hardcoding `MaintenanceDialect::DuckDb`
/// (`docs/specs/multi_backend.md` §"Whole-row MERGE"): the not-matched
/// arm on BigQuery spells `INSERT ROW`, not `INSERT *`
/// (`smelt_logical::maintenance::emit::whole_row_insert_arm`). The
/// DuckDB leg of this same call must stay byte-identical to
/// `test_build_cumulative_merge_sql` above — dialect-threading must not
/// perturb the existing default-target output.
#[test]
fn build_cumulative_merge_sql_honours_target_dialect() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY 1";

    let bigquery_sql = build_cumulative_merge_sql(
        "main",
        "device_edges",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::BigQuery,
    );
    assert!(
        bigquery_sql.contains("WHEN NOT MATCHED THEN INSERT ROW"),
        "BigQuery target must emit `INSERT ROW`, got: {bigquery_sql}"
    );
    assert!(
        !bigquery_sql.contains("INSERT *"),
        "BigQuery target must not emit `INSERT *`, got: {bigquery_sql}"
    );

    let duckdb_sql = build_cumulative_merge_sql(
        "main",
        "device_edges",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    assert!(
        duckdb_sql.contains("WHEN NOT MATCHED THEN INSERT *"),
        "DuckDB target must keep emitting `INSERT *`, got: {duckdb_sql}"
    );
}

/// A locality-admitted model's `MERGE` carries a target-side partition
/// predicate over the slice (`docs/specs/incremental_shapes.md` §"Key
/// temporal locality") — a non-time-partitioned keyed model's SQL (the
/// `None` case above) stays byte-unchanged; passing `Some` only adds the
/// extra `AND` clause, nothing else in the statement shifts.
#[test]
fn build_cumulative_merge_sql_with_slice_carries_target_partition_predicate() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string(), "event_date".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, event_date, COUNT(*) AS event_count FROM events \
                      WHERE event_date = '2026-01-02' GROUP BY 1, 2";
    let slice = TargetSlicePredicate::Range {
        partition_column: "event_date".to_string(),
        lower: "2026-01-02".to_string(),
        upper: "2026-01-02".to_string(),
        column_type: smelt_logical::maintenance::emit::PartitionColumnType::Undeclared,
    };
    let without_slice = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    let with_slice = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        Some(&slice),
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    assert!(
        with_slice.contains("AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'"),
        "expected slice predicate in: {with_slice}"
    );
    assert_eq!(
        with_slice,
        format!(
            "{} AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'{}",
            &without_slice[..without_slice.find(" WHEN MATCHED").unwrap()],
            &without_slice[without_slice.find(" WHEN MATCHED").unwrap()..]
        ),
        "the slice predicate must be the ONLY difference from the unsliced merge"
    );
}

/// Route 2 (key-determined) locality carries a `DeltaValues` slice
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality", route
/// 2) — the target scan is pruned to exactly the partition-column
/// values the step's own delta relation carries, read off that same
/// relation rather than a caller-precomputed range.
#[test]
fn build_cumulative_merge_sql_with_delta_values_slice_carries_in_subquery_predicate() {
    let classification = CumulativeClassification {
        unique_key: vec!["transaction_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "max_amount".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.transactions".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT transaction_id, MIN(transaction_timestamp) AS first_seen_at, \
                      MAX(amount) AS max_amount FROM transactions GROUP BY 1";
    let slice = TargetSlicePredicate::DeltaValues {
        partition_column: "first_seen_at".to_string(),
        delta_select: delta_sql.to_string(),
    };
    let with_slice = build_cumulative_merge_sql(
        "main",
        "transaction_first_seen",
        delta_sql,
        &classification,
        Some(&slice),
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    assert!(
        with_slice.contains(
            "AND target.first_seen_at IN (SELECT DISTINCT first_seen_at FROM (SELECT \
             transaction_id, MIN(transaction_timestamp) AS first_seen_at, MAX(amount) AS \
             max_amount FROM transactions GROUP BY 1) AS __locality_delta_values)"
        ),
        "expected DeltaValues predicate in: {with_slice}"
    );
    assert!(
        !with_slice.contains("BETWEEN"),
        "route 2's slice must never render as a margin-based range: {with_slice}"
    );
}

/// `docs/plans/20260715-composed-axes-conditional-maintenance.md`
/// Phase C6: a `WriteSuppression::Suppressed` verdict dispatches to
/// `emit_keyed_fold_suppressed` instead of the unconditional
/// `emit_keyed_fold` — the matched arm gains an `IS DISTINCT FROM`
/// guard over exactly the compared fold columns, byte-identical to a
/// direct emitter call.
#[test]
fn build_cumulative_merge_sql_dispatches_suppressed_variant() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY device_id";
    let suppression = WriteSuppression::Suppressed {
        compared_columns: vec!["event_count".to_string()],
    };
    let sql = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        None,
        &suppression,
        MaintenanceDialect::DuckDb,
    );

    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &classification.unique_key,
        &[(
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        )],
        delta_sql,
        None,
        &["event_count".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        sql, expected.statements[0].sql,
        "build_cumulative_merge_sql must dispatch Suppressed to emit_keyed_fold_suppressed, \
         byte-identical to a direct emitter call"
    );
    assert!(sql.contains("IS DISTINCT FROM"));
}

/// Phase C6's own claim: a composed (key + time) model's suppressed
/// `MERGE` carries **both** predicates (the locality slice on the
/// target read, `IS DISTINCT FROM` on the matched arm); a bare keyed
/// model with no established locality slice carries only the
/// suppression arm — never an invented slice. Both shapes dispatch
/// through the SAME `build_cumulative_merge_sql` call, `slice` being
/// the only thing that differs.
#[test]
fn build_cumulative_merge_sql_composed_suppression_carries_both_predicates_bare_carries_only_one() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string(), "event_date".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "max_amount".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, event_date, MAX(amount) AS max_amount FROM events \
                      WHERE event_date = '2026-01-02' GROUP BY 1, 2";
    let suppression = WriteSuppression::Suppressed {
        compared_columns: vec!["max_amount".to_string()],
    };

    // Bare keyed (no established locality slice): suppression arm only,
    // no invented slice.
    let bare = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        None,
        &suppression,
        MaintenanceDialect::DuckDb,
    );
    assert!(
        bare.contains("IS DISTINCT FROM"),
        "bare keyed suppressed merge must carry the suppression arm: {bare}"
    );
    assert!(
        !bare.contains("BETWEEN") && !bare.contains(" IN ("),
        "bare keyed suppressed merge must never invent a slice: {bare}"
    );

    // Composed (key + time): both predicates, on the same ON clause.
    let slice = TargetSlicePredicate::Range {
        partition_column: "event_date".to_string(),
        lower: "2026-01-02".to_string(),
        upper: "2026-01-02".to_string(),
        column_type: smelt_logical::maintenance::emit::PartitionColumnType::Undeclared,
    };
    let composed = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        Some(&slice),
        &suppression,
        MaintenanceDialect::DuckDb,
    );
    assert!(
        composed.contains("AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'"),
        "composed suppressed merge must carry the slice predicate: {composed}"
    );
    assert!(
        composed.contains("IS DISTINCT FROM"),
        "composed suppressed merge must ALSO carry the suppression arm: {composed}"
    );

    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &classification.unique_key,
        &[(
            "max_amount".to_string(),
            "GREATEST(target.max_amount, delta.max_amount)".to_string(),
        )],
        delta_sql,
        Some(&slice),
        &["max_amount".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        composed, expected.statements[0].sql,
        "composed suppression+slice dispatch must be byte-identical to a direct emitter call"
    );
}

/// Phase C6: `execute_cumulative_aggregate`'s own suppression resolver —
/// a fully comparable fold over the classifier's own proven `unique_key`
/// resolves `Suppressed`, naming exactly the fold's own output columns
/// (mirrors `events_deduped.sql`'s `MIN`-folded shape).
#[test]
fn resolve_cumulative_write_suppression_admits_comparable_min_fold() {
    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "device_id".to_string(),
                per_partition_agg: "MIN".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Min,
                state: None,
            },
            AggregatorColumn {
                output_name: "first_seen_date".to_string(),
                per_partition_agg: "MIN".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Min,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let sql = "SELECT event_id, MIN(device_id) AS device_id, \
                MIN(CAST(event_date AS DATE)) AS first_seen_date \
                FROM smelt.sources.raw.events GROUP BY event_id";
    let suppression =
        resolve_cumulative_write_suppression(&classification, sql, &EffectiveOverride::default())
            .expect("no override pinned — resolve_write_variant never refuses here");
    assert_eq!(
        suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["device_id".to_string(), "first_seen_date".to_string()]
        },
        "a MIN-folded group over a proven key must admit suppression, not refuse: \
         {suppression:?}"
    );
}

/// `docs/outcomes/20260815-definition-delta-migrate/phases/33-plan.md`:
/// a hard `technique: unconditional` pin addressing the keyed fold's
/// driving source must reach the resolver — the write must fall back to
/// the plain unconditional matched arm even though P2/P3 both admit
/// suppression. Before this phase `resolve_cumulative_write_suppression`
/// never consulted the override ladder at all, so this pin was silently
/// ignored.
#[test]
fn keyed_fold_unconditional_pin_reaches_the_emitted_merge() {
    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "device_id".to_string(),
            per_partition_agg: "MIN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Min,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let sql = "SELECT event_id, MIN(device_id) AS device_id FROM smelt.sources.raw.events \
                GROUP BY event_id";
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(smelt_core::config::CellTechnique::Unconditional),
    };
    let suppression = resolve_cumulative_write_suppression(&classification, sql, &overrides)
        .expect("technique: unconditional never refuses");
    assert!(
        matches!(suppression, WriteSuppression::Unconditional { .. }),
        "pinned technique: unconditional must resolve the unconditional variant even \
         though P2/P3 admit suppression: {suppression:?}"
    );

    let delta_sql = "SELECT event_id, MIN(device_id) AS device_id FROM events GROUP BY \
                      event_id";
    let merge_sql = build_cumulative_merge_sql(
        "main",
        "events_deduped",
        delta_sql,
        &classification,
        None,
        &suppression,
        MaintenanceDialect::DuckDb,
    );
    assert!(
        !merge_sql.contains("IS DISTINCT FROM"),
        "the pinned unconditional arm must not emit the change-suppressed guard: \
         {merge_sql}"
    );
}

/// `technique: suppress` over a proof that refused (P2 `WholeRow`
/// identity here) must propagate the resolver's `ChoiceRefusal`, never
/// silently fall back to the unconditional arm.
#[test]
fn keyed_fold_suppress_pin_over_refused_proof_refuses() {
    let classification = CumulativeClassification {
        unique_key: vec![],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "device_id".to_string(),
            per_partition_agg: "MIN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Min,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let sql = "SELECT MIN(device_id) AS device_id FROM smelt.sources.raw.events";
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(smelt_core::config::CellTechnique::Suppress),
    };
    let result = resolve_cumulative_write_suppression(&classification, sql, &overrides);
    assert!(
        result.is_err(),
        "technique: suppress over a WholeRow (no proven key) row identity must refuse, \
         not silently emit the unconditional arm: {result:?}"
    );
}

/// The soft `prefer: unconditional` bias flips the structural default
/// (suppressed, since P2/P3 both admit it here) without ever refusing.
#[test]
fn keyed_fold_prefer_unconditional_soft_biases_without_refusing() {
    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "device_id".to_string(),
            per_partition_agg: "MIN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Min,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let sql = "SELECT event_id, MIN(device_id) AS device_id FROM smelt.sources.raw.events \
                GROUP BY event_id";

    let unconditional_bias = EffectiveOverride {
        prefer: Some(smelt_core::config::TechniquePreference::Unconditional),
        technique: None,
    };
    let suppression =
        resolve_cumulative_write_suppression(&classification, sql, &unconditional_bias)
            .expect("a soft prefer never refuses");
    assert!(matches!(
        suppression,
        WriteSuppression::Unconditional { .. }
    ));

    // A `prefer: suppress` bias over a WholeRow proof refusal falls
    // back silently — never a hard refusal like the pin above.
    let whole_row_classification = CumulativeClassification {
        unique_key: vec![],
        ..classification
    };
    let suppress_bias = EffectiveOverride {
        prefer: Some(smelt_core::config::TechniquePreference::Suppress),
        technique: None,
    };
    let fallback_suppression = resolve_cumulative_write_suppression(
        &whole_row_classification,
        "SELECT MIN(device_id) AS device_id FROM smelt.sources.raw.events",
        &suppress_bias,
    )
    .expect("a soft prefer never refuses even over a WholeRow proof failure");
    assert!(matches!(
        fallback_suppression,
        WriteSuppression::Unconditional { .. }
    ));
}

/// Regression: with no `maintenance:` overrides at all
/// (`EffectiveOverride::default()`), the emitted merge SQL is
/// byte-identical to what it was before this phase folded the ladder
/// in — the trigger derivation (`Trigger::NewData`, `ledger_catch_up:
/// false`) must not perturb the unpinned default.
#[test]
fn keyed_fold_unpinned_write_is_byte_identical() {
    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "device_id".to_string(),
            per_partition_agg: "MIN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Min,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let sql = "SELECT event_id, MIN(device_id) AS device_id FROM smelt.sources.raw.events \
                GROUP BY event_id";
    let suppression =
        resolve_cumulative_write_suppression(&classification, sql, &EffectiveOverride::default())
            .expect("no override pinned — never refuses");
    assert_eq!(
        suppression,
        WriteSuppression::Suppressed {
            compared_columns: vec!["device_id".to_string()]
        }
    );
}

/// `BIT_XOR` is an **additive fold** (`docs/specs/incremental_models.md`
/// §"The column-family catalogue") — a commutative *group*, not an
/// idempotent lattice. Re-merging an already-reflected delta computes
/// `x XOR d XOR d == x`, which CANCELS that window's contribution rather
/// than converging, so the cell must keep a ledger (`Grade::Additive`)
/// and refuse the reprocessed window rather than silently corrupting
/// state (§"The transactional merge ledger").
#[test]
fn bit_xor_only_model_is_ledger_graded_additive() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "xor_bits".to_string(),
            per_partition_agg: "BIT_XOR".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::BitXor,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert_eq!(
        classification.ledger_grade(),
        Grade::Additive,
        "BIT_XOR is self-inverse: re-merging an already-reflected delta cancels it, so the \
         cell must keep a reconciliation ledger"
    );
}

/// The genuinely idempotent lattice combiners still skip the ledger —
/// the fix above must not widen ledger keeping to every keyed model.
#[test]
fn lattice_only_model_stays_ledger_free() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "max_val".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
                state: None,
            },
            AggregatorColumn {
                output_name: "and_bits".to_string(),
                per_partition_agg: "BIT_AND".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::BitAnd,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert_eq!(classification.ledger_grade(), Grade::Idempotent);
}

/// `ledger_grade` delegates to `smelt_logical::execution_postures`
/// rather than re-deriving re-run tolerance
/// (`docs/outcomes/20260815-keyed-grain-residue` phase 4) — over a
/// mixed additive+idempotent column set, the runtime's grade must be
/// exactly the shared derivation's `rerun_tolerant` verdict.
#[test]
fn ledger_grade_agrees_with_shared_posture_derivation() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "total_amount".to_string(),
                per_partition_agg: "SUM".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
                state: None,
            },
            AggregatorColumn {
                output_name: "max_val".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let postures = smelt_logical::execution_postures(&classification.aggregator_columns);
    let expected_grade = if postures.rerun_tolerant.holds {
        Grade::Idempotent
    } else {
        Grade::Additive
    };
    assert_eq!(classification.ledger_grade(), expected_grade);
    assert_eq!(classification.ledger_grade(), Grade::Additive);
}

/// The `WindowedKeyedRule` impl must refuse a non-monoid combiner
/// independently of the classifier that produced it — defense in depth
/// against ever merging one approximately (`model_transforms.md`
/// §Constraints "Equivalence or refusal").
#[test]
fn refuses_non_monoid_combiner_independently_of_classifier() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "median_latency".to_string(),
            per_partition_agg: "MEDIAN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let reason = classification.refuse();
    assert!(reason.is_some(), "MEDIAN is not a monoid combiner");
    assert!(reason.unwrap().contains("MEDIAN"));
}

#[test]
fn admits_monoid_combiner() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert!(classification.refuse().is_none());
}

/// An `AVG` column's presented `Recomputed` combiner carries `state:
/// Some(..)` whose state columns are both `Sum` — `ledger_grade` must
/// grade the cell `Additive` (`docs/outcomes/20260809-rung2-state-shapes`
/// row 7): the hidden state folds additively even though the presented
/// column's own combiner (`Recomputed`) says nothing about algebra.
#[test]
fn avg_model_is_ledger_graded_additive() {
    use smelt_logical::analysis::decomposed_state::{DecomposedState, StateColumn};

    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "avg_amount".to_string(),
            per_partition_agg: "AVG".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Recomputed,
            state: Some(DecomposedState {
                state_columns: vec![
                    StateColumn {
                        name: "avg_amount__sum".to_string(),
                        per_partition_expr: "SUM(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                    StateColumn {
                        name: "avg_amount__count".to_string(),
                        per_partition_expr: "COUNT(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                ],
                presentation_expr: "avg_amount__sum / avg_amount__count".to_string(),
            }),
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert_eq!(
        classification.ledger_grade(),
        Grade::Additive,
        "AVG's hidden (sum, count) state folds additively — the cell must keep a \
         reconciliation ledger"
    );
}

/// Regression: `MAX_BY`'s `(v, o)` state and once-write's `(value,
/// written)` state are NOT additive — neither combiner is `Sum`/`BitXor`
/// — so both must stay `Idempotent`, exactly as before state-awareness
/// was added to `ledger_grade`.
#[test]
fn max_by_and_once_write_state_stay_idempotent() {
    use smelt_logical::analysis::decomposed_state::{DecomposedState, StateColumn};

    let max_by_classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "status".to_string(),
            per_partition_agg: "MAX_BY".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::OrderMonotone {
                ordering_column: "status__o".to_string(),
                prefer_greater: true,
            },
            state: Some(DecomposedState {
                state_columns: vec![
                    StateColumn {
                        name: "status__v".to_string(),
                        per_partition_expr: "ARG_MAX(status, updated_at)".to_string(),
                        combiner: CrossPartitionCombiner::OrderMonotone {
                            ordering_column: "status__o".to_string(),
                            prefer_greater: true,
                        },
                    },
                    StateColumn {
                        name: "status__o".to_string(),
                        per_partition_expr: "MAX(updated_at)".to_string(),
                        combiner: CrossPartitionCombiner::Max,
                    },
                ],
                presentation_expr: "status__v".to_string(),
            }),
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert_eq!(max_by_classification.ledger_grade(), Grade::Idempotent);

    let once_write_classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "first_referrer".to_string(),
            per_partition_agg: "COALESCE".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::OnceWrite,
            state: Some(DecomposedState {
                state_columns: vec![
                    StateColumn {
                        name: "first_referrer__value".to_string(),
                        per_partition_expr: "MAX(signup_referrer)".to_string(),
                        combiner: CrossPartitionCombiner::OnceWrite,
                    },
                    StateColumn {
                        name: "first_referrer__written".to_string(),
                        per_partition_expr: "(MAX(signup_referrer)) IS NOT NULL".to_string(),
                        combiner: CrossPartitionCombiner::BoolOr,
                    },
                ],
                presentation_expr: "first_referrer__value".to_string(),
            }),
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert_eq!(once_write_classification.ledger_grade(), Grade::Idempotent);
}

/// The defense-in-depth `refuse()` pass admits a state-bearing `AVG`
/// column (its state columns are recognised `Sum` combiners), and
/// separately catches the internal-invariant violation of a
/// `Recomputed` column carrying no state at all — a `Recomputed`
/// column must always be state-bearing by construction
/// (`docs/outcomes/20260809-rung2-state-shapes` row 7).
#[test]
fn refuse_accepts_state_bearing_avg_column() {
    use smelt_logical::analysis::decomposed_state::{DecomposedState, StateColumn};

    let admitted = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "avg_amount".to_string(),
            per_partition_agg: "AVG".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Recomputed,
            state: Some(DecomposedState {
                state_columns: vec![
                    StateColumn {
                        name: "avg_amount__sum".to_string(),
                        per_partition_expr: "SUM(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                    StateColumn {
                        name: "avg_amount__count".to_string(),
                        per_partition_expr: "COUNT(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                ],
                presentation_expr: "avg_amount__sum / avg_amount__count".to_string(),
            }),
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    assert!(
        admitted.refuse().is_none(),
        "a state-bearing AVG column with recognised Sum state combiners must be admitted"
    );

    let internal_invariant_violation = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "avg_amount".to_string(),
            per_partition_agg: "AVG".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Recomputed,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let reason = internal_invariant_violation.refuse();
    assert!(
        reason.is_some(),
        "a Recomputed column with no state must be refused"
    );
    assert!(reason.unwrap().contains("internal error"));
}

#[test]
fn test_collect_refs_simple() {
    let sql = "SELECT * FROM smelt.silver.events_parsed WHERE id > 0";
    let refs = collect_refs_from_sql(sql);
    assert_eq!(refs, vec!["smelt.silver.events_parsed".to_string()]);
}

#[test]
fn test_collect_refs_skips_functions() {
    let sql = "SELECT smelt.functions.foo(x) FROM smelt.silver.events";
    let refs = collect_refs_from_sql(sql);
    assert_eq!(refs, vec!["smelt.silver.events".to_string()]);
}

/// A state-bearing classification's `MERGE` folds each hidden state
/// column by its own combiner and recomputes the presented column from
/// the merged state — byte-identical to a direct `emit_keyed_fold` call
/// over the state-expanded fold set (`docs/specs/incremental_models.md`
/// §"Decomposed state (rung 2) in keyed models").
#[test]
fn build_cumulative_merge_sql_folds_state_columns() {
    use smelt_logical::analysis::decomposed_state::{DecomposedState, StateColumn};

    let classification = CumulativeClassification {
        unique_key: vec!["customer_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "avg_amount".to_string(),
            per_partition_agg: "AVG".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::PlainOverwrite,
            state: Some(DecomposedState {
                state_columns: vec![
                    StateColumn {
                        name: "avg_amount__sum".to_string(),
                        per_partition_expr: "SUM(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                    StateColumn {
                        name: "avg_amount__count".to_string(),
                        per_partition_expr: "COUNT(amount)".to_string(),
                        combiner: CrossPartitionCombiner::Sum,
                    },
                ],
                presentation_expr: "avg_amount__sum / avg_amount__count".to_string(),
            }),
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT customer_id, SUM(amount) AS avg_amount__sum, COUNT(amount) AS \
                      avg_amount__count FROM events GROUP BY 1";
    let sql = build_cumulative_merge_sql(
        "main",
        "customer_stats",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );

    let expected = emit_keyed_fold(
        "main.customer_stats",
        &classification.unique_key,
        &[
            (
                "avg_amount__sum".to_string(),
                "target.avg_amount__sum + delta.avg_amount__sum".to_string(),
            ),
            (
                "avg_amount__count".to_string(),
                "target.avg_amount__count + delta.avg_amount__count".to_string(),
            ),
            (
                "avg_amount".to_string(),
                "(target.avg_amount__sum + delta.avg_amount__sum) / \
                 (target.avg_amount__count + delta.avg_amount__count)"
                    .to_string(),
            ),
        ],
        delta_sql,
        None,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        sql, expected.statements[0].sql,
        "state-bearing merge must be byte-identical to a direct emitter call over the \
         state-expanded fold set"
    );
}

/// A classification with no state-bearing column (every family admitted
/// today) folds exactly as before this mechanism existed — the
/// no-admission-widening guard, mirrored at the runtime layer.
#[test]
fn stateless_merge_sql_is_unchanged() {
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY device_id";
    let sql = build_cumulative_merge_sql(
        "main",
        "device_daily",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    let expected = emit_keyed_fold(
        "main.device_daily",
        &classification.unique_key,
        &[(
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        )],
        delta_sql,
        None,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(sql, expected.statements[0].sql);
}

/// `WindowedKeyedRule::write_group`'s unpinned (`KeyedWriteMechanism::
/// Merge`) arm must stay byte-identical to `merge_sql`'s own output —
/// the driver's mechanism-aware dispatch (27g) must not perturb the
/// pre-existing unpinned path any callers already depend on.
#[test]
fn write_group_with_no_pin_is_byte_identical_to_the_merge() {
    use smelt_logical::maintenance::choice::KeyedWriteMechanism;

    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY device_id";
    let suppression = unconditional();

    let direct_sql = classification.merge_sql(
        "main",
        "device_daily",
        delta_sql,
        None,
        &suppression,
        MaintenanceDialect::DuckDb,
    );
    let group = classification.write_group(
        "main",
        "device_daily",
        delta_sql,
        None,
        &KeyedWriteMechanism::Merge(suppression),
        MaintenanceDialect::DuckDb,
        &smelt_backend::BackendCapabilities::duckdb(),
    );
    assert_eq!(group.statements.len(), 1);
    assert_eq!(group.statements[0].sql, direct_sql);
    assert!(!group.transactional);
}

/// A `staged_candidate` pin's mechanism must realise the merge-less
/// staged-candidate group over the fold's own post-fold candidate rows —
/// byte-identical to a direct `emit_staged_candidate_conditional` call
/// over `keyed_fold_candidate_select`'s own candidate SQL (`docs/
/// outcomes/20260815-definition-delta-migrate/phases/27g-plan.md`).
#[test]
fn staged_candidate_pin_selects_the_staged_candidate_group() {
    use smelt_logical::maintenance::choice::KeyedWriteMechanism;
    use smelt_logical::maintenance::emit::{
        emit_staged_candidate_conditional, keyed_fold_candidate_select, StagedRelation,
    };

    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.silver.events_parsed".to_string(),
            timeseries: Some(dummy_ts()),
        },
    };
    let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY device_id";
    let compared_columns = vec!["event_count".to_string()];

    let group = classification.write_group(
        "main",
        "device_daily",
        delta_sql,
        None,
        &KeyedWriteMechanism::StagedCandidate {
            compared_columns: compared_columns.clone(),
        },
        MaintenanceDialect::DuckDb,
        &smelt_backend::BackendCapabilities::duckdb(),
    );

    let folds = vec![(
        "event_count".to_string(),
        "target.event_count + delta.event_count".to_string(),
    )];
    let candidate_select = keyed_fold_candidate_select(
        "main.device_daily",
        &classification.unique_key,
        &folds,
        delta_sql,
        MaintenanceDialect::DuckDb,
    );
    let expected = emit_staged_candidate_conditional(
        "main.device_daily",
        &StagedRelation::session_temporary("__smelt_staged_device_daily"),
        &classification.unique_key,
        &candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group, expected);
    assert_eq!(group.statements.len(), 5);
    assert!(group.transactional);
}
