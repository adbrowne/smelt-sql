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

use chrono::{Datelike, NaiveDate};
use smelt_backend::Backend;
use smelt_logical::contract::ContractPoint;

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
        self.s_at_for_point(k, &ContractPoint::Default)
    }

    /// `S_k` generalised over a contract-lattice `point`
    /// (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`):
    /// identical to [`Self::s_at`] except each recorded run's own
    /// `[start, end)` window is first narrowed through
    /// `smelt_logical::contract::restrict_run_window` before the per-window
    /// row filter — the SAME transform the real write-eligibility clamp
    /// calls, so the oracle and the write clamp can never drift apart. The
    /// identity for [`ContractPoint::Default`] and [`ContractPoint::Deferral`]
    /// (deferral restricts by settled cutoff, not by per-run window — see
    /// [`Self::s_at_settled`]) reproduces [`Self::s_at`] exactly.
    pub fn s_at_for_point(&self, k: usize, point: &ContractPoint) -> Vec<GenRow> {
        let mut seen: HashSet<GenRow> = HashSet::new();
        for run in self.runs.iter().take(k + 1) {
            let start_days = run.start.num_days_from_ce() as i64;
            let end_days = run.end.num_days_from_ce() as i64;
            let (restricted_start_days, restricted_end_days) =
                smelt_logical::contract::restrict_run_window(point, start_days, end_days);
            let restricted_start =
                NaiveDate::from_num_days_from_ce_opt(restricted_start_days as i32)
                    .unwrap_or(run.start);
            let restricted_end =
                NaiveDate::from_num_days_from_ce_opt(restricted_end_days as i32).unwrap_or(run.end);
            for row in &run.snapshot {
                let t = row.event_time();
                if t >= restricted_start && t < restricted_end {
                    seen.insert(row.clone());
                }
            }
        }
        let mut rows: Vec<GenRow> = seen.into_iter().collect();
        rows.sort_by_key(|a| (a.d, a.id));
        rows
    }

    /// [`Self::s_at_for_point`] further restricted to rows strictly before
    /// `point`'s settled cutoff (`smelt_logical::contract::settled_cutoff`) —
    /// the deferral bracket's lower leg: `full_refresh(S_settled)`. Returns
    /// `s_at_for_point(k, point)` unchanged when `point` has no settled
    /// cutoff (every non-`Deferral` point).
    pub fn s_at_settled(
        &self,
        k: usize,
        point: &ContractPoint,
        input_frontier: i64,
    ) -> Vec<GenRow> {
        let rows = self.s_at_for_point(k, point);
        match smelt_logical::contract::settled_cutoff(point, input_frontier) {
            Some(cutoff_days) => {
                let Some(cutoff) = NaiveDate::from_num_days_from_ce_opt(cutoff_days as i32) else {
                    return rows;
                };
                rows.into_iter()
                    .filter(|r| r.event_time() < cutoff)
                    .collect()
            }
            None => rows,
        }
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
    /// oracle_<source_name>` on `backend` — the S-restricted oracle's input
    /// table (design §6: "materializes `S_k` into temp tables"). Routed
    /// through [`smelt_backend::Backend::execute_sql`] rather than a raw
    /// `duckdb::Connection`
    /// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2) —
    /// every real caller passes the SAME `Backend`/connection it will next
    /// query the materialized temp table through (DuckDB `TEMP TABLE`s are
    /// scoped to the connection that created them, so a fresh connection per
    /// call would make the table invisible to the follow-up oracle query).
    pub async fn materialize_s(&self, backend: &dyn Backend, k: usize) -> anyhow::Result<()> {
        self.materialize_rows(backend, self.s_at(k)).await
    }

    /// [`Self::materialize_s`] generalised over a contract-lattice `point`
    /// (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`):
    /// materializes `s_at_for_point(k, point)` rather than the unrestricted
    /// `s_at(k)`. Reproduces [`Self::materialize_s`] exactly for
    /// [`ContractPoint::Default`].
    pub async fn materialize_s_for_point(
        &self,
        backend: &dyn Backend,
        k: usize,
        point: &ContractPoint,
    ) -> anyhow::Result<()> {
        self.materialize_rows(backend, self.s_at_for_point(k, point))
            .await
    }

    /// [`Self::materialize_s_for_point`]'s settled counterpart — materializes
    /// [`Self::s_at_settled`] (the deferral bracket's `S_settled` leg) into
    /// the SAME temp table name [`Self::materialize_s`] uses, so
    /// [`Self::s_restricted_oracle_sql`] renders unchanged against whichever
    /// leg was materialized most recently.
    pub async fn materialize_s_settled(
        &self,
        backend: &dyn Backend,
        k: usize,
        point: &ContractPoint,
        input_frontier: i64,
    ) -> anyhow::Result<()> {
        self.materialize_rows(backend, self.s_at_settled(k, point, input_frontier))
            .await
    }

    /// The DROP/CREATE/INSERT sequence every `materialize_s*` variant shares
    /// — only WHICH rows differs.
    async fn materialize_rows(
        &self,
        backend: &dyn Backend,
        rows: Vec<GenRow>,
    ) -> anyhow::Result<()> {
        let table = self.oracle_table_name();
        backend
            .execute_sql(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .map_err(|e| anyhow::anyhow!("drop stale oracle temp table {table}: {e}"))?;
        backend
            .execute_sql(&format!(
                "CREATE TEMP TABLE {table} ({d} DATE, {id} INTEGER, {val} INTEGER)",
                d = self.source.clock_column,
                id = self.source.key_column,
                val = self.source.payload_column,
            ))
            .await
            .map_err(|e| anyhow::anyhow!("create oracle temp table {table}: {e}"))?;
        for row in rows {
            backend
                .execute_sql(&format!(
                    "INSERT INTO {table} VALUES (DATE '{}', {}, {})",
                    row.d.format("%Y-%m-%d"),
                    row.id,
                    row.val,
                ))
                .await
                .map_err(|e| anyhow::anyhow!("insert into oracle temp table {table}: {e}"))?;
        }
        Ok(())
    }

    /// Spark/Delta-safe counterpart of [`Self::materialize_s`]
    /// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 3):
    /// Spark SQL has no `CREATE TEMP TABLE` DDL (a DuckDB-ism) — this
    /// materializes `S_k` as a `CREATE OR REPLACE TEMPORARY VIEW` over
    /// [`Self::s_view_select_sql`]'s row-set query instead, portable ANSI-ish
    /// SQL DuckDB, Spark SQL, AND GoogleSQL all accept identically (a temp
    /// view is scoped to the session/connection that created it, exactly
    /// like DuckDB's `TEMP TABLE`, so the same "same backend creates and
    /// queries it" contract [`Self::materialize_s`]'s doc comment states
    /// still applies). Kept as a SEPARATE method rather than replacing
    /// [`Self::materialize_s`] — the DuckDB gate's existing `TEMP TABLE`
    /// path is untouched (plan Phase 3 review checklist: "DuckDB standing
    /// gate untouched").
    pub async fn materialize_s_as_view(
        &self,
        backend: &dyn Backend,
        k: usize,
    ) -> anyhow::Result<()> {
        let view = self.oracle_table_name();
        backend
            .execute_sql(&format!("DROP VIEW IF EXISTS {view}"))
            .await
            .map_err(|e| anyhow::anyhow!("drop stale oracle view {view}: {e}"))?;

        let select = self.s_view_select_sql(&self.s_at(k));

        backend
            .execute_sql(&format!(
                "CREATE OR REPLACE TEMPORARY VIEW {view} AS {select}"
            ))
            .await
            .map_err(|e| anyhow::anyhow!("create oracle view {view}: {e}"))?;
        Ok(())
    }

    /// The row-set query [`Self::materialize_s_as_view`] materializes —
    /// factored out as a pure, backend-free string builder so its exact
    /// emitted SQL can be asserted by test without a live connection
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5b).
    /// A `FROM (VALUES …) AS t(cols)` table-value constructor — the prior
    /// shape — is DuckDB/Spark-only: GoogleSQL has no table-value
    /// constructor in `FROM` position (`400 Syntax error: Expected keyword
    /// JOIN but got ')'`, measured live against BigQuery 2026-08-17). The
    /// portable form every dialect accepts is chained
    /// `SELECT … UNION ALL SELECT …` — column names/types come from the
    /// FIRST branch's aliased/typed literals, exactly as ANSI SQL's union
    /// column-naming rule requires, so only the first branch carries
    /// `AS {d}`/`AS {id}`/`AS {val}`. The zero-row case has no branch to
    /// union, so it keeps the "one typed row, filtered out" guard from the
    /// prior shape, minus the `VALUES`-in-`FROM` wrapper: a bare
    /// `SELECT … WHERE 1 = 0` (no `FROM` needed — DuckDB, Spark SQL, and
    /// GoogleSQL all accept a `WHERE` clause on a `FROM`-less `SELECT`).
    fn s_view_select_sql(&self, rows: &[GenRow]) -> String {
        let d = &self.source.clock_column;
        let id = &self.source.key_column;
        let val = &self.source.payload_column;
        if rows.is_empty() {
            format!("SELECT DATE '1970-01-01' AS {d}, 0 AS {id}, 0 AS {val} WHERE 1 = 0")
        } else {
            rows.iter()
                .enumerate()
                .map(|(i, r)| {
                    if i == 0 {
                        format!(
                            "SELECT DATE '{}' AS {d}, {} AS {id}, {} AS {val}",
                            r.d.format("%Y-%m-%d"),
                            r.id,
                            r.val
                        )
                    } else {
                        format!(
                            "SELECT DATE '{}', {}, {}",
                            r.d.format("%Y-%m-%d"),
                            r.id,
                            r.val
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join(" UNION ALL ")
        }
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
            key_recurrence: None,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn sorted(mut rows: Vec<GenRow>) -> Vec<GenRow> {
        rows.sort_by_key(|a| (a.d, a.id));
        rows
    }

    /// `s_at_under_frozen_horizon_drops_the_late_frozen_row` (phase 6 TDD
    /// list, test 5): a row landing in an already-frozen partition is in
    /// `s_at` (the default point) but absent from
    /// `s_at_for_point(FrozenHorizon)` — the per-run window narrowing
    /// excludes a run whose own window nominally covers the frozen
    /// partition, once that run's own `end - h` floor has passed it.
    #[test]
    fn s_at_under_frozen_horizon_drops_the_late_frozen_row() {
        let source = events_source();
        let mut tracker = STracker::new(&source);

        let a = GenRow {
            d: date(2024, 1, 1),
            id: 1,
            val: 10,
        };
        let late = GenRow {
            d: date(2024, 1, 1),
            id: 2,
            val: 999,
        };

        // A wide rerun of window [2024-01-01, 2024-01-11) whose snapshot now
        // contains the late row landed back in the already-old partition
        // 2024-01-01.
        let k = tracker.record_run(
            date(2024, 1, 1),
            date(2024, 1, 11),
            vec![a.clone(), late.clone()],
        );

        // Default point: the run's own window covers 2024-01-01, and the
        // snapshot contains both rows — both are in S.
        assert_eq!(
            sorted(tracker.s_at(k)),
            sorted(vec![a.clone(), late.clone()]),
            "the default point's S must include everything the run's own \
             window+snapshot covers"
        );

        // Frozen horizon H = 2 days: this run's frozen floor is
        // end - h = 2024-01-11 - 2 = 2024-01-09, well past 2024-01-01 — the
        // run's write-eligible start narrows to 2024-01-09, so 2024-01-01
        // never enters S under this point.
        let point = smelt_logical::contract::ContractPoint::FrozenHorizon { h: 2 };
        let restricted = tracker.s_at_for_point(k, &point);
        assert!(
            !restricted.contains(&a) && !restricted.contains(&late),
            "a frozen partition's rows must be dropped by \
             s_at_for_point(FrozenHorizon), got {restricted:?}"
        );
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

    /// `s_view_select_sql_emits_chained_union_all_not_a_values_table_constructor`
    /// (plan `docs/plans/20260817-bigquery-generative-conformance.md` Phase
    /// 5b, TDD test 1, populated-row branch): the row set
    /// [`STracker::materialize_s_as_view`] builds must be a portable chained
    /// `SELECT … UNION ALL SELECT …`, never `FROM (VALUES …) AS t(cols)` —
    /// GoogleSQL has no table-value constructor in `FROM` position (`400
    /// Syntax error: Expected keyword JOIN but got ')'`, measured live
    /// 2026-08-17).
    #[test]
    fn s_view_select_sql_emits_chained_union_all_not_a_values_table_constructor() {
        let source = events_source();
        let mut tracker = STracker::new(&source);
        let w1 = (date(2024, 1, 1), date(2024, 1, 2));
        let a = GenRow {
            d: w1.0,
            id: 1,
            val: 10,
        };
        let b = GenRow {
            d: w1.0,
            id: 2,
            val: 20,
        };
        let k = tracker.record_run(w1.0, w1.1, vec![a, b]);

        let sql = tracker.s_view_select_sql(&tracker.s_at(k));

        assert_eq!(
            sql,
            "SELECT DATE '2024-01-01' AS d, 1 AS id, 10 AS val \
             UNION ALL SELECT DATE '2024-01-01', 2, 20"
        );
        assert!(
            !sql.to_uppercase().contains("VALUES") && !sql.contains("FROM ("),
            "emitted row-set SQL must not use a `FROM (VALUES …)` table-value \
             constructor — GoogleSQL rejects it in FROM position: {sql:?}"
        );
    }

    /// `s_view_select_sql_zero_row_guard_uses_a_from_less_where_clause`
    /// (plan Phase 5b, TDD test 1, zero-row guard branch): with no rows to
    /// union, the guard must still avoid `FROM (VALUES …)` — a bare
    /// `SELECT … WHERE 1 = 0` (no `FROM`) is accepted by DuckDB, Spark SQL,
    /// and GoogleSQL alike.
    #[test]
    fn s_view_select_sql_zero_row_guard_uses_a_from_less_where_clause() {
        let source = events_source();
        let tracker = STracker::new(&source);

        let sql = tracker.s_view_select_sql(&[]);

        assert_eq!(
            sql,
            "SELECT DATE '1970-01-01' AS d, 0 AS id, 0 AS val WHERE 1 = 0"
        );
        assert!(
            !sql.to_uppercase().contains("VALUES") && !sql.contains("FROM ("),
            "the zero-row guard must not use a `FROM (VALUES …)` table-value \
             constructor either: {sql:?}"
        );
    }
}
