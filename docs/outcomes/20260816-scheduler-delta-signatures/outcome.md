# Outcome: The scheduler consumes delta signatures

**Created:** 2026-08-16
**Status:** active
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 2)
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md`,
`docs/specs/run_state.md`

> **Operator note (design risk).** This is the highest-design-risk outcome in the programme:
> it changes the run loop's currency from whole day-intervals to typed delta components.
> Andrew reviews the first Opus-planned phase (`phases/01-plan.md`, committed by the PLAN
> step) before or shortly after the loop starts implementing it — see the handoff.

## The outcome

The DAG scheduler's currency for "what needs re-running" stops being whole day-intervals and
becomes the typed delta components the derivation layer already produces. A key-addressed repair
cell derived for a `KeyedUpsert` upstream feeding a `grain: partition` downstream is actually
dispatched — the key-addressed route works outside the `grain: key` run branch. Keyed dirt-sets
carry affected key *values*, not just key columns and provenance. `--since-upstream` reads the
recorded observed-delta table live instead of requiring the operator to restate what landed, and
a persisted per-source watermark makes cross-model incremental runs work without command-line
delta declarations. `smelt explain` becomes the verification surface for all of it: the
delta-signature headline is the first line, followed by per-column guarantees and the derived
run shape.

## Success criteria (checkable)

1. **Key-addressed dispatch outside `grain: key`.** A clockless `keyed upsert` upstream feeding
   a `grain: partition` downstream has its derived key-addressed repair cell dispatched by the
   run loop (incremental, not the correct-but-full ordinary route). Fixture-backed.
2. **Key-valued dirt-sets.** Keyed dirt carries affected key values end to end through the
   graph layer (a key-level dirt representation exists — intervals are no longer the graph's
   only currency); value-level discovery feeds the scheduler, not only the run-time mechanism.
3. **Live observed-delta consumption.** `--since-upstream` reads the recorded delta table live;
   the settle-bound × observed-delta composition gets its live "delta empty" leg.
4. **Persisted per-source watermark.** Cross-model runs no longer require the operator to state
   what landed upstream; the watermark is a state family with its `state.mode` /absent-state
   behaviour specified (building on outcome `20260816-state-residency`).
5. **`smelt explain` verification surface.** The delta-signature headline is printed first, with
   the per-column guarantee summary and the derived run shape; refusals surfaced pre-execution
   where the spec requires. Closes `incremental_models.md` "`smelt explain` does not yet print
   the delta-signature headline."
6. **Spec truth restored.** `incremental_models.md` "The scheduler does not yet consume delta
   signatures end to end" and "Delta detection for `--since-upstream` is explicit-only in v1"
   are removed or narrowed to exactly the residue that remains; `/smelt:validate
   incremental_models` shows no drift for the closed bullets.
7. All standing gates green, including `maintenance_conformance` extended to cover at least one
   scheduler-driven keyed→partition cross-model recipe.

## Out of scope

- Live fold machinery for `change_feed` delta shapes, and `change_feed` `UpstreamMutation`
  cells — decision track / Future Extensions.
- The cost model between two admissible techniques.
- Per-cell `deferral` scheduling and `diff_patch` runtime lowering (per-cell frontier
  addressing) — keyed/partition residue outcomes, after the decision track grows their scope.
- The definition-delta vertical (programme outcome 3).
- Automatic source diffing beyond the persisted watermark (snapshot diffing stays future work).
- `derive_affected_keys` projecting every grain column into a cell's `KeyScope` (instead of
  intersecting with the upstream's own proven key columns) — phase 11 characterized it precisely;
  it blocks only the honest `GROUP BY d, id` spelling of a recipe whose constant-projection
  spelling already covers criteria 1/7, so it belongs to `20260816-keyed-grain-residue-v2`.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta first: pin the scheduler-currency design (typed delta components, key-valued dirt, watermark semantics) in `incremental_models.md`/`run_state.md` §Design before wiring — **Andrew reviews this plan** | done |
| 2 | Dispatch the derived key-addressed repair cell outside the `grain: key` branch (`KeyedUpsert` → `grain: partition` fixture, red-green) | done |
| 3 | Key-valued dirt-sets in the graph layer: `KeyedDirt` carries resolved key values (distinct from the unresolved-symbolic form); keyed seeds enter `propagate` as pure input; composition projects keys onto each consumer's key scope, widening never narrowing; plumbed through `plan_since_upstream` | done |
| 4 | Dispatch composition in the run loop: every resolved key-addressed cell dispatches in one tick (lifts phase 2's single-edge substitution gate to a coverage gate); an uncovered inbound input still widens to the ordinary route but reports the downgrade | done |
| 5 | Propagated key restrictions reach the key-addressed cell: a request-level keyed-restriction channel, unioned (never intersected) into the affected-key relation, with `--since-upstream` passing `keyed_dirty` through | done |
| 6 | Live observed-delta consumption: `--since-upstream` reads the recorded `_smelt_observed_delta` table off the backend instead of trusting the command line; the settle-bound × observed-delta "delta empty" leg goes live | done |
| 7 | Live keyed-seed resolution: keyed seeds resolved from the group-grain sidecar diff at plan time (unioned across consumers), so `--since-upstream` produces a real non-empty keyed restriction end to end | done |
| 8 | Persisted per-source watermark with `state.mode`-aware residency; cross-model runs need no command-line landed-delta declarations | done |
| 9 | `smelt explain` headline: the model's derived delta signature + addressing + grain label + derived run shape, printed as the report's first line (text and `--json`) | done |
| 10 | `smelt explain` per-column guarantee ledger (equivalence contract × settle bound per column) and pre-execution refusal surfacing | done |
| 11 | Walk fix: `group_by_output_keys` matches `GROUP BY` keys against output aliases, not only select-item expression text (unblocks grouped-with-derived-columns recipes) | done |
| 12 | Conformance extension: scheduler-driven keyed→partition cross-model recipe(s) in the generative suite | planned |
| 13 | Validate + close out: divergence bullets removed/narrowed, docs-site updated, full standing-gate sweep | pending |

## Decision log

- 2026-08-16: outcome activated; phase 1 planned. No reshape of the phase table — this is the
  first phase, there is no prior summary in this outcome to reshape from, and the eight rows
  map one-to-one onto success criteria 1–7 plus close-out.
- 2026-08-16 (phase 1 planning): pinned three design choices for Andrew's review rather than
  leaving them to the implementing phases — dispatch typed by *addressing* not model grain;
  key *values* seed a still-pure propagation function (rejecting backend I/O inside
  propagation, and rejecting symbolic-only dirt resolved at run time); the per-source
  watermark is a field on the existing observability-classified landed-delta family rather
  than a new correctness-classified state family (which would contradict `state.md`'s
  optionality rule).
- 2026-08-16 (phase 1 implemented): spec deltas landed in `incremental_models.md` (new
  §"Dispatch — from propagated components to run units", value-carrying "Keyed dirt-sets and
  the narrowed refusal", three §Design paragraphs, CLI/run-flags wording, two reworded Known
  Divergences bullets) and `run_state.md`/`state.md` (new §"Per-source watermark", inventory
  row updated). One incidental edit to `incremental_shapes.md` to distinguish the new
  propagation watermark from the rejected "watermark store" design paragraphs it would
  otherwise appear to contradict. No production code touched, per plan. Andrew's review of
  `phases/01-plan.md` is still pending/parallel to this implementation, per the outcome's
  operator note — phases 2+ implement exactly what this phase pinned, so a requested change
  here should land as a follow-up spec amendment before phase 2 proceeds far.
- 2026-08-16 (phase 2 planning): reshape — phase 3's row now explicitly carries dispatch
  *composition* (a model receiving several components in one tick). Phase 2 substitutes the
  key-addressed cell for the ordinary route only when every inbound ref of the model is a
  key-addressed edge that resolved a cell; a partition-grain downstream with an additional
  uncovered source keeps its ordinary route rather than risking a silently dropped component.
  That residue serves success criteria 1–2, so it stays inside the outcome (phase 3) rather
  than being deferred out. Confirmed the derivation half already admits the cell for a
  `grain: partition` downstream (`derive.rs::append_model_edge_cells`, clock route inapplicable
  for a clockless upstream) — phase 2 is a dispatch-only gap, and the
  `keyed_partition_sink_dag` testkit fixture plus its oracle test already exist to extend.
- 2026-08-16 (phase 2 implemented): dispatch lands — the key-addressed cell now runs for a
  clockless `keyed upsert` → `grain: partition` edge when the substitution gate holds (single
  resolved edge, no other inbound ref). Found and worked around (not fixed) a pre-existing
  `smelt-logical` walk gap along the way: `group_by_output_keys` matches `GROUP BY` keys against
  select-item expression text, not output alias, so grouping by a projected alias fails grain
  proof entirely rather than dropping just that column — flagged for a future phase, see
  `phases/02-summary.md` "For the next planner". No reshape of the phase table.

- 2026-08-16 (phase 3 planning): reshape — old row 3 split into two (3: key-valued dirt
  representation + pure composition + plumbing, in `smelt-logical`/`smelt-runtime::propagation`;
  4: dispatch composition in the run loop, lifting phase 2's substitution gate). One phase
  cannot honestly carry both the graph-layer currency change and the run-loop dispatch change.
  Live *keyed-seed* resolution (the backend sidecar read that fills the seeds) folded into the
  live-consumption row (now 5) beside the observed-delta read — both are the same "read the
  warehouse instead of trusting the command line" work, and criterion 2's "value-level discovery
  feeds the scheduler" is only met once that lands. Added row 8 for the
  `group_by_output_keys` alias gap phase 2's summary flagged: it serves criteria 1/7 (grouped
  recipes with derived columns currently hard-refuse grain proof), so it stays inside the
  outcome rather than being deferred out; placed before the conformance extension so recipes may
  use alias grouping. Old rows 4–8 renumbered 5–10; nothing dropped.

- 2026-08-16 (phase 3 implemented): `KeyValues`/`KeyedDirt::values`, `Edge::consumer_key_scope`,
  `propagate_with_keys` (with `propagate` as a delegating wrapper), and the keyed-only-node visit
  fix land in `smelt-logical::maintenance::propagate`; `smelt-runtime::propagation` folds
  `PlanCell::key_scope` into `Edge::consumer_key_scope` and adds
  `plan_since_upstream_with_keyed_seeds` plus `SinceUpstreamPlan::keyed_dirty`. All six planned
  tests pass; no reshape of the phase table. Fan-in merge across more than one admitted inbound
  keyed edge is implemented but untested (no fixture in this phase's list exercised it) —
  flagged for phase 4+ if a real fan-in scenario surfaces.

- 2026-08-16 (operator review of phase 1, follow-up spec amendment): Andrew's review of
  `phases/01-plan.md` upheld the three pinned design decisions and landed four amendments
  before phase 4+ builds on them. (1) Absent-watermark behaviour contradiction resolved in
  favour of the CLI's **refuse-loudly leg**: a source named with neither `--landed` nor a
  watermark propagates nothing, naming the missing watermark — the run_state bullet's
  "recompute the full dirty set" degradation is gone (unbounded, unasked-for cost), and
  `state.md` §"The optionality rule" swaps its now-wrong forward-propagation example for
  `--auto`. (2) Watermark granularity pinned: per source, advanced only by a run that
  completed every consumer; selective runs stall it, never advance it; per-`(source,
  consumer)` recorded as rejected-for-now in §Design. (3) `watermark → now` semantics for
  external sources pinned: the span itself is the landed delta; observed-delta refinement
  applies only where a record exists (model upstreams — only smelt's conditional-write path
  writes one). (4) The stale "no key-level dirt representation exists" clause removed from
  the Graph-layer-gaps divergence bullet (phase 3 landed the representation; the scheduler
  bullet owns the remaining residue). Phases 5–6 implement the amended semantics.

- 2026-08-16 (phase 4 planning): reshape — old row 4 split into two (4: dispatch composition,
  i.e. resolving and dispatching every key-addressed cell a model's inbound edges yield, plus the
  visible widen-never-narrow downgrade when an input is uncovered; 5: the propagated key
  restriction actually reaching the cell). The two halves are independent seams — composition is
  entirely inside `execute.rs`/`maintenance_driver.rs`, while the restriction needs a new
  request-level channel (`ExecuteRequest`, ~70 struct literals) plus an emitter parameter —
  and phase 3's precedent ("one phase cannot honestly carry both") applies. Nothing left the
  outcome; old rows 5–10 renumbered 6–11. Also pinned for row 5, so the implementer does not have
  to rediscover it: the restriction must be **unioned** with the sidecar-discovered key set, never
  intersected — the sidecar refresh runs in the same transaction as the write, so narrowing the
  repaired set would advance the comparandum past keys that were never consumed (the wrong-and-quiet
  outcome §"Upstream model edges" forbids). Phase 4 also lands the reporter-visible downgrade the
  spec's §"Widen-never-narrow at dispatch" already requires and phase 2 left silent; the
  `smelt explain` half of that visibility stays with row 8.

- 2026-08-16 (phase 4 implemented): `resolve_live_key_addressed_model_edge_cells` (plural)
  lands in `maintenance_driver.rs`; both `execute.rs` dispatch sites compose every resolved
  cell in one tick; the non-keyed site is now a coverage gate (licensed only when EVERY inbound
  ref resolved a key-addressed cell) with a `RunReporter::dispatch_widened` downgrade report on
  refusal. Caught and fixed a real bug along the way: `EventSink` (the wavefront scheduler's
  per-model event buffer) silently drops any `RunReporter` method it does not explicitly buffer
  and replay — `dispatch_widened` needed its own `ReporterEvent` variant + buffer/replay arms,
  or it would have silently no-op'd under the real concurrent run loop. Multi-edge admission
  (both the new unit test and the new e2e fixture) required a COALESCE/composite-grain SQL
  shape rather than a plain equi-join — `derive_affected_keys`'s provenance walk traces literal
  SELECT-list lineage, not join-predicate equality. No reshape of the phase table.

- 2026-08-16 (phase 5 planning): no reshape — phase 4's summary explicitly confirms row 5's scope
  is unchanged by the coverage gate, and the `EventSink` gotcha it flagged is already recorded
  against row 8. Two things pinned so the implementer does not have to rediscover them. (1) The
  union rule the phase-4 planning entry pinned needs a *spec* home before it is wired: the
  restriction-composition rule is user-visible correctness and is currently stated nowhere in
  `incremental_models.md`, so the phase opens with a §"Dispatch — from propagated components to run
  units" paragraph rather than encoding the rule only in code comments. (2) The union is computed
  in `resolve_key_addressed_affected_keys` on the *inputs* to
  `emit_key_addressed_affected_keys_select`, never by a second statement author — set arithmetic on
  key values is not statement authoring, so maintenance-plan purity is upheld with the emitter
  still the single owner of the affected-keys SELECT. Also noted: with no live keyed seeds until
  row 6, the CLI's `--since-upstream` leg is a wired pass-through whose resolved-value set is
  empty in practice today, so the conversion is tested directly against a seeded plan rather than
  through the CLI.

- 2026-08-16 (phase 5 implemented): `ExecuteRequest::keyed_restrictions` +
  `KeyedRestriction` land in `smelt-runtime::types`; `propagation::keyed_restrictions_from_plan`
  converts `SinceUpstreamPlan::keyed_dirty` to the wire shape; `maintenance_driver::
  union_affected_keys` (pure, unit-tested) unions the sidecar's own `changed_keys` with the
  restriction before `emit_key_addressed_affected_keys_select` is called; all three
  `dispatch_key_addressed_model_edge` call sites in `execute.rs` now look up and pass the
  request's restriction for the dispatching `(model, edge)` pair; `run_since_upstream` populates
  the map once per `--since-upstream` invocation. The load-bearing e2e test corrupts a downstream
  row directly (bypassing the sidecar entirely) to prove the restriction alone — not the sidecar
  diff — drives the repair; the union rule itself is proven as a pure unit test rather than
  through a real backend. No reshape of the phase table.

- 2026-08-16 (phase 6 planning): reshape — old row 6 ("live consumption") split into two (6:
  the observed-delta read; 7: live keyed-seed resolution). They are different seams, not one
  phase: the observed-delta half is a state-table read whose *pure* consumer already exists and
  is tested (`plan_since_upstream_with_observed_deltas`), so the work is a read function plus CLI
  wiring; the keyed-seed half needs a plan-time group-grain sidecar diff, which requires physical
  table names, the downstream's digest projection, and a union across consumers with differing
  digest identities — phase 3/5's "one phase cannot honestly carry both" precedent applies.
  Nothing left the outcome (criterion 2's "value-level discovery feeds the scheduler" is row 7,
  criterion 3 is row 6); old rows 7–11 renumbered 8–12. Two things pinned for row 6 so the
  implementer does not rediscover them. (1) Which origins are consulted must stay single-owned:
  the planner already decides eligibility via `derive_clamp_and_locality`'s `key_locality_slice`,
  so the live resolver asks the planner module for the key list (a new pure
  `observed_delta_keys_to_read`) rather than re-deriving "is this origin locality-admitted".
  (2) The live read runs under `--dry-run` too — it is a read plus the same idempotent
  `CREATE TABLE IF NOT EXISTS` every other state read performs, and a dry run that printed a
  different dirty set from the live run it previews would be worse than the write.

- 2026-08-16 (phase 6 implemented): `maintenance_driver::read_observed_delta` (decodes both
  `changed_keys` and `partitions`; `read_observed_delta_changed_keys` now delegates to it),
  `propagation::observed_delta_keys_to_read` (pure, consults the same `key_locality_slice` the
  planner reads), and a new `smelt-runtime::propagation_live::resolve_observed_delta_lookup`
  (the live backend read) land; `run_since_upstream` now creates a real backend, resolves the
  live lookup, and calls `plan_since_upstream_with_observed_deltas` in place of the always-empty
  `plan_since_upstream`. Two Known Divergences bullets narrowed in `incremental_models.md` — the
  observed-delta-consumption bullet loses its "doesn't read live"/"delta-empty leg" clauses, and
  the scheduler-currency bullet's live-resolution parenthetical loses its observed-delta clause
  (key-value live resolution stays open, row 7). All five planned tests pass; no reshape of the
  phase table.

- 2026-08-16 (phase 7 planning): no reshape — phase 6's summary flagged nothing that changes
  row 7's scope, and its "ask the planner module for the key list" precedent is exactly the
  shape this phase reuses. Three things pinned so the implementer does not rediscover them.
  (1) The sidecar partition identity is per `(upstream, consumer)` — the identity hashes the
  *consumer's* digest projection — so an upstream's seed is the **union** of its consumers'
  diffs; that composition rule is user-visible correctness and gets a spec sentence in
  §"Keyed dirt-sets and the narrowed refusal" before it is wired, matching phase 5's precedent
  for the restriction-union rule. (2) The upstream identity formula
  (`smelt.models.{edge}` + the upstream's own per-model target schema) is currently inline in
  `dispatch_key_addressed_model_edge`; the seed resolver must call a shared extraction of it,
  because a divergence silently misses the sidecar partition rather than failing loudly.
  (3) The plan-time read must not refresh any sidecar — the refresh stays inside the write
  transaction, so the run-time diff re-derives the same set and the dispatch union only widens.
  Also confirmed the non-DuckDB leg has a spec home already (§"Unresolved seeds"): an
  unsupported-dialect diff becomes `KeyValues::Unresolved`, not a run failure and not an empty
  resolved set.

- 2026-08-16 (phase 7 implemented): `propagation::keyed_seed_diffs_to_read` (pure descriptor
  enumeration), `keyed_seed_diff_result_to_key_values` + `fold_keyed_seed_values` (classify/fold),
  and `propagation_live::resolve_keyed_seeds` (the live backend read) land; `execute::
  model_edge_source_identity` extracts the shared upstream-identity formula; `run_since_upstream`
  now resolves live keyed seeds alongside the existing observed-delta read and calls the newly
  `pub` `plan_since_upstream_live`. Found and fixed a real pre-existing bug along the way:
  `model_edges_for`'s edge-address lookup used the raw joined ref segments instead of stripping
  the `models`/`sources` breadcrumb, so it silently found no edge for any ref spelled
  `smelt.models.<addr>` (only bare `smelt.<addr>` worked) — this affected the existing phase 2-4
  dispatch branches too, not just this phase's new code. No reshape of the phase table.

- 2026-08-16 (phase 8 planning): no reshape — phase 7's summary flagged nothing that changes
  row 8's scope, and its two live-read precedents are read-side only, while this row's new work
  is the write side. Three things pinned so the implementer does not rediscover them. (1) The
  `--landed`-optional surface needs a *pairing* rule the spec's "repeatable and optional per
  source" phrasing leaves implicit, so the phase opens with a §Surface sentence: bare
  `<start>..<end>` pairs positionally (today's equal-count rule, unchanged) or
  `<address>=<start>..<end>` pairs by address; the two spellings do not mix. (2) The
  absent-watermark refusal is a **named run error**, not a per-source skip — a skip would let a
  named source silently contribute nothing and under-propagate, which the fail-loud discipline
  and §"never a silent no-op" forbid; this is stated in §Surface rather than left to code. (3)
  The advance's consumer set comes from the SAME `build_forward_graph` propagation already
  builds, and a graph-build failure means coverage is unprovable → no advance (stall is the safe
  direction), never a speculative one. Also noted for the next planner: a `--since-upstream`
  sweep runs one `execute_project` per model and is therefore selective by construction, so it
  never advances a watermark — only a run completing every consumer of the source does, exactly
  as §Design "per source, not per `(source, consumer)`" intends.

- 2026-08-16 (phase 8 implemented): `SourceLanding::watermark` + `LandedDeltaStore::
  watermark`/`advance_watermark` land in `smelt-state::landed_deltas`; pure
  `smelt-runtime::watermark::watermark_advances` (every consumer of a source completed this run)
  is called at `execute.rs`'s single success path, deriving `consumers_by_source` from the SAME
  `build_forward_graph` propagation already builds. `propagation::pair_source_deltas_with_
  watermarks` adds the two `--landed` spellings (bare positional / `<address>=<start>..<end>`
  qualified) and resolves an unpaired source from its watermark, refusing by name when neither
  exists; `pair_source_deltas` keeps today's behaviour exactly, delegating with no store.
  `run.rs::run_since_upstream` now loads the landed-delta store and calls the watermark-aware
  pairing. Spec + docs-site updated; both flagged Known Divergences bullets narrowed to the
  residue that remains (automatic snapshot diffing only). No reshape of the phase table.

- 2026-08-16 (phase 9 planning): reshape — old row 9 split into two (9: the signature headline —
  derived delta signature, addressing, grain label, run shape, printed first in text and `--json`;
  10: the per-column guarantee ledger and pre-execution refusal surfacing). They are different
  derivations, not one rendering change: the headline reads verdicts the output-delta layer
  already produces and needs only a pure formatter plus one plumbed field, while the guarantee
  ledger needs a per-column composition of effective contract × settle bound that exists nowhere
  today, and refusal surfacing is a behavioural gate rather than a print. Phase 3/5/6's
  "one phase cannot honestly carry both" precedent applies; nothing left the outcome (criterion 5
  is met only when both rows land). Old rows 10–12 renumbered 11–13. Three things pinned for row 9
  so the implementer does not rediscover them. (1) The headline's *derivation* is single-owned in
  `smelt-logical` (a pure formatter over per-group `OutputDelta` + `Addressing`, beside
  `maintenance/edge_type.rs`'s existing projection rules) — the CLI formats nothing itself, so
  text and `--json` cannot drift. (2) The model's own per-group verdicts must come from the SAME
  `derive_output_delta_with_model_verdicts` call shape `ref_model_edge` uses for an edge's
  `output_shape`, extracted to a shared helper — two independent folds would let a model's own
  headline disagree with how its consumers type it. (3) The run shape is read off the keyed
  classification `smelt-db`'s `maintenance_plan_report` already runs
  (`CumulativeClassification::is_snapshot_reconcile`), never re-derived from "does it have a
  clock" at the CLI. Also noted: phase 8's summary flagged a stale keyed-seed clause in the
  scheduler-currency Known Divergences bullet — that belongs to row 13's close-out sweep, not
  here.

- 2026-08-16 (phase 9 implemented): `smelt_logical::maintenance::signature`
  (new module) lands `KeyedRunShape` + `SignatureHeadline` +
  `derive_signature_headline`, a pure formatter reusing `edge_type::
  Addressing`'s three-way mapping. `smelt-db`'s `own_output_delta_verdicts`
  is extracted from `ref_model_edge`'s inline fold and now also called by
  `maintenance_plan_report`, which populates the new `MaintenancePlanResult`
  fields `own_output_delta`/`run_shape` (the latter from
  `CumulativeClassification::is_snapshot_reconcile` for `grain: key`, from
  `metadata.timeseries` for `grain: partition`). `smelt-cli`'s
  `build_maintenance_plan_report` prints the headline first; `--json` gets a
  matching `signature` object built by the new `explain_signature_json`,
  reading the identical `SignatureHeadline` value (byte-equal fields,
  test-verified). Spec, docs-site `cli.md`, and the web-analytics tutorial
  fixture updated. No reshape of the phase table.

- 2026-08-16 (phase 10 planning): no reshape — phase 9's summary confirms row 10 is fully
  unblocked and its own types are reusable only as inputs, exactly as the row assumes. Three
  things pinned so the implementer does not rediscover them. (1) The ledger is a **model-level**
  block, not a per-cell one: the contract point varies per column *group*, the settle bound is
  model-level (`KeyLocality::settle_bound`), and printing a copy of the ledger under every cell
  would multiply one fact by the cell count; the spec's §Surface "Per cell" bullet is edited to
  move it out. (2) Settle bound is never fabricated — a model with no established key-temporal
  locality prints `not derived` rather than a zero interval or an assumed `never`; only routes
  1–3 have a derivation today. (3) The determinism exemption print lands **here**, not with the
  determinism-scope divergence: the per-column verdict already exists
  (`analysis::walk`'s `PropertyVector::determinism`, computed in the same `smelt-db` function
  that builds the report), so the exemption is a plumb-and-print, while that divergence's real
  residue (runtime pinning, oracle exemption, technique gates) is untouched and stays. Refusal
  surfacing is scoped as: one rendering, moved from the report's tail to immediately after the
  headline and given a diagnostic-code-named form, plus the `--json` array — `smelt build`'s
  own refusal gate is already the `smelt-db` diagnostic path and is not re-litigated here.
- 2026-08-16 (phase 10 implementation): shipped as planned — `maintenance::ledger` landed with
  the three pinned decisions intact. One implementation-level call not pinned by the plan: a
  column group's effective contract is resolved against its own first (lexicographically least)
  `mutation_sensitivity` source as the trigger address, since `derive_guarantee_ledger` has no
  per-cell context to resolve against — documented as a known under-report for a multi-trigger
  group with differing per-cell `deferral` overrides, never a fabricated merge.

- 2026-08-16 (phase 11 planning): no reshape of the rows — phase 10's summary confirms row 11 is
  unaffected by it, and the walk gap is exactly as phase 2 recorded. Three things pinned so the
  implementer does not rediscover them. (1) The defect is shared: `scope_group_by_alignment`
  (`analysis/mod.rs`) compares raw `GROUP BY` key text to the partition item's *expression*, so a
  scope grouping by the partition column's alias reports `NotAligned` — the same false negative in
  a second consumer. Both call sites get one shared pure resolver rather than two copies of the
  alias rule, so the walk-rule single-ownership posture holds. (2) The resolution rule is
  user-visible correctness (a family of ordinary grouped models stops being refused), so it gets a
  spec sentence in `model_properties.md` §"Region row identity" plus the alignment clause in
  `incremental_shapes.md` §"Safety checks" before wiring — phase 5/7's precedent. (3) The
  fail-closed leg does **not** change: a key resolving to neither expression text, alias, nor
  ordinal still yields `Grain::unkeyed()` for the scope; widening only covers keys the engines
  themselves resolve. Also scoped out explicitly: `derive_affected_keys` returning every grain
  column into `KeyScope` (phase 2's second flagged sharp edge) is NOT touched here — if the
  testkit's `PartitionOverKeyedId` workaround still cannot be reverted after the walk fix, the
  phase records the real remaining reason in the comments rather than chasing it.
- 2026-08-16: phase 10's summary flagged that `Refusal::ReachNotDerivable`,
  `RepairKeysNotDiscoverable`, and `RepairSliceUnbounded` have no `DiagnosticCode` variant; that
  residue is folded into row 13's close-out sweep rather than getting its own row (it is a
  catalogue alignment, not outcome-criterion work).
- 2026-08-16 (phase 11 implemented): `resolve_group_by_key_to_output` lands as the shared
  resolver in `smelt-logical::analysis::mod`; both `group_by_output_keys` (walk) and
  `scope_group_by_alignment` (partition-alignment) now use it. Spec deltas landed as planned. The
  scoped-out edge did fire exactly as anticipated: restoring the honest `GROUP BY {d}, {id}`
  shape in both phase-2 workaround sites now clears the walk's grain proof (`[d, id]`) but trips
  `MaintenanceKeyScopeColumnMissing` from `derive_affected_keys`, which still projects every grain
  column (not just the upstream's own proven key columns) into the cell's `key_scope`. Reverted
  both sites to their original constant-projection shape and rewrote the comments to name that
  real cause instead of the now-fixed walk explanation, per the phase's own instruction not to
  chase `derive_affected_keys` here. That gap is now precisely characterized (two comments name
  the exact diagnostic and the fix shape) for whoever picks it up. No reshape of the phase table.

- 2026-08-16 (phase 12 planning): no row changes — phase 11's summary confirms row 12 is
  unblocked and row 13's close-out scope is unchanged. One item moved to "## Out of scope":
  the `derive_affected_keys` `KeyScope` over-projection phase 11 characterized. It is not
  criterion work (the constant-projection recipe shape already exercises the keyed→partition
  dispatch this outcome exists to prove; only the alternative `GROUP BY d, id` spelling is
  blocked), and it is squarely the keyed-grain residue outcome's territory. Three things
  pinned so the implementer does not rediscover them. (1) The test must NOT hand-roll the
  CLI's live-plan sequence — a hand-rolled copy would stay green while `run.rs` drifted — so
  the phase opens by extracting `propagation_live::resolve_live_plan` and having `run.rs`
  delegate; the conformance test calls the same function. (2) Two scenarios are needed
  because they resolve different seeds: a source-rooted sweep plans before the keyed upstream
  re-runs, so its plan-time sidecar diff is legitimately empty (the run-time union at
  dispatch does the repair), while a model-rooted `--source dag_kpart_a` sweep planned after
  that rebuild is the one that yields real live key values — criterion 2's evidence. (3) If
  the live seed resolves `Unresolved` rather than to values, the test is not to be weakened
  into vacuity: the oracle + incrementality legs stand and the real reason is recorded for
  row 13.

## Blocked

_(empty)_
