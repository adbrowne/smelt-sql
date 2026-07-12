//! Schema-generic data + schedule generation for the append-only
//! partition-grain conformance pool
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3;
//! design doc `docs/research/20260711-generative-maintenance-conformance.md`
//! §5 "Data and schedule generation"). Replaces the fixed `events(d,id,val)`
//! hard-coding in [`crate::run_schedule`] with generation keyed off a
//! [`crate::recipe::ModelRecipe`]'s own [`crate::recipe::SourceRecipe`]
//! shape — today that shape is always the `events(d DATE, id INTEGER, val
//! INTEGER)` append-only source `recipe.rs`'s Phase 1/2 pool generates, so
//! this module's row type ([`GenRow`]) is concretely that shape rather than
//! a fully column-generic representation; widening `SourceRecipe` to other
//! schemas is future scope, and this module widens alongside it.
//!
//! [`crate::run_schedule`] (the `events(d,id,val)`-over-`f64` catalogue
//! harness) stays untouched here — it retires in Phase 11 per the plan.

#![allow(dead_code)]

use chrono::NaiveDate;
use duckdb::Connection;
use proptest::prelude::*;

use crate::recipe::{arb_payload_value, ModelRecipe, SourceRecipe};

/// One row of the `events`-shaped source every Phase 1/2 recipe stages —
/// integer-valued `val` (design §5's numeric-payload discipline), unlike
/// `run_schedule::EventRow`'s legacy `f64`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenRow {
    pub d: NaiveDate,
    pub id: i64,
    pub val: i64,
}

impl GenRow {
    /// The row's event-time value — the field [`crate::s_tracker::STracker`]'s
    /// window filter reads.
    pub fn event_time(&self) -> NaiveDate {
        self.d
    }
}

/// One step of a generated conformance schedule. Only two of the four
/// existing `run_schedule::ScheduleStep` kinds apply to the append-only pool
/// (Phase 3 scope, per the plan's Critical files: no mutable-source
/// machinery this phase) — `InPlaceUpdate`/`InPlaceDelete` are Phase 4 scope.
#[derive(Debug, Clone)]
pub enum ConformanceStep {
    /// Insert `rows` into the source (each landing inside `[start, end)`, or
    /// empty for a pure catch-up re-run), then run `execute_project` over
    /// `[start, end)`.
    RunWindow {
        start: NaiveDate,
        end: NaiveDate,
        rows: Vec<GenRow>,
    },
    /// Insert a row into the source with NO accompanying run — a late
    /// arrival landing back inside an already-processed window's range. Its
    /// window must be re-run later in the schedule (a `RunWindow` with the
    /// same `[start, end)` and empty `rows`) for the S-tracker to ever admit
    /// it (design §6: "a genuinely late row is outside S until its window is
    /// re-run").
    AppendLateRow(GenRow),
}

/// A generated sequence of [`ConformanceStep`]s over one [`SourceRecipe`].
#[derive(Debug, Clone)]
pub struct ConformanceSchedule(pub Vec<ConformanceStep>);

/// Schema-generic schedule generator (design §5): 2-3 disjoint one-day
/// windows, each seeded with 1-2 rows landing inside its own range before
/// the window is first run; a subset of windows additionally receive a late
/// row AFTER their first run (landing back inside their own range); the
/// schedule always ends with a catch-up re-run (design §5: "every generated
/// schedule ends by re-running every window affected by late … steps") of
/// every window that received a late row, so a settled point always exists
/// for the S-restricted assertion to land on.
pub fn arb_schedule_for(recipe: &ModelRecipe) -> impl Strategy<Value = ConformanceSchedule> {
    let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date");
    let source = recipe.source.clone();
    (2_usize..=3).prop_flat_map(move |n_windows| {
        let source = source.clone();
        (
            proptest::collection::vec(
                proptest::collection::vec(arb_payload_value(), 1..=2),
                n_windows,
            ),
            proptest::collection::vec(any::<bool>(), n_windows),
        )
            .prop_map(move |(window_vals, gets_late)| {
                build_schedule(base, &source, &window_vals, &gets_late)
            })
    })
}

fn build_schedule(
    base: NaiveDate,
    _source: &SourceRecipe,
    window_vals: &[Vec<i64>],
    gets_late: &[bool],
) -> ConformanceSchedule {
    let mut steps = Vec::new();
    let mut next_id = 1_i64;
    let mut late_windows: Vec<(NaiveDate, NaiveDate)> = Vec::new();
    let mut windows = Vec::new();

    for (i, vals) in window_vals.iter().enumerate() {
        let start = base + chrono::Duration::days(i as i64);
        let end = start + chrono::Duration::days(1);
        windows.push((start, end));

        let rows = vals
            .iter()
            .map(|val| {
                let row = GenRow {
                    d: start,
                    id: next_id,
                    val: *val,
                };
                next_id += 1;
                row
            })
            .collect();
        steps.push(ConformanceStep::RunWindow { start, end, rows });
    }

    for (i, &(start, _end)) in windows.iter().enumerate() {
        if gets_late.get(i).copied().unwrap_or(false) {
            let late_row = GenRow {
                d: start,
                id: next_id,
                val: 7,
            };
            next_id += 1;
            steps.push(ConformanceStep::AppendLateRow(late_row));
            late_windows.push(windows[i]);
        }
    }

    // Catch-up: re-run every window that received a late row, so a settled
    // point always exists (design §5).
    for (start, end) in late_windows {
        steps.push(ConformanceStep::RunWindow {
            start,
            end,
            rows: Vec::new(),
        });
    }

    ConformanceSchedule(steps)
}

/// Read the full current contents of `source`'s physical table
/// (`main.sources_<name>`) — the S-tracker's per-run snapshot input.
///
/// `d` goes through `CAST(... AS VARCHAR)` and is parsed back — the
/// workspace `duckdb` dependency has no `chrono` feature enabled
/// (`run_schedule::snapshot`'s documented workaround, mirrored here).
pub fn read_source_snapshot(conn: &Connection, source: &SourceRecipe) -> Vec<GenRow> {
    let table = format!("main.sources_{}", source.name);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST({d} AS VARCHAR), {id}, {val} FROM {table} ORDER BY {d}, {id}",
            d = source.clock_column,
            id = source.key_column,
            val = source.payload_column,
        ))
        .expect("prepare source snapshot query");
    stmt.query_map([], |row| {
        let d_text: String = row.get(0)?;
        Ok((d_text, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
    })
    .expect("query source snapshot")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect source snapshot rows")
    .into_iter()
    .map(|(d_text, id, val)| GenRow {
        d: NaiveDate::parse_from_str(&d_text, "%Y-%m-%d").expect("parse snapshot date"),
        id,
        val,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{arb_recipe, RecipePool};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// Every generated schedule contains at least one `RunWindow` step, and
    /// every `AppendLateRow`'s window is re-run afterward (a self-check on
    /// the generator itself — the gate's driver relies on this invariant).
    #[test]
    fn generated_schedule_reruns_every_late_window() {
        let mut runner = TestRunner::deterministic();
        let recipe_strat = arb_recipe(RecipePool::partition_append_only());

        for _ in 0..30 {
            let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
            let schedule = arb_schedule_for(&recipe)
                .new_tree(&mut runner)
                .unwrap()
                .current();

            let mut run_windows: Vec<(NaiveDate, NaiveDate)> = Vec::new();
            let mut late_windows: Vec<NaiveDate> = Vec::new();
            for step in &schedule.0 {
                match step {
                    ConformanceStep::RunWindow { start, end, .. } => {
                        run_windows.push((*start, *end))
                    }
                    ConformanceStep::AppendLateRow(row) => late_windows.push(row.d),
                }
            }
            assert!(
                !run_windows.is_empty(),
                "schedule {schedule:?} has no RunWindow step"
            );
            for late_day in late_windows {
                let reruns = run_windows
                    .iter()
                    .filter(|(s, e)| late_day >= *s && late_day < *e)
                    .count();
                assert!(
                    reruns >= 2,
                    "late row landing on {late_day} must have its window run at least \
                     twice (initial + catch-up) in schedule {schedule:?}"
                );
            }
        }
    }
}
