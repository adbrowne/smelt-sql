//! Cell `P0-2` (`docs/research/20260705-property-discovery-loop.md` §3b;
//! `docs/plans/20260705-property-discovery-loop.md` phase C). The run-schedule
//! generator: a proptest **value** — not just a strategy over abstract shapes,
//! but a concrete sequence of steps a `RunScheduleDriver` replays against a
//! staged `LinkCProject` — over the `events(d DATE, id BIGINT, val DOUBLE)`
//! source shape every `model_shapes` construct seeds today.
//!
//! The defining capability this module exists for (vs a harness that only
//! advances the event-time window): a schedule can **mutate the source between
//! runs** — append a row into an already-processed range, or update/delete a
//! row in place — and the driver **snapshots the source table after every
//! step**, so a cell's oracle can full-refresh over "source state at step k"
//! (design N3) rather than a single pre-populated snapshot that would mask
//! lateness/mutation bugs.

#![allow(dead_code)]

use chrono::NaiveDate;
use duckdb::Connection;
use proptest::prelude::*;

use crate::link_c_harness::{base_request, LinkCProject};

/// One row of the `events` source shape used across `model_shapes`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub d: NaiveDate,
    pub id: i64,
    pub val: f64,
}

/// A full read-back of a source (or output) table, ordered by all columns so
/// two snapshots compare independent of physical row order.
pub type Snapshot = Vec<EventRow>;

/// One step of a run schedule. Every kind but `AdvanceWindowAndRun` mutates the
/// source table directly (bypassing smelt) — the between-run mutation design
/// §3 calls out as the reachable-corner substitute for a full CDF driver.
#[derive(Debug, Clone)]
pub enum ScheduleStep {
    /// Advance the event-time window and run `execute_project` over
    /// `[start, end)`.
    AdvanceWindowAndRun { start: NaiveDate, end: NaiveDate },
    /// Append a row into the source — used to land a "late" row into an
    /// already-processed range between two runs (the SC-1 shape).
    AppendLateRow(EventRow),
    /// Update `val` for an existing row `id` in place (the SC-2 shape:
    /// mutating an already-processed partition between runs).
    InPlaceUpdate { id: i64, new_val: f64 },
    /// Delete an existing row `id` in place.
    InPlaceDelete { id: i64 },
}

/// A generated sequence of `ScheduleStep`s over a source seeded with
/// `events(d, id, val)`.
#[derive(Debug, Clone)]
pub struct RunSchedule(pub Vec<ScheduleStep>);

/// A small schedule: 2-4 window-advance runs, each optionally followed by a
/// late-row append landing back inside the range just processed. `next_id`
/// disambiguates appended rows from the seed set (`seed_id_start..`).
pub fn arb_schedule(base: NaiveDate, seed_id_start: i64) -> impl Strategy<Value = RunSchedule> {
    (2_usize..=4).prop_flat_map(move |n_windows| {
        (
            proptest::collection::vec(1_i64..=2, n_windows),
            proptest::collection::vec(any::<bool>(), n_windows),
            proptest::collection::vec(-1_i64..=1_i64, n_windows),
        )
            .prop_map(move |(spans, append_late, val_jitter)| {
                let mut steps = Vec::new();
                let mut cursor = 0_i64;
                let mut next_id = seed_id_start;
                for i in 0..n_windows {
                    let start = base + chrono::Duration::days(cursor);
                    let end = start + chrono::Duration::days(spans[i]);
                    steps.push(ScheduleStep::AdvanceWindowAndRun { start, end });
                    if append_late[i] {
                        // Land the late row back inside the window just
                        // processed (event-time in [start, end)) — a row
                        // absent at the earlier run, appended after it.
                        steps.push(ScheduleStep::AppendLateRow(EventRow {
                            d: start,
                            id: next_id,
                            val: 10.0 * (val_jitter[i] as f64),
                        }));
                        next_id += 1;
                    }
                    cursor += spans[i];
                }
                RunSchedule(steps)
            })
    })
}

/// A declared upstream shape (`smelt-core/src/sources.rs::MutationProfile`
/// mirrored here for the harness — see design §3, cell `P0-4`). The label a
/// generator claims for the data it emits; the self-check below verifies the
/// claim rather than trusting it (F7: an unverified label silently poisons a
/// verdict — a schedule declared `AppendOnly` that actually mutates in place
/// would make an `AppendOnly` cell's HOLDS verdict meaningless).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationProfile {
    /// Only ever appends new rows (including "late" appends into an
    /// already-processed range) — never rewrites or removes an existing row.
    AppendOnly,
    /// May additionally rewrite (`InPlaceUpdate`) or remove (`InPlaceDelete`)
    /// an existing row between runs.
    Mutable,
}

/// Cell `P0-4`: assert a schedule's steps actually match its declared
/// `MutationProfile`. `AppendOnly` forbids any `InPlaceUpdate`/`InPlaceDelete`
/// step; `Mutable` allows (but does not require) them. Returns the index and
/// step of the first violation.
pub fn check_profile(
    schedule: &RunSchedule,
    profile: MutationProfile,
) -> Result<(), (usize, ScheduleStep)> {
    if profile == MutationProfile::AppendOnly {
        for (i, step) in schedule.0.iter().enumerate() {
            if matches!(
                step,
                ScheduleStep::InPlaceUpdate { .. } | ScheduleStep::InPlaceDelete { .. }
            ) {
                return Err((i, step.clone()));
            }
        }
    }
    Ok(())
}

/// A schedule generator for the `Mutable` profile: like `arb_schedule`, but
/// each window-advance run may additionally be followed by an in-place
/// `UPDATE` of a previously-seeded row (`seed_id_start - 1`, the last row of
/// the fixed two-row seed every `model_shapes` fixture uses) — the SC-2
/// shape. At least one step across the generated schedule is guaranteed to be
/// a mutation, so the profile's claimed shape is actually exercised.
pub fn arb_mutable_schedule(
    base: NaiveDate,
    seed_id_start: i64,
) -> impl Strategy<Value = RunSchedule> {
    (2_usize..=4).prop_flat_map(move |n_windows| {
        (
            proptest::collection::vec(1_i64..=2, n_windows),
            proptest::collection::vec(-5.0_f64..=5.0, n_windows),
            0..n_windows,
        )
            .prop_map(move |(spans, new_vals, mutate_after)| {
                let mut steps = Vec::new();
                let mut cursor = 0_i64;
                for i in 0..n_windows {
                    let start = base + chrono::Duration::days(cursor);
                    let end = start + chrono::Duration::days(spans[i]);
                    steps.push(ScheduleStep::AdvanceWindowAndRun { start, end });
                    if i == mutate_after {
                        // Mutate a row from the fixed seed set that the run
                        // just processed — an in-place rewrite of an
                        // already-processed partition between runs.
                        steps.push(ScheduleStep::InPlaceUpdate {
                            id: seed_id_start - 1,
                            new_val: new_vals[i],
                        });
                    }
                    cursor += spans[i];
                }
                RunSchedule(steps)
            })
    })
}

/// Read back `SELECT d, id, val FROM {table}` ordered for stable comparison.
///
/// `d` is cast to `VARCHAR` in the query and parsed back into a `NaiveDate`
/// here — the workspace `duckdb` dependency doesn't enable the `chrono`
/// feature (a shared manifest knob no other crate needs), so `DATE` has no
/// `FromSql` impl available; going through text avoids widening that shared
/// dependency for a disposable test harness.
pub fn snapshot(conn: &Connection, table: &str) -> Snapshot {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST(d AS VARCHAR), id, val FROM {table} ORDER BY d, id, val"
        ))
        .expect("prepare snapshot query");
    stmt.query_map([], |row| {
        let d_text: String = row.get(0)?;
        Ok((d_text, row.get::<_, i64>(1)?, row.get::<_, f64>(2)?))
    })
    .expect("query snapshot")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect snapshot rows")
    .into_iter()
    .map(|(d_text, id, val)| EventRow {
        d: NaiveDate::parse_from_str(&d_text, "%Y-%m-%d").expect("parse snapshot date"),
        id,
        val,
    })
    .collect()
}

/// Drives a `RunSchedule` against a staged `LinkCProject` + its seeded
/// `source_table` (e.g. `main.sources_events`), snapshotting the source
/// contents after every step. Returns the per-step snapshots, in step order —
/// the step-`k` full-refresh oracle baseline (design N3).
pub struct RunScheduleDriver<'a> {
    pub project: &'a LinkCProject,
    pub target: &'a str,
    pub source_table: &'a str,
}

impl<'a> RunScheduleDriver<'a> {
    pub async fn execute(&self, schedule: &RunSchedule) -> Vec<Snapshot> {
        let mut snapshots = Vec::with_capacity(schedule.0.len());
        for (i, step) in schedule.0.iter().enumerate() {
            match step {
                ScheduleStep::AdvanceWindowAndRun { start, end } => {
                    let mut request = base_request(self.target);
                    request.start = Some(start.format("%Y-%m-%d").to_string());
                    request.end = Some(end.format("%Y-%m-%d").to_string());
                    self.project
                        .run_quiet(&format!("run-{i}"), request)
                        .await
                        .expect("execute_project run must succeed");
                }
                ScheduleStep::AppendLateRow(row) => {
                    let conn = self.project.connect().expect("connect for mutation");
                    conn.execute(
                        &format!(
                            "INSERT INTO {} VALUES ('{}', {}, {:.1})",
                            self.source_table,
                            row.d.format("%Y-%m-%d"),
                            row.id,
                            row.val
                        ),
                        [],
                    )
                    .expect("append late row");
                }
                ScheduleStep::InPlaceUpdate { id, new_val } => {
                    let conn = self.project.connect().expect("connect for mutation");
                    conn.execute(
                        &format!(
                            "UPDATE {} SET val = {:.1} WHERE id = {}",
                            self.source_table, new_val, id
                        ),
                        [],
                    )
                    .expect("in-place update");
                }
                ScheduleStep::InPlaceDelete { id } => {
                    let conn = self.project.connect().expect("connect for mutation");
                    conn.execute(
                        &format!("DELETE FROM {} WHERE id = {}", self.source_table, id),
                        [],
                    )
                    .expect("in-place delete");
                }
            }
            let conn = self.project.connect().expect("connect for snapshot");
            snapshots.push(snapshot(&conn, self.source_table));
        }
        snapshots
    }
}
