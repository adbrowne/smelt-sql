use super::*;
use crate::propagation::clamp_locality::{
    derive_clamp_and_locality, refuse_bare_keyed_origins, ClampAndLocality,
};

/// One propagated run: `model` must run over `[start, end)` (ISO dates), or
/// the whole table when `start`/`end` are `None` (an unclocked source's
/// delta — `incremental_models.md` §"The graph layer": "A delta on an
/// unclocked source dirties the whole model... the full-table run is a
/// declared cost").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropagatedRun {
    pub model: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// The full `--since-upstream` plan: the propagated per-model runs (in
/// dependency order) plus a human-readable rendering of the dirty set
/// (per-edge and per-model) to print **before** any run executes
/// (`incremental_models.md` §CLI: "Prints the dirty set before acting").
#[derive(Debug, Clone, Default)]
pub struct SinceUpstreamPlan {
    pub runs: Vec<PropagatedRun>,
    pub dirty_set_report: String,
}

/// The day ordinal at or before `secs` — the rendering seam's own outward
/// alignment (`smelt_logical::maintenance::propagate`'s module doc: "A
/// `smelt run` window rendered from a propagated interval still aligns
/// outward to whole days... because the run-window surface is
/// date-valued"). A propagated interval already aligned to Day (or
/// coarser) grain floors/ceils to itself here — a no-op.
fn iso_floor(secs: i64) -> String {
    ordinal_to_iso(secs.div_euclid(DAY_SECONDS))
}

/// The day ordinal at or after `secs` — the exclusive-end half of the
/// rendering seam's outward alignment; see [`iso_floor`].
fn iso_ceil(secs: i64) -> String {
    let floor = secs.div_euclid(DAY_SECONDS);
    let ordinal = if floor * DAY_SECONDS == secs {
        floor
    } else {
        floor + 1
    };
    ordinal_to_iso(ordinal)
}

fn render_interval(iv: &PartitionInterval) -> String {
    if iv.is_whole() {
        "whole table".to_string()
    } else if iv.is_open_ended() {
        format!("[{}, →)", iso_floor(iv.start))
    } else {
        format!("[{}, {})", iso_floor(iv.start), iso_ceil(iv.end))
    }
}

/// Compute the forward-propagation plan for `deltas` over the real
/// per-workspace graph (`build_forward_graph`), in `order` (the caller's
/// topological execution order — models absent from `order` are ignored,
/// mirroring how `propagate` only ever dirties nodes reachable from the
/// declared edges).
///
/// A thin wrapper over [`plan_since_upstream_with_observed_deltas`] with an
/// empty observed-delta lookup — every model-edge origin falls back to its
/// declared `--landed` window unwidened-and-unprojected (the D1
/// widen-never-narrow rule's "absent" case, since an empty lookup can never
/// contain a recorded row). Consumed by the testkit and the generative
/// conformance harness's DAG families, which need the pure/empty-lookup
/// form; `smelt run --since-upstream`'s real CLI path instead loads a live
/// lookup via [`load_observed_delta_lookup`] and calls
/// [`plan_since_upstream_with_observed_deltas`] directly.
pub fn plan_since_upstream(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
) -> Result<SinceUpstreamPlan> {
    // `now` is unreachable for this wrapper: an empty lookup can never
    // produce a `Some(od) if od.is_empty()` match, the only arm that reads
    // it — so no real clock value is needed here.
    plan_since_upstream_with_observed_deltas(
        models,
        source_infos,
        order,
        deltas,
        &BTreeMap::new(),
        "",
    )
}

/// One `(model, window_start, window_end)` key into the observed-delta
/// lookup [`plan_since_upstream_with_observed_deltas`] consults — the SAME
/// three-part key `smelt_state::ddl_duckdb`'s `_smelt_observed_delta` table
/// uses as its own `PRIMARY KEY`. `window_start`/`window_end` are ISO date
/// strings, matching [`ordinal_to_iso`]'s own rendering of a
/// [`SourceDelta::landed`] window.
pub type ObservedDeltaKey = (String, String, String);

/// The read-side lookup for `--since-upstream`'s observed-delta
/// consultation: `None` for a `(model, window)` key means "never recorded"
/// (`docs/specs/incremental_models.md` §"The graph layer" — "Empty and
/// absent are distinct"; the widen-never-narrow fallback to the declared
/// `--landed` window applies), while `Some` — even with both vectors empty
/// — means a conditional write ran and recorded its (possibly empty)
/// changed-row set for that exact window.
pub type ObservedDeltaLookup = BTreeMap<ObservedDeltaKey, smelt_state::ddl_duckdb::ObservedDelta>;

/// Load the live observed-delta lookup [`plan_since_upstream_with_observed_deltas`]
/// consults, for `--since-upstream`'s real CLI caller. One read per delta
/// whose origin names a maintained model (`model_names`, the caller's
/// already-discovered model set — the CLI's own `model_by_addr`); a delta
/// whose origin is a raw source is skipped (never a valid observed-delta
/// key, `sources.*` never records one). Keyed exactly `(model,
/// iso(landed.start), iso(landed.end))`, matching
/// [`plan_since_upstream_with_observed_deltas`]'s own lookup key
/// construction. A non-DuckDB backend yields an empty map (every delta
/// falls back to the declared window unwidened) via
/// [`crate::maintenance_driver::read_observed_delta`]'s own read-side
/// fallback — never an error, matching every other observed-delta read.
pub async fn load_observed_delta_lookup(
    backend: &dyn smelt_backend::Backend,
    schema: &str,
    deltas: &[SourceDelta],
    model_names: &BTreeSet<String>,
) -> Result<ObservedDeltaLookup> {
    let mut lookup = ObservedDeltaLookup::new();
    for delta in deltas {
        if !model_names.contains(&delta.source) {
            continue;
        }
        let window_start = iso_floor(delta.landed.start);
        let window_end = iso_ceil(delta.landed.end);
        let observed = crate::maintenance_driver::read_observed_delta(
            backend,
            schema,
            &delta.source,
            &window_start,
            &window_end,
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to read observed delta for '{}': {e}", delta.source)
        })?;
        if let Some(od) = observed {
            lookup.insert((delta.source.clone(), window_start, window_end), od);
        }
    }
    Ok(lookup)
}

/// [`plan_since_upstream`]'s full form: consults `observed` for every
/// delta origin that names a locality-admitted composed model
/// (`docs/specs/incremental_shapes.md` §"What the composed shape uniquely
/// enables" — exact `--landed` for model edges, Phase D3). For such an
/// origin:
/// - **absent** from `observed` (no entry for `(model, window_start,
///   window_end)`): falls back to the declared `--landed` window unchanged
///   (D1's widen-never-narrow rule — a consumer must not assume a narrower
///   form exists just because this machinery is live).
/// - **present and empty** (`ObservedDelta::is_empty()`): the origin
///   contributes **no** dirt at all — the graph half of the no-op cascade
///   (a fully-suppressed run's downstream has nothing to do).
/// - **present and non-empty**: the recorded `partitions` are projected to
///   exact partition-day intervals via
///   `smelt_logical::maintenance::propagate::project_observed_delta`,
///   using the SAME established [`smelt_logical::maintenance::locality::
///   LocalitySlice`] route `smelt explain`'s own `key_locality` reads
///   (never re-derived) — replacing the whole declared window with the
///   observed, possibly-narrower (or, under route 3, differently-widened)
///   partition set.
///
/// A delta origin that is a raw `sources.*` address, or a maintained model
/// without an admitted key-temporal-locality route (a bare `grain:
/// partition` model, say), is never looked up in `observed` at all — its
/// delta is always the declared `--landed` window, exactly as before this
/// phase.
pub fn plan_since_upstream_with_observed_deltas(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
    observed: &ObservedDeltaLookup,
    now: &str,
) -> Result<SinceUpstreamPlan> {
    refuse_bare_keyed_origins(models, source_infos, deltas)?;
    let edges = build_forward_graph(models, source_infos)?;
    let ClampAndLocality {
        key_locality_slice,
        key_locality_settle_bound,
        ..
    } = derive_clamp_and_locality(models, source_infos)?;

    let mut source_deltas: BTreeMap<String, Vec<PartitionInterval>> = BTreeMap::new();
    // The settle-bound × observed-delta composition's reporting leg
    // (`docs/specs/incremental_models.md` §"Observed deltas on model
    // edges"): a present-and-empty delta whose window lies behind the
    // origin's own derived settle bound is a provably-final settled no-op,
    // distinct from one still inside the bound — a REPORTING distinction
    // only, since both arms already contribute zero dirt above (this never
    // prunes further work).
    let mut empty_delta_notes: Vec<String> = Vec::new();
    for d in deltas {
        let projected: Vec<PartitionInterval> = match key_locality_slice.get(&d.source) {
            Some(slice) => {
                let window_start = iso_floor(d.landed.start);
                let window_end = iso_ceil(d.landed.end);
                let key = (d.source.clone(), window_start.clone(), window_end.clone());
                match observed.get(&key) {
                    // Present and empty: a fully-suppressed run recorded
                    // nothing to propagate — the graph half of the no-op
                    // cascade.
                    Some(od) if od.is_empty() => {
                        if let Some(bound) = key_locality_settle_bound.get(&d.source) {
                            let verdict =
                                smelt_logical::maintenance::locality::settled_empty_verdict(
                                    bound,
                                    &window_end,
                                    now,
                                    true,
                                );
                            let label = match verdict {
                                smelt_logical::maintenance::locality::SettledEmptyVerdict::SettledNoOp => {
                                    "settled no-op (behind the settle bound)"
                                }
                                _ => "empty this run (not yet settled)",
                            };
                            empty_delta_notes.push(format!(
                                "  {}: recorded delta is empty for [{window_start}, \
                                 {window_end}) — {label}\n",
                                d.source
                            ));
                        }
                        Vec::new()
                    }
                    // Present and non-empty: project the recorded
                    // partitions to exact dirt via the model's own
                    // established locality route.
                    Some(od) => smelt_logical::maintenance::propagate::project_observed_delta(
                        slice,
                        &od.partitions,
                    ),
                    // Absent: never recorded (e.g. a run that predates
                    // conditional-write recording) — widen-never-narrow
                    // fallback to the declared window.
                    None => vec![d.landed],
                }
            }
            // Not a locality-admitted composed-model origin at all (a raw
            // source, or a model with no admitted key-temporal-locality
            // route) — always the declared window, unchanged.
            None => vec![d.landed],
        };
        source_deltas
            .entry(d.source.clone())
            .or_default()
            .extend(projected);
    }

    let prop = propagate(&edges, &source_deltas)
        .map_err(|e| anyhow::anyhow!("MaintenanceGraphUnsupportedNode: {e}",))?;

    let mut report = String::from("Dirty set (--since-upstream):\n");
    for note in &empty_delta_notes {
        report.push_str(note);
    }
    if prop.per_edge.is_empty() {
        report.push_str("  (no source landed a delta that any model reads — nothing to run)\n");
    }
    for ((downstream, upstream), intervals) in &prop.per_edge {
        // Column-group-scoped dirt (`incremental_models.md` §"The graph
        // layer" → "Column-group-scoped dirt"): rendered only when this
        // edge's own scope narrowed — an unscoped line stays byte-identical
        // to before.
        let groups_suffix = prop
            .per_edge_groups
            .get(&(downstream.clone(), upstream.clone()))
            .map(|groups| format!(" [groups: {}]", groups.join(", ")))
            .unwrap_or_default();
        for iv in intervals {
            if downstream == upstream {
                report.push_str(&format!(
                    "  {downstream} <-(self, unrolled) {upstream}: {}{groups_suffix}\n",
                    render_interval(iv)
                ));
            } else {
                report.push_str(&format!(
                    "  {downstream} <- {upstream}: {}{groups_suffix}\n",
                    render_interval(iv)
                ));
            }
        }
    }
    // The keyed channel's per-edge counterpart (`incremental_models.md`
    // §"The graph layer" → "Keyed dirt-sets and the narrowed refusal"):
    // rendered distinguishably from an interval line, naming the affected
    // key columns rather than a day range (the keyed channel has no
    // interval axis).
    for ((downstream, upstream), records) in &prop.per_edge_keys {
        for kd in records {
            report.push_str(&format!(
                "  {downstream} <-(keyed) {upstream} [keys: {}]\n",
                kd.keys.join(", ")
            ));
        }
    }
    // Only real models (nodes in the caller's topological `order`) are ever
    // run — `prop.dirty` also carries the seeded deltas themselves (a delta
    // origin is its own "dirty" entry before any edge reflects it). A raw
    // *source* origin is filtered by `order_set` (sources are never in the
    // topological model order); a *maintained-model* origin (a `--source
    // <model-address>` delta, `incremental_models.md` §"Upstream model edges":
    // "a model's landed delta is the output window a completed run wrote for
    // it") IS in `order`, but its run already happened — it must propagate to
    // its downstreams without being re-run itself. `origin_names` excludes
    // both.
    let order_set: BTreeSet<&str> = order.iter().map(|s| s.as_str()).collect();
    let origin_names: BTreeSet<&str> = deltas.iter().map(|d| d.source.as_str()).collect();
    for (model, intervals) in &prop.dirty {
        if !order_set.contains(model.as_str()) || origin_names.contains(model.as_str()) {
            continue;
        }
        for iv in intervals {
            report.push_str(&format!("  RUN {model}: {}\n", render_interval(iv)));
        }
    }
    // A node carrying keyed dirt but no interval dirt (both endpoints of its
    // own inbound edge keyed-grain — `smelt_logical::maintenance::propagate::
    // propagate`'s own cascade) is still a dirty node: report it as a
    // whole-table keyed run, naming the affected key columns, distinct from
    // an interval `RUN` line. A node that ALSO carries interval dirt is
    // already reported/scheduled above — this only ever adds the
    // keyed-only case.
    for (model, records) in &prop.keyed_dirty {
        if !order_set.contains(model.as_str())
            || origin_names.contains(model.as_str())
            || prop.dirty.contains_key(model)
        {
            continue;
        }
        let keys: BTreeSet<&str> = records
            .iter()
            .flat_map(|kd| kd.keys.iter().map(|k| k.as_str()))
            .collect();
        report.push_str(&format!(
            "  RUN {model}: keyed (keys: {})\n",
            keys.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    let mut runs = Vec::new();
    for name in order {
        if origin_names.contains(name.as_str()) {
            continue;
        }
        let mut scheduled = false;
        if let Some(intervals) = prop.dirty.get(name) {
            for iv in intervals {
                scheduled = true;
                if iv.is_whole() {
                    runs.push(PropagatedRun {
                        model: name.clone(),
                        start: None,
                        end: None,
                    });
                } else if iv.is_open_ended() {
                    runs.push(PropagatedRun {
                        model: name.clone(),
                        start: Some(iso_floor(iv.start)),
                        end: None,
                    });
                } else {
                    runs.push(PropagatedRun {
                        model: name.clone(),
                        start: Some(iso_floor(iv.start)),
                        end: Some(iso_ceil(iv.end)),
                    });
                }
            }
        }
        // A keyed-only-dirty node (no interval `dirty` entry at all) is
        // still scheduled — a single whole-table run (the keyed channel has
        // no interval axis to bound it by), deduplicated against an
        // interval-dirt run already pushed above so a node carrying both
        // never gets two `PropagatedRun`s.
        if !scheduled && prop.keyed_dirty.contains_key(name) {
            runs.push(PropagatedRun {
                model: name.clone(),
                start: None,
                end: None,
            });
        }
    }

    Ok(SinceUpstreamPlan {
        runs,
        dirty_set_report: report,
    })
}

/// Narrow a [`SinceUpstreamPlan`] to `--select`/`--exclude` (`incremental_models.md`
/// §CLI): propagation itself is always whole-workspace (dirt must compose
/// through unselected intermediates), but only the `selected` models
/// actually execute. `upstreams` is the direct (one-hop) model-dependency
/// map (a model's own declared refs, not transitively expanded) — the same
/// shape `DependencyGraph::get_upstream` returns per model.
///
/// A retained (selected and dirty) run whose direct upstream is ALSO dirty
/// (present in `plan.runs`) but was dropped by the selector is refused
/// fail-loud rather than silently run against a stale input — the same
/// posture `cli.md` §"`--exclude` and working-set consistency" already
/// takes for the ordinary selector path. A dirty-but-clean-upstream (an
/// upstream that never appears in `plan.runs`) is never checked: dropping an
/// already-current model from the selection cannot stale anything.
///
/// Deselected dirty models are not dropped silently — they are appended to
/// the returned report as `SUPPRESSED (not selected)` lines, so the printed
/// dirty set still shows the whole propagated set per `incremental_models.md`'s
/// "prints the dirty set before acting" rule.
pub fn scope_plan_to_selection(
    plan: &SinceUpstreamPlan,
    selected: &BTreeSet<String>,
    upstreams: &BTreeMap<String, BTreeSet<String>>,
) -> Result<SinceUpstreamPlan> {
    let dirty_models: BTreeSet<&str> = plan.runs.iter().map(|r| r.model.as_str()).collect();

    let mut retained: Vec<PropagatedRun> = Vec::new();
    let mut suppressed: Vec<&str> = Vec::new();
    for run in &plan.runs {
        if selected.contains(&run.model) {
            retained.push(run.clone());
        } else {
            suppressed.push(run.model.as_str());
        }
    }

    for run in &retained {
        let Some(ups) = upstreams.get(&run.model) else {
            continue;
        };
        for up in ups {
            if dirty_models.contains(up.as_str()) && !selected.contains(up) {
                bail!(
                    "'{model}' is dirty and retained by the selector, but its dirty upstream \
                     '{upstream}' was dropped by the selector — add '+{model}' to pull the \
                     upstream in, or drop '{model}' from the selection",
                    model = run.model,
                    upstream = up
                );
            }
        }
    }

    let mut report = plan.dirty_set_report.clone();
    if !suppressed.is_empty() {
        report.push_str("Suppressed by selector:\n");
        for model in &suppressed {
            report.push_str(&format!("  SUPPRESSED (not selected): {model}\n"));
        }
    }

    Ok(SinceUpstreamPlan {
        runs: retained,
        dirty_set_report: report,
    })
}

/// Resolve an open-ended propagated run (`start: Some(_), end: None` — a
/// time-unrolled self-edge's frontier, `incremental_models.md` §"Time-unrolled
/// self-edges") to a finite `[start, end)` window a real run can execute.
///
/// The dirty *set* stays open-ended — that is the honest statement of what is
/// dirty — but a *run* needs a closed region, so at scheduling time (here,
/// never in the plan itself) the open end is resolved to `today + 1 day`
/// (today's partition inclusive), against the caller-supplied `now` (the SAME
/// clock value the propagation planner already takes, so a self-edge's
/// forward widening and this resolution agree on "today"). A closed or
/// whole-table run is returned unchanged. A `start` on or after the resolved
/// end (the frontier's own start is already past today) is a fail-loud
/// refusal naming the model — a silently empty window would be
/// wrong-and-quiet.
pub fn resolve_run_window(run: &PropagatedRun, now: &str) -> Result<PropagatedRun> {
    let (Some(start), None) = (&run.start, &run.end) else {
        return Ok(run.clone());
    };

    let now_date = now
        .get(0..10)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .with_context(|| format!("Invalid `now` value for run-window resolution: {now}"))?;
    let resolved_end = now_date + chrono::Duration::days(1);

    let start_date = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date in propagated run: {start}"))?;
    if start_date >= resolved_end {
        bail!(
            "'{model}' has an open-ended propagated run starting {start} — on or after the \
             resolved window end {end} — nothing to run",
            model = run.model,
            end = resolved_end.format("%Y-%m-%d")
        );
    }

    Ok(PropagatedRun {
        model: run.model.clone(),
        start: Some(start.clone()),
        end: Some(resolved_end.format("%Y-%m-%d").to_string()),
    })
}

/// The resolved backward-resolution plan for `smelt build --include-upstreams`:
/// the per-ancestor required slices (raw sources to stage, model regions to
/// build) plus the ancestor-first/target-last build order, and a
/// human-readable rendering to print **before** any build executes (mirrors
/// [`SinceUpstreamPlan`]'s "print before acting" shape).
#[derive(Debug, Clone, Default)]
pub struct ResolvedBuildPlan {
    /// The models to build, in dependency order, target last. Raw sources
    /// are never in this list — they are staged (verified to exist), not
    /// built.
    pub build_order: Vec<PropagatedRun>,
    pub report: String,
}

/// Resolve what must exist upstream for `target` to be correct over
/// `period`, over the SAME real per-workspace graph
/// [`build_forward_graph`] assembles for `--since-upstream` — the graph
/// layer's two directions share one edge object
/// (`incremental_models.md` §"Windowed maintenance and the horizon"). Delegates the
/// actual reverse-topological resolution to
/// [`smelt_logical::maintenance::propagate::required_inputs`]; this
/// function only assembles the graph, renders the report, and shapes the
/// per-model build order the CLI executes.
///
/// Deliberately never consults the observed-delta record
/// (`docs/specs/incremental_models.md` §"Backward resolution — what must
/// exist"): the resolved slices answer an existence question ("what must
/// exist over this period"), and a change record cannot soundly narrow
/// that — narrowing on delta evidence alone would under-cover the resolved
/// period, breaking `forward(backward(P)) ⊇ P`. A stated non-goal, not
/// unfinished work.
pub fn resolve_build_plan(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    target: &str,
    period: PartitionInterval,
) -> Result<ResolvedBuildPlan> {
    let edges = build_forward_graph(models, source_infos)?;

    let resolved = required_inputs(&edges, target, period)
        .map_err(|e| anyhow::anyhow!("MaintenanceGraphUnsupportedNode: {e}"))?;

    // A required node with at least one inbound edge is a model to build;
    // everything else (a leaf of the ancestor sub-DAG) is a raw source to
    // stage — exactly `required_inputs`'s own `build_order` membership test.
    let buildable: BTreeSet<&str> = resolved.build_order.iter().map(|s| s.as_str()).collect();

    let mut report = String::from("Required upstream slices (--include-upstreams):\n");
    for (node, intervals) in &resolved.required {
        let verb = if buildable.contains(node.as_str()) {
            "BUILD"
        } else {
            "STAGE"
        };
        for iv in intervals {
            report.push_str(&format!("  {verb} {node}: {}\n", render_interval(iv)));
        }
    }
    report.push_str(&format!(
        "Build order: {}\n",
        resolved.build_order.join(", ")
    ));

    let mut build_order = Vec::with_capacity(resolved.build_order.len());
    for name in &resolved.build_order {
        let intervals = resolved.required.get(name).cloned().unwrap_or_default();
        for iv in intervals {
            if iv.is_whole() {
                build_order.push(PropagatedRun {
                    model: name.clone(),
                    start: None,
                    end: None,
                });
            } else {
                build_order.push(PropagatedRun {
                    model: name.clone(),
                    start: Some(iso_floor(iv.start)),
                    end: Some(iso_ceil(iv.end)),
                });
            }
        }
    }

    Ok(ResolvedBuildPlan {
        build_order,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_source_address_strips_smelt_and_sources_prefixes() {
        assert_eq!(normalize_source_address("bronze"), "bronze");
        assert_eq!(normalize_source_address("sources.bronze"), "bronze");
        assert_eq!(normalize_source_address("smelt.sources.bronze"), "bronze");
        assert_eq!(normalize_source_address("sources.raw.bronze"), "raw.bronze");
    }

    #[test]
    fn parse_landed_range_parses_iso_dates() {
        let iv = parse_landed_range("2026-01-03..2026-01-04").expect("parse");
        assert_eq!(iv.start + DAY_SECONDS, iv.end);
    }

    #[test]
    fn parse_landed_range_rejects_malformed_input() {
        assert!(parse_landed_range("not-a-range").is_err());
        assert!(parse_landed_range("2026-01-04..2026-01-03").is_err());
        assert!(parse_landed_range("bogus..2026-01-03").is_err());
    }

    #[test]
    fn pair_source_deltas_rejects_length_mismatch() {
        let err = pair_source_deltas(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &["2026-01-01..2026-01-02".to_string()],
        )
        .expect_err("mismatched counts must error");
        assert!(err.to_string().contains("--source and --landed"));
    }

    #[test]
    fn pair_source_deltas_pairs_positionally() {
        let deltas = pair_source_deltas(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &[
                "2026-01-01..2026-01-02".to_string(),
                "2026-01-05..2026-01-06".to_string(),
            ],
        )
        .expect("pair");
        assert_eq!(deltas[0].source, "bronze");
        assert_eq!(deltas[1].source, "aux");
    }
}
