# Outcome: Probe-backed world-facts

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §2 P-C, §6 step 3
**Spec anchors:** `docs/specs/model_properties.md` (model-scoped declarations), `docs/specs/incremental_models.md` (declared contract facts)

## The outcome

Every declared world-fact derives a cheap runtime probe that can falsify it —
the way the recurrence-bound and count-preservation probes already work.
"Declared" comes to mean "checked at run time", not "trusted forever": a
declaration is admissible only if a probe exists for it, a firing probe is a
named diagnostic with a remedy path, and the declared-facts surface becomes
safe to grow (the contract lattice will grow it).

## Success criteria (checkable)

1. The `referential_integrity` tripwire exists: the closure narrowing it
   licenses is verified by a probe in the runs that rely on it (closing the
   admitted-ahead-of-verification divergence in `model_properties.md`).
2. Declared functional dependencies, `bounded_domain`, source posture
   (append-only), and `assert_monotonic` each have a probe emitter in the
   single-owner maintenance layer; probe statements pass `statement_parity`.
3. The spec states the admissibility rule: no probe, no declaration; each
   declaration's section names its probe and firing semantics.
4. A firing probe produces a named diagnostic carrying the violated fact and
   the remedy (repair/refresh the affected cells), never a silent continue.
5. Probe cadence is controllable (per-run default, off/periodic override) and
   probe cost is visible in `smelt explain`.
6. Conformance gate includes fact-violation recipes: a violated declaration is
   caught by its probe, not by wrong output. All standing gates green.

## Out of scope

- Declared source lateness wiring into live scans (belongs to the contract
  lattice's frozen-horizon work).
- New declaration kinds — this outcome hardens the existing ones.
- Widening *which* maintenance cells consult a declared `referential_integrity`
  closure (today only the source-enrichment `UpstreamMutation` route can; a
  model-edge creation cell's closure is always derived with RI `None`). Making
  the tripwire fire for every run that relies on the declaration is criterion 1;
  giving more runs a reason to rely on it is a separate narrowing-widening
  decision, tracked in `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: the probe obligation rule — per-declaration probe, firing semantics, cadence, admissibility | done |
| 2 | Probe emitters for FD, bounded_domain, append-only posture, assert_monotonic | done |
| 3 | `referential_integrity` tripwire wired into the runs that consume the closure narrowing | done |
| 4 | Runtime probe policy: `probes:` in `smelt-core`'s `Config`, cadence decision, single-owner dispatch + firing → named diagnostic (fact, cell, remedy); the two already-wired probes routed through it | done |
| 5 | Live dispatch of the model-scoped probes (`assert_monotonic`, `functional_dependencies:`, `bounded_domain:`) at the pre-write site | planned |
| 6 | Live dispatch of the source append-only posture probe (recorded per-partition counts + frontier-fingerprint re-check) | pending |
| 7 | Conformance recipes: violated-fact scenarios caught by probes | pending |
| 8 | Surface: `ModelRunRecord.probes` population from the dispatch sites, explain rendering of probes + cost, docs-site update | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-10: Phase 3 done. `SkeletonSourceClosure::Closed` names its row-preservation route
  (`RowPreservation::JoinShape`/`DeclaredReferentialIntegrity{source}`); `emit_count_preservation_
  probe_from_body` builds the tripwire directly from a model's compiled body (matching the join by
  physical table identifier, a new `TableRef::bare_path_text`, never `smelt.<path>`-ref resolution
  — compiled bodies carry no unresolved refs); `execute_delete_insert_with_delta_restriction`
  dispatches it before trusting a declared-route restriction, failing loud
  (`SourceCountPreservationViolated`) before any write, or falling back to the widened scan when
  unbuildable. `derive_model_maintenance_plan` gained a `source_referential_integrity` param,
  threaded with real facts at the production Salsa call sites (`build_source_referential_integrity`,
  mirroring `build_key_recurrences`). Runtime dispatch reachability for the declared route stays
  scoped to the model-edge call site (model edges never carry RI, per Out of scope); a live
  UpstreamMutation-driven dispatch path is unbuilt and not this phase's target. See
  `phases/03-summary.md`.

- 2026-08-10: Outcome activated. Phase table kept as scaffolded (no prior summary to reshape from). Phase 1 plans the probe registry as a spec-parsed standing gate (`crates/smelt-logical/tests/probe_obligation.rs`) so later phases flip Status cells from `not-yet` to `built` under test, rather than the table drifting from the emitters.
- 2026-08-10: Phase 1 done. §"Probe obligation" lands in `model_properties.md` with an 8-row registry (2 built/built-unwired, 5 not-yet, 2 exempt); `sources.md`/`diagnostics.md`/`smelt_yml.md` cross-reference it. `diagnostics.md`'s unified table also picked up `SourceMutationProfileViolated`/`SourceUniqueKeyViolated`, previously defined only in `sources.md`'s own local table — a pre-existing gap the registry's citations exposed. `probes:` (`smelt_yml.md`) is spec-only; no `Config` field yet. See `phases/01-summary.md`.
- 2026-08-10: Reshape — phase 4's row now names `probes:` landing in `crates/smelt-core/src/config.rs`
  explicitly (phase 1's summary found no such field exists, and cadence has no runtime effect without
  it); the work stays inside the outcome rather than becoming a rediscovery. Phase 2 planned as
  pure emitters only: the four probes land as registry Status `built (unwired)`, all sharing one
  `violation_count`/`sample_keys` result row (the contract `maintenance_driver`'s existing recurrence
  gate already reads), with real-DuckDB executability tests since dispatch is phases 3–4. The
  `unique_key`/`delta_identity` registry row stays `not-yet` — it is outside success criterion 2's
  named four and is already recorded in `model_properties.md` §Known Divergences.

- 2026-08-10: Phase 2 done. Four new pure emitters (`emit_functional_dependency_probe`,
  `emit_bounded_domain_probe`, `emit_monotonicity_probe`, `emit_append_only_posture_probe`) land
  in `crates/smelt-logical/src/maintenance/emit.rs`, each proven against a real DuckDB to
  discriminate conforming from violating data; registry rows move `not-yet` → `built (unwired)`.
  A design correction surfaced by the executability tests: the monotonicity probe's `LAG` must
  order by a processed-row ordinal (`ROW_NUMBER() OVER ()`), never by the event-time column
  itself, or every partition trivially sorts and no violation is detectable. See
  `phases/02-summary.md`.

- 2026-08-10: Phase 3 planned. Two facts found while planning shaped it: (a)
  `SkeletonSourceClosure::Closed` records no *route* for its row-preservation
  conjunct, so a consumer cannot tell a `LEFT JOIN`-proven closure from a
  declaration-licensed one — phase 3 adds `Closed { row_preservation }` so the
  probe obligation is structural, not a comment; (b) the production Salsa
  derivation (`smelt-db/src/queries/maintenance.rs`) always passes an *empty*
  `SourceReferentialIntegrity` map, so `derive_maintenance_plan_with_referential_integrity`
  exists but has no production caller — the plan plumbs it, otherwise the wiring
  would be unreachable in a real run. No phase rows added or reordered; the
  which-cells-consult-RI widening is recorded under "## Out of scope" instead,
  since criterion 1 asks that every run *relying* on the declaration probe, not
  that more runs rely on it.

- 2026-08-10: Reshape + phase 4 planned. The old phase 4 row bundled three separable jobs —
  policy (`probes:` config + cadence), dispatch mechanics (execute, read the one-row contract,
  raise the named diagnostic), and *four* new live dispatch sites with different scopes. Split:
  phase 4 lands the policy + the one dispatch helper and re-routes the two probes that already
  dispatch; phase 5 wires the three model-scoped probes (all share the run's compiled delta as
  `scope_select`); phase 6 wires the append-only posture probe, whose scope is per-partition
  recorded counts + a frontier fingerprint and therefore needs persisted state the other three
  do not. Nothing left the outcome — criterion 6's fact-violation recipes need all four
  dispatched, so all four keep a row. Conformance and surface rows shift to 7 and 8.
  Decisions taken while planning: (a) phase 3's open question — cadence *does* govern the RI
  and recurrence dispatches, per `smelt_yml.md` Semantics 10's project-wide policy; (b) a
  **policy skip** trusts the declaration and records it unverified on the run, while a
  **probe that cannot be built** stays fail-closed exactly as today — two distinct
  non-dispatch cases that must not be collapsed; (c) `periodic`'s run ordinal comes from the
  model's existing manifest history (`HistoryQuery::for_model`), so no new counter state.

- 2026-08-10: Phase 4 done. `probes:` lands in `Config` (`ProbesConfig`/`ProbeCadence`,
  fail-loud `periodic` cross-validation); `smelt-logical::maintenance::probe_cadence::
  should_dispatch` is the pure cadence decision; `smelt-runtime::probes::dispatch_probe` is the
  shared executor for probes speaking the `violation_count`/`sample_keys` contract. The
  recurrence-bound probe routes through it directly; the count-preservation probe's
  `driving_count`/`enriched_count` row shape (locked by `statement_parity`'s golden SQL) doesn't
  fit the shared contract, so that site consults `should_dispatch` directly and reuses only the
  shared `probe_violation_suffix` trailer — `dispatch_probe` stays the generic path for probes
  that do speak the contract (phases 5–6's four). `ModelRunRecord.probes` exists and round-trips
  legacy manifests but is not yet populated by any dispatch site — recorded as follow-up, not a
  blocker. See `phases/04-summary.md`.

- 2026-08-10: Reshape + phase 5 planned. Phase 4's deferred follow-up (`ModelRunRecord.probes` is
  declared and round-trips but no dispatch site populates it) serves criterion 5 — probe status
  visible in `smelt explain` — so it stays inside the outcome: phase 8's row now names it
  explicitly rather than leaving it as a loose note in a summary. It is placed there, not in
  phase 5, because phase 6 adds a fourth dispatch site and `explain` is the only consumer;
  wiring the log once, after every site exists, avoids touching the sites twice. Phase 5 itself
  is scoped to one new owner (`smelt-runtime::model_probes`) plus two `execute.rs` call sites —
  the full-refresh/standard pre-write site and the incremental batch pre-write site — both of
  which already have the run's `compiled.sql` (the run's own processed rows, exactly the
  `scope_select` the three phase-2 emitters expect) and the existing
  `probe_policy_for_model(...)` policy in scope.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
