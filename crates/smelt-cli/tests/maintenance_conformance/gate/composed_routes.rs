//! Routes 2 (key-determined) and 3 (recurrence-bounded) of the composed family: direct maintenance-driver execution against a real `DuckDbBackend`.

use super::support::no_retry_policy;
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_maintenance_testkit::recipe::{
    ComposedKeyedRecipe, ComposedRoute3Schedule, KeyedSchedule,
};
use smelt_maintenance_testkit::schedule_gen::GenRow;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::check_runner::batches_to_rows;
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

// ---- Routes 2/3: direct-driver execution against a real DuckDbBackend --

/// The driving source's own `timeseries:` declaration every composed
/// recipe's classification carries (`event_time_column`/`partition_column`
/// both `d`, `day` granularity — the fixed `events(d, id, val)` shape).
pub(crate) fn composed_driving_timeseries() -> smelt_core::config::TimeseriesConfig {
    smelt_core::config::TimeseriesConfig {
        event_time_column: "d".to_string(),
        partition_column: "d".to_string(),
        granularity: smelt_core::config::Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

pub(crate) fn composed_route2_classification(
    recipe: &ComposedKeyedRecipe,
) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "total".to_string(),
            per_partition_agg: "SUM".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

/// Route 2's **derived** sub-route: `unique_key` is `[id, d]` (both `id` and
/// `d` — unlike `composed_route2_classification`'s `[id]` alone), so a
/// merge matches on the full `(id, d)` pair and `pdate` (a deterministic
/// function of `d`) is write-once trivially — the same `(id, d)` pair is
/// never revisited with a different `d`.
pub(crate) fn composed_derived_classification(
    recipe: &ComposedKeyedRecipe,
) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string(), "d".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "total".to_string(),
            per_partition_agg: "SUM".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

pub(crate) fn composed_route3_classification(
    recipe: &ComposedKeyedRecipe,
) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

pub(crate) fn composed_route3_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    }
}

/// One window's own row list, rendered as a literal `VALUES` relation —
/// the per-step delta is built directly from the window's own rows rather
/// than filtered off a physical table by a `d = <date>` predicate, which
/// would wrongly require a redelivered row's own event-time to equal the
/// window that delivers it (`ComposedRoute3Window`'s own doc comment names
/// this as exactly the "out-of-order redelivery" shape the pool must be
/// able to express).
pub(crate) fn composed_delta_values_sql(rows: &[GenRow]) -> String {
    let values: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "({}, DATE '{}', {})",
                r.id,
                r.d.format("%Y-%m-%d"),
                r.val_sql()
            )
        })
        .collect();
    format!("(VALUES {}) AS t(id, d, val)", values.join(", "))
}

pub(crate) fn composed_route2_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM {} GROUP BY id, d",
        composed_delta_values_sql(rows)
    )
}

pub(crate) fn composed_route3_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {} GROUP BY id",
        composed_delta_values_sql(rows)
    )
}

/// Route 2's derived sub-route delta: grouped by `(id, d)`, matching
/// `composed_derived_classification`'s `unique_key`.
pub(crate) fn composed_derived_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM {} GROUP BY id, d",
        composed_delta_values_sql(rows)
    )
}

/// The route-2 oracle: `pdate` is write-once (never re-merged — see
/// `ComposedKeyedRecipe`'s doc comment), so its true end-state value is the
/// event-time of whichever window *first* delivered that key — the
/// minimum `d` across all of that key's accumulated rows (every row in
/// this pool's route-2 schedule carries `d == its own window's run date`,
/// and windows always run in ascending order).
pub(crate) fn composed_route2_oracle_sql(source_name: &str) -> String {
    format!(
        "SELECT id, CAST(MIN(d) AS DATE) AS pdate, SUM(val) AS total FROM main.sources_{source_name} \
         GROUP BY id"
    )
}

/// The derived sub-route's oracle: since `unique_key` is `(id, d)`, a full
/// group-by over the same pair is a plain additive fold — no write-once
/// reasoning needed (`pdate` is a pure deterministic function of the key
/// column `d`).
pub(crate) fn composed_derived_oracle_sql(source_name: &str) -> String {
    format!(
        "SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM main.sources_{source_name} \
         GROUP BY id, d"
    )
}

pub(crate) fn composed_route3_oracle_sql(source_name: &str) -> String {
    format!("SELECT id, MAX(d) AS last_seen FROM main.sources_{source_name} GROUP BY id")
}

/// Convert a batch of Arrow results into a sorted `Vec` of `(column,
/// value)` row vectors — a multiset comparator over two such `Vec`s (via
/// plain `==` after sorting) that does not require a `duckdb::Connection`
/// (`oracle::multiset_equal`'s own contract), since routes 2/3 query
/// through a live `DuckDbBackend` instead (mirrors
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`'s own
/// `execute_sql`-only discipline — never open a second, independent
/// connection to the same DuckDB file while the backend holds one open).
pub(crate) fn rows_as_sorted_multiset(
    batches: &[arrow::array::RecordBatch],
) -> Vec<Vec<(String, String)>> {
    let mut rows: Vec<Vec<(String, String)>> = batches_to_rows(batches)
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();
    rows.sort();
    rows
}

pub(crate) async fn assert_backend_multiset_equal(
    backend: &DuckDbBackend,
    left_sql: &str,
    right_sql: &str,
    context: &str,
) -> anyhow::Result<()> {
    let left = backend.execute_sql(left_sql).await?;
    let right = backend.execute_sql(right_sql).await?;
    let left_rows = rows_as_sorted_multiset(&left);
    let right_rows = rows_as_sorted_multiset(&right);
    if left_rows != right_rows {
        anyhow::bail!(
            "{context}: multiset mismatch\n  left  ({left_sql:?}): {left_rows:?}\n  right \
             ({right_sql:?}): {right_rows:?}"
        );
    }
    Ok(())
}

pub(crate) async fn assert_composed_route2_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route2_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-2 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 2 (`incremental_shapes.md` §"Key temporal
/// locality (the time-partitioned output)"): route 2 never settles by date — its slice is the
/// delta's own partition **values**, not a date-range window — so the
/// natural slice here is one distinct `pdate` value; each such slice must
/// equal the oracle restricted to that same value.
pub(crate) async fn assert_composed_route2_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(pdate AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE pdate = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE pdate = DATE '{v}'",
            composed_route2_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-2 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn assert_composed_derived_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_derived_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-2 derived-sub-route equivalence",
    )
    .await
}

/// Per-slice equivalence for the derived sub-route: `pdate` is a pure
/// function of the key column `d`, so — unlike route 2's declared
/// sub-route — this is a plain full-refresh-per-value check, same shape as
/// [`assert_composed_route2_per_slice`].
pub(crate) async fn assert_composed_derived_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(pdate AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE pdate = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE pdate = DATE '{v}'",
            composed_derived_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-2 derived-sub-route per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn assert_composed_route3_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route3_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-3 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 3: `last_seen` genuinely settles
/// (`AfterRecurrenceBound`), so each distinct `last_seen` date-value slice
/// must equal the oracle restricted to that same value.
pub(crate) async fn assert_composed_route3_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(last_seen AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE last_seen = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE last_seen = DATE '{v}'",
            composed_route3_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-3 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

/// Append `rows` to the driving source's accumulation-log table
/// (`main.sources_<name>`, created by `render::stage_composed`) — used only
/// as the oracle's own read side for routes 2/3; the direct driver's own
/// per-step delta never reads this table (`composed_delta_values_sql`'s doc
/// comment).
pub(crate) async fn insert_composed_rows_via_backend(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    rows: &[GenRow],
) -> anyhow::Result<()> {
    for row in rows {
        backend
            .execute_sql(&format!(
                "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                row.d.format("%Y-%m-%d"),
                row.id,
                row.val_sql(),
            ))
            .await?;
    }
    Ok(())
}

/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase C6
/// TDD item 4: the composed pool runs with suppression enabled and must
/// stay equivalent under redelivery schedules — `total`/`last_seen` are
/// both registry-backed deterministic aggregates (`SUM`/`MAX`), Comparable
/// under the P3 change-comparability walk, over the recipe's own proven
/// `id` key, so a hand-built `Suppressed` verdict here mirrors exactly what
/// `resolve_write_suppression` would resolve for these fixed classifications
/// (`crate::cumulative::resolve_cumulative_write_suppression`'s own
/// production wiring), without re-deriving the walk over generated SQL this
/// testkit's classifications are never actually parsed from.
pub(crate) fn composed_route2_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["total".to_string()],
    }
}

pub(crate) fn composed_route3_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["last_seen".to_string()],
    }
}

pub(crate) fn composed_derived_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["total".to_string()],
    }
}

pub(crate) async fn drive_composed_route2_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let classification = composed_route2_classification(recipe);
    // `Some(&composed_route2_slice())` — the real `DeltaValues` slice a
    // route-2 model is admitted with — is deliberately **not** passed
    // here. Doing so renders `emit_keyed_fold`'s `target.<col> IN (SELECT
    // DISTINCT <col> FROM (<delta_select>))` predicate, and real DuckDB
    // (confirmed directly against the `duckdb` CLI, v1.5.4/v1.10504)
    // refuses to bind ANY `MERGE` whose `ON` clause combines a derived
    // `USING` subquery with that `IN (SELECT DISTINCT … FROM (subquery))`
    // shape at all — `Invalid Input Error: BindMerge - expected to find an
    // operator of type LOGICAL_GET but got FILTER` — independently of
    // whether the delta is a `VALUES` literal or a real table scan. This
    // is a genuine DuckDB backend limitation for the `DeltaValues`
    // slice-predicate shape, recorded verbatim in `incremental_models.md`
    // §Known Divergences under "Key temporal locality" (the paragraph
    // starting "Route 2's slice-pruned merge … is unexercised against a
    // real backend") and cross-referenced again in that section's §Tests
    // bullet — distinct from the already-documented NOT-NULL nullability
    // blocker. Fixing the emitted
    // predicate shape is production code in `smelt-logical::maintenance::
    // emit`, outside this testkit-only phase's Critical files — flagged
    // here rather than silently worked around. Passing `None` still
    // exercises the real merge mechanics this test actually asserts
    // (write-once `pdate`, additive `total`) against real DuckDB; only the
    // target-scan **pruning** optimisation itself goes unexercised
    // (`incremental_shapes.md` §"Key temporal locality": "pruning is not a
    // write clamp" — every delta row still merges with or without it).
    let slice: Option<&LocalitySlice> = None;

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route2_delta_sql(&rows))
        };
        let steps = driving_steps(
            &window.start.format("%Y-%m-%d").to_string(),
            &window.end.format("%Y-%m-%d").to_string(),
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            slice,
            &composed_route2_suppression(),
            None,
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("composed route-2 window {i} merge failed: {e}"))?;

        assert_composed_route2_equivalence(backend, recipe).await?;
        assert_composed_route2_per_slice(backend, recipe).await?;
    }
    Ok(())
}

/// Drives route 2's derived sub-route through the same direct-driver
/// channel `drive_composed_route2_and_assert` uses for the declared
/// sub-route. `slice` is passed as `None` for the identical reason that
/// function documents (a real DuckDB MERGE-binder limitation on the
/// `DeltaValues` slice-predicate shape) — this recipe's admitted slice is
/// also `LocalitySlice::DeltaValues`.
pub(crate) async fn drive_composed_derived_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let classification = composed_derived_classification(recipe);
    let slice: Option<&LocalitySlice> = None;

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_derived_delta_sql(&rows))
        };
        let steps = driving_steps(
            &window.start.format("%Y-%m-%d").to_string(),
            &window.end.format("%Y-%m-%d").to_string(),
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            slice,
            &composed_derived_suppression(),
            None,
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("composed derived-sub-route window {i} merge failed: {e}"))?;

        assert_composed_derived_equivalence(backend, recipe).await?;
        assert_composed_derived_per_slice(backend, recipe).await?;
    }
    Ok(())
}

pub(crate) async fn drive_composed_route3_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &ComposedRoute3Schedule,
) -> anyhow::Result<()> {
    let classification = composed_route3_classification(recipe);
    let slice = composed_route3_slice();

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route3_delta_sql(&rows))
        };
        let run_date_str = window.run_date.format("%Y-%m-%d").to_string();
        let next_day_str = (window.run_date + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let steps = driving_steps(
            &run_date_str,
            &next_day_str,
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            Some(&slice),
            &composed_route3_suppression(),
            None,
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "composed route-3 window {i} (in-bound redelivery) unexpectedly refused: {e}"
            )
        })?;

        assert_composed_route3_equivalence(backend, recipe).await?;
        assert_composed_route3_per_slice(backend, recipe).await?;
    }
    Ok(())
}
