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

use crate::recipe::{arb_payload_value, ModelEdit, ModelRecipe, SourceRecipe};

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
///
/// Phase 6 (`docs/plans/20260712-generative-maintenance-conformance.md`;
/// design §5 "Schedule enrichment") adds three adversarial step kinds:
/// [`ConformanceStep::RerunWindow`] (redelivery), [`ConformanceStep::FullRefreshRun`]
/// (a `full_refresh` interleave), and [`ConformanceStep::BackfillRegion`] (an
/// explicit operator-directed backfill) — used directly by
/// `gate.rs`'s hand-written hazard cases; not yet folded into
/// [`arb_schedule_for`]'s own generator (that is future scope once these
/// hazards grow their own proptest shrinkers).
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
    /// Re-deliver an ALREADY-PROCESSED window with no new rows (Phase 6):
    /// the partition-grain DELETE+INSERT full-replace technique must stay
    /// idempotent under redelivery — never double-count
    /// (`maintenance_plan.md` §"The reconciliation ledger": "never fold a
    /// delta already reflected in the state").
    RerunWindow { start: NaiveDate, end: NaiveDate },
    /// Run with no time window at all — `execute_project`'s unwindowed
    /// full-refresh arm, which drops and rebuilds the maintained table from
    /// the CURRENT full source contents (Phase 6; design §5 "full_refresh
    /// interleave"). Resets coverage: everything the run reads is, by
    /// construction, reflected in the rebuilt table.
    FullRefreshRun,
    /// An explicit backfill run over `[start, end)` with no accompanying
    /// insert (Phase 6) — the operator-directed counterpart of `RunWindow`:
    /// same execution shape, but the design intent is catching up a region
    /// rather than processing newly-landed data or a late-row catch-up.
    BackfillRegion { start: NaiveDate, end: NaiveDate },
    /// A definition-change step (Phase 9;
    /// `docs/plans/20260712-generative-maintenance-conformance.md`
    /// "Definition-change steps"): rewrites `models/<name>.sql` on disk with
    /// `edit` applied (`crate::render::render_model_file_with_edit`), with
    /// NO accompanying run — the next `RunWindow`/`RerunWindow`/
    /// `BackfillRegion` step re-discovers the model from disk (already the
    /// harness's behaviour, `link_c_harness::LinkCProject`'s doc comment)
    /// and compiles/executes whatever SQL is currently on disk. Asserts
    /// TODAY's contract: the model-hash the run records changes
    /// (`smelt_state::intervals::compute_model_hash`, keyed per model name)
    /// invalidates the interval store's coverage bookkeeping for this
    /// model — the run pipeline itself always uses the CURRENT on-disk SQL
    /// regardless, so a full DELETE+INSERT recompute of whichever window a
    /// later step names always reflects the rewritten body. This is
    /// deliberately NOT the spec's `SkeletonAdd`/`PureBackfill`/
    /// `UpstreamRederive` definition-change classification
    /// (`maintenance_plan.md` §"The definition-change trigger") — that
    /// classification is unbuilt (no `derive_model_maintenance_plan` caller
    /// reads a prior definition to classify an added column against it).
    RewriteModel { edit: ModelEdit },
}

/// Whether every window in `schedule` is mutually independent under
/// reordering (Phase 6; design §5 "window order permutations" — the
/// order/set-determinacy corollary, `maintenance_plan.md` §"The equivalence
/// invariant": "the right-hand side depends only on the SET S, never the
/// order it was processed"). A schedule containing an [`ConformanceStep::AppendLateRow`]
/// step is NOT eligible: a late row's catch-up rerun has a genuine ordering
/// dependency on its own prior insert, so arbitrarily reordering steps could
/// run the catch-up before the row even lands.
pub fn is_permutable(schedule: &ConformanceSchedule) -> bool {
    !schedule
        .0
        .iter()
        .any(|s| matches!(s, ConformanceStep::AppendLateRow(_)))
}

/// Reorder a [`is_permutable`] schedule's steps according to `order` (a
/// permutation of `0..schedule.0.len()`) — every step still executes with
/// its own original `start`/`end`/`rows`, just in a different sequence.
/// Panics on an internal contract violation (not a user-facing error): `order`
/// must be a permutation of the schedule's own index set, and the schedule
/// must be [`is_permutable`].
pub fn reorder_windows(schedule: &ConformanceSchedule, order: &[usize]) -> ConformanceSchedule {
    assert!(
        is_permutable(schedule),
        "reorder_windows requires a permutable schedule (no AppendLateRow steps): {schedule:?}"
    );
    let mut sorted_order = order.to_vec();
    sorted_order.sort_unstable();
    let expected: Vec<usize> = (0..schedule.0.len()).collect();
    assert_eq!(
        sorted_order,
        expected,
        "order {order:?} must be a permutation of the schedule's own step indices \
         0..{}",
        schedule.0.len()
    );
    ConformanceSchedule(order.iter().map(|&i| schedule.0[i].clone()).collect())
}

/// A generated sequence of [`ConformanceStep`]s over one [`SourceRecipe`].
#[derive(Debug, Clone)]
pub struct ConformanceSchedule(pub Vec<ConformanceStep>);

/// The shared base date every generated schedule (append-only or mixed)
/// starts from — a hardcoded literal, so infallible by construction; factored
/// out once so [`arb_schedule_for`] and [`arb_mixed_schedule`] don't each
/// carry their own `.expect(` on the same constant.
fn base_date() -> NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date")
}

/// Schema-generic schedule generator (design §5): 2-3 disjoint one-day
/// windows, each seeded with 1-2 rows landing inside its own range before
/// the window is first run; a subset of windows additionally receive a late
/// row AFTER their first run (landing back inside their own range); the
/// schedule always ends with a catch-up re-run (design §5: "every generated
/// schedule ends by re-running every window affected by late … steps") of
/// every window that received a late row, so a settled point always exists
/// for the S-restricted assertion to land on.
pub fn arb_schedule_for(recipe: &ModelRecipe) -> impl Strategy<Value = ConformanceSchedule> {
    let base = base_date();
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

/// One step of a generated mixed (fact + mutable dimension) schedule
/// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 4;
/// design §5 "multi-source interleave" / §6 "Mixed models").
#[derive(Debug, Clone)]
pub enum MixedStep {
    /// Insert `rows` into the fact source (or none, for a pure catch-up
    /// re-run), then run `execute_project` over `[start, end)` — the mixed
    /// pool's analogue of [`ConformanceStep::RunWindow`].
    RunWindow {
        start: NaiveDate,
        end: NaiveDate,
        rows: Vec<GenRow>,
    },
    /// In-place `UPDATE` of the mutable dimension row keyed by `id` — no
    /// accompanying run. Leaves the maintained output's dimension-sensitive
    /// column group stale for `id`'s own window until a later `RunWindow`
    /// step re-runs THAT SAME window (MP11's `ColumnScopedMerge` write is
    /// scoped to the run's own window — design §6 "Mixed models").
    MutateDimension { id: i64, new_attr: i64 },
}

/// A generated sequence of [`MixedStep`]s.
#[derive(Debug, Clone)]
pub struct MixedSchedule(pub Vec<MixedStep>);

/// Schema-generic mixed-pool schedule generator (design §5 "multi-source
/// interleave"): 2-3 disjoint one-day fact windows (mirrors
/// [`arb_schedule_for`]'s window shape), each seeded with 1-2 rows; exactly
/// one window is chosen to receive a dimension mutation (targeting the FIRST
/// row that window produced, so the mutation is always observable), and the
/// schedule always ends by re-running that same window — the catch-up
/// (design §5: "every generated schedule ends by re-running every window
/// affected by late/mutating steps"), so a settled point always exists.
pub fn arb_mixed_schedule() -> impl Strategy<Value = MixedSchedule> {
    let base = base_date();
    (2_usize..=3).prop_flat_map(move |n_windows| {
        (
            proptest::collection::vec(
                proptest::collection::vec(arb_payload_value(), 1..=2),
                n_windows,
            ),
            0..n_windows,
            arb_payload_value(),
        )
            .prop_map(move |(window_vals, mutate_window, new_attr)| {
                build_mixed_schedule(base, &window_vals, mutate_window, new_attr)
            })
    })
}

fn build_mixed_schedule(
    base: NaiveDate,
    window_vals: &[Vec<i64>],
    mutate_window: usize,
    new_attr: i64,
) -> MixedSchedule {
    let mut steps = Vec::new();
    let mut next_id = 1_i64;
    let mut mutated_window: Option<(NaiveDate, NaiveDate)> = None;

    for (i, vals) in window_vals.iter().enumerate() {
        let start = base + chrono::Duration::days(i as i64);
        let end = start + chrono::Duration::days(1);

        let mut first_id = None;
        let rows: Vec<GenRow> = vals
            .iter()
            .map(|val| {
                let row = GenRow {
                    d: start,
                    id: next_id,
                    val: *val,
                };
                first_id.get_or_insert(next_id);
                next_id += 1;
                row
            })
            .collect();
        steps.push(MixedStep::RunWindow { start, end, rows });

        if i == mutate_window {
            if let Some(id) = first_id {
                steps.push(MixedStep::MutateDimension { id, new_attr });
                mutated_window = Some((start, end));
            }
        }
    }

    // Catch-up: re-run the mutated window so a settled point always exists
    // (design §5/§6).
    if let Some((start, end)) = mutated_window {
        steps.push(MixedStep::RunWindow {
            start,
            end,
            rows: Vec::new(),
        });
    }

    MixedSchedule(steps)
}

/// The mixed-pool `check_profile` self-check (design §5: "The
/// `MutationProfile` self-check … extends to every new step kind: a
/// schedule's declared posture is verified against its actual steps, never
/// trusted"): verifies every `MutateDimension` step's target `id` was
/// actually produced by some `RunWindow` step, AND that id's own window is
/// re-run again strictly after the mutation — the structural invariant
/// [`arb_mixed_schedule`] is supposed to uphold by construction, checked
/// rather than assumed.
pub fn check_profile(schedule: &MixedSchedule) -> Result<(), String> {
    let mut id_window: std::collections::HashMap<i64, (NaiveDate, NaiveDate)> =
        std::collections::HashMap::new();
    for step in &schedule.0 {
        if let MixedStep::RunWindow { start, end, rows } = step {
            for row in rows {
                id_window.insert(row.id, (*start, *end));
            }
        }
    }

    let mutation_positions: Vec<(usize, i64)> = schedule
        .0
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            MixedStep::MutateDimension { id, .. } => Some((i, *id)),
            _ => None,
        })
        .collect();

    if mutation_positions.is_empty() {
        return Err(
            "schedule declares the mixed profile but contains no MutateDimension step \
                     — the generator's own guarantee was violated"
                .to_string(),
        );
    }

    for (pos, id) in mutation_positions {
        let window = id_window.get(&id).copied().ok_or_else(|| {
            format!(
                "MutateDimension targets id {id} which no RunWindow step in \
                                     the schedule ever produced"
            )
        })?;
        let rerun_after = schedule.0[pos + 1..].iter().any(
            |s| matches!(s, MixedStep::RunWindow { start, end, .. } if (*start, *end) == window),
        );
        if !rerun_after {
            return Err(format!(
                "mutated id {id}'s window {window:?} is never re-run after the mutation \
                 step at index {pos} — no settled point exists for it"
            ));
        }
    }
    Ok(())
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

/// Three rows placed relative to one source's derived [`smelt_logical::maintenance::ScanClamp`]
/// (Phase 6; design §5 "Boundary-value data placement") — never a hand-coded
/// margin:
/// - `just_inside`: `d = start − before_days`, the earliest day the run's
///   scan will actually read; must be reflected in the maintained output
///   after the run that covers `window`.
/// - `at_edge`: `d = end + after_days − 1 day`, the latest day the run's scan
///   will read (inclusive) — the reach's other edge.
/// - `just_outside`: `d = start − before_days − 1 day`, one calendar day
///   before the scan's lower bound; must NEVER be reflected by a run over
///   `window` (it is outside the derived reach, by construction).
#[derive(Debug, Clone)]
pub struct BoundaryRows {
    pub just_inside: GenRow,
    pub at_edge: GenRow,
    pub just_outside: GenRow,
}

/// Ceil a [`smelt_logical::analysis::source_bounds::Seconds`] margin to whole
/// days — mirrors `smelt_logical::maintenance::propagate`'s private
/// `clamp_days` (kept as a separate copy here since that module is outside
/// this phase's edit scope — see the plan's Critical files list).
fn clamp_days(seconds: smelt_logical::analysis::source_bounds::Seconds) -> i64 {
    const DAY_SECONDS: u64 = 86_400;
    seconds.0.div_ceil(DAY_SECONDS) as i64
}

/// Derive [`BoundaryRows`] for `window` from `clamp` — the admitted plan's OWN
/// derived scan reach, read via [`scan_clamp_for`], never a hand-computed
/// margin (plan Phase 6 Implementation shape: "Boundary placement is
/// plan-aware: `classify` runs before data generation"). `next_id` is
/// threaded through so a caller stamping multiple boundary triples never
/// collides row ids.
pub fn boundary_rows_for(
    clamp: &smelt_logical::maintenance::ScanClamp,
    window: (NaiveDate, NaiveDate),
    next_id: &mut i64,
) -> BoundaryRows {
    let before_days = clamp_days(clamp.before);
    let after_days = clamp_days(clamp.after);
    let (start, end) = window;

    let mut make = |d: NaiveDate, val: i64| {
        let row = GenRow {
            d,
            id: *next_id,
            val,
        };
        *next_id += 1;
        row
    };

    let just_inside = make(start - chrono::Duration::days(before_days), 11);
    let at_edge = make(
        end + chrono::Duration::days(after_days) - chrono::Duration::days(1),
        13,
    );
    let just_outside = make(
        start - chrono::Duration::days(before_days) - chrono::Duration::days(1),
        17,
    );

    BoundaryRows {
        just_inside,
        at_edge,
        just_outside,
    }
}

/// Read the derived [`smelt_logical::maintenance::ScanClamp`] for `source_name`
/// off an ALREADY-DERIVED [`smelt_logical::maintenance::MaintenancePlan`]'s
/// `Trigger::NewData` cell — consumed, never re-derived (root `CLAUDE.md`
/// §"Maintenance-plan purity"). `None` when no `NewData` cell is admitted for
/// `source_name`, or its scan could not be bounded (`PartitionLocal::No`) —
/// both legitimate "this recipe has nothing to place boundary rows against"
/// outcomes, not errors.
pub fn scan_clamp_for<'a>(
    plan: &'a smelt_logical::maintenance::MaintenancePlan,
    source_name: &str,
) -> Option<&'a smelt_logical::maintenance::ScanClamp> {
    plan.cell_for(&smelt_logical::maintenance::Trigger::NewData {
        source: source_name.to_string(),
    })
    .and_then(|cell| cell.scans.iter().find(|c| c.source == source_name))
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
                    // Phase 6/9 hazard steps never appear in `arb_schedule_for`'s
                    // own generated output (see that fn's doc comment) — this
                    // self-check is scoped to the generator, not hand-written
                    // schedules.
                    ConformanceStep::RerunWindow { .. }
                    | ConformanceStep::FullRefreshRun
                    | ConformanceStep::BackfillRegion { .. }
                    | ConformanceStep::RewriteModel { .. } => {}
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

    /// `check_profile_verifies_mutable_schedules` (plan Phase 4 TDD list):
    /// the `check_profile` self-check extends to multi-source interleave
    /// steps — every generated [`MixedSchedule`] passes its own declared-
    /// posture check, and the check actually fires (rather than being
    /// vacuously true) when the catch-up step is stripped out.
    #[test]
    fn check_profile_verifies_mutable_schedules() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_mixed_schedule();

        for i in 0..30 {
            let schedule = strat.new_tree(&mut runner).unwrap().current();
            check_profile(&schedule).unwrap_or_else(|e| {
                panic!(
                    "case {i}: generated mixed schedule {schedule:?} failed its own \
                     self-check: {e}"
                )
            });
        }

        // Negative case: strip the trailing catch-up `RunWindow` so the
        // self-check must actually catch the violation — never trust an
        // unverified posture label (design §5 "F7").
        let schedule = strat.new_tree(&mut runner).unwrap().current();
        assert!(
            matches!(schedule.0.last(), Some(MixedStep::RunWindow { .. })),
            "generator's own guarantee: schedule must end with the catch-up RunWindow"
        );
        let mut corrupted_steps = schedule.0.clone();
        corrupted_steps.pop();
        let corrupted = MixedSchedule(corrupted_steps);
        assert!(
            check_profile(&corrupted).is_err(),
            "removing the catch-up RunWindow must be caught by check_profile, got Ok for \
             {corrupted:?}"
        );
    }

    /// `rewrite_model_step_changes_the_hash` (plan Phase 9 TDD list): the
    /// `RewriteModel` step's rendered body
    /// (`crate::render::render_model_body_with_edit`) differs from the
    /// original recipe's own body under
    /// `smelt_state::intervals::compute_model_hash` — the SAME hash
    /// function `execute.rs`'s interval-store bookkeeping keys coverage on
    /// (`compute_model_hash(&plan.sql)`), consumed here rather than
    /// re-implemented, so this test actually exercises production hashing
    /// rather than a harness-local stand-in.
    #[test]
    fn rewrite_model_step_changes_the_hash() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());
        let mut checked = 0;
        for _ in 0..30 {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            if !recipe
                .evolution
                .contains(&crate::recipe::ModelEdit::AddPayloadColumn)
            {
                continue;
            }
            let before = crate::render::render_model_body(&recipe);
            let after = crate::render::render_model_body_with_edit(
                &recipe,
                crate::recipe::ModelEdit::AddPayloadColumn,
            );
            assert_ne!(
                smelt_state::intervals::compute_model_hash(&before),
                smelt_state::intervals::compute_model_hash(&after),
                "AddPayloadColumn rewrite must change the model hash (recipe {recipe:?}); \
                 an unchanged hash would mean the interval store never invalidates \
                 coverage on a real definition change"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "no recipe in the deterministic sample carried the AddPayloadColumn evolution \
             — generator regression"
        );
    }

    /// `boundary_placement_targets_derived_clamp_edges` (plan Phase 6 TDD
    /// list): [`boundary_rows_for`] places rows relative to the ACTUAL
    /// derived `ScanClamp`, never a hand-coded margin. Part 1 exercises the
    /// placement arithmetic against a synthetic non-zero clamp directly;
    /// part 2 classifies every admitted append-only-pool recipe through the
    /// real derivation (`crate::verdict::classify`) and checks the same
    /// arithmetic against WHATEVER clamp that recipe's own admission
    /// actually derives (today: every pool construct is partition-local with
    /// no lookback, so `before = after = 0` — the placement function must
    /// still read those zero margins from the plan rather than assuming
    /// them).
    #[test]
    fn boundary_placement_targets_derived_clamp_edges() {
        // Part 1: synthetic non-zero clamp — the placement math itself.
        let clamp = smelt_logical::maintenance::ScanClamp {
            source: "events".to_string(),
            column: "d".to_string(),
            before: smelt_logical::analysis::source_bounds::Seconds(2 * 86_400),
            after: smelt_logical::analysis::source_bounds::Seconds(86_400),
        };
        let window = (
            NaiveDate::from_ymd_opt(2024, 1, 5).expect("valid date"),
            NaiveDate::from_ymd_opt(2024, 1, 6).expect("valid date"),
        );
        let mut next_id = 1_i64;
        let rows = boundary_rows_for(&clamp, window, &mut next_id);
        assert_eq!(
            rows.just_inside.d,
            NaiveDate::from_ymd_opt(2024, 1, 3).expect("valid date"),
            "just_inside must land at start - before_days (2 days), not a hand-coded margin"
        );
        assert_eq!(
            rows.at_edge.d,
            NaiveDate::from_ymd_opt(2024, 1, 6).expect("valid date"),
            "at_edge must land at end + after_days (1 day) - 1 day"
        );
        assert_eq!(
            rows.just_outside.d,
            NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date"),
            "just_outside must land one day before just_inside"
        );
        assert_eq!(next_id, 4, "next_id must advance once per generated row");

        // Part 2: read the clamp off a REAL classified plan (never
        // re-derived).
        let mut runner = TestRunner::deterministic();
        let recipe_strat = arb_recipe(RecipePool::partition_append_only());
        let mut checked = 0;
        for i in 0..20 {
            let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().join("project");
            let db_path = tmp.path().join("db.duckdb");
            std::fs::create_dir_all(&project_dir).expect("create project dir");
            let project = crate::render::stage(&recipe, &project_dir, &db_path)
                .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

            let verdict = crate::verdict::classify(&project, &recipe)
                .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
            let plan = match verdict {
                crate::verdict::Verdict::Admitted(plan) => plan,
                crate::verdict::Verdict::Refused(_) => continue,
            };

            let clamp = match scan_clamp_for(&plan, &recipe.source.name) {
                Some(clamp) => clamp,
                None => continue,
            };

            let before_days = clamp_days(clamp.before);
            let after_days = clamp_days(clamp.after);
            let window = (
                NaiveDate::from_ymd_opt(2024, 6, 10).expect("valid date"),
                NaiveDate::from_ymd_opt(2024, 6, 11).expect("valid date"),
            );
            let mut next_id = 100_i64;
            let rows = boundary_rows_for(clamp, window, &mut next_id);

            assert_eq!(
                rows.just_inside.d,
                window.0 - chrono::Duration::days(before_days),
                "case {i}: just_inside must read the recipe's OWN derived before-margin"
            );
            assert_eq!(
                rows.at_edge.d,
                window.1 + chrono::Duration::days(after_days) - chrono::Duration::days(1),
                "case {i}: at_edge must read the recipe's OWN derived after-margin"
            );
            assert_eq!(
                rows.just_outside.d,
                window.0 - chrono::Duration::days(before_days) - chrono::Duration::days(1),
                "case {i}: just_outside must fall strictly outside the derived reach"
            );
            assert!(
                rows.just_outside.d < rows.just_inside.d,
                "case {i}: just_outside must strictly precede just_inside"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "no admitted recipe with a bounded scan was reached across the deterministic \
             sample — generator/derivation regression"
        );
    }
}
