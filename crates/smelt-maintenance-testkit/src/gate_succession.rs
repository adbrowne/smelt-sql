//! The keyed-succession (SCD2) family's own stage/insert/assert/drive
//! quartet (`docs/outcomes/20260906-scd2-keyed-succession/phases/
//! 07a-plan.md`), modelled on `families::gate_keyed`'s shape and naming.
//!
//! **Not under `families/`.** That module is file-wide gated
//! (`#![cfg(any(feature = "spark", feature = "bigquery"))]`) because its
//! `ConformanceBackend` abstraction exists to share test bodies across the
//! Spark/BigQuery generative-conformance twins, which the succession grain
//! does not have yet (Spark/BigQuery take the recorded availability
//! downgrade for this grain — `docs/outcomes/20260906-scd2-keyed-succession/
//! outcome.md` §Out of scope). A DuckDB-only family that the per-PR
//! reference leg (`crates/smelt-cli/tests/maintenance_conformance/`, built
//! with no optional feature) must call at all cannot live inside a module
//! gated off by default — so this quartet lives here instead, ungated,
//! alongside this crate's other always-compiled modules, in the plainer
//! `project.connect()`/`project.backend()` idiom
//! `crates/smelt-cli/tests/maintenance_conformance/gate/keyed_support.rs` /
//! `keyed_oracle.rs` already use for the DuckDB reference leg's own keyed
//! family, rather than `families::gate_keyed`'s target-generalized
//! `dyn Backend`/`ConformanceTarget` parametrization (which exists to share
//! bodies with backends this grain does not support yet).
//! [`stage_succession_recipe_for`] still takes a
//! [`crate::recipe::ConformanceTarget`] and still refuses non-DuckDB (via
//! [`crate::render::stage_succession_for_target`]) so a later phase widening
//! this family to more backends only has to change the render-layer
//! staging, not this quartet's shape.

use anyhow::Result;
use chrono::{Datelike, NaiveDate, NaiveDateTime};

use crate::link_c_harness::{base_request, LinkCProject};
use crate::oracle::multiset_equal_via_backend;
use crate::recipe::{ConformanceTarget, SuccessionRecipe};
use crate::render;

/// One event row for a [`SuccessionRecipe`]'s driving source: `key` +
/// `event_time` (the succession clock, `changed_at`) + `arrival` (the
/// source's declared `timeseries.partition_column`, `arrival_date` — a run's
/// window scans THIS column, not `event_time`) + `payload` (the `tier`
/// VARCHAR column) + `is_deleted` (the optional `QUALIFY NOT is_deleted`
/// delete flag, always physically present and `NOT NULL`).
#[derive(Debug, Clone)]
pub struct SuccessionEventRow {
    pub key: i64,
    pub event_time: NaiveDateTime,
    pub arrival: NaiveDate,
    pub payload: String,
    pub is_deleted: bool,
}

impl SuccessionEventRow {
    /// A non-deleted event landing on its own event day (`arrival ==
    /// event_time`'s date) — the common case every splice/lag smoke case
    /// starts from.
    pub fn new(key: i64, event_time: NaiveDateTime, payload: &str) -> Self {
        Self {
            key,
            event_time,
            arrival: event_time.date(),
            payload: payload.to_string(),
            is_deleted: false,
        }
    }

    /// [`Self::new`], additionally declaring a distinct arrival date — the
    /// late-arrival/splice shape: an event whose `event_time` places it
    /// between two already-processed events of the same key, but whose
    /// `arrival` lands in a LATER window than either of them.
    pub fn late(key: i64, event_time: NaiveDateTime, payload: &str, arrival: NaiveDate) -> Self {
        Self {
            key,
            event_time,
            arrival,
            payload: payload.to_string(),
            is_deleted: false,
        }
    }

    /// [`Self::new`]'s tombstoning counterpart: a DELETE event landing on its
    /// own event day (`is_deleted: true`).
    pub fn deleted(key: i64, event_time: NaiveDateTime, payload: &str) -> Self {
        Self {
            key,
            event_time,
            arrival: event_time.date(),
            payload: payload.to_string(),
            is_deleted: true,
        }
    }

    /// [`Self::late`]'s tombstoning counterpart: a DELETE event whose
    /// `arrival` is distinct from (and later than) its `event_time`'s date.
    pub fn deleted_late(
        key: i64,
        event_time: NaiveDateTime,
        payload: &str,
        arrival: NaiveDate,
    ) -> Self {
        Self {
            key,
            event_time,
            arrival,
            payload: payload.to_string(),
            is_deleted: true,
        }
    }
}

/// Stage a [`SuccessionRecipe`] into a fresh temp project dir targeting
/// `target` (`render::stage_succession_for_target`'s DuckDB-only staging;
/// non-DuckDB refuses loudly rather than silently doing nothing).
pub fn stage_succession_recipe_for(
    recipe: &SuccessionRecipe,
    tmp: &tempfile::TempDir,
    target: ConformanceTarget,
) -> Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_succession_for_target(recipe, &project_dir, &db_path, target)
}

/// Insert one row into a [`SuccessionRecipe`]'s staged driving-source table.
///
/// The column list is derived from `recipe.source`'s own `partition_column`/
/// `delete_flag_column` `Option`s (rather than assuming both are physically
/// present), matching the DDL `render::stage_succession_for_target` staged:
/// key, clock, an arrival column ONLY when `partition_column` is distinct
/// from `clock_column`, payload, then the delete flag ONLY when
/// `delete_flag_column` is declared. An event-time-partitioned source
/// (`SourceRecipe::succession_events_event_time_partitioned`) has no arrival
/// column to insert into.
pub fn insert_row_succession_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    row: &SuccessionEventRow,
) -> Result<()> {
    let src = &recipe.source;
    let partition_col = src.partition_column.as_deref().unwrap_or(&src.clock_column);

    let mut values = format!(
        "{key}, TIMESTAMP '{event_time}'",
        key = row.key,
        event_time = row.event_time.format("%Y-%m-%d %H:%M:%S"),
    );
    if partition_col != src.clock_column {
        values.push_str(&format!(", DATE '{}'", row.arrival.format("%Y-%m-%d")));
    }
    values.push_str(&format!(", '{}'", row.payload));
    if src.delete_flag_column.is_some() {
        values.push_str(if row.is_deleted { ", TRUE" } else { ", FALSE" });
    }

    let conn = project.connect()?;
    conn.execute(
        &format!("INSERT INTO main.sources_{} VALUES ({values})", src.name),
        [],
    )?;
    Ok(())
}

/// Mutate a staged driving-source row's payload IN PLACE — an `UPDATE`
/// naming `key`/`event_time` exactly (never a range), so the row count for
/// its partition is unchanged and only its content fingerprint moves. Pairs
/// with the append-only posture probe's fingerprint leg
/// (`crate::gate_succession`'s sibling `crates/smelt-runtime/tests/
/// succession_probes.rs`, `docs/outcomes/20260906-scd2-keyed-succession/
/// phases/06c-plan.md`): applied to a row in an already-baselined (closed)
/// partition, the next run must fail loud with
/// `SourceMutationProfileViolated` rather than silently accepting the
/// mutation as a legitimate append.
pub fn mutate_row_payload_in_place_succession(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    key: i64,
    event_time: NaiveDateTime,
    new_payload: &str,
) -> Result<()> {
    let src = &recipe.source;
    let conn = project.connect()?;
    let updated = conn.execute(
        &format!(
            "UPDATE main.sources_{name} SET {payload_col} = '{new_payload}' WHERE {key_col} = \
             {key} AND {clock_col} = TIMESTAMP '{event_time}'",
            name = src.name,
            payload_col = src.payload_column,
            key_col = src.key_column,
            clock_col = src.clock_column,
            event_time = event_time.format("%Y-%m-%d %H:%M:%S"),
        ),
        [],
    )?;
    if updated != 1 {
        anyhow::bail!(
            "mutate_row_payload_in_place_succession: expected to update exactly one row for \
             key {key} at {event_time}, updated {updated}"
        );
    }
    Ok(())
}

/// The end-state equivalence assertion for a [`SuccessionRecipe`]: the
/// maintained table's full contents equal the model's own SQL evaluated over
/// the CURRENT physical source table (`render::render_succession_oracle_body_over`) —
/// no S-restricted materialization needed (unlike the keyed pool's
/// `STracker`): the succession-patch technique's own equivalence invariant
/// is against a full-refresh oracle over every event landed so far, exactly
/// the leg `crates/smelt-runtime/tests/statement_parity/succession.rs`'s
/// `succession_patch_result_equals_full_refresh` already proves for one
/// fixed recipe — this reuses that same `oracle_relation` seam
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/07a-plan.md` task
/// 5) rather than introducing a second comparator.
pub async fn assert_succession_equivalence_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
) -> Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let source_ref = format!("main.sources_{}", recipe.source.name);
    let oracle_sql = render::render_succession_oracle_body_over(recipe, &source_ref);
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "succession end-state equivalence violated for model {:?}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// [`assert_succession_equivalence_for`] parameterised by contract-lattice
/// point (phase 7d), mirroring `crates/smelt-cli/tests/
/// maintenance_conformance/gate/partition_pool.rs::assert_equivalence_at_point_with_frontier`'s
/// dispatch on `smelt_logical::contract::oracle_obligation` rather than
/// re-deriving a per-point comparator: `Exact`/`ExactOverRestrictedS` (the
/// succession grain has no `frozen_horizon` posture of its own, but the
/// dispatch stays exhaustive over `OracleObligation` for symmetry with the
/// keyed pool's own comparator) delegate to
/// [`assert_succession_equivalence_for`]'s unrestricted oracle;
/// `ExactOverProcessedSWithLagBound` (`deferral`) compares against
/// [`render::render_succession_oracle_body_over`] evaluated over the
/// PROCESSED source restriction (`arrival < processed_arrival_frontier`,
/// reusing that function's own relation-substitution seam — no second
/// comparator), then calls
/// [`smelt_logical::contract::deferral::settled_lag_bound`] over the
/// landed-but-unprocessed event times (rows whose arrival is at/after the
/// processed frontier) read back from the source. `processed_arrival_frontier`
/// is the caller-tracked maintained frontier (days-from-CE, over the
/// source's declared arrival/partition column) — the succession family has
/// no `STracker` to derive it from, so the caller (which knows which window
/// it last drove) supplies it directly. `input_frontier` is only consulted
/// for the deferral obligation, exactly like the keyed pool's own signature.
pub async fn assert_succession_equivalence_at_point(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    point: &smelt_logical::contract::ContractPoint,
    processed_arrival_frontier: i64,
    input_frontier: Option<i64>,
) -> Result<()> {
    use smelt_logical::contract::{oracle_obligation, ContractPoint, OracleObligation};

    match oracle_obligation(point) {
        OracleObligation::Exact | OracleObligation::ExactOverRestrictedS => {
            assert_succession_equivalence_for(project, recipe).await
        }
        OracleObligation::ExactOverProcessedSWithLagBound => {
            let d = match point {
                ContractPoint::Deferral { d } => *d,
                _ => anyhow::bail!(
                    "ExactOverProcessedSWithLagBound is only licensed for \
                     ContractPoint::Deferral, got {point:?}"
                ),
            };

            let backend = project.backend().await?;
            let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
            let source_ref = format!("main.sources_{}", recipe.source.name);
            let partition_col = recipe
                .source
                .partition_column
                .as_deref()
                .unwrap_or(&recipe.source.clock_column);
            let frontier_date =
                NaiveDate::from_num_days_from_ce_opt(processed_arrival_frontier as i32)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                    "invalid processed_arrival_frontier {processed_arrival_frontier} (not a \
                     valid days-from-CE date)"
                )
                    })?;
            let frontier_date_str = frontier_date.format("%Y-%m-%d");
            let processed_source_ref = format!(
                "(SELECT * FROM {source_ref} WHERE {partition_col} < DATE '{frontier_date_str}')"
            );
            let oracle_sql =
                render::render_succession_oracle_body_over(recipe, &processed_source_ref);
            let equal =
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
            if !equal {
                anyhow::bail!(
                    "succession equivalence violated for model {:?} under point {point:?}: \
                     maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                    recipe.model_name
                );
            }

            let input_frontier = input_frontier.ok_or_else(|| {
                anyhow::anyhow!(
                    "point {point:?} has an ExactOverProcessedSWithLagBound oracle obligation \
                     but no input_frontier was supplied"
                )
            })?;

            let conn = project.connect()?;
            let clock_col = &recipe.source.clock_column;
            // `duckdb`'s `FromSql` has no chrono impl in this crate's feature
            // set, so the event time's date component is read back as text
            // (`strftime`) and parsed rather than bound to a `NaiveDateTime`
            // column type.
            let mut stmt = conn.prepare(&format!(
                "SELECT strftime(CAST({clock_col} AS DATE), '%Y-%m-%d') FROM {source_ref} WHERE \
                 {partition_col} >= DATE '{frontier_date_str}'"
            ))?;
            let unprocessed_event_times: Vec<i64> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .map(|s| {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .map(|d| d.num_days_from_ce() as i64)
                        .map_err(|e| {
                            anyhow::anyhow!("parse event-time date {s:?} for {clock_col}: {e}")
                        })
                })
                .collect::<Result<Vec<_>>>()?;

            smelt_logical::contract::deferral::settled_lag_bound(
                &unprocessed_event_times,
                input_frontier,
                d,
            )
            .map_err(|violation| {
                anyhow::anyhow!(
                    "deferral lag bound violated for succession model {:?}: {violation:?}",
                    recipe.model_name
                )
            })?;
            Ok(())
        }
    }
}

/// Drive one window's worth of `rows` against `project`/`recipe` through the
/// real `execute_project` pipeline (`LinkCProject::run_quiet`), then assert
/// end-state equivalence — the succession-family counterpart of
/// `families::gate_keyed::drive_keyed_and_assert_for`, restricted to a
/// single window per call so a multi-window smoke schedule can insert a
/// splice/late row between calls.
pub async fn drive_succession_window_and_assert_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    run_id: &str,
    window_start: NaiveDate,
    window_end: NaiveDate,
    rows: &[SuccessionEventRow],
) -> Result<()> {
    for row in rows {
        insert_row_succession_for(project, recipe, row)?;
    }

    let mut request = base_request("dev");
    request.start = Some(window_start.format("%Y-%m-%d").to_string());
    request.end = Some(window_end.format("%Y-%m-%d").to_string());
    project
        .run_quiet(run_id, request)
        .await
        .map_err(|e| anyhow::anyhow!("succession run {run_id:?} failed: {e}"))?;

    assert_succession_equivalence_for(project, recipe).await
}

/// Snapshot a relation's full contents as a comparable, order-independent
/// value (`ORDER BY ALL` + `smelt_runtime::check_runner::batches_to_rows`) —
/// this crate's counterpart of `crates/smelt-cli/tests/
/// maintenance_conformance/gate/support.rs::snapshot_table_rows`'s idiom
/// (that helper lives in the `smelt-cli` test binary and isn't reachable
/// from here).
async fn snapshot_relation_rows(
    backend: &dyn smelt_backend::Backend,
    relation: &str,
) -> Result<Vec<std::collections::BTreeMap<String, String>>> {
    let batches = backend
        .execute_sql(&format!("SELECT * FROM {relation} ORDER BY ALL"))
        .await
        .map_err(|e| anyhow::anyhow!("snapshot {relation:?}: {e}"))?;
    Ok(smelt_runtime::check_runner::batches_to_rows(&batches))
}

/// Drive one window expecting `run_quiet` to FAIL with a probe refusal (e.g.
/// `SuccessionClockTie`) — snapshots the presented table and the tombstone
/// ledger before driving, asserts BOTH are byte-identical after the refused
/// run (the probe must run before any write), and returns the run's error
/// message for the caller to assert the refusal reason against. The
/// tombstone table name follows the phase-1 `<presented table>__tombstones`
/// pin (`smelt_logical::maintenance::emit::tombstone_table_name`).
pub async fn drive_succession_window_expect_probe_failure(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    run_id: &str,
    window_start: NaiveDate,
    window_end: NaiveDate,
    rows: &[SuccessionEventRow],
) -> Result<String> {
    for row in rows {
        insert_row_succession_for(project, recipe, row)?;
    }

    let backend = project.backend().await?;
    let presented = format!("main.{}", recipe.model_name);
    let tombstones = format!(
        "main.{}",
        smelt_logical::maintenance::emit::tombstone_table_name(&recipe.model_name)
    );
    let before_presented = snapshot_relation_rows(backend.as_ref(), &presented).await?;
    let before_tombstones = snapshot_relation_rows(backend.as_ref(), &tombstones).await?;

    let mut request = base_request("dev");
    request.start = Some(window_start.format("%Y-%m-%d").to_string());
    request.end = Some(window_end.format("%Y-%m-%d").to_string());
    let message = match project.run_quiet(run_id, request).await {
        Ok(_) => anyhow::bail!(
            "expected succession run {run_id:?} to fail with a probe refusal, but it succeeded"
        ),
        Err(e) => format!("{e:#}"),
    };

    let after_presented = snapshot_relation_rows(backend.as_ref(), &presented).await?;
    let after_tombstones = snapshot_relation_rows(backend.as_ref(), &tombstones).await?;
    if before_presented != after_presented {
        anyhow::bail!(
            "presented table {presented:?} changed despite the probe refusal for run {run_id:?}"
        );
    }
    if before_tombstones != after_tombstones {
        anyhow::bail!(
            "tombstone ledger {tombstones:?} changed despite the probe refusal for run \
             {run_id:?}"
        );
    }

    Ok(message)
}

#[cfg(test)]
mod tests {
    use crate::recipe::{ConformanceTarget, SourceRecipe, SuccessionRecipe};

    use super::*;

    /// `succession_equivalence_at_default_point_matches_the_unrestricted_oracle`
    /// (phase 7d test 3): harness self-check —
    /// [`assert_succession_equivalence_at_point`] under
    /// `ContractPoint::Default` is byte-for-byte
    /// [`assert_succession_equivalence_for`]'s own behaviour: both succeed
    /// after a clean window, and both fail identically once the maintained
    /// table is corrupted.
    #[tokio::test]
    async fn succession_equivalence_at_default_point_matches_the_unrestricted_oracle() {
        let recipe = SuccessionRecipe::new_lead();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
            .expect("stage succession recipe");

        drive_succession_window_and_assert_for(
            &project,
            &recipe,
            "succession-default-point-1",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
            &[SuccessionEventRow::new(
                1,
                NaiveDate::from_ymd_opt(2026, 1, 1)
                    .unwrap()
                    .and_hms_opt(8, 0, 0)
                    .unwrap(),
                "gold",
            )],
        )
        .await
        .expect("window 1 must succeed and match the oracle");

        let point = smelt_logical::contract::ContractPoint::Default;
        assert_succession_equivalence_at_point(&project, &recipe, &point, 0, None)
            .await
            .expect(
                "assert_succession_equivalence_at_point(Default) must hold whenever \
                 assert_succession_equivalence_for holds",
            );

        // Corrupt the maintained table: both comparators must now fail.
        let backend = project.backend().await.expect("backend");
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("DELETE FROM main.{}", recipe.model_name),
        )
        .await
        .expect("delete every row from the maintained table");

        let unrestricted_result = assert_succession_equivalence_for(&project, &recipe).await;
        let at_point_result =
            assert_succession_equivalence_at_point(&project, &recipe, &point, 0, None).await;
        assert!(
            unrestricted_result.is_err() && at_point_result.is_err(),
            "both comparators must reject the corrupted state: unrestricted={unrestricted_result:?}, \
             at_point={at_point_result:?}"
        );
    }

    /// `succession_event_row_deleted_carries_the_flag` (phase 7b test 11):
    /// [`SuccessionEventRow::deleted`]/[`SuccessionEventRow::deleted_late`]
    /// set `is_deleted`, and [`insert_row_succession_for`] omits the arrival
    /// column for an event-time-partitioned source (proved by the insert
    /// succeeding at all against a table with no arrival column, then
    /// reading the flag back).
    #[test]
    fn succession_event_row_deleted_carries_the_flag() {
        let deleted = SuccessionEventRow::deleted(
            1,
            NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap(),
            "gold",
        );
        assert!(deleted.is_deleted);
        let deleted_late = SuccessionEventRow::deleted_late(
            1,
            NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap(),
            "gold",
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
        );
        assert!(deleted_late.is_deleted);
        assert_eq!(
            deleted_late.arrival,
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap()
        );

        let recipe = SuccessionRecipe::new_lead()
            .with_delete_filter()
            .with_source(SourceRecipe::succession_events_event_time_partitioned());
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
            .expect("stage event-time-partitioned recipe");
        insert_row_succession_for(&project, &recipe, &deleted)
            .expect("insert must omit the arrival column and succeed");

        let conn = project.connect().expect("connect");
        let is_deleted: bool = conn
            .query_row(
                &format!(
                    "SELECT is_deleted FROM main.sources_{} WHERE customer_id = 1",
                    recipe.source.name
                ),
                [],
                |r| r.get(0),
            )
            .expect("read back is_deleted");
        assert!(is_deleted, "the deleted row's is_deleted flag must persist");
    }

    /// `mutate_row_payload_in_place_succession_changes_content_not_count`
    /// (phase 6c task 6): the helper updates exactly the named row's
    /// payload, leaving the source table's total row count unchanged — the
    /// fingerprint-only mutation the append-only posture probe's
    /// closed-partition leg must catch.
    #[tokio::test]
    async fn mutate_row_payload_in_place_succession_changes_content_not_count() {
        let recipe = SuccessionRecipe::new_lead();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_succession_recipe_for(&recipe, &tmp, ConformanceTarget::DuckDb)
            .expect("stage succession recipe");
        let event_time = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap();
        insert_row_succession_for(
            &project,
            &recipe,
            &SuccessionEventRow::new(1, event_time, "gold"),
        )
        .expect("insert row");

        let conn = project.connect().expect("connect");
        let count_before: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM main.sources_{}", recipe.source.name),
                [],
                |r| r.get(0),
            )
            .expect("count before");

        mutate_row_payload_in_place_succession(&project, &recipe, 1, event_time, "platinum")
            .expect("mutate in place");

        let count_after: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM main.sources_{}", recipe.source.name),
                [],
                |r| r.get(0),
            )
            .expect("count after");
        assert_eq!(count_before, count_after, "row count must not change");

        let after_conn = project.connect().expect("reconnect");
        let payload: String = after_conn
            .query_row(
                &format!(
                    "SELECT tier FROM main.sources_{} WHERE customer_id = 1",
                    recipe.source.name
                ),
                [],
                |r| r.get(0),
            )
            .expect("read back tier");
        assert_eq!(payload, "platinum");
    }
}
