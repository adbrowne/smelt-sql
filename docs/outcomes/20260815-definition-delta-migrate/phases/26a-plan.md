# Phase 26a — derived (not assumed) write-footprint mirror

## Objective

Close the first clause of success criterion 16: a `ScanClamp` stops implying a write
footprint by mirroring its own read reach and instead carries the **derived** footprint or
none at all. For a keyed-grain output the footprint question becomes posable — against the
model's declared `timeseries.partition_column` (the very axis a locality-admitted keyed node
composes on in the graph) — and propagation stops reflecting a clamp whose footprint was
never derived.

## Spec delta (first)

- `docs/specs/model_properties.md`
  - §"Footprint reflection / bounded write footprint": state that the footprint verdict a clamp
    carries is the derived one; that a keyed-grain output poses the question against its declared
    event-time axis when it has one; and that a clamp with no derivable footprint carries **no**
    write-scope claim (consumers widen, never mirror).
  - §Known Divergences: delete the "A keyed-grain output poses no partition-locality question"
    bullet; the capability-table row (line ~52) drops its "keyed-grain output poses no locality
    question — see Known Divergences" qualifier, keeping only what remains true (a *bare* keyed
    output with no declared axis has no output axis to project onto, and gets no footprint claim).
- `docs/specs/incremental_models.md` §Known Divergences, "Locality and diagnostic residues on the
  maintenance-plan proofs": drop the first clause (the assumed/underived mirror); keep the
  column-group, hour-granularity and grain-alignment clauses verbatim for 26c/26d.

## Tests (red-green)

`crates/smelt-logical/tests/keyed_footprint.rs` (new):
1. `keyed_output_with_declared_axis_carries_the_derived_footprint` — a keyed model with a
   `timeseries` axis and a bounded lookback links with `write_footprint == Some(derived)`.
2. `keyed_output_with_a_trajectory_column_refuses_the_clamp` — a keyed model whose axis carries a
   running/cumulative fold reflects `Unbounded`, so the source is `Unlinked` (today: clamped).
3. `bare_keyed_output_clamp_carries_no_footprint_claim` — no declared axis ⇒ clamp still built off
   the read bound, `write_footprint == None`.
4. `partition_addressed_clamp_carries_the_derived_footprint_numbers` — the footprint stored is the
   `FootprintResult::Bounded` value, asserted against a deliberately asymmetric read reach.

`crates/smelt-logical/tests/maintenance_tracer_propagation.rs` (extend):
5. `edge_reflects_the_derived_footprint_not_the_read_mirror` — an edge built from a clamp whose
   derived footprint differs from `(after, before)` dirties by the derived pair.
6. `edge_without_a_derived_footprint_dirties_the_whole_downstream` — `write_footprint: None` ⇒
   `reflect` yields `DayInterval::WHOLE` (safe widening); `require` (read direction) is unchanged.

`crates/smelt-db/tests/maintenance_diagnostics.rs` (extend):
7. `keyed_model_time_axis_reaches_plan_derivation` — the frontmatter `timeseries.partition_column`
   of a `grain: key` model arrives as `ModelInputs::keyed_time_axis`, evidenced by a clamp that
   carries a footprint.

## Tasks

1. Write the spec delta above.
2. `ScanClamp` (`crates/smelt-logical/src/maintenance/mod.rs`) gains
   `pub write_footprint: Option<(Seconds, Seconds)>`; `footprint()` returns `Option<(Seconds,
   Seconds)>` and stops computing the `(after, before)` mirror. Rewrite its doc comment (the
   current one documents the residue this phase removes).
3. `ModelInputs` (`derive.rs`) gains `pub keyed_time_axis: Option<&'a str>` — the declared
   `timeseries.partition_column` of a `Grain::Key` model, `None` otherwise. Mechanically add
   `keyed_time_axis: None` at the existing literal sites the compiler names.
4. `project_source_link`: for a partition-addressed output populate `write_footprint` from the
   already-consulted `FootprintResult::Bounded` value (not a re-mirror). For a keyed output with
   `keyed_time_axis: Some(axis)`, call `reflect_footprint(sql, ctx, Some(axis))`: `Bounded` ⇒
   clamp with the derived footprint; `Unbounded`/`NotDerivable` ⇒ `SourceLink::Unlinked` with a
   why naming the underived write scope. With `keyed_time_axis: None`, keep today's read-side
   linking rule but set `write_footprint: None`. Thread the footprint map into the keyed branch
   (it is currently built only when `output_partition_col()` is `Some`).
5. `propagate.rs`: `Edge` gains `footprint_days: Option<(i64, i64)>`, populated by
   `Edge::from_clamp` from `clamp.footprint()` (ceiled with the existing `clamp_days`);
   `Edge::reflect` uses it, returning `DayInterval::WHOLE` when it is `None`. `Edge::require` is
   untouched. Update the module doc's "Known boundaries" note accordingly.
6. Thread the axis at the real construction sites: `crates/smelt-db/src/queries/maintenance.rs`,
   `crates/smelt-runtime` (maintenance driver / propagation graph builders) and
   `crates/smelt-maintenance-testkit` recipes, so a locality-admitted keyed model's clamp is
   derived in production, not only in unit tests.
7. Re-run the keyed examples end to end and fix any fallout in place (see Verification) — a keyed
   model that loses a clamp because its footprint is genuinely underivable is the intended
   tightening; one that loses it because the axis was not threaded is a bug in task 6.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_footprint --test locality_projection --test
  maintenance_tracer_propagation --test maintenance_tracer_evolution`
- `cargo test -p smelt-runtime --test tracer_propagation --test tracer_evolution`
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream_composed_web_analytics`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(maintenance): carry a derived write footprint on scan clamps instead of a mirrored one`
