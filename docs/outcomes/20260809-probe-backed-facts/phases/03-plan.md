# Phase 3 plan — `referential_integrity` tripwire wired into the runs that consume the closure narrowing

**Advances:** success criterion 1 (the RI tripwire fires in the runs that rely on
the narrowing), and criterion 4 for this one probe (named diagnostic, remedy, no
silent continue).

## Objective

`emit_count_preservation_probe` is emitter-built but nothing dispatches it, so a
`SkeletonSourceClosure::Closed` whose row-preservation leg came from a *declared*
`referential_integrity` licenses a delta-restricted recompute that no run ever
verifies. This phase makes the verdict carry **which route** proved row
preservation, and makes the single live consumer of that narrowing
(`execute_delete_insert_with_delta_restriction`) probe before it writes whenever
the route is the declaration.

## Spec delta (implement step makes these edits first)

- `docs/specs/model_properties.md` §"Skeleton-source closure": `Closed` names its
  row-preservation route (join shape vs declared `referential_integrity`), and a
  sentence stating the obligation — a declared-route `Closed` may only narrow a
  run that dispatches the count-preservation probe over the touched region first;
  an unbuildable probe drops the narrowing (widened scan), never proceeds.
- Same file §"Probe obligation": the `referential_integrity` row's Status
  `built (unwired)` → `built`.
- Same file §Known Divergences: the "narrowing admitted ahead of its runtime
  verification" sentence in the skeleton-source-closure entry is removed/replaced
  by the wired statement; the probe-obligation entry drops RI from the
  "no live dispatch" list.
- `docs/specs/sources.md` §Known Divergences and `docs/specs/diagnostics.md`
  (`SourceCountPreservationViolated` entry, line ~508): the tripwire is
  implemented at the delta-restricted recompute site; the remaining gap is that
  no *other* cell family consults a declared-RI closure yet.

## Tests (red-green)

1. `smelt-logical` `skeleton_closure.rs` unit: a `LEFT JOIN` enrichment proves
   `Closed { row_preservation: JoinShape }`; an inner join under a declared
   `referential_integrity` proves `Closed { DeclaredReferentialIntegrity { source } }`.
2. `smelt-logical` `emit_statements.rs`: `emit_count_preservation_probe_from_body`
   returns a probe whose driving side omits the enrichment join clause and whose
   enriched side keeps it, both carrying the body's own `WHERE`.
3. Same file: the builder returns `None` (fail-closed) for a body it cannot
   reconstruct — no top-level `SELECT`, or no join against the named source.
4. `smelt-logical/tests/probe_execution.rs`: against a real DuckDB, the built
   probe reports `enriched_count == driving_count` on conforming data and
   `enriched_count < driving_count` when a fact row's key is missing from the
   dimension.
5. `smelt-runtime` (`technique_lowering.rs`): `execute_delete_insert_with_delta_restriction`
   given a declared-RI `Closed` and a dangling fact key fails with an error
   containing `SourceCountPreservationViolated`, the source name, the counts, and
   the remedy — **and the target table is byte-unchanged** (probe runs before any
   write).
6. Same file: the same call over conforming data succeeds and returns the
   delta-restricted `StatementGroup` unchanged (the probe does not perturb the
   emitted statements — `statement_parity` stays green).
7. Same file: a declared-RI `Closed` whose body the probe builder cannot handle
   executes the ordinary widened-scan group (narrowing dropped), never the
   restricted one.
8. `smelt-logical/tests/probe_obligation.rs`: the RI registry row's Status is
   `built` and still names `emit_count_preservation_probe`.
9. `smelt-db` maintenance query test: a source declaring `referential_integrity`
   in `sources.yml` reaches the derivation as a `SourceReferentialIntegrity`
   entry (today the production call always passes an empty map).

## Tasks

1. Spec edits above.
2. `SkeletonSourceClosure::Closed` becomes `Closed { row_preservation: RowPreservation }`
   with `JoinShape` / `DeclaredReferentialIntegrity { source: String }`; fix all
   construction/match sites (`is_closed()` keeps its signature).
3. Keep `grouping.rs`'s membership-pruning exclusion of the declared route exactly
   as-is — it now matches on `RowPreservation::JoinShape` explicitly rather than
   re-reading the join type.
4. Add `emit_count_preservation_probe_from_body(body_sql, enrichment_source) ->
   Option<MaintenanceStatement>` in `maintenance/emit.rs`: parse the body, splice
   the enrichment join clause out by text range, emit both sides as
   `SELECT 1 FROM …` (+ the body's own `WHERE`), then delegate to the existing
   `emit_count_preservation_probe`. All SQL authoring stays in the single owner.
5. `execute_delete_insert_with_delta_restriction`: when the restriction is
   actually taken and the closure is `Closed { DeclaredReferentialIntegrity { source } }`,
   build + `execute_sql` the probe first; `enriched_count < driving_count` →
   `bail!("SourceCountPreservationViolated: …")` naming source, counts, region and
   the remedy (backfill the dimension's missing key, or drop the declaration);
   unbuildable probe → `tracing::warn!` and fall back to the widened-scan group.
6. Plumb the declared `referential_integrity` facts from the resolved sources into
   `smelt-db/src/queries/maintenance.rs`'s derivation call
   (`derive_maintenance_plan_with_referential_integrity`), replacing the
   always-empty map so the declared route is real in production plans.
7. Update `docs/ROADMAP.md` if it lists the tripwire as unbuilt.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --test probe_execution --test probe_obligation --test skeleton_closure --test skeleton_closure_pinned --test maintenance_referential_integrity`
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(probes): dispatch the referential-integrity count-preservation tripwire before a declared-route narrowing writes`
