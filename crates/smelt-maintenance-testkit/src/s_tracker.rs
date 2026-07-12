//! The S-tracker (`docs/plans/20260712-generative-maintenance-conformance.md`
//! Phase 3; design doc
//! `docs/research/20260711-generative-maintenance-conformance.md` §6 "The
//! equivalence oracle, generalized"): records `(window, per-source
//! snapshot)` per run and derives `S_k` — "the rows visible-in-window at
//! some processed run ≤ k" — the append-only pool's full-refresh oracle
//! baseline. Lateness needs no special-casing (design §6): a row appended
//! between two runs simply isn't in any run's recorded
//! snapshot-restricted-to-that-run's-own-window until a run's snapshot
//! actually contains it AND that run's own window covers it.
//!
//! Phase 4 (`docs/plans/20260712-generative-maintenance-conformance.md`;
//! design §6 "Mixed models") extends the tracker with per-window outstanding-
//! dimension-mutation bookkeeping: [`STracker::record_dimension_mutation`]
//! marks a window's region as having an unresolved mutation against a
//! mutable dimension the model reads; [`STracker::record_run`] clears any
//! outstanding mutation for THAT SAME window (a re-run of the affected
//! window is the catch-up — MP11's `ColumnScopedMerge` write is scoped to
//! the run's own window, so re-running an unrelated window does not resync
//! it); [`STracker::oracle_mode`] reports [`crate::oracle_modes::OracleMode`]
//! accordingly.

#![allow(dead_code)]

use std::collections::HashSet;

use chrono::NaiveDate;
use duckdb::Connection;

use crate::oracle_modes::OracleMode;
use crate::recipe::{ModelEdit, ModelRecipe, SourceRecipe};
use crate::render;
use crate::schedule_gen::GenRow;

struct RunRecord {
    start: NaiveDate,
    end: NaiveDate,
    snapshot: Vec<GenRow>,
}

/// Tracks per-run `(window, source snapshot)` pairs for one [`SourceRecipe`]
/// and derives `S_k` (design §6). Also tracks outstanding mutable-dimension
/// mutations by the window they affect (Phase 4).
pub struct STracker {
    source: SourceRecipe,
    runs: Vec<RunRecord>,
    outstanding_mutated_windows: HashSet<(NaiveDate, NaiveDate)>,
}

impl STracker {
    pub fn new(source: &SourceRecipe) -> Self {
        Self {
            source: source.clone(),
            runs: Vec::new(),
            outstanding_mutated_windows: HashSet::new(),
        }
    }

    /// Record one run: `snapshot` is the source's full contents AT THE TIME
    /// this run executed (`schedule_gen::read_source_snapshot`, taken
    /// immediately before `execute_project` is called for this window).
    /// Returns the run's index — `s_at(index)` is `S_k` after this run. A
    /// run of window `[start, end)` also clears any outstanding dimension
    /// mutation recorded against THAT SAME window (Phase 4's catch-up rule —
    /// see the module doc comment for why re-running an unrelated window
    /// does not clear it).
    pub fn record_run(&mut self, start: NaiveDate, end: NaiveDate, snapshot: Vec<GenRow>) -> usize {
        self.outstanding_mutated_windows.remove(&(start, end));
        self.runs.push(RunRecord {
            start,
            end,
            snapshot,
        });
        self.runs.len() - 1
    }

    /// Record a `full_refresh` run (Phase 6; design §5 "full_refresh
    /// interleave"): the run reads and reflects the ENTIRE current source
    /// snapshot, not a bounded window, so it is recorded as if its own
    /// window covered every possible date — every row in `snapshot` enters
    /// `S` from this point on, regardless of which window it lands in. This
    /// is exactly what a real `full_refresh` run does: recompute the whole
    /// model over everything currently in the source, resetting the ledger's
    /// coverage for every region the refresh touched.
    pub fn record_full_refresh(&mut self, snapshot: Vec<GenRow>) -> usize {
        self.record_run(NaiveDate::MIN, NaiveDate::MAX, snapshot)
    }

    /// Mark `window` as having an outstanding mutable-dimension mutation
    /// (Phase 4; design §6 "Mixed models"): full equivalence assertion for
    /// this window defers to the next run of THAT SAME window (the catch-up
    /// — [`Self::record_run`] clears it).
    pub fn record_dimension_mutation(&mut self, window: (NaiveDate, NaiveDate)) {
        self.outstanding_mutated_windows.insert(window);
    }

    /// The tracker's current oracle mode (design §6 "Mixed models"):
    /// [`OracleMode::SettledPoint`] while any window has an outstanding
    /// dimension mutation, [`OracleMode::SRestricted`] once every recorded
    /// mutation has been caught up by a re-run of its own window.
    pub fn oracle_mode(&self) -> OracleMode {
        if self.outstanding_mutated_windows.is_empty() {
            OracleMode::SRestricted
        } else {
            OracleMode::SettledPoint
        }
    }

    /// `S_k`: the deduplicated union, over every run `0..=k`, of that run's
    /// snapshot restricted to that run's own `[start, end)` window (design
    /// §6). A row present in a later run's snapshot but outside every run's
    /// own window (i.e. appended but never covered by a re-run) never enters
    /// this set — the horizon semantics fall out with no special-casing.
    pub fn s_at(&self, k: usize) -> Vec<GenRow> {
        let mut seen: HashSet<GenRow> = HashSet::new();
        for run in self.runs.iter().take(k + 1) {
            for row in &run.snapshot {
                let t = row.event_time();
                if t >= run.start && t < run.end {
                    seen.insert(row.clone());
                }
            }
        }
        let mut rows: Vec<GenRow> = seen.into_iter().collect();
        rows.sort_by_key(|a| (a.d, a.id));
        rows
    }

    /// The most recently recorded run's index, or `None` before any run has
    /// been recorded.
    pub fn latest_index(&self) -> Option<usize> {
        self.runs.len().checked_sub(1)
    }

    fn oracle_table_name(&self) -> String {
        format!("oracle_{}", self.source.name)
    }

    /// Materialize `S_k` (`Self::s_at(k)`) into a fresh `TEMP TABLE
    /// oracle_<source_name>` on `conn` — the S-restricted oracle's input
    /// table (design §6: "materializes `S_k` into temp tables").
    pub fn materialize_s(&self, conn: &Connection, k: usize) -> anyhow::Result<()> {
        let table = self.oracle_table_name();
        conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS {table}; \
             CREATE TEMP TABLE {table} ({d} DATE, {id} INTEGER, {val} INTEGER);",
            d = self.source.clock_column,
            id = self.source.key_column,
            val = self.source.payload_column,
        ))?;
        for row in self.s_at(k) {
            conn.execute(
                &format!(
                    "INSERT INTO {table} VALUES (DATE '{}', {}, {})",
                    row.d.format("%Y-%m-%d"),
                    row.id,
                    row.val,
                ),
                [],
            )?;
        }
        Ok(())
    }

    /// The oracle body query for `recipe` evaluated over the materialized
    /// `S_k` table rather than the physical source table — the same body
    /// [`render::render_model_body`] produces (design §4 "renders once,
    /// serves three"), with `smelt.sources.<x>` swapped for `oracle_<x>`
    /// (this module's `TEMP TABLE`) instead of
    /// [`render::render_oracle_sql`]'s `main.sources_<x>` (the full-input
    /// oracle). Kept here rather than in `render.rs` (outside this phase's
    /// edit scope, plan Critical files) but reuses `render_model_body` so
    /// the body itself is still rendered exactly once.
    pub fn s_restricted_oracle_sql(&self, recipe: &ModelRecipe) -> String {
        render::render_model_body(recipe).replace(
            &format!("smelt.sources.{}", self.source.name),
            &self.oracle_table_name(),
        )
    }

    /// The S-restricted oracle body query for `recipe` AFTER a `RewriteModel`
    /// step has applied `edit` (Phase 9) — [`render::render_model_body_with_edit`]
    /// (the REWRITTEN body, never the pre-rewrite one — the plan Phase 9
    /// review checklist: "Oracle re-renders post-rewrite, never compares
    /// old body against new output") over the materialized `S_k` table,
    /// mirroring [`Self::s_restricted_oracle_sql`]'s un-rewritten
    /// counterpart.
    pub fn s_restricted_oracle_sql_with_edit(
        &self,
        recipe: &ModelRecipe,
        edit: ModelEdit,
    ) -> String {
        render::render_model_body_with_edit(recipe, edit).replace(
            &format!("smelt.sources.{}", self.source.name),
            &self.oracle_table_name(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{KeyShape, SourcePosture};

    fn events_source() -> SourceRecipe {
        SourceRecipe {
            name: "events".to_string(),
            clock_column: "d".to_string(),
            key_column: "id".to_string(),
            payload_column: "val".to_string(),
            key_shape: KeyShape::Single,
            posture: SourcePosture::AppendOnly,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn sorted(mut rows: Vec<GenRow>) -> Vec<GenRow> {
        rows.sort_by_key(|a| (a.d, a.id));
        rows
    }

    /// `s_matches_hand_computed_set_on_fixed_schedule` (plan Phase 3 TDD
    /// list): for a hand-written 3-run schedule with one late append, `S_k`
    /// per step equals the hand-computed row multiset.
    #[test]
    fn s_matches_hand_computed_set_on_fixed_schedule() {
        let source = events_source();
        let mut tracker = STracker::new(&source);

        let w1 = (date(2024, 1, 1), date(2024, 1, 2));
        let w2 = (date(2024, 1, 2), date(2024, 1, 3));

        let a = GenRow {
            d: w1.0,
            id: 1,
            val: 10,
        };
        let b = GenRow {
            d: w2.0,
            id: 2,
            val: 20,
        };
        // C is a late row landing back inside w1's range.
        let c = GenRow {
            d: w1.0,
            id: 3,
            val: 5,
        };

        // Run 1: window w1, snapshot = {A}.
        let k0 = tracker.record_run(w1.0, w1.1, vec![a.clone()]);
        assert_eq!(sorted(tracker.s_at(k0)), sorted(vec![a.clone()]));

        // Run 2: window w2, snapshot = {A, B}.
        let k1 = tracker.record_run(w2.0, w2.1, vec![a.clone(), b.clone()]);
        assert_eq!(sorted(tracker.s_at(k1)), sorted(vec![a.clone(), b.clone()]));

        // C is appended out of band here (no run recorded for the append
        // itself — it is a bare source mutation).

        // Run 3: RE-RUN w1, snapshot now = {A, B, C}.
        let k2 = tracker.record_run(w1.0, w1.1, vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(
            sorted(tracker.s_at(k2)),
            sorted(vec![a.clone(), b.clone(), c.clone()]),
            "S after the w1 re-run must include the late row C"
        );
    }

    /// `late_row_is_outside_s_until_its_window_reruns` (plan Phase 3 TDD
    /// list): the spec's horizon semantics fall out of S-tracking with no
    /// special-casing.
    #[test]
    fn late_row_is_outside_s_until_its_window_reruns() {
        let source = events_source();
        let mut tracker = STracker::new(&source);

        let w1 = (date(2024, 1, 1), date(2024, 1, 2));
        let w2 = (date(2024, 1, 2), date(2024, 1, 3));
        let a = GenRow {
            d: w1.0,
            id: 1,
            val: 10,
        };
        let b = GenRow {
            d: w2.0,
            id: 2,
            val: 20,
        };
        let c = GenRow {
            d: w1.0,
            id: 3,
            val: 5,
        };

        tracker.record_run(w1.0, w1.1, vec![a.clone()]);
        let k1 = tracker.record_run(w2.0, w2.1, vec![a.clone(), b.clone()]);

        // C has been "appended" to the source (out of band) but no run has
        // processed it yet — the most recent S must not contain it, even
        // though C's own window (w1) already ran once.
        assert!(
            !tracker.s_at(k1).contains(&c),
            "late row must be outside S until its window is re-run — no \
             special-casing needed, it just fell out of no run's snapshot \
             covering it yet"
        );

        // w1 re-runs, now seeing C.
        let k2 = tracker.record_run(w1.0, w1.1, vec![a.clone(), b.clone(), c.clone()]);
        assert!(
            tracker.s_at(k2).contains(&c),
            "late row must enter S once its window has re-run with a \
             snapshot that includes it"
        );
    }

    /// `outstanding_mutation_flips_to_settled_point_mode` (plan Phase 4 TDD
    /// list): mode selection — an in-place update on a mutable source defers
    /// full assertion to the next catch-up run covering the affected region
    /// (the mutation's own window); re-running a DIFFERENT window must not
    /// clear it.
    #[test]
    fn outstanding_mutation_flips_to_settled_point_mode() {
        let source = events_source();
        let mut tracker = STracker::new(&source);

        let w1 = (date(2024, 1, 1), date(2024, 1, 2));
        let w2 = (date(2024, 1, 2), date(2024, 1, 3));

        tracker.record_run(w1.0, w1.1, vec![]);
        assert_eq!(
            tracker.oracle_mode(),
            OracleMode::SRestricted,
            "no mutation has been recorded yet"
        );

        // A dimension mutation affecting w1's rows is now outstanding.
        tracker.record_dimension_mutation(w1);
        assert_eq!(
            tracker.oracle_mode(),
            OracleMode::SettledPoint,
            "an outstanding mutation must defer full assertion until its \
             own window's catch-up run"
        );

        // Re-running an UNRELATED window (w2) must not catch it up — MP11's
        // ColumnScopedMerge write is scoped to the run's own window.
        tracker.record_run(w2.0, w2.1, vec![]);
        assert_eq!(
            tracker.oracle_mode(),
            OracleMode::SettledPoint,
            "re-running a different window must not clear an unrelated \
             outstanding mutation"
        );

        // Re-running w1 itself is the catch-up.
        tracker.record_run(w1.0, w1.1, vec![]);
        assert_eq!(
            tracker.oracle_mode(),
            OracleMode::SRestricted,
            "re-running the mutation's own window must settle it back to \
             S-restricted mode"
        );
    }

    /// `full_refresh_run_reflects_the_whole_current_snapshot` (Phase 6):
    /// [`STracker::record_full_refresh`] treats its recorded run as if its
    /// own window covered every date — every row in the snapshot it was
    /// given enters `S`, regardless of which day it lands on, and a later
    /// windowed run still composes correctly on top of it.
    #[test]
    fn full_refresh_run_reflects_the_whole_current_snapshot() {
        let source = events_source();
        let mut tracker = STracker::new(&source);

        let w1 = (date(2024, 1, 1), date(2024, 1, 2));
        let w2 = (date(2024, 1, 2), date(2024, 1, 3));

        let a = GenRow {
            d: w1.0,
            id: 1,
            val: 10,
        };
        let b = GenRow {
            d: w2.0,
            id: 2,
            val: 20,
        };

        // A full refresh at this point sees both rows already in the
        // source, even though only w1 has ever been "run" as a window.
        let k0 = tracker.record_full_refresh(vec![a.clone(), b.clone()]);
        assert_eq!(
            sorted(tracker.s_at(k0)),
            sorted(vec![a.clone(), b.clone()]),
            "a full-refresh run must reflect the ENTIRE current snapshot, not just \
             previously-run windows"
        );

        // A subsequent normal windowed run still composes on top of it.
        let c = GenRow {
            d: w2.0,
            id: 3,
            val: 30,
        };
        let k1 = tracker.record_run(w2.0, w2.1, vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(
            sorted(tracker.s_at(k1)),
            sorted(vec![a, b, c]),
            "a windowed run after a full refresh must still union correctly"
        );
    }
}
