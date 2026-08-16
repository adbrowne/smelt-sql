//! Plan-claim probes (design doc
//! `docs/research/20260711-generative-maintenance-conformance.md` §7
//! "Plan-claim probes — checking that derived properties hold";
//! `docs/plans/20260712-generative-maintenance-conformance.md` Phase 7): a
//! direct runtime check that a derived plan claim actually holds, beyond
//! end-state equivalence alone — end-state equivalence can miss a claim
//! being wrong in a compensating way.
//!
//! Each probe is `fn(&CaseContext) -> ProbeOutcome`
//! (`Checked(Result<()>)`/`Skipped(reason)`); a probe that cannot
//! structurally apply to a case is skipped explicitly, counted by
//! [`ReachabilityReport`] rather than silently absent — "a probe that never
//! fires is a visible generator gap, not a silent one" (design §7/§8).
//!
//! ## The `maintenance.cells[].technique` pin wiring gap
//!
//! Design §7's technique-interchangeability row reads "the same seed +
//! schedule runs once with `maintenance.cells[].technique: fold` and once
//! with `recompute`". Implementing that probe surfaced a real, confirmed
//! production gap rather than a generated-case divergence: the
//! `maintenance.cells[].technique` frontmatter pin
//! (`smelt_core::config::MaintenanceCellConfig::technique`) is parsed and
//! validated, and its resolver logic exists and is unit-tested in TWO places
//! (`smelt_logical::maintenance::choice::resolve_cell_choice`,
//! `smelt_runtime::maintenance_driver::resolve_cell_technique`) — but neither
//! resolver is ever invoked with a real pin value anywhere in the execute
//! path. `resolve_cell_technique`'s one production call site
//! (`smelt_runtime::maintenance_driver::resolve_live_column_scoped_cell`)
//! hardcodes `pin: None`; no other call site exists. `derive_model_maintenance_plan`
//! itself never even takes a `MaintenanceConfig` as an argument. A pin set in
//! a model's frontmatter today has **zero** effect on which technique
//! executes, for every trigger/grain shape — confirmed by exhaustive `rg`
//! across `smelt-db`, `smelt-logical`, and `smelt-runtime`.
//!
//! Per this plan's execution conventions ("if a generated case exposes a
//! REAL production divergence... do NOT weaken the oracle... a discovered
//! production bug is a deliverable, not a blocker — fixing it is its own
//! red-green change outside this plan"), [`technique_pins_agree_at_fixed_s`]
//! does not assert against the inert frontmatter pin (that would either be
//! vacuously true for every case, since the pin never changes execution, or
//! require production wiring outside this phase's Critical files). Instead
//! it exercises the two REAL, wired execution paths that stand in for the
//! same spec claim (`incremental_models.md` §"Per-cell admission"
//! "Interchangeability and choice") on the one pool where a genuine
//! fold-vs-recompute choice actually exists end-to-end: a `grain: key`
//! model's windowed `KeyedFold` runs (the fold family) versus its no-window
//! full-table recompute (`smelt_runtime::execute`'s "single-shot full
//! refresh of the keyed SELECT" arm — the always-available recompute family,
//! `incremental_models.md` §"The plan matrix": "a whole-table recompute is
//! exactly a region taken to its limit"). This is recorded as a finding in
//! `docs/plans/20260712-generative-maintenance-conformance.md`'s "Deferred
//! during implementation" section; wiring the pin into execution is
//! out-of-scope follow-up work, not silently absorbed here.

#![allow(dead_code)]

use std::collections::BTreeMap;

use chrono::NaiveDate;

use smelt_core::config::CellTechnique;
use smelt_logical::maintenance::{MaintenancePlan, Trigger};
use smelt_runtime::maintenance_driver::resolve_cell_technique;

use crate::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use crate::recipe::{KeyedRecipe, ModelRecipe};
use crate::render;
use crate::schedule_gen::{boundary_rows_for, GenRow};
use crate::verdict::{self, Verdict};

/// Which generated pool a [`CaseContext`] came from — the probes need to
/// know this to decide relevance (design §7: "opt-in by what the plan
/// admits"), since the partition-grain and keyed-grain pools exercise
/// different corners of the plan matrix.
#[derive(Debug, Clone)]
pub enum CaseRecipe {
    Partition(ModelRecipe),
    Keyed(KeyedRecipe),
}

/// One staged, classified, admitted case — the shared context every probe
/// reads from (Phase 7 Implementation shape: `fn(&CaseContext) ->
/// ProbeOutcome`). Owns the staged project's tempdir so the case stays alive
/// for the probe's duration.
pub struct CaseContext {
    pub recipe: CaseRecipe,
    pub plan: MaintenancePlan,
    pub project: LinkCProject,
    _tmp: tempfile::TempDir,
}

impl CaseContext {
    /// Stage + classify a [`ModelRecipe`] (partition-grain pool). `None` when
    /// the recipe's plan admits no cell — the caller decides whether that is
    /// a skip or a hard failure for its own case selection.
    pub fn stage_partition(recipe: ModelRecipe) -> anyhow::Result<Option<Self>> {
        let tmp = tempfile::TempDir::new()?;
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir)?;
        let project = render::stage(&recipe, &project_dir, &db_path)?;
        match verdict::classify(&project, &recipe)? {
            Verdict::Admitted(plan) => Ok(Some(Self {
                recipe: CaseRecipe::Partition(recipe),
                plan,
                project,
                _tmp: tmp,
            })),
            Verdict::Refused(_) => Ok(None),
        }
    }

    /// Stage + classify a [`KeyedRecipe`] (`grain: key` pool) — the keyed
    /// counterpart of [`Self::stage_partition`].
    pub fn stage_keyed(recipe: KeyedRecipe) -> anyhow::Result<Option<Self>> {
        let tmp = tempfile::TempDir::new()?;
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir)?;
        let project = render::stage_keyed(&recipe, &project_dir, &db_path)?;
        match verdict::classify_keyed(&project, &recipe)? {
            Verdict::Admitted(plan) => Ok(Some(Self {
                recipe: CaseRecipe::Keyed(recipe),
                plan,
                project,
                _tmp: tmp,
            })),
            Verdict::Refused(_) => Ok(None),
        }
    }
}

/// The outcome of running one probe against one [`CaseContext`]
/// (Implementation shape: `Checked(Result)`, `Skipped(reason)`).
pub enum ProbeOutcome {
    /// The probe applied to this case; `Ok(())` if the claim held, `Err` if
    /// it did not.
    Checked(anyhow::Result<()>),
    /// The probe structurally does not apply to this case — never a silent
    /// absence; the reason is folded into [`ReachabilityReport`].
    Skipped(String),
}

impl ProbeOutcome {
    /// Panic with the probe's own error message when `Checked(Err(_))`; a
    /// no-op for `Checked(Ok(_))`/`Skipped`. The four TDD tests call this so
    /// a failing claim reads exactly like any other `assert!` failure.
    pub fn expect_checked_ok(self, probe_name: &str) {
        match self {
            ProbeOutcome::Checked(Ok(())) => {}
            ProbeOutcome::Checked(Err(e)) => panic!("probe {probe_name:?} failed: {e:#}"),
            ProbeOutcome::Skipped(reason) => {
                panic!("probe {probe_name:?} was skipped, expected it to apply: {reason}")
            }
        }
    }
}

/// Per-probe skip accounting, folded across a sample of cases (design §7
/// "skipped explicitly, never silently vacuous"; §8 "generator health" —
/// pattern copied from `type_property_tests.rs::reachability`). A probe
/// tallying zero `Checked` outcomes across the whole sample never actually
/// fired: a visible generator gap, made structural by
/// [`Self::assert_no_probe_fully_skipped`].
#[derive(Debug, Default)]
pub struct ReachabilityReport {
    tally: BTreeMap<&'static str, Tally>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    checked_ok: usize,
    checked_err: usize,
    skipped: usize,
}

impl ReachabilityReport {
    pub fn record(&mut self, probe: &'static str, outcome: &ProbeOutcome) {
        let entry = self.tally.entry(probe).or_default();
        match outcome {
            ProbeOutcome::Checked(Ok(())) => entry.checked_ok += 1,
            ProbeOutcome::Checked(Err(_)) => entry.checked_err += 1,
            ProbeOutcome::Skipped(_) => entry.skipped += 1,
        }
    }

    pub fn checked(&self, probe: &str) -> usize {
        self.tally
            .get(probe)
            .map(|t| t.checked_ok + t.checked_err)
            .unwrap_or(0)
    }

    pub fn skipped(&self, probe: &str) -> usize {
        self.tally.get(probe).map(|t| t.skipped).unwrap_or(0)
    }

    /// Fail loud when a probe registered in the report never actually fired
    /// (100% skip): "a probe that never fires is a visible generator gap,
    /// not a silent one" (design §7).
    pub fn assert_no_probe_fully_skipped(&self) {
        let vacuous: Vec<String> = self
            .tally
            .iter()
            .filter(|(_, t)| t.skipped > 0 && t.checked_ok + t.checked_err == 0)
            .map(|(name, t)| format!("{name}: {} skipped, 0 checked", t.skipped))
            .collect();
        assert!(
            vacuous.is_empty(),
            "probe(s) never fired across the sample (100% skip): {vacuous:#?}\nfull tally: \
             {:#?}",
            self.tally,
        );
    }
}

/// Ceil a [`smelt_logical::analysis::source_bounds::Seconds`] margin to whole
/// days — mirrors `schedule_gen::clamp_days` /
/// `smelt_logical::maintenance::propagate`'s private `clamp_days` (kept as a
/// separate copy since neither module is in this phase's edit scope — the
/// plan's Critical files list).
fn clamp_days(seconds: smelt_logical::analysis::source_bounds::Seconds) -> i64 {
    const DAY_SECONDS: u64 = 86_400;
    seconds.0.div_ceil(DAY_SECONDS) as i64
}

async fn insert_row(project: &LinkCProject, source_name: &str, row: &GenRow) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{source_name} VALUES (DATE '{}', {}, {})",
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Plan claim (design §7 row 1): "The compiled SQL captured by
/// `SqlCapturingReporter` carries exactly the claimed filter (plan-vs-execution
/// consistency); boundary-placed data (§5) makes an under-derived clamp
/// diverge in the oracle." Checks the compiled SQL TEXT directly — a
/// distinct claim from `boundary_rows_within_reach_are_reflected`'s (Phase
/// 6) materialized-output check, which could pass even if the emitted filter
/// happened to be right for the wrong reason (e.g. two compensating bugs).
///
/// Applies only to [`CaseRecipe::Partition`] cases with an admitted `NewData`
/// scan clamp on the driving source — `grain: key` has no write-eligibility
/// clamp at all (`incremental_shapes.md` §"No write-eligibility clamp").
pub async fn compiled_sql_matches_derived_clamp(ctx: &CaseContext) -> ProbeOutcome {
    let CaseRecipe::Partition(recipe) = &ctx.recipe else {
        return ProbeOutcome::Skipped(
            "scan-clamp/compiled-SQL consistency is a partition-grain claim — grain: key has no \
             write-eligibility clamp"
                .to_string(),
        );
    };
    let Some(clamp) = ctx
        .plan
        .cell_for(&Trigger::NewData {
            source: recipe.source.name.clone(),
        })
        .and_then(|cell| cell.scans.iter().find(|c| c.source == recipe.source.name))
    else {
        return ProbeOutcome::Skipped(format!(
            "no admitted NewData scan clamp for source {:?}",
            recipe.source.name
        ));
    };

    let window = (
        NaiveDate::from_ymd_opt(2024, 1, 10).expect("valid date"),
        NaiveDate::from_ymd_opt(2024, 1, 11).expect("valid date"),
    );
    let before_days = clamp_days(clamp.before);
    let after_days = clamp_days(clamp.after);
    let filter_start = window.0 - chrono::Duration::days(before_days);
    let filter_end = window.1 + chrono::Duration::days(after_days);

    let mut next_id: i64 = 1;
    let boundary = boundary_rows_for(clamp, window, &mut next_id);
    if let Err(e) = insert_row(&ctx.project, &recipe.source.name, &boundary.just_inside).await {
        return ProbeOutcome::Checked(Err(e));
    }
    if let Err(e) = insert_row(&ctx.project, &recipe.source.name, &boundary.just_outside).await {
        return ProbeOutcome::Checked(Err(e));
    }

    let reporter = SqlCapturingReporter::new();
    let mut request = base_request("dev");
    request.start = Some(window.0.format("%Y-%m-%d").to_string());
    request.end = Some(window.1.format("%Y-%m-%d").to_string());
    if let Err(e) = ctx.project.run("probe-clamp-run", request, &reporter).await {
        return ProbeOutcome::Checked(Err(e));
    }

    let compiled = reporter.sql_for(&recipe.model_name);
    let expected = format!(
        "{col} >= '{start}' AND {col} < '{end}'",
        col = recipe.source.clock_column,
        start = filter_start.format("%Y-%m-%d"),
        end = filter_end.format("%Y-%m-%d"),
    );
    let found = compiled.iter().any(|sql| sql.contains(&expected));
    ProbeOutcome::Checked(if found {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "compiled SQL for model {:?} never carried the derived clamp's own filter {expected:?} \
             — plan-vs-execution consistency violated; captured SQL={compiled:#?}",
            recipe.model_name,
        ))
    })
}

/// Read every column of `main.<model_name>` back as text for rows matching
/// `predicate`, sorted for stable comparison — the construct-agnostic
/// full-state snapshot used by the write-window probe (mirrors
/// `crates/smelt-cli/tests/maintenance_conformance/probes.rs`'s
/// `read_full_output_as_text`, scoped to a `WHERE` clause here).
fn read_region_as_text(
    conn: &duckdb::Connection,
    model_name: &str,
    predicate: &str,
) -> anyhow::Result<Vec<Vec<Option<String>>>> {
    let columns: Vec<String> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = '{model_name}' \
             ORDER BY ordinal_position",
        ))?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    anyhow::ensure!(
        !columns.is_empty(),
        "model {model_name:?} reported zero columns via information_schema — staging bug"
    );

    let select_list = columns
        .iter()
        .map(|c| format!("CAST({c} AS VARCHAR)"))
        .collect::<Vec<_>>()
        .join(", ");
    let order_list = columns.join(", ");
    let sql = format!(
        "SELECT {select_list} FROM main.{model_name} WHERE {predicate} ORDER BY {order_list}"
    );
    let ncols = columns.len();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], |row| {
            (0..ncols)
                .map(|i| row.get::<_, Option<String>>(i))
                .collect::<Result<Vec<_>, _>>()
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Plan claim (design §7 row 2): "Write window = output window" — output
/// rows outside the write window are byte-unchanged across a run
/// (`incremental_models.md` §Constraints "Write window = output window, per
/// cell: the DELETE/merge target and the output clamp range over the same
/// output-axis column and the same window, by construction"). Snapshots the
/// complement region (every row outside the SECOND window) before that
/// window's triggering run, then re-snapshots the same predicate after —
/// byte-equal.
///
/// Applies only to [`CaseRecipe::Partition`] cases: partition-grain batched
/// writes have a `DELETE`+`INSERT` target region a "write window" names
/// directly; `grain: key` writes are per-key `MERGE`s with no partition
/// write-window concept to check against.
pub async fn rows_outside_write_window_are_byte_unchanged(ctx: &CaseContext) -> ProbeOutcome {
    let CaseRecipe::Partition(recipe) = &ctx.recipe else {
        return ProbeOutcome::Skipped(
            "write window = output window is a partition-grain batched-write claim".to_string(),
        );
    };
    if ctx
        .plan
        .cell_for(&Trigger::NewData {
            source: recipe.source.name.clone(),
        })
        .is_none()
    {
        return ProbeOutcome::Skipped(format!(
            "no admitted NewData cell for source {:?}",
            recipe.source.name
        ));
    }

    let window1 = (
        NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"),
        NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date"),
    );
    let window2 = (
        NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date"),
        NaiveDate::from_ymd_opt(2024, 1, 3).expect("valid date"),
    );

    if let Err(e) = insert_row(
        &ctx.project,
        &recipe.source.name,
        &GenRow {
            d: window1.0,
            id: 1,
            val: 11,
        },
    )
    .await
    {
        return ProbeOutcome::Checked(Err(e));
    }
    let mut request1 = base_request("dev");
    request1.start = Some(window1.0.format("%Y-%m-%d").to_string());
    request1.end = Some(window1.1.format("%Y-%m-%d").to_string());
    if let Err(e) = ctx.project.run_quiet("probe-window1", request1).await {
        return ProbeOutcome::Checked(Err(e));
    }

    // The complement of window2 — every output row NOT in [window2.0,
    // window2.1) — snapshotted BEFORE window2's triggering run.
    let complement_predicate = format!(
        "{col} < DATE '{start}' OR {col} >= DATE '{end}'",
        col = recipe.grain.partition_column,
        start = window2.0.format("%Y-%m-%d"),
        end = window2.1.format("%Y-%m-%d"),
    );
    let before = match ctx
        .project
        .connect()
        .and_then(|conn| read_region_as_text(&conn, &recipe.model_name, &complement_predicate))
    {
        Ok(rows) => rows,
        Err(e) => return ProbeOutcome::Checked(Err(e)),
    };

    if let Err(e) = insert_row(
        &ctx.project,
        &recipe.source.name,
        &GenRow {
            d: window2.0,
            id: 2,
            val: 22,
        },
    )
    .await
    {
        return ProbeOutcome::Checked(Err(e));
    }
    let mut request2 = base_request("dev");
    request2.start = Some(window2.0.format("%Y-%m-%d").to_string());
    request2.end = Some(window2.1.format("%Y-%m-%d").to_string());
    if let Err(e) = ctx.project.run_quiet("probe-window2", request2).await {
        return ProbeOutcome::Checked(Err(e));
    }

    let after = match ctx
        .project
        .connect()
        .and_then(|conn| read_region_as_text(&conn, &recipe.model_name, &complement_predicate))
    {
        Ok(rows) => rows,
        Err(e) => return ProbeOutcome::Checked(Err(e)),
    };

    ProbeOutcome::Checked(if before == after {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "rows outside the write window {window2:?} changed across the triggering run — \
             write window = output window violated: before={before:#?} after={after:#?}",
        ))
    })
}

/// Read `main.<keyed model>`'s full contents (`key, agg_alias`) as text,
/// sorted by key — the keyed-pool counterpart of `read_region_as_text` for
/// the whole (unfiltered) table.
fn read_keyed_output_as_text(
    conn: &duckdb::Connection,
    recipe: &KeyedRecipe,
) -> anyhow::Result<Vec<(i64, Option<String>)>> {
    let (_, alias) = recipe.combiner.agg_and_alias();
    let mut stmt = conn.prepare(&format!(
        "SELECT {key}, CAST({alias} AS VARCHAR) FROM main.{model} ORDER BY {key}",
        key = recipe.source.key_column,
        model = recipe.model_name,
    ))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Plan claim (design §7 row "Technique interchangeability"): "For a cell
/// admitting both families, the same seed + schedule runs once with
/// `maintenance.cells[].technique: fold` and once with `recompute`; final
/// states must be identical." See this module's doc comment for why this
/// probe compares the fold family's real windowed `KeyedFold` runs against
/// the recompute family's real no-window full-table rebuild, rather than the
/// (confirmed unwired) `maintenance.cells[].technique` frontmatter pin.
///
/// Also asserts the review checklist's "a pin naming an unadmitted cell
/// still refuses" claim directly against the real derived plan: this
/// recipe's admitted `NewData` technique is `Technique::KeyedFold`, never
/// `Technique::ColumnScopedMerge`, so pinning `rederive_columns` for it
/// through `resolve_cell_technique` (consumed, never re-derived) must
/// refuse.
///
/// Applies only to [`CaseRecipe::Keyed`] cases.
pub async fn technique_pins_agree_at_fixed_s(ctx: &CaseContext) -> ProbeOutcome {
    let CaseRecipe::Keyed(recipe) = &ctx.recipe else {
        return ProbeOutcome::Skipped(
            "technique interchangeability (fold vs whole-table recompute) is exercised on the \
             grain: key pool only — see this module's doc comment for the confirmed \
             maintenance.cells[].technique wiring gap that rules out the partition-grain pool \
             (its NewData cell is unconditionally Technique::DeleteInsert, never a fold \
             alternative)"
                .to_string(),
        );
    };

    let trigger = Trigger::NewData {
        source: recipe.source.name.clone(),
    };
    let unadmitted_pin = resolve_cell_technique(
        &ctx.plan,
        &trigger,
        Some(CellTechnique::RederiveColumns),
        true,
    );
    if unadmitted_pin.is_ok() {
        return ProbeOutcome::Checked(Err(anyhow::anyhow!(
            "pinning `rederive_columns` for {:?}'s NewData cell (admitted technique {:?}, never \
             ColumnScopedMerge) must refuse, never silently resolve: got {unadmitted_pin:?}",
            recipe.model_name,
            ctx.plan.cell_for(&trigger).map(|c| &c.technique),
        )));
    }

    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).expect("valid date");
    // Deliberate key re-touch (id=1 appears in two windows) — the
    // interesting case for an additive combiner's fold-vs-recompute
    // agreement (design §5 "Key-recurrence control").
    let rows = [
        GenRow {
            d: d1,
            id: 1,
            val: 10,
        },
        GenRow {
            d: d1,
            id: 2,
            val: 20,
        },
        GenRow {
            d: d2,
            id: 1,
            val: 5,
        },
        GenRow {
            d: d3,
            id: 3,
            val: 30,
        },
    ];

    // Project A (`ctx.project`, the fold family): windowed runs, each
    // folding its own window's delta via KeyedFold.
    for row in &rows {
        if let Err(e) = insert_row(&ctx.project, &recipe.source.name, row).await {
            return ProbeOutcome::Checked(Err(e));
        }
    }
    for (start, end) in [(d1, d2), (d2, d3), (d3, d3 + chrono::Duration::days(1))] {
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        if let Err(e) = ctx.project.run_quiet("probe-fold", request).await {
            return ProbeOutcome::Checked(Err(e));
        }
    }

    // Project B (the recompute family): the SAME seed rows, but one
    // no-window run — smelt_runtime::execute's "single-shot full refresh of
    // the keyed SELECT" arm, recomputing the whole table from the full
    // current source contents.
    let tmp_b = match tempfile::TempDir::new() {
        Ok(t) => t,
        Err(e) => return ProbeOutcome::Checked(Err(e.into())),
    };
    let project_dir_b = tmp_b.path().join("project");
    let db_path_b = tmp_b.path().join("db.duckdb");
    if let Err(e) = std::fs::create_dir_all(&project_dir_b) {
        return ProbeOutcome::Checked(Err(e.into()));
    }
    let project_b = match render::stage_keyed(recipe, &project_dir_b, &db_path_b) {
        Ok(p) => p,
        Err(e) => return ProbeOutcome::Checked(Err(e)),
    };
    for row in &rows {
        if let Err(e) = insert_row(&project_b, &recipe.source.name, row).await {
            return ProbeOutcome::Checked(Err(e));
        }
    }
    let mut recompute_request = base_request("dev");
    recompute_request.full_refresh = true;
    recompute_request.start = None;
    recompute_request.end = None;
    if let Err(e) = project_b
        .run_quiet("probe-recompute", recompute_request)
        .await
    {
        return ProbeOutcome::Checked(Err(e));
    }

    let (rows_a, rows_b) = {
        let conn_a = match ctx.project.connect() {
            Ok(c) => c,
            Err(e) => return ProbeOutcome::Checked(Err(e)),
        };
        let conn_b = match project_b.connect() {
            Ok(c) => c,
            Err(e) => return ProbeOutcome::Checked(Err(e)),
        };
        let a = match read_keyed_output_as_text(&conn_a, recipe) {
            Ok(r) => r,
            Err(e) => return ProbeOutcome::Checked(Err(e)),
        };
        let b = match read_keyed_output_as_text(&conn_b, recipe) {
            Ok(r) => r,
            Err(e) => return ProbeOutcome::Checked(Err(e)),
        };
        (a, b)
    };

    ProbeOutcome::Checked(if rows_a == rows_b {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "fold-family (windowed KeyedFold runs) and recompute-family (no-window full rebuild) \
             final states diverged at fixed input set for recipe {:?}: fold={rows_a:#?} \
             recompute={rows_b:#?}",
            recipe.model_name,
        ))
    })
}
