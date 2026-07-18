# Plan: Composed axes (key + time) and conditional maintenance

**Date**: 2026-07-17 (supersedes the 2026-07-15 skeleton at this path)
**Spec**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) (primary); companion diffs in [`models.md`](../specs/models.md), [`sources.md`](../specs/sources.md), [`model_properties.md`](../specs/model_properties.md), [`model_transforms.md`](../specs/model_transforms.md), [`multi_backend.md`](../specs/multi_backend.md), [`output_fingerprint.md`](../specs/output_fingerprint.md)
**Spec diff**: `219f4f28..HEAD` — the composed-axes diff (orthogonality doctrine, pruning taxonomy, composed-shape capabilities) **plus** the 2026-07-17 Relation Contract diff (`58e717f7`: grain-as-derived-label, per-cell write addressing, open write-pattern registry, source-side contract)
**Research**: `docs/research/20260715-conditional-maintenance-without-cdf.md` (M1/M2/M3, P1–P4, T1–T5); `docs/research/20260716-relation-contract-and-per-cell-addressing.md`; `docs/research/20260705-keyed-time-superset.md`
**Tracking PR / branch**: `spec-incremental-models-consolidation` (PR TBD)
**Docs**: code+docs
**Supersedes**: `docs/plans/20260705-keyed-time-partitioned-output.md` (absorbed into Group A; that plan cites the retired `keyed_models.md` spec and the removed `refresh: keyed` surface. Its Phase 5 — the `key_recurrence` source declaration parse — already landed and is consumed by Phase A4 here; everything else was never started.)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/incremental_models.md` §"The two axes are orthogonal", §"Key temporal locality", §"What the composed shape uniquely enables", §"Windowed maintenance and the horizon" (pruning taxonomy), §"Per-cell write addressing", §"The graph layer" — the spec is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `spec-incremental-models-consolidation`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` (the web-analytics workspace is this plan's flagship fixture; `examples/timeseries/` and dedicated broken-fixture workspaces carry the refusal cases).
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`: Salsa purity, property-composition-walk rule (new proofs land walk-composed, `walk_coverage` gate), maintenance-plan purity (emitters are the single statement author; extend `statement_parity.rs` for every new/changed emitter), fail-loud discipline, layered single-ownership.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to specs and `docs-site/` describe the feature as if it has always existed — no `### Phase A — …` headings, no `(Phase B)` inline labels, no `[deferred to Phase E1]` callouts. If a phase ships an incomplete surface, the *spec* records the gap under **Known Divergences** in behavioural terms. The Progress table is where "what landed when" lives.

## Context

Two workstreams gate each other and share their best demo. The composed shape (locality-admitted `grain: key` + `timeseries:` — spec §"Key temporal locality") is what makes conditional maintenance *affordable* (slice-bounded compares) and *propagatable* (exact key→partition dirt projection); conditional maintenance (spec §"Windowed maintenance and the horizon" category 2; §Future Extensions M1/M2/M3) is what makes the composed shape *pay off* downstream (empty-delta no-op cascades). The flagship fixture is one workspace: `examples/web_analytics` — event-grain dedupe under `key_recurrence` is exactly where change-suppressed writes turn redelivery storms into zero-write no-ops, and the events→sessions→identity chain is the model-edge delta-restriction demo. The Relation Contract diff (`models.md` §"The Relation Contract"; grain demotion; per-cell write addressing) landed after the skeleton and is folded in as Groups S and R.

Sequencing follows research §11: locality first (Group A — it is the enabling shape), the web-analytics tracer immediately after (Group W — proof we are on the right track before the deep machinery), surface alignment (S), graph (B), suppression (C), observed deltas (D), delta-restricted compute (E), external-source sidecars (F), choice + docs sweep (G).

## Scope

### In scope (spec coverage)
- `incremental_models.md` §"Key temporal locality" — the admission gate, all three routes, slice-pruned merge target, route-3 transactional check, settle bound, output-as-clocked-source.
- `incremental_models.md` §"What the composed shape uniquely enables" — all four capabilities (propagation admissibility, exact dirt projection, slice-bounded suppression, settle × observed-delta composition).
- `incremental_models.md` §"The graph layer" — the refined keyed-node refusal ("without an admitted time axis"); composed nodes as clocked propagation participants.
- `incremental_models.md` §"Windowed maintenance and the horizon" category 2 — no-op write elimination (M1), with its fail-closed comparability rules.
- `incremental_models.md` §Future Extensions "Conditional maintenance without a change feed" — M1/M2/M3 graduate to surface via per-group spec-diff phases (C1, D1, E1, F1).
- `models.md` §"Refresh axis" + §"The Relation Contract" — facts-as-surface (top-level `unique_key:`), grain as derived label + check-only assertion, source-side derived grain.
- `incremental_models.md` §"Per-cell write addressing" — the available-addressings rule, the open write-pattern registry, the `maintenance.cells[].write` pin, `MaintenanceWriteAddressingRefused` / `MaintenanceWritePatternUnavailable`.
- `sources.md` — `key_recurrence` consumption (parse already landed); landed-delta refinement; referential-integrity world-fact (trust rule: narrowing ⇒ tripwire).
- User docs: web-analytics tutorial gains the composed-shape chapter early (Group W); guide/reference pages ride each group's docs phase.

### Explicitly deferred
- **The trajectory grain (`key_per_partition`) beyond an honest refusal** (A0). Backfill-cascade discipline and lateness truncation get their own plan when demand exists.
- **Granularity relaxation** (daily driver → weekly output partitions); **snapshot-reconcile locality**; **slice-scoped deletion** (`NOT MATCHED BY SOURCE`) — spec Open Questions, unchanged.
- **Keyed dirt-sets and time-unrolled self-edges** in the graph — bare keyed nodes still refuse; composed nodes propagate via partition projection only.
- **Automatic watermark-diffed `--since-upstream`** — the explicit `--source`/`--landed` form stays v1 (spec §Future Extensions).
- **`versioning: interval`** — unrelated to this diff; tracked by its own divergences.
- **Hourly driver granularity** — inherited `maintenance_driver.rs::driving_steps` limitation, orthogonal.
- **Sub-day propagation axes** — the propagation layer stays day-ordinal.

## Progress tracking

| Phase | Description | Status |
|---|---|---|
| A0 | `key_per_partition` fail-loud refusal (stop the silent collapse) | done (2026-07-17) |
| A1 | Locality gate seam: three-route `KeyedForbidsTimeseries` message; narrowed `KeyedGroupByContainsPartitionColumn`; real `unique_key` threaded into the plan | done (2026-07-17) |
| A2 | Route 1 (key-embedded): admission + slice-pruned merge target scan | done (2026-07-17) |
| A3 | Route 2 (key-determined, once-write provenance) | done (2026-07-18) |
| A4 | Route 3 (recurrence-bounded): consume `key_recurrence` + transactional `KeyedRecurrenceBoundViolated` check | done (2026-07-18) |
| A5 | Output as clocked source; settle-bound derivation + `smelt explain` surface | done (2026-07-18) |
| A6 | Composed-shape conformance recipes (testkit family + generative gate legs) | done (2026-07-18) |
| W0 | Extremal-aggregate nullability: `MIN`/`MAX` over a NOT NULL argument infers NOT NULL in grouped context (unblocks W1) | done (2026-07-18) |
| W1 | Web-analytics tracer: composed `events_deduped` model, redelivery demo, project tests | blocked (2026-07-18 — new gap found: `derive_fold_spec` single-non-key-aggregate limit, distinct from the MIN/MAX nullability gap W0 fixed) |
| W2 | Web-analytics tutorial chapter + docs-site guide for the composed shape | blocked (2026-07-18 — transitively blocked on W1) |
| S1 | Facts-as-surface: top-level `unique_key:`, `refresh: incremental` admitted on facts alone, grain derived + check-only assertion | done (2026-07-18) |
| S2 | Relation Contract read-side: derived grain for sources; `smelt explain` prints both providers' contract | done (2026-07-18) |
| B1 | Graph admissibility for locality-admitted composed nodes (edge construction at declared granularity) | done (2026-07-18) |
| B2 | Key→partition dirt projection (exact routes 1–2; widen-by-`r` route 3) in forward propagation + backward resolution | pending |
| B3 | `--since-upstream` accepts a composed node as `--source`; adjointness tests extended | pending |
| C1 | Spec diff: `model_transforms.md` T1/T2 variants; `multi_backend.md` capability flags (incl. `supports_column_scoped_merge` into the capability struct) | pending |
| C2 | P3 change-comparability per column (walk lattice fold; `plausible`/pinned-`NOW()` ⇒ Incomparable) | pending |
| C3 | P2 region row identity (declared `unique_key` → proven grain key → `WholeRow` multiset) | pending |
| C4 | T1 change-suppressed column-scoped MERGE (+ statement-parity leg; suppressed-vs-unconditional bit-equality at fixed `S`) | pending |
| C5 | T1 on the keyed fold; T2 staged-candidate conditional DELETE+INSERT (staged temp relation in the statement group) | pending |
| C6 | Slice-bounded compare on composed models (compose C4/C5 with A2) | pending |
| C7 | Docs: conditional writes (explain output, cost notes, `prefer`/`technique` steering) | pending |
| R1 | Open write-pattern registry + `maintenance.cells[].write` pin + the two write-addressing refusal diagnostics | pending |
| D1 | Spec diff: `sources.md` landed-delta refinement; storage home + transactionality of recorded deltas | pending |
| D2 | T5 observed output delta recording (comparable columns only; byproduct of C4/C5 writes) | pending |
| D3 | Partition projection of observed deltas via locality → exact `--landed` for model edges | pending |
| D4 | `smelt explain` observed-delta/settle surface; docs | pending |
| E1 | Spec diff: `model_properties.md` P1 skeleton-source closure; `sources.md` referential-integrity world-fact + count-preservation tripwire | pending |
| E2 | P1 skeleton-source-closure proof (fail-closed to `Open`) | pending |
| E3 | T3 delta-restricted compute over model edges (web-analytics events→sessions chain demo) | pending |
| E4 | Conformance legs: delta-restricted vs widened-scan equivalence; empty-delta no-op cascade end-to-end | pending |
| F1 | Spec diff: fingerprint sidecar (naming, storage, transactionality, invalidation; digest stance vs `output_fingerprint.md`); P4 projection derivation | pending |
| F2 | P4 fingerprint-projection derivation (fail-closed: unprojectable ⇒ full-row digest) | pending |
| F3 | T4 sidecar DDL/DML via emitters, upserted in the consuming write's transaction; external `mutable_snapshot` delta derivation | pending |
| F4 | Sidecar invalidation (definition change / schema evolution ⇒ "everything changed", widen-never-narrow) | pending |
| F5 | T3 over external sources (fixture must fail the closure proof without the RI declaration — the proof must discriminate) | pending |
| G1 | Conditional variants in per-cell technique choice (first-build admit-not-prefer; bakeoff stays deferred) | pending |
| G2 | Docs sweep + `/smelt:validate incremental_models` drift report | pending |

## Decisions taken while fleshing out the skeleton

1. **Absorbed** `docs/plans/20260705-keyed-time-partitioned-output.md` (see header). Its route/phase detail became A1–A5; its `key_recurrence` parse phase had already landed.
2. **The tracer comes before the machinery.** Group W (web-analytics composed model + tutorial chapter) runs immediately after Group A so the flagship shape is demonstrable and documented before the graph/suppression work starts. W uses the surface that parses at that point (`grain: key` + `timeseries:`); S1 later makes `grain:` check-only without invalidating it.
3. **`plausible` under suppression** stays fail-closed refusal of the conditional technique (spec pins this — C2); revisiting is a spec change, not a plan decision.
4. **Digest stance**: exact `IS DISTINCT FROM` for write suppression (C4/C5); SHA-256-class digests only for the F-group sidecar, with the soundness invariant stated in F1's spec diff and oracle-gated.
5. **Sidecar lifecycle and observed-delta trust boundary** are settled in the D1/F1 spec-diff phases (warehouse-resident beside the merge ledger, same-transaction, is the default posture); those phases block their groups until the spec says otherwise.
6. **v1 delta posture**: record key-level, propagate partition-level (widen-never-narrow) — D3.
7. **S1 ran ahead of its stated `W2` pre-condition.** W2 (and its transitive blocker W1) were `blocked` on the pre-existing MIN/MAX NOT-NULL inference gap, unrelated to S1's own scope. S1's TDD tests, implementation shape, and critical files were self-contained (no dependency on W2's tutorial content existing), so it proceeded; its named fixture `examples/web_analytics/models/silver/events_deduped.sql` doesn't exist yet (that's W1's undelivered artifact), so S1 substituted the existing `device_user_edges.sql` model to demonstrate the declared-`unique_key` spelling staying diagnostic-clean. `derive_grain` also landed as `-> Option<Grain>` rather than the plan's literal `-> Grain` (`None` represents "neither fact declared"; both call sites handle it without unwrap/panic).
8. **S2 substituted its real-fixture example.** The plan's literal fixture (`events_deduped`'s inbound edge to the raw events source) doesn't exist yet — still W1's undelivered artifact. S2 used two already-landed `examples/timeseries` fixtures instead (`daily_events_status` for two differently-shaped source edges, `user_spend_running_total` for a model edge), which exercise the identical `RelationContractView`/report code paths and satisfy the "source edge and model edge render through the same rows" requirement. S2 also touched `docs/specs/cli.md` (not listed in the phase's stated "Docs touched") to keep the spec's `smelt explain` surface description in sync with the new contract rows — same spec-first rule as everywhere else, just not anticipated when the phase was scoped.
9. **B1 substituted its real-fixture example, and had to plumb a pre-existing gap in the same
   file it was already allowed to touch.** The plan's literal fixture
   (`examples/web_analytics/models/silver/events_deduped.sql` mid-chain) is still W1's undelivered
   artifact. B1 used the already-landed `examples/timeseries` composed chain instead —
   `sources.raw.transactions -> user_daily_spend` (`grain: key` + `timeseries:`, route 1
   key-embedded) `-> user_spend_rollup` (`grain: partition`) — whose own doc comments already cite
   this plan's Phase A5, confirming it as the intended tracer analogue. Exercising that real
   fixture through `build_forward_graph` surfaced a second, narrower gap in the same function:
   the call site passed `driving_source_granularity: None` unconditionally (a pre-existing,
   explicitly-commented placeholder from an earlier phase, MP15/MP16), which fails
   `establish_locality`'s granularity-equality structural precondition unconditionally — so no
   `grain: key` model could ever actually admit locality through this call site, regardless of
   B1's own classification fix. Plumbing a real value (the "exactly one clocked declared-source
   candidate, else undecided" rule via `single_clocked_granularity`, the same computation
   `smelt-db`'s `check_file_diagnostics` already performs for the `smelt explain` path) was
   necessary for the real-fixture test to pass at all, stayed entirely inside
   `crates/smelt-runtime/src/propagation.rs` (the phase's own critical file, doing exactly the
   "node classification from the locality verdict" work the phase describes), and does not
   implement any key→partition dirt projection (B2's scope, still deferred) — it only lets a
   `grain: key` model driven purely by declared `sources.*` refs reach the SAME admission verdict
   `smelt explain` already reports for it. The recursive case (a driving source that is itself
   another maintained model's own composed output) is left unplumbed here, matching
   `smelt-db::lib.rs`'s wider `model_source_granularities` handling that this call site does not
   replicate — out of scope, and harmless to defer since an unplumbed candidate there still just
   yields no edge rather than a wrong one.

## Blocked phases

- **2026-07-18 — W1** (`Web-analytics tracer: composed events_deduped model`). Blocked by a
  pre-existing, out-of-scope defect: the type-inference registry infers `MIN`/`MAX` as nullable
  unconditionally, regardless of argument nullability. W1's flagship shape needs an extremal-fold
  (`MIN(event_date)`-class) `timeseries.partition_column` — exactly the route-2/route-3 shape
  `docs/specs/incremental_models.md` lines ~1810-1867 already documents as carrying this gap. The
  block fires *earlier* than that spec text anticipated: not only at the manually-driven runtime
  harness, but already at static-diagnostic time (`example_diagnostics`/LSP), because the
  `timeseries.md` NOT-NULL precondition on `partition_column` can never be satisfied by a
  `MIN`/`MAX` projection under the current inference. Reproduced directly: staging
  `examples/web_analytics/models/silver/events_deduped.sql` (`grain: key`, `timeseries: {
  event_time_column: first_seen_date, partition_column: first_seen_date, granularity: day }`, body
  `SELECT event_id, MIN(device_id) AS device_id, MIN(user_id) AS user_id, MIN(CAST(event_time AS
  TIMESTAMP)) AS event_ts, MIN(CAST(event_date AS DATE)) AS first_seen_date, MIN(utm_campaign) AS
  utm_campaign, MIN(payload) AS payload FROM smelt.sources.raw.events GROUP BY event_id`, no
  `WHERE`/`unique_key:`/`safety_overrides:`) alongside a `mutation_profile.key_recurrence: {key:
  [event_id], window: '1 day'}` + matching `timeseries:` block added to
  `examples/web_analytics/models/sources/raw/events.yml`, then running
  `cargo test -p smelt-cli --test example_diagnostics web_analytics_no_diagnostics -- --nocapture`
  produces:
  ```
  [Error] models/silver/events_deduped.sql: timeseries partition_column 'first_seen_date' must be NOT NULL — a nullable value silently escapes the pruning window
  [Error] models/silver/events_deduped.sql: no maintenance technique admits trigger NewData { source: "raw.events" }: keyed grain with no fold specification
  ```
  The second diagnostic is a downstream consequence of the same failed classification
  (`crates/smelt-logical/src/maintenance/derive.rs` around line 311-317, `inputs.fold == None`),
  not an independent bug. Because the diagnostic-clean requirement fires at test 1
  (`example_diagnostics`) and test 2 (`example_workspaces` via the real LSP) — both of W1's own TDD
  list — there is no way to land any of the four planned tests green with the flagship model
  present in the tree until the nullability gap is fixed; fixing it is production-code work this
  phase's own scope excludes ("Implementation shape. Example + source-YAML work only ... no
  production code"). All investigation changes were reverted; the tree is clean and matches HEAD.
  **Candidate options:** (a) fix the `MIN`/`MAX` nullable-unconditionally inference gap in a
  dedicated phase first (likely in `smelt-db`'s type-inference registry, propagating real argument
  nullability through extremal aggregates) and re-open W1 after; (b) reshape W1's flagship SQL to
  avoid an extremal-fold `partition_column` (e.g. a route that doesn't need `MIN`/`MAX` on the
  clock column) — but this would abandon the "extremal-fold family" demonstration the tracer is
  meant to showcase and may not be achievable for a dedupe-by-first-arrival shape; (c) narrow W1 to
  a `smelt explain`-only admission demo (drop the real `smelt run`/e2e leg and the
  diagnostic-clean requirement) pending the fix. **Recommendation:** (a) — the gap is already
  tracked as a known, separately-trackable defect by the spec text itself; fixing it once likely
  unblocks route 2's identical documented gap at the same time, and every other option either
  weakens the flagship or defers real coverage indefinitely.

- **2026-07-18 — W2** (`Web-analytics tutorial chapter + composed-shape guide`). Blocked
  transitively: W2's stated pre-condition is W1 (the tracer must exist to be teachable —
  the tutorial page and staged workspace are built around `events_deduped`, which W1 was
  supposed to land), and W1 is itself blocked on the pre-existing MIN/MAX NOT-NULL
  inference gap above. No independent W2 work is in scope until W1 unblocks.
  **Candidate options:** (a) wait for W1's blocker (the MIN/MAX inference fix) to land,
  then resume W1 → W2 in order; (b) re-scope W2 to a flagship shape that doesn't depend on
  W1's exact fixture (out of scope for this phase — would require a plan edit).
  **Recommendation:** (a) — same as W1's recommendation; no autonomous action fixes this
  faster than resolving the shared blocker.

- **2026-07-18 — RESOLVED (W1, W2): option (a) taken.** Phase **W0** (extremal-aggregate
  nullability: grouped `MIN`/`MAX` over a NOT NULL argument infers NOT NULL) is scaffolded
  in Group W and registered `pending` in the Progress table; W1 and W2 are flipped back to
  `pending` with W0 added to W1's pre-conditions. The two blocked entries above are retained
  for the record; do not re-block on the same cause — if W0 lands and the repro still fails,
  that is a new finding.

- **2026-07-18 — W1, NEW FINDING (W0 fix confirmed working; second independent gap
  surfaced).** With W0 landed, staging the exact flagship repro from the first W1 blocked
  entry (`events_deduped.sql` with `key_recurrence` on the raw events source) advances past
  the nullability error — confirmed by trimming to a single-aggregate variant
  (`SELECT event_id, MIN(CAST(event_date AS DATE)) AS first_seen_date FROM
  smelt.sources.raw.events GROUP BY event_id`), which passes `example_diagnostics` cleanly.
  But the full six-column flagship shape (`MIN(device_id)`, `MIN(user_id)`, `MIN(event_ts)`,
  `MIN(first_seen_date)`, `MIN(utm_campaign)`, `MIN(payload)`) still fails:
  ```
  [Error] models/silver/events_deduped.sql: no maintenance technique admits trigger
  NewData { source: "raw.events" }: keyed grain with no fold specification
  ```
  Root cause is independent of nullability: `derive_fold_spec` in
  `crates/smelt-db/src/queries/maintenance.rs` (~line 128-151) admits a `grain: key` model's
  `NewData` cell only when the outermost `SELECT` has **exactly one** non-key aggregate
  column (`if aggregates.len() != 1 { return None; }`). `derive_new_data`'s `Grain::Key`
  branch (`crates/smelt-logical/src/maintenance/derive.rs:311-317`) refuses the cell when
  `inputs.fold` is `None`, which is what happens for any multi-aggregate `SELECT`. The prior
  blocked-phase note misdiagnosed this second diagnostic as "a downstream consequence of the
  same failed [nullability] classification" — it is not; it reproduces identically with a
  fully NOT-NULL, single-column fold once more than one aggregate column is added.
  `silver/device_user_edges.sql` (3 aggregates: COUNT/MIN/MAX) is not a counterexample — it
  reads from an upstream **model** (`silver.events_parsed`), reaching maintenance derivation
  via a different trigger path than a raw-source-driven `NewData` cell, not because
  multi-aggregate folds are generally supported on this route. There is no declarative
  escape hatch: `maintenance.cells[].technique` only pins a technique on an already-derived
  cell, and no cell exists here to pin. All investigation changes were reverted; tree is
  clean and matches HEAD. **Candidate options:** (a) extend `derive_fold_spec` /
  `FoldSpec` to support a per-column combiner list for the source-driven `NewData` path
  (mirroring the comment already in `device_user_edges.sql` describing
  COUNT→SUM/MIN→MIN/MAX→MAX composition) — production code, needs its own phase; (b) narrow
  W1's flagship SQL to a single-aggregate shape — loses the multi-column payload-carrying
  demonstration the tracer is meant to show; (c) explain-only admission demo, no real
  `smelt run`/e2e leg, pending the fix. **Recommendation:** (a) — same shape as the first W1
  blocker's resolution: fix the shared production-code gap in a dedicated phase (a new W0b
  or similarly-scoped phase extending `derive_fold_spec` to multi-column extremal folds),
  then resume W1 with the flagship shape intact. This is a human plan-scaffolding decision,
  not something this iteration should pick unilaterally.

---

## Group A — the composed shape exists at all (key temporal locality)

### Phase A0: `key_per_partition` fail-loud refusal

**Goal.** Replace the silent grain collapse (`Grain::KeyPerPartition` → `PlanGrain::Key { unique_key: vec![] }`) with an explicit not-yet-supported refusal naming this plan. One-commit hygiene fix, shippable immediately.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_plan_refusals.rs::key_per_partition_refuses_not_silently_collapses` — a `key_per_partition` model derives a plan-level refusal (a new `Refusal` variant naming the unsupported grain), not a keyed plan with an empty key.
- Real fixture: new broken workspace `examples/timeseries_broken_key_per_partition/` with a `grain: key_per_partition` model; `crates/smelt-cli/tests/example_diagnostics.rs` asserts the specific not-yet-supported diagnostic (intentional-diagnostic case, like the existing `timeseries_broken_*` workspaces).
- `crates/smelt-cli/tests/explain_model.rs` — `smelt explain` on that fixture prints the refusal, not a keyed cell table.

**Implementation shape.** In `crates/smelt-db/src/queries/maintenance.rs` (the `ConfigGrain::Key | ConfigGrain::KeyPerPartition` arm at ~`:178`): split the arms; `KeyPerPartition` maps to a refusal (new `Refusal` variant in `crates/smelt-logical/src/maintenance/mod.rs`) surfaced through `maintenance_plan_diagnostics`. Do **not** build any trajectory machinery.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/maintenance.rs` — the grain-mapping arm.
- `crates/smelt-logical/src/maintenance/mod.rs` — the `Refusal` variant.
- `crates/smelt-db/src/queries/maintenance.rs` / `crates/smelt-db/src/lib.rs` — diagnostic fold-in.
- `examples/timeseries_broken_key_per_partition/` — new broken fixture.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: replace the "`grain: key_per_partition` is surface-only and collapses silently" entry with the honest-refusal behaviour (timeless wording).

**Review checklist** (material findings only):
- [ ] The refusal names the grain and the tracking plan; no trajectory execution attempted.
- [ ] No keyed plan is ever derived for `key_per_partition` (assert the negative).
- [ ] Fail-loud discipline: diagnostic, not `Unknown`/default.
- [ ] Spec divergence entry updated, timeless.

**Commit.** `fix(maintenance): refuse grain: key_per_partition fail-loud instead of silently collapsing to an empty keyed plan`

---

### Phase A1: Locality gate seam + three-route refusal message + real `unique_key` in the plan

**Goal.** Stop treating keyed + `timeseries:` as an unconditional frontmatter hard error; route it to a single fail-closed locality-gate entry point that (until A2 wires route 1) still refuses via `KeyedForbidsTimeseries` — but with the spec's three-routes-and-nearest-missing-fact message. Narrow `KeyedGroupByContainsPartitionColumn` to the no-`timeseries:` case. Thread the model's real derived `unique_key` (from the keyed classifier's GROUP BY) into `PlanGrain::Key` and `SourceFacts` instead of the hardcoded `vec![]`.

**Pre-conditions.** A0.

**TDD tests to write first.**
- `crates/smelt-core/tests/refresh_axis.rs` — keyed + `timeseries:` is no longer rejected at `metadata.rs` frontmatter validation (the `is_keyed() && timeseries.is_some()` arm at ~`:568` is removed); the combination reaches plan derivation.
- `crates/smelt-logical/tests/` (locality unit tests, new `maintenance/locality.rs` module tests) — with no route implemented, the gate returns a refusal rendered as `KeyedForbidsTimeseries` whose message names all three routes and the nearest missing fact (assert message substrings for each route's hint).
- `crates/smelt-logical/src/rules/cumulative.rs` tests — `KeyedGroupByContainsPartitionColumn` fires **only** when the model has no `timeseries:` block.
- `crates/smelt-logical/tests/maintenance_plan_*.rs` — a keyed model's derived plan carries the classifier's real `unique_key` (not `vec![]`); `smelt explain` prints it.
- Real fixture: `examples/timeseries_broken_cumulative_with_timeseries/` still fails, but `example_diagnostics.rs` now asserts the three-route message text.

**Implementation shape.** New pure module `crates/smelt-logical/src/maintenance/locality.rs`: `establish_locality(inputs) -> Result<LocalitySlice, LocalityRefusal>`; A1 body returns `Err(NoRouteEstablished { nearest_missing_fact })`. Move the refusal from `smelt-core` frontmatter validation into plan derivation (the seam every later phase widens). The message is rendered where `Keyed*` diagnostics already render (`rule_diagnostics.rs` / `smelt-db` fold-in). `derive_model_maintenance_plan` populates `unique_key` from the classifier output.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/metadata.rs` — remove the unconditional arm (keep `KeyedForbidsBatched`).
- `crates/smelt-logical/src/maintenance/locality.rs` (new) + `mod.rs` — the gate types.
- `crates/smelt-logical/src/rules/cumulative.rs`, `crates/smelt-logical/src/rules/rule_diagnostics.rs` — message + narrowed diagnostic.
- `crates/smelt-db/src/queries/maintenance.rs`, `crates/smelt-db/src/lib.rs` — unique_key threading; diagnostic fold-in.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences → "The key grain": narrow the "refused unconditionally, blanket wording" entry to "refused pending route establishment; the gate and the three-route message exist" (behavioural, timeless).

**Review checklist** (material findings only):
- [ ] Single entry point: no second place decides keyed+timeseries admissibility.
- [ ] Everything stays fail-closed — no route admits yet.
- [ ] The `MetadataError` exhaustiveness gate still compiles (variant kept, raised from the new seam or retired cleanly).
- [ ] `unique_key` threading does not change any existing plan's admitted techniques (assert on an existing fixture's explain output).
- [ ] Spec/docs edits timeless.

**Commit.** `feat(maintenance): fail-closed key-temporal-locality gate with the three-route refusal message; thread real unique_key into the plan`

---

### Phase A2: Route 1 (key-embedded) + structural preconditions + slice-pruned merge target

**Goal.** Implement the structural preconditions (window-forward run shape; `partition_column` a projection in an admitted family, provably NOT NULL; granularity equality) and route 1 — `partition_column ∈ unique_key`. On admission, derive `LocalitySlice` = scan window widened by derived lateness/skew margins, and wire the physical payoff: the `merge_into` target scan carries the slice predicate. Pruning is read-side only; every scanned delta row still merges.

**Pre-conditions.** A1 gate in place.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (locality) — preconditions: snapshot-reconcile + `timeseries:` refused; nullable partition projection refused; driver-granularity ≠ output-granularity refused (each with the nearest-missing-fact message).
- `crates/smelt-logical/tests/` — route 1 admits when `partition_column ∈ unique_key`; slice equals `[scan_start − margins, scan_end]`.
- `crates/smelt-runtime/src/cumulative.rs` merge-builder tests — a locality-admitted model's MERGE carries a target-side partition predicate over the slice; a non-time-partitioned keyed model's SQL is byte-unchanged; a delta row whose key would live out-of-slice still appears in the `USING (...)` delta.
- `crates/smelt-runtime/tests/statement_parity.rs` — the slice-predicated keyed-fold MERGE gets a parity leg (executed == emitted).
- Real fixture (DuckDB, runtime): a per-`(entity_id, day)` keyed aggregate in `examples/timeseries/` compiles clean and, over a two-window run, window 2 does not re-scan window 1's target partition, yet end state equals a full refresh.

**Implementation shape.** Route 1 + preconditions in `locality.rs` (reusing `analysis/source_bounds.rs` margin derivation and the monotonicity trace — walk-composed, never a raw text scan). `emit_keyed_fold` gains an optional slice predicate parameter (emitter stays the single author); `maintenance_driver.rs` threads the per-step slice.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/locality.rs` — preconditions + route 1.
- `crates/smelt-logical/src/maintenance/emit.rs` — `emit_keyed_fold` slice predicate.
- `crates/smelt-runtime/src/cumulative.rs`, `crates/smelt-runtime/src/maintenance_driver.rs` — threading.
- `examples/timeseries/models/` — the per-(key, day) fixture.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: route 1 + slice-pruned merge admitted (behavioural).

**Review checklist** (material findings only):
- [ ] Preconditions match the spec's bulleted list exactly (§"Key temporal locality").
- [ ] Pruning is target-read-side only; no write clamp introduced (spec: only proofs prune).
- [ ] Emitter is the single author of the predicate; parity leg added.
- [ ] End-state equivalence on the real fixture.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(maintenance): key-embedded locality route with slice-pruned merge target scan`

---

### Phase A3: Route 2 (key-determined, once-write provenance)

**Goal.** Admit locality when the partition projection is a per-key constant under the once-write provenance proof (key-derived expression, or a declared functional dependency over a provably non-null column). Slice = the delta's own partition values — exact regardless of key age; a key's partition value never moves under this route.

**Pre-conditions.** A2.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` — route 2 admits a key-derived partition projection (e.g. `MIN(event_date)` under a once-write key); the slice is the delta's partition values, not the scan window; an FD over a nullable column is refused fail-closed.
- `crates/smelt-runtime/` — for a years-old key redelivered today, the emitted slice predicate covers exactly that key's home partition (assert emitted SQL), and end state equals a full refresh.
- Real fixture: an `examples/timeseries/` dedupe-by-`event_id` model whose `first_seen_date` is key-determined; asserted via `example_diagnostics` (clean) + a runtime equivalence test.

**Implementation shape.** Route 2 in `locality.rs`, consuming the existing once-write/functional-dependency proofs (`analysis/functional_dependency.rs`) via the walk — no new raw-text classification. Slice derivation switches from window-based to delta-valued (a per-step set/interval of partition values).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/locality.rs` — route 2.
- `crates/smelt-runtime/src/cumulative.rs` — delta-valued slice threading.
- `examples/timeseries/models/` — the dedupe fixture.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: route 2 admitted (behavioural).

**Review checklist** (material findings only):
- [ ] Route 2 slice is exact (delta partition values), never the widened window.
- [ ] Nullable-FD refusal is fail-closed with the named column.
- [ ] Row movement impossible under this route (asserted).
- [ ] Walk-composed: `walk_coverage` gate passes with the new consumption classified.

**Commit.** `feat(maintenance): key-determined locality route via once-write provenance`

---

### Phase A4: Route 3 (recurrence-bounded) + transactional runtime check

**Goal.** Admit locality under a key-recurrence bound `r`: derived from the SQL where statically decidable, else read from the driving source's declared `key_recurrence` (parse already landed in `crates/smelt-core/src/sources.rs`). Slice = scan window widened backward by `r` + margins. A **declared** `r` is admitted only **checked**: the merge transaction verifies no delta row matched (or would duplicate) a stored key outside the slice; violation rolls back with `KeyedRecurrenceBoundViolated` (count + sample keys). Row movement admitted in-slice.

**Pre-conditions.** A2–A3. The merge ledger provides the transactional boundary.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` — a derivable `r` admits with a proof-backed (uncheck-flagged) slice; a declared-only `r` (source `key_recurrence` whose `key` matches the model's `unique_key` exactly) admits with `checked = true`; a `key_recurrence` whose key ≠ the model's `unique_key` does not establish the route.
- `crates/smelt-runtime/` — the checked route-3 merge emits the out-of-slice match probe inside the same transaction; an in-bound redelivery merges cleanly; an out-of-bound redelivery rolls back with `KeyedRecurrenceBoundViolated` (assert count + sample keys, and that the target table is unchanged after rollback); a **derived** `r` never emits the check.
- `crates/smelt-runtime/tests/statement_parity.rs` — parity leg for the checked-merge statement group.
- Real fixture (DuckDB): 3-day dedupe over `examples/timeseries/` — `key_recurrence: { key: [event_id], window: '3 days' }`, day-by-day window-forward, end state equals full refresh; an injected 4-day-late duplicate trips the check.

**Implementation shape.** Route 3 in `locality.rs` (SQL derivation first; declared fallback marks the slice checked). The probe + rollback ride the existing ledger transaction in `maintenance_driver.rs`; the probe statement is emitted by the emitter layer (single author), not hand-formatted in the runtime. New diagnostic `KeyedRecurrenceBoundViolated` through the standard `Keyed*` rendering path.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/locality.rs`, `.../emit.rs` — route 3 + probe emitter.
- `crates/smelt-runtime/src/cumulative.rs`, `.../maintenance_driver.rs` — checked-merge transaction.
- `crates/smelt-logical/src/rules/rule_diagnostics.rs`, `crates/smelt-db/src/diagnostics_types.rs` — the diagnostic.
- `examples/timeseries/` — the dedupe fixture + its source YAML.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: route 3 + runtime check built.
- `docs/specs/sources.md` — Known Divergences: `key_recurrence` now consumed (the "consumed by nothing" caveat narrows).

**Review checklist** (material findings only):
- [ ] Derived `r` prunes without a check; declared `r` always emits the check (never trusted).
- [ ] Violation fails transactionally — target unchanged; never a silent drop.
- [ ] Probe text is emitter-authored (structural no-authoring gate extended).
- [ ] `diagnostics.md` catalogue entry agrees with the spec's severity/trigger.

**Commit.** `feat(maintenance): recurrence-bounded locality route with transactional out-of-slice check`

---

### Phase A5: Output as clocked source; settle bound + explain surface

**Goal.** Register an admitted composed output's `timeseries:` so downstream models receive source-filter pushdown against it and a downstream keyed model may take it as its clocked driving source — the clock propagates through the DAG instead of stopping at the keyed stage. Derive and surface the **settle bound** in `smelt explain` (route 1: source lateness; route 2: never settles; route 3: `r` + margins).

**Pre-conditions.** A2 (at least one route admits); A3–A4 for the per-route settle values.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` — a composed output resolves as a clocked source for a downstream model: a downstream partition-grain model derives source-filter pushdown against it; a downstream keyed model selects it as driving source (window-forward, not snapshot-reconcile).
- `crates/smelt-logical/tests/` — the derived settle bound matches the route (assert all three).
- `crates/smelt-cli/tests/explain_model.rs` — `smelt explain` prints the locality verdict (route, slice form) and settle bound for a composed model.
- Real fixture: a two-stage `examples/timeseries/` pipeline — composed dedupe → downstream daily partition-grain aggregate — where the downstream compiled SQL carries pushdown (the clock-sink is gone).

**Implementation shape.** Publish the admitted output's `TimeseriesConfig` into the same source-shape registry a declared source populates (so pushdown and driving-source resolution consume it unchanged). Settle bound derived in `locality.rs`, printed by `crates/smelt-cli/src/explain.rs::build_maintenance_plan_report`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/locality.rs` — settle bound.
- `crates/smelt-logical/src/analysis/` — model-output timeseries into the shape registry consumed by pushdown/driving-source resolution.
- `crates/smelt-cli/src/explain.rs` — report rows.
- `examples/timeseries/` — the two-stage fixture.
- `crates/smelt-db/src/queries/maintenance.rs` — fold the admitted `LocalitySlice`/settle bound (the `Ok` branch of `establish_locality`, previously discarded) into the derived plan, and publish a referenced upstream model's own admitted composed output as a `SourceFacts` driving-source candidate for a downstream `grain: key` model — the call site's own pre-existing comment named this phase as where that folding lands.
- `crates/smelt-db/src/lib.rs` — the recursive upstream-model source-facts resolver (`ref_model_source_facts`) lives here, plus its wiring into `maintenance_plan`/`maintenance_plan_report`.
- `crates/smelt-logical/src/maintenance/mod.rs` — carry the admitted key-temporal-locality verdict (slice + settle bound, a new `KeyLocality` type) on `MaintenancePlan` so `smelt-db` and `smelt explain` can consume it without re-deriving admission.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: output-as-clocked-source + settle bound built (clears the clock-sink half of the composed-shape divergence).

**Review checklist** (material findings only):
- [ ] Downstream partition-grain gets pushdown; downstream keyed selects window-forward.
- [ ] Route 2 = never settles is honest in the explain output (not a large sentinel).
- [ ] A re-written slice is changed input to staleness (no stale downstream).
- [ ] Run-flag semantics unchanged: `--event-time-*` still addresses the *driving source's* clock.

**Commit.** `feat(maintenance): publish composed keyed output as a clocked source; derive and explain the settle bound`

---

### Phase A6: Composed-shape conformance recipes

**Goal.** Extend the generative equivalence oracle with a composed-shape recipe family: keyed + `timeseries:` models across all three routes, driven through the real `execute_project` pipeline against DuckDB, asserted equal to the full-refresh oracle after every run step — plus per-slice equivalence spot checks.

**Pre-conditions.** A2–A5.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/recipe.rs` — a `RecipePool::composed_keyed()` (new `KeyShape`/pool variants covering route-1 key-embedded, route-2 key-determined, route-3 declared-recurrence with in-bound redeliveries); `render.rs` emits the frontmatter + source YAML (incl. `key_recurrence`).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::composed_keyed_pool_upholds_equivalence` — the standing proptest leg (deterministic seed), mirroring `append_only_partition_pool_upholds_equivalence`, including adversarial schedules (out-of-order where order-independent, redelivery storms).
- A per-slice equivalence probe: for an admitted recipe, each output slice equals the model SQL over the slice's derived reach.
- An admission-rate floor for the pool (so route refusals can't silently hollow the gate).

**Implementation shape.** Testkit-only phase; no production code. Follow the existing pool/gate pattern.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/{recipe,render,oracle*,feed,schedule_gen}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/{gate,pinned,registry}.rs`

**Docs touched.**
- `docs/specs/incremental_models.md` — References: the conformance gate's coverage note gains the composed family (timeless).

**Review checklist** (material findings only):
- [ ] All three routes generated; route-3 recipes include redeliveries that stay in-bound.
- [ ] Equivalence asserted after **every** step, not just the end.
- [ ] Admission-rate floor present.
- [ ] Deterministic seed; runtime bounded.

**Commit.** `test(conformance): composed keyed+timeseries recipe family across all three locality routes`

---

## Group W — the web-analytics tracer (flagship fixture + docs, early)

### Phase W0: Extremal-aggregate nullability inference (unblock the tracer)

**Goal.** Fix the pre-existing inference gap that blocked W1: `MIN`/`MAX` (and the extremal family generally) currently infer **nullable unconditionally**, so an extremal-fold `timeseries.partition_column` (`MIN(event_date) AS first_seen_date`) can never satisfy the `timeseries.md` NOT-NULL precondition. Propagate argument nullability through extremal aggregates **in grouped context only**: `MIN(x)`/`MAX(x)` over a provably NOT NULL argument, under a `GROUP BY`, is NOT NULL (every group has at least one row). A global (ungrouped) aggregate over possibly-empty input stays nullable — that is a soundness boundary, not a limitation.

**Pre-conditions.** None (independent of Groups A–S; scheduled here because W1 consumes it).

**TDD tests to write first.**
- `crates/smelt-db/src/type_inference/tests.rs` (or the module's inference unit tests) — `SELECT k, MIN(x) FROM t GROUP BY k` with `x` NOT NULL infers the MIN column NOT NULL; with `x` nullable infers nullable; `SELECT MIN(x) FROM t` (no GROUP BY) infers **nullable** even for NOT NULL `x` (empty-input soundness); same matrix for `MAX`.
- `cargo test -p smelt-db --test nullability_property_tests` — the nullability-soundness oracle stays green (an inferred NOT NULL that DuckDB can make NULL is exactly what this oracle exists to catch; run with the default 256 cases).
- `cargo test -p smelt-db --test type_property_tests` — the type-oracle strictness gate stays green (no new `known_unknowns`/`divergences` entries needed; if one is, it must be reviewed, not blanket-added).
- The W1 repro from the Blocked-phases entry — the staged `events_deduped` + source-YAML shape — now passes `example_diagnostics` cleanly when staged locally (assert as a temporary fixture or unit-level admission test; W1 lands the real fixture).

**Implementation shape.** The blanket seam is `crates/smelt-db/src/type_inference/function_call.rs` (~line 130): registry-backed inference wraps every result `TypedColumn::nullable(dt)`. Thread a nullability rule for the extremal aggregates: when the call is classified aggregate, the query scope is grouped, and every argument column is NOT NULL, produce a non-nullable `TypedColumn`. Prefer expressing the rule as registry data (a per-function nullability-propagation tag in `crates/smelt-types/src/signatures.rs::BuiltinRegistry`) over a name-matched special case, honoring the function-registry single-ownership invariant; scope the tag to `MIN`/`MAX` first (the other extremal/lattice aggregates — `BOOL_AND`/`BOOL_OR`/`BIT_AND`/`BIT_OR` — may adopt it in the same change only if the oracle stays green).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/function_call.rs` — the nullability rule.
- `crates/smelt-types/src/signatures.rs` — the registry nullability-propagation tag (if taken as registry data).
- `crates/smelt-db/src/type_inference/` — grouped-scope detection plumbing only.

**Docs touched.**
- `docs/specs/types.md` (or the type-semantics home) — the aggregate-nullability rule stated (grouped extremal over NOT NULL ⇒ NOT NULL; ungrouped stays nullable), timeless.
- `docs/specs/incremental_models.md` — Known Divergences: the extremal-fold `partition_column` nullability caveat (~lines 1810–1867 per the W1 block entry) narrows.

**Review checklist** (material findings only):
- [ ] Ungrouped aggregates stay nullable — the empty-input case is covered by an explicit test, not an assumption.
- [ ] Nullability-soundness oracle green at default depth; no divergence entries added without review.
- [ ] Rule lives registry-first (or the hand-match site is counted by the migration ratchet honestly) — function-registry single-ownership honored.
- [ ] The W1 repro shape passes diagnostics; W1's Blocked entry's candidate option (a) is thereby discharged.

**Commit.** `fix(types): grouped MIN/MAX over a NOT NULL argument infers NOT NULL — unblocks extremal-fold partition columns`

---

### Phase W1: Composed-shape model in `examples/web_analytics`

**Goal.** Land the flagship composed model in the real web-analytics workspace: `silver/events_deduped.sql` — event-grain dedupe keyed by `event_id`, time-partitioned by `first_seen_date`, over the raw events source's declared `key_recurrence` (the datagen `redelivery:` block already produces the duplicate storms it absorbs). Rewire `events_parsed`'s QUALIFY-dedup consumers to read the composed model, retiring the safety-override workaround where the narrative wants it. This is the tracer: if this model doesn't fall out naturally, Group A got the shape wrong.

**Pre-conditions.** W0 (extremal-aggregate nullability — the flagship model's `partition_column` is an extremal fold); A2–A5 (routes + clocked-source publication; A4 for `key_recurrence`).

**TDD tests to write first.**
- `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/web_analytics` stays diagnostic-clean with the new model.
- `crates/smelt-lsp/tests/example_workspaces.rs` — same via the real LSP backend.
- `examples/web_analytics/tests/` — a `.test.sql` invariant: `events_deduped` has exactly one row per `event_id`, and its row count equals `COUNT(DISTINCT event_id)` of parsed events.
- `crates/smelt-cli/tests/` (e2e, following `web_analytics_backfill.rs`'s pattern) — datagen with `redelivery:` → incremental run windows → `events_deduped` end state equals a full refresh; downstream `sessions` still receives pushdown (clock propagates through the composed stage).

**Implementation shape.** Example + source-YAML work only (`key_recurrence` on the raw events source, window matched to the datagen redelivery profile); no production code. Keep `events_parsed` (the tutorial's refusal narrative still needs the QUALIFY stage) — `events_deduped` sits beside it as the composed-shape alternative consumed by the enrichment/session chain.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/events_deduped.sql` (new) + downstream ref updates.
- `examples/web_analytics/sources/raw/events.yml` — `key_recurrence`.
- `examples/web_analytics/tests/`, `examples/web_analytics/datagen.yaml` (only if the redelivery window needs tuning).

**Docs touched.**
- None (W2 is the docs phase; spec untouched).

**Review checklist** (material findings only):
- [ ] The model is route-establishable as declared (recurrence-bounded or key-determined — assert which via `smelt explain` in the e2e test).
- [ ] Downstream chain still windows (no clock-sink regression).
- [ ] Redelivery equivalence e2e green against real DuckDB.
- [ ] No production-code edits smuggled in.

**Commit.** `feat(examples): web-analytics events_deduped — the composed keyed+timeseries dedupe stage over key_recurrence`

---

### Phase W2: Web-analytics tutorial chapter + composed-shape guide

**Goal.** Make the tracer teachable: a new tutorial page (dedupe done right — from the QUALIFY workaround to the composed shape), a new `tutorial_stages/06_composed_dedupe/` staged workspace, and the docs-site guide section for the composed shape ("partitioned or keyed is a category error" framing, the three routes, `key_recurrence`, the settle bound).

**Pre-conditions.** W1.

**TDD tests to write first.**
- `crates/smelt-cli/tests/tutorial_freshness.rs::web_analytics_tutorial_pages_are_fresh` — extended to the new page; every `smelt-generate` block re-derives byte-identically against the compiled binary.
- `crates/smelt-cli/tests/example_diagnostics.rs` — the new staged workspace is clean (or asserts its intentional pre-fix diagnostic, per the stage narrative).
- Docs-site build (`docs-site/` CI) passes with the new nav entries.

**Implementation shape.** New template `examples/web_analytics/tutorial_pages/deduplication.md` (generated into `docs-site/docs/examples/web-analytics/`), staged workspace under `tutorial_stages/06_composed_dedupe/`, `python3 generate_tutorial.py` to regenerate, guide section under `docs-site/docs/guide/` (incremental-models page). The narrative arc: stage 02's refusal → stage 03's override → the composed shape that makes the workaround unnecessary.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/tutorial_pages/`, `examples/web_analytics/tutorial_stages/06_composed_dedupe/`, `examples/web_analytics/generate_tutorial.py` (page list only).
- `docs-site/docs/examples/web-analytics/` (generated), `docs-site/docs/guide/`, `docs-site/mkdocs.yml` (nav).
- `crates/smelt-cli/tests/tutorial_freshness.rs` — page registration.

**Docs touched.**
- `docs/specs/incremental_models.md` — References → User docs: point at the new guide/tutorial pages.
- All tutorial/guide prose timeless — the composed shape has always existed; the orthogonality framing ("declaring identity is load-bearing, not a dedup footnote") appears in user vocabulary.

**Review checklist** (material findings only):
- [ ] Every SQL block is backed by a compiling staged fixture; freshness gate green.
- [ ] The guide teaches facts-first vocabulary compatible with S1's later surface (declaring `timeseries:` + key; `grain:` presented as the shape's name, not the driver).
- [ ] No phase vocabulary anywhere in user docs.
- [ ] Spec References updated.

**Commit.** `docs(tutorial): web-analytics composed-shape dedupe chapter + guide for keyed+timeseries models`

---

## Group S — facts-as-surface (the Relation Contract lands)

### Phase S1: Facts-as-surface — top-level `unique_key:`, grain as derived label + check-only assertion

**Goal.** Land the grain demotion (models.md §"Refresh axis"): the declared surface becomes the shape-defining facts — top-level `unique_key:` parses in `.sql` frontmatter and `smelt.yml` model overrides; `refresh: incremental` is admitted on the facts alone (clock and/or identity; neither ⇒ the "no shape-defining fact declared" hard error); a written `grain:` becomes an optional **check-only assertion** — derived from `(clock?, identity?, partition_column ∈ key?)`, error on mismatch, drives nothing.

**Pre-conditions.** W2 (the tutorial teaches facts-first vocabulary this phase makes literal). Groups A/W landed on the `grain:`-driven surface; this phase re-bases the driver without changing any derived plan.

**TDD tests to write first.**
- `crates/smelt-core/tests/refresh_axis.rs::top_level_unique_key_parses` — `unique_key: [order_id]` (list and single-string forms) parses onto `ModelMetadata`; same via `smelt.yml` model override; frontmatter wins when both set it.
- `crates/smelt-core/tests/refresh_axis.rs::incremental_admitted_on_facts_alone` — `refresh: incremental` + `unique_key:` and no `grain:` derives the key shape; `refresh: incremental` + `timeseries:` and no `grain:` derives the partition shape; `refresh: incremental` with neither fact is the models.md §"Constraint violations" hard error naming the missing facts.
- `crates/smelt-core/tests/refresh_axis.rs::grain_assertion_is_check_only` — `grain: partition` on a model whose facts derive `key` errors naming both labels; `grain: key` on a facts-derived key shape passes; the derived label for clock + identity + `partition_column ∈ key` is `key_per_partition` (still refused by A0's plan-level refusal — assert the two diagnostics compose, not collide).
- `crates/smelt-logical/tests/` (classifier) — a declared `unique_key` that disagrees with the keyed classifier's GROUP-BY-derived key errors naming both lists; agreement passes and the plan carries the declared key.
- Real fixture: every existing `examples/` workspace declaring `grain:` stays diagnostic-clean (`crates/smelt-cli/tests/example_diagnostics.rs` unchanged in expectations — the assertion passes everywhere); `examples/web_analytics/models/silver/events_deduped.sql` gains the declared `unique_key:` spelling and stays clean.

**Implementation shape.** `ModelMetadata` gains `unique_key: Option<Vec<String>>` (`crates/smelt-core/src/metadata.rs`) with the `smelt.yml` merge in the existing precedence order. A pure `derive_grain(clock: bool, identity: Option<&[String]>, partition_col: Option<&str>) -> Grain` beside the `Grain` enum (`crates/smelt-core/src/config.rs`); frontmatter validation calls it, compares any written `grain:` (mismatch ⇒ new `MetadataError` variant — the exhaustiveness gate forces the `smelt-db` mapping arm), and downstream consumers (`crates/smelt-db/src/queries/maintenance.rs` grain mapping) read the **derived** label. The `batched:` sub-block still carries `unique_key`/`safety_overrides` for partition-grain models — migrating/retiring it is **out of scope**; record under "Deferred during implementation".

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/metadata.rs` — `unique_key:` field, derivation call, the mismatch `MetadataError`.
- `crates/smelt-core/src/config.rs` — `derive_grain`, `Grain` docs.
- `crates/smelt-db/src/queries/maintenance.rs`, `crates/smelt-db/src/lib.rs` — consume the derived label; diagnostic mapping arm.
- `examples/web_analytics/models/silver/events_deduped.sql` — declared-`unique_key` spelling.

**Docs touched.**
- `docs/specs/models.md` — Known Divergences: the grain-demotion divergence narrows (facts admit, label derived, assertion checked; `batched:` sub-block migration still open — behavioural wording).
- `docs/specs/incremental_models.md` — Known Divergences: first bullet ("grain-demotion … spec-ahead-of-code") narrows to the surviving gaps (`write:` pin, registry — Group R).
- `docs-site/docs/reference/smelt-yml.md` — `unique_key:` documented as the identity fact; `grain:` documented as an optional assertion.
- `docs-site/docs/guide/` (incremental guide) — surface examples lead with the facts; `grain:` shown as the shape's name (timeless).

**Review checklist** (material findings only):
- [ ] No derived plan changes for any existing fixture — the phase moves the driver, not the shapes (assert via unchanged `smelt explain` output on a pinned fixture).
- [ ] Neither-fact error and grain-mismatch error both name the facts, fail-loud, and route through the `MetadataError` exhaustiveness gate.
- [ ] Declared-vs-GROUP-BY key mismatch is an error, never a silent preference for either list.
- [ ] `batched:` sub-block untouched; deferral recorded.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(models): shape facts are the declared surface — top-level unique_key:, derived grain label, check-only grain: assertion`

---

### Phase S2: Relation Contract read-side — derived grain for sources; explain prints both providers

**Goal.** Compute the derived grain label for **sources** exactly as for model outputs (models.md §"The Relation Contract": the derivation `(clock?, identity?, partition_column ∈ key?)` is provider-independent), and make `smelt explain <model>` print the contract: the model's own slots (clock, identity, derived grain) and, per inbound edge, the provider's filled slots — source or upstream model, uniformly. No new declared surface.

**Pre-conditions.** S1 (the derivation exists as a pure function).

**TDD tests to write first.**
- `crates/smelt-core/tests/source_world_facts.rs::source_derived_grain` — a source with `timeseries:` and no `unique_key` derives the clocked-fact label; `unique_key` and no clock derives keyed-dimension; both with `partition_column ∈ key` derives the trajectory label; neither derives the unclassified label (reported, never an error — a source without shape facts is legal).
- `crates/smelt-cli/tests/explain_model.rs::explain_prints_relation_contract` — for a web-analytics model, the report shows the model's clock/identity/derived-grain rows and one contract block per inbound edge; a source edge and a model edge render through the same rows (assert the shared field names appear for both).
- `crates/smelt-cli/tests/explain_model.rs` (JSON leg) — `smelt explain --json` carries the contract slots for both providers with identical field paths (clock/identity), per §"The Relation Contract".
- Real fixture: `examples/web_analytics` — `events_deduped`'s explain output shows its own derived `key` (time-partitioned) label and the raw events source's clocked-fact label on the inbound edge.

**Implementation shape.** Reuse S1's `derive_grain` over `SourceInfo` (`crates/smelt-core/src/sources.rs` already carries `timeseries`/`unique_key`); a small provider-agnostic `RelationContractView` assembled where the report already resolves upstream facts (`crates/smelt-cli/src/explain.rs::build_maintenance_plan_report`), rendered as new report rows and JSON fields. No Salsa changes — the report path is non-Salsa.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/sources.rs` — derived-grain accessor on `SourceInfo`.
- `crates/smelt-cli/src/explain.rs` — `RelationContractView` + report/JSON rows.
- `crates/smelt-cli/src/commands/explain.rs` — plumbing only.

**Docs touched.**
- `docs/specs/models.md` / `docs/specs/sources.md` — Known Divergences: the "derived grain surfaced for sources as well as models" gap narrows (behavioural).
- `docs-site/docs/reference/cli.md` — `smelt explain`'s contract block documented (timeless).

**Review checklist** (material findings only):
- [ ] One derivation, two providers — no source-specific reimplementation of the grain rule.
- [ ] Shared slots (clock, identity) render with identical field paths for source and model providers; provider-only slots (e.g. `mutation_profile`) stay provider-labelled.
- [ ] A shape-fact-free source is reported, not refused.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(explain): relation contract read-side — derived grain for sources, contract slots for both providers`

---

## Group B — the graph layer admits composed nodes

### Phase B1: Graph admissibility for locality-admitted composed nodes

**Goal.** Refine the graph layer's keyed refusal (spec §"The graph layer"): a **locality-admitted composed node** is a clocked node contributing edges at its declared `timeseries.granularity` like any other; a **bare** keyed node (no admitted time axis) still refuses `MaintenanceGraphUnsupportedNode`, with the message refined to name the missing time axis and the composed alternative.

**Pre-conditions.** A5 (locality verdict + published output clock available to the graph builder); W1 (the real composed fixture exists).

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs::composed_node_contributes_edges` — a chain `source → composed(keyed+timeseries) → partition-grain` builds edges through the composed node at its declared granularity; `forward(backward(P)) ⊇ P` holds across the chain (extend `assert_forward_backward_containment`).
- `crates/smelt-logical/tests/` (propagate unit) — `refuse_keyed_nodes` no longer fires for a node carrying an admitted output clock; a bare keyed node still refuses, and the message names "without an admitted time axis" plus the `timeseries:`+locality fix.
- `crates/smelt-runtime/tests/since_upstream_propagation.rs::composed_node_in_the_chain` — `build_forward_graph` over a workspace containing a composed model yields edges into **and** out of it; the bare-keyed workspace case still errors.
- Real fixture: `examples/web_analytics` — the graph builds with `events_deduped` mid-chain (no `MaintenanceGraphUnsupportedNode`), asserted via the propagation planner over the real workspace.

**Implementation shape.** Thread the locality verdict into graph construction: `crates/smelt-runtime/src/propagation.rs::build_forward_graph` (~`:181`) classifies a keyed model with an admitted output clock as `PartitionGrain`-bearing (its declared output granularity) instead of `PartitionGrain::Keyed`; `crates/smelt-logical/src/maintenance/propagate.rs::refuse_keyed_nodes` (~`:184`) keeps refusing only nodes still classified keyed. Edge margins for the composed node's **outbound** edges are placeholder-exact in this phase (its own dirt projection is B2); inbound edges use the ordinary clamp derivation.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/propagation.rs` — node classification from the locality verdict.
- `crates/smelt-logical/src/maintenance/propagate.rs` — refined refusal + message.
- `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs`, `crates/smelt-runtime/tests/since_upstream_propagation.rs` — extended spines.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: the "no keyed node is propagation-admissible" leg of the composed-shape divergence narrows (composed nodes admitted; bare keyed still refuses — behavioural).

**Review checklist** (material findings only):
- [ ] Admission keys off the **locality verdict**, never off the mere presence of a `timeseries:` block.
- [ ] Bare keyed refusal preserved with the refined message (fail-loud, names the fix).
- [ ] Adjointness law green over the composed chain.
- [ ] No dirt-projection logic smuggled in (B2's scope).

**Commit.** `feat(graph): admit locality-established composed nodes as clocked propagation participants; refine the bare-keyed refusal`

---

### Phase B2: Key→partition dirt projection through composed nodes

**Goal.** Give a composed node its outbound dirt semantics (spec §"What the composed shape uniquely enables"): forward propagation projects what a run changed to partition intervals — **exact** under routes 1–2 (the keys' own partitions), **widened backward by `r` plus the derived margins** under route 3 — and backward resolution applies the same projection in reverse. Widen-never-narrow; the widening lives in the projection, not the edge clamp.

**Pre-conditions.** B1.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs::composed_projection_adjoint` — the adjointness law `forward(backward(P)) ⊇ P` extended over composed nodes for all three routes (route-parameterised cases; route 3 asserts the `r`-widened containment, not equality).
- `crates/smelt-logical/tests/` (propagate unit) — an upstream delta `[a, b)` through a route-1/2 composed node dirties downstream exactly the projected partitions (no widening); through a route-3 node dirties `[a − r − margins, b)`-derived intervals; narrowing is impossible by construction (assert a property: projected ⊇ exact).
- `crates/smelt-logical/tests/` — backward: a requested downstream period resolves through the composed node to the upstream slices its reach requires, route-aware.
- Real fixture: `examples/web_analytics` — a one-day raw-events delta propagates through `events_deduped` to `sessions` as exactly the projected day set (assert the planned regions, not just non-refusal).

**Implementation shape.** A route-aware projection function in `crates/smelt-logical/src/maintenance/propagate.rs` (pure, beside `propagate`/`required_inputs`), consuming the node's locality verdict (route + `r` + margins) carried on the `Edge`/node metadata from B1; `propagate` and `required_inputs` call it when crossing a composed node. No key-level dirt representation anywhere — intervals in, intervals out (spec: "no keyed dirt-sets").

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/propagate.rs` — the projection + its call sites in `propagate`/`required_inputs`.
- `crates/smelt-runtime/src/propagation.rs` — carry route/`r`/margins onto the graph's node metadata.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: the "exact key→partition dirt projection" leg narrows (routes 1–2 exact, route 3 widened — behavioural).

**Review checklist** (material findings only):
- [ ] Routes 1–2 project exactly; route 3 widens backward by `r` + margins — asserted separately.
- [ ] Widen-never-narrow holds as a property, not just on examples.
- [ ] No key-level dirt structure introduced; the graph stays interval-typed.
- [ ] Adjointness spine green for all three routes.

**Commit.** `feat(graph): route-aware key→partition dirt projection through composed nodes, forward and backward`

---

### Phase B3: `--since-upstream` with a composed node as the delta origin

**Goal.** Close the CLI loop: `smelt run --since-upstream --source <address> --landed <start>..<end>` accepts a **composed model** as the delta origin (its landed delta stated on its own declared output axis), and `smelt build --include-upstreams` resolves required slices through a composed ancestor — so the composed stage sits inside a propagation chain end to end, operator-visible.

**Pre-conditions.** B2.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/since_upstream_propagation.rs::composed_model_as_source` — `plan_since_upstream` with a composed model in `--source` reflects the supplied intervals through its outbound edges (route-aware, per B2) and prints the dirty set; a bare keyed `--source` still errors.
- `crates/smelt-cli/tests/` (CLI unit) — `--source smelt.silver.events_deduped --landed 2026-03-01..2026-03-03` parses and pairs (reusing `parse_landed_range`/`pair_source_deltas`); an address that resolves to neither a source nor a maintained model errors naming the address.
- `crates/smelt-runtime/tests/` — `resolve_build_plan` for a downstream period walks through the composed ancestor and lists its required slices + build order (composed node before its consumers, after its own upstreams).
- Real fixture (e2e, DuckDB): `examples/web_analytics` — land a one-day raw-events delta, run `--since-upstream` through `events_deduped` to `sessions`/marts; assert **exactly** the projected regions ran (planned-region list and written partitions), and a second identical invocation with an empty `--landed` set runs nothing.

**Implementation shape.** `run_since_upstream` (`crates/smelt-cli/src/commands/run.rs`) resolves `--source` addresses against models with plans, not just declared sources — the resolution seam `plan_since_upstream` (`crates/smelt-runtime/src/propagation.rs:385`) already distinguishes origin kinds for model edges; extend it to accept composed origins (the origin model itself is never re-run — its landed delta is the window a completed run wrote). `--landed` grammar unchanged (`explain.rs:695` mirror stays in sync).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/commands/run.rs`, `crates/smelt-cli/src/main.rs` — origin resolution + help text.
- `crates/smelt-runtime/src/propagation.rs` — composed-origin acceptance in `plan_since_upstream`/`resolve_build_plan`.

**Docs touched.**
- `docs-site/docs/reference/cli.md` — forward propagation: a `--source` may name any clocked provider, a declared source or a maintained model with a time axis (timeless).
- `docs/specs/incremental_models.md` — Known Divergences: the composed-shape propagation divergence closes to its residue (whatever remains after B1–B3 — behavioural).

**Review checklist** (material findings only):
- [ ] The origin model is never re-run; only downstream projected regions execute.
- [ ] Bare keyed origin still refuses with the B1 message.
- [ ] The e2e asserts the exact region set, not just success.
- [ ] Docs describe providers, not phases; CLI help and reference agree.

**Commit.** `feat(cli): --since-upstream accepts a composed model as delta origin; backward resolution through composed ancestors`

---

## Group C — change-suppressed writes (M1)

### Phase C1: Spec diff — T1/T2 transform variants + capability flags

**Goal.** The spec-first gate for the group: catalogue T1 (change-suppressed MERGE) and T2 (staged-candidate conditional DELETE+INSERT) as *variants* in `model_transforms.md` — a property licenses them, never chooses — and bring `multi_backend.md`'s capability matrix up to date with the conditional-write flags, fixing the pre-existing drift that `supports_column_scoped_merge` lives as a `Backend` trait method rather than in the capability struct.

**Pre-conditions.** None (may run in parallel with Group A; C2 blocks on it).

**TDD tests to write first.** None — spec-diff phase; the group's later phases carry the tests.

**Implementation shape.** In `model_transforms.md`: T1 as a variant of the column-scoped MERGE and keyed-fold MERGE entries — the matched arm gains an `IS DISTINCT FROM` predicate over the cell's mutation-sensitive **comparable** columns (comparing only that group is sound because the other groups are proven insensitive — cite `incremental_models.md` §"Windowed maintenance and the horizon" category 2); note the `WHEN NOT MATCHED BY SOURCE` vs scoped-DELETE dialect split. T2 as the merge-less realisation: stage candidates into a temp relation, then a conditional DELETE+INSERT reads it, one transaction. Both entries state the licence (P3 comparability on every compared column, P2 row identity) and the fixed-`S` bit-equality obligation. In `multi_backend.md`: add `supports_column_scoped_merge` (currently a `Backend` trait method at `crates/smelt-backend/src/lib.rs:324`, absent from `BackendCapabilities` at `crates/smelt-dialect/src/dialect.rs:29`) and the conditional-write flags to the capability matrix, specifying the target state — flags live in the struct, queried by admission (the migration itself is R1). Record the code-lags-spec state in each spec's Known Divergences.

**Critical files (allowed to touch in this phase).**
- `docs/specs/model_transforms.md` — T1/T2 variant entries.
- `docs/specs/multi_backend.md` — capability matrix + Known Divergences.
- `docs/specs/incremental_models.md` — cross-references from the pruning taxonomy to the now-named variants.

**Review checklist** (material findings only):
- [ ] T1/T2 are catalogued as licensed variants, not chosen modes (validator-not-chooser preserved).
- [ ] The comparability licence and the fixed-`S` bit-equality obligation are stated on the variants themselves.
- [ ] The dialect split (`WHEN NOT MATCHED BY SOURCE` vs scoped DELETE) is recorded where the emitters will read it.
- [ ] Capability-matrix drift entry names the trait method and the target struct field.
- [ ] All edits timeless — no phase vocabulary, gaps recorded behaviourally in Known Divergences.

**Commit.** `spec(transforms): change-suppressed MERGE and staged-candidate conditional DELETE+INSERT as licensed variants; capability-matrix conditional-write flags`

---

### Phase C2: P3 change-comparability per column (walk lattice fold)

**Goal.** Derive, per output column, whether its value is a pure function of the processed inputs — and therefore *comparable* across runs for suppression purposes. A per-column lattice fold in the property walk: `Comparable` ⊑ `Incomparable`, fail-closed. `contract: plausible` columns and run-pinned `NOW()` are Incomparable (comparable *within* a run, not *across* runs); any unrecognised construct is Incomparable.

**Pre-conditions.** C1 (the licence this proof discharges is spec'd).

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/walk.rs` unit tests — a plain aggregate column folds `Comparable`; a column containing `NOW()`/`RANDOM()` folds `Incomparable`; an unrecognised opaque expression folds `Incomparable` (fail-closed, never a default `Comparable`).
- `crates/smelt-logical/tests/` — a `columns.<c>.contract: plausible` declaration forces that column `Incomparable` regardless of its SQL; a pinned-`NOW()` column (run-determinism in the existing `Determinism` lattice) is `Incomparable` even though it is deterministic within a run.
- Composition cases — comparability folds through CTEs, set operations, and joins via the walk's operator rule (union = lub); a column comparable in one arm and incomparable in the other folds `Incomparable`.
- `cargo test -p smelt-logical --test walk_coverage` — the new fold's consumption sites carry the classified doc comments; the gate stays green.

**Implementation shape.** Extend `PropertyVector` (`crates/smelt-logical/src/analysis/walk.rs:1578`) with a per-column comparability field and extend `PropertyTransfer`'s leaf/operator impls, modelled directly on the `Determinism` per-column lattice (which already classifies `NOW()`/`RANDOM()` — comparability's leaf rule can consume the determinism verdict: `Clean` ⇒ comparable, `Run`/`Row` ⇒ incomparable). The `plausible` override applies where the vector is assembled against model metadata, not inside the walk. No consumer yet — C4 wires admission.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/walk.rs` — `PropertyVector` field + transfer rules.
- `crates/smelt-logical/src/maintenance/mod.rs` — carry the verdict on the plan-facing types (plumbing only).

**Docs touched.**
- `docs/specs/model_properties.md` — the derived-proofs table: the change-comparability row moves from `not-yet` to built (behavioural description, timeless).

**Review checklist** (material findings only):
- [ ] Fail-closed: every unhandled leaf/operator case lands `Incomparable`, asserted by a test.
- [ ] Pinned-`NOW()` is Incomparable — the within-run/across-run distinction is honoured.
- [ ] The verdict is walk-composed — no raw SQL text scan (walk_coverage green, doc comments classified).
- [ ] No admission or emitter consumes the verdict yet (no scope creep into C4).

**Commit.** `feat(properties): per-column change-comparability lattice fold in the property walk`

---

### Phase C3: P2 region row identity

**Goal.** Derive, per cell, the identity the suppression compare joins stored rows to candidate rows on: the declared `unique_key` where present; else the walk's proven grain key; else `WholeRow` multiset semantics. Derived, fail-closed — a cell whose identity cannot be established gets `WholeRow`, never a guessed key.

**Pre-conditions.** C2 (shares the plan-facing plumbing); A1 (real `unique_key` threaded into the plan).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` — a model with a declared `unique_key` derives `RowIdentity::Key(declared)`; a keyed model whose classifier proves the GROUP BY key derives `RowIdentity::Key(proven)` with declared taking precedence on both-present; a keyless partition-grain model derives `RowIdentity::WholeRow`.
- `crates/smelt-logical/tests/` — a proven grain key that does not cover the output (fan-out join, `has_fan_out_join` set in the `PropertyVector`) falls back to `WholeRow`, not the partial key (fail-closed).
- `crates/smelt-cli/tests/explain_model.rs` — `smelt explain` prints the cell's row identity alongside its technique.

**Implementation shape.** A pure derivation in `crates/smelt-logical/src/maintenance/` (new `row_identity` fn beside the cell-derivation in `derive.rs`), consuming the declared key off `OutputSpec`/`Grain` and the proven `KeySet` off the walk's `PropertyVector.grain`. Attach the verdict to `PlanCell` so C4/C5's emitters and admission read it as plain data.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/derive.rs`, `.../mod.rs` — the derivation + `PlanCell` field.
- `crates/smelt-cli/src/explain.rs` — report row.

**Docs touched.**
- `docs/specs/model_properties.md` — the region-row-identity proof described (derived precedence: declared → proven → `WholeRow`), timeless.

**Review checklist** (material findings only):
- [ ] Precedence is declared → proven → `WholeRow`; both-present mismatch is surfaced, not silently resolved.
- [ ] Fan-out / uncovering keys fall back to `WholeRow` (fail-closed), asserted.
- [ ] The verdict is plan data (pure derivation), not re-derived by any consumer.
- [ ] No emitter change yet (no scope creep into C4/C5).

**Commit.** `feat(maintenance): per-cell region row identity — declared key, proven grain key, or WholeRow`

---

### Phase C4: T1 on the column-scoped MERGE

**Goal.** The single cheapest high-value change in the plan: `emit_column_scoped_merge` gains the matched-arm suppression predicate — `AND (t.c1 IS DISTINCT FROM s.c1 OR …)` over the cell's comparable mutation-sensitive columns — so an unchanged dimension re-run writes zero rows. Admission is fail-closed: any Incomparable column in the compared group refuses the conditional variant and keeps the unconditional one. Directly mitigates the recorded "dispatch fires on every run unconditionally" divergence.

**Pre-conditions.** C1–C3.

**TDD tests to write first.**
- `crates/smelt-logical/src/maintenance/emit.rs` unit tests — the conditional variant's matched arm carries `IS DISTINCT FROM` over exactly the compared column set; the unconditional variant's text is byte-unchanged from today.
- `crates/smelt-logical/tests/` (admission) — a cell whose mutation-sensitive group contains one Incomparable column (P3) refuses the conditional variant with a named-column reason and admits the unconditional one; a fully comparable group admits both as interchangeable.
- `crates/smelt-runtime/tests/statement_parity.rs` — new leg: a run that dispatches the suppressed column-scoped MERGE executes byte-identical text to a direct emitter call over the same inputs.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — equivalence leg: suppressed and unconditional variants produce bit-identical state at fixed `S` over the pool (the interchangeability obligation from C1's spec diff).
- `crates/smelt-runtime/tests/technique_lowering.rs` — e2e on the existing fact+dimension fixture: first run writes; an unchanged-input re-run writes **zero rows** (assert via a row-count/`changes()` probe against real DuckDB), and state equals a full refresh.

**Implementation shape.** `emit_column_scoped_merge` (`crates/smelt-logical/src/maintenance/emit.rs:163`) takes the compared column list (from the cell's P3 verdicts × mutation-sensitive group) and emits the predicate; the choice layer (`choice.rs::resolve_cell_choice`) offers conditional/unconditional as interchangeable techniques for a fully comparable group. Suppression compares only — it never changes what is evaluated: the delta SELECT and its scan are untouched.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs` — the predicate.
- `crates/smelt-logical/src/maintenance/choice.rs` — conditional-variant admission.
- `crates/smelt-runtime/src/maintenance_driver.rs` — dispatch plumbing (no new statement text).

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: the "every emitted MERGE writes all matched rows unconditionally" clause of the no-conditional-technique entry narrows (behavioural).

**Review checklist** (material findings only):
- [ ] Suppression never skips **evaluating** a scanned input — the delta SELECT/scan is byte-unchanged; only the write is conditional (compute restriction is Group E's licence, not this phase's).
- [ ] Fail-closed admission: one Incomparable column refuses the conditional variant, named in the reason.
- [ ] Fixed-`S` bit-equality proven by the conformance leg, not asserted by hand.
- [ ] Emitter remains the single author; statement-parity leg added.
- [ ] Zero-write re-run proven against real DuckDB, not a mock.

**Commit.** `feat(maintenance): change-suppressed column-scoped MERGE — unchanged inputs write zero rows`

---

### Phase C5: T1 on the keyed fold; T2 staged-candidate conditional DELETE+INSERT

**Goal.** Extend suppression to the keyed-fold MERGE, and build the genuinely new machinery: the **staged-candidate statement group** — a temp relation of computed candidates, then a conditional DELETE+INSERT that touches only rows whose applied effect is not the identity, one transaction, cleanup guaranteed. This is T2, the merge-less realisation — and the first keyed-shaped path for backends without MERGE (Spark-over-Parquet).

**Pre-conditions.** C4 (predicate machinery, admission); A2 (keyed-fold emitter shape current).

**TDD tests to write first.**
- `crates/smelt-logical/src/maintenance/emit.rs` unit tests — `emit_keyed_fold`'s conditional variant carries the suppression arm over the comparable fold columns; the unconditional text is unchanged.
- `crates/smelt-logical/src/maintenance/emit.rs` unit tests — the staged-candidate group emits: temp-relation `CREATE`, candidate `INSERT`, conditional `DELETE`+`INSERT` reading the staged relation with the `IS DISTINCT FROM` restriction, `DROP` — ordered, flagged one-transaction.
- `crates/smelt-runtime/tests/statement_parity.rs` — legs for both the suppressed keyed fold and the staged-candidate group (executed == emitted); the structural no-authoring scan extended to the staged shapes (temp-relation `CREATE`/`DROP` outside the emitter module is a failure).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — the staged-candidate variant equals the MERGE variant and the full-refresh oracle at fixed `S` over the keyed pool.
- Runtime e2e (DuckDB): a keyed model re-run over unchanged input writes zero rows under both realisations; an interrupted staged run leaves no temp relation behind (transaction rollback covers the group).

**Implementation shape.** `emit_keyed_fold` (`crates/smelt-logical/src/maintenance/emit.rs:222`) gains the same compared-column predicate as C4. `StatementGroup` (`emit.rs:45`) grows a staged-relation concept — a named temp relation plus dependent statements, transactional as a unit — rather than a flat statement list; `Backend::execute_statement_group` executes it unchanged in shape (backends still never author). Spark's merge-less path selects T2 via the capability check (flag from C1's spec'd matrix; struct migration itself is R1 — read whichever surface is live).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs` — keyed-fold predicate; `StatementGroup` staged-relation concept; the T2 emitter.
- `crates/smelt-logical/src/maintenance/choice.rs` — T2 as an admissible realisation where MERGE is unavailable.
- `crates/smelt-backend/src/lib.rs`, `crates/smelt-backend-duckdb/`, `crates/smelt-backend-spark/` — group execution (no authored text).
- `crates/smelt-runtime/tests/statement_parity.rs` — legs + gate extension.

**Docs touched.**
- `docs/specs/incremental_models.md` / `docs/specs/model_transforms.md` — Known Divergences: T1 keyed-fold and T2 staged-candidate built (behavioural).

**Review checklist** (material findings only):
- [ ] The staged group is one transaction: a mid-group failure leaves target and temp namespace untouched (asserted).
- [ ] Backends execute the group; zero authored statement text outside the emitter (structural gate extended and green).
- [ ] T2 admitted only where the capability check says MERGE is unavailable or the pin asks for it — never a silent substitution.
- [ ] Conformance proves T2 ≡ T1 ≡ full refresh at fixed `S`.
- [ ] Spark leg exercised (or explicitly gated behind the Spark-parity job with the gate named).

**Commit.** `feat(maintenance): change-suppressed keyed fold + staged-candidate conditional DELETE+INSERT statement group`

---

### Phase C6: Slice-bounded compare on composed models

**Goal.** Compose suppression with locality: on a composed (key + time) output, the suppression compare's read of stored state carries A2's slice predicate, so compare cost is proportional to the slice, not the key space — the third capability of §"What the composed shape uniquely enables", and what makes suppression affordable at volume.

**Pre-conditions.** C4–C5; A2 (slice-pruned target scan); W1 (the flagship fixture exists).

**TDD tests to write first.**
- `crates/smelt-logical/src/maintenance/emit.rs` unit tests — a composed model's suppressed MERGE carries **both** predicates (slice on the target read, `IS DISTINCT FROM` on the matched arm); a bare keyed model's suppressed MERGE carries only the suppression arm (no invented slice).
- `crates/smelt-runtime/tests/statement_parity.rs` — parity leg for the doubly-predicated statement.
- e2e on `examples/web_analytics` (`events_deduped`): a redelivery-storm re-run (datagen `redelivery:` duplicates, unchanged payloads) writes **zero rows**, and the emitted SQL's target read is slice-bounded (assert both predicates in the recorded statement); end state equals a full refresh.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — the composed pool (A6) runs with suppression enabled and stays equivalent under redelivery schedules.

**Implementation shape.** Threading, not new machinery: the slice predicate A2 already passes to the emitters applies to the conditional variants' target read unchanged. The phase's substance is proving the composition — emitted-SQL assertions plus the storm e2e.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs` — predicate composition (if any gap).
- `crates/smelt-runtime/src/cumulative.rs` — threading (if any gap).
- Test files named above.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: the slice-bounded-compare bullet of the composed-shape capabilities is built (behavioural).

**Review checklist** (material findings only):
- [ ] Both predicates present on composed; no slice invented for bare keyed (fail-closed inheritance from A2).
- [ ] The storm e2e measures writes (zero) — not just end-state equality.
- [ ] Compare read is provably slice-bounded in the emitted SQL, not inferred from timing.
- [ ] Conformance composed pool green with suppression on.

**Commit.** `feat(maintenance): slice-bounded suppression compare on composed keyed+timeseries outputs`

---

### Phase C7: Docs — conditional writes

**Goal.** Make conditional maintenance teachable: what write suppression is, what it costs and saves, when it is refused (`plausible` columns, non-derivable comparability), how `prefer`/`technique` steer between proven-interchangeable variants, and how `smelt explain` reports a cell's conditional admission.

**Pre-conditions.** C4–C6 landed (the surface being documented exists).

**TDD tests to write first.**
- `crates/smelt-cli/tests/example_diagnostics.rs` — any new/changed example fixtures backing doc code blocks stay clean.
- Docs-site build passes with the new sections; `crates/smelt-cli/tests/tutorial_freshness.rs` stays green if any tutorial page gains a generated block.

**Implementation shape.** A conditional-writes section in the incremental-models guide (`docs-site/docs/guide/`), the `smelt explain` reference rows for conditional admission/refusal, and the `maintenance:` steering documentation (`prefer`/`technique`) in the smelt-yml reference. Refusal reasons documented in user vocabulary (a `plausible` column makes its group incomparable — and why that is a feature).

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/guide/` — the conditional-writes section.
- `docs-site/docs/reference/cli.md`, `docs-site/docs/reference/smelt-yml.md` — explain + steering surface.
- `docs/specs/incremental_models.md` — Known Divergences: the "No conditional maintenance technique exists" entry narrows to the genuinely remaining gaps (observed deltas — Group D; compute restriction — Group E).

**Review checklist** (material findings only):
- [ ] Every claimed behaviour exists and is gated by a named test; every SQL block backed by a compiling fixture.
- [ ] Refusal cases documented as prominently as the happy path (fail-closed is user-visible surface).
- [ ] Timeless prose — no phase vocabulary, no "new in" framing.
- [ ] Spec divergence entry names what remains, behaviourally.

**Commit.** `docs(incremental): conditional write suppression — guide, explain reference, steering`

---

## Group R — per-cell write addressing (open registry + write: pin)

### Phase R1: Open write-pattern registry + `maintenance.cells[].write` pin

**Goal.** Replace the closed technique admission with the registry the spec describes: each write pattern declares the contract facts it requires (identity? partition axis?), its equivalence obligation, and its backend-capability key; the available-addressings rule (facts × trigger × invariant × capability) computes a cell's admissible set. The `maintenance.cells[].write` frontmatter pin parses and validates against that set — an unrecognised or backend-unavailable name is `MaintenanceWritePatternUnavailable`, an equivalence-violating pin is `MaintenanceWriteAddressingRefused`, never a silent downgrade. `supports_column_scoped_merge` migrates from `Backend` trait method into `BackendCapabilities` (implementing C1's spec'd target state).

**Pre-conditions.** C4–C5 (the conditional variants exist and register alongside the original patterns); C1 (capability matrix spec'd).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (registry) — every existing pattern (region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild) plus the conditional variants resolves in the registry with its declared required facts; a keyed pattern is not admissible for a cell with no identity; a region pattern is not admissible with no partition axis (the available-addressings rule, per case).
- `crates/smelt-core/tests/` (frontmatter) — `maintenance.cells[].write: <name>` parses as an open string, not an enum; a malformed cells entry is a metadata error.
- `crates/smelt-logical/tests/` / `crates/smelt-db/tests/` — a pin naming an unrecognised pattern, or one the target backend's capabilities do not provide, produces `MaintenanceWritePatternUnavailable` naming the pattern and backend; a pin naming an admissible-by-capability but equivalence-violating addressing (e.g. `keyed` on an identity-free output) produces `MaintenanceWriteAddressingRefused` naming the cell; neither ever downgrades silently (assert the plan contains no substituted technique).
- `crates/smelt-runtime/tests/technique_lowering.rs` — a valid pin selects among admissible mechanisms end-to-end (e.g. pinning `region` on a backfill cell of a composed model yields DELETE+INSERT); capability gating via the struct field now, with the old trait-method call sites gone (compile-time: method deleted).
- `crates/smelt-cli/tests/explain_model.rs` — `smelt explain` prints each cell's admissible pattern set and any active pin.

**Implementation shape.** A registry table in `crates/smelt-logical/src/maintenance/` (pattern name → required facts, obligation tag, capability key) consumed by `choice.rs::resolve_cell_choice`; `Technique` stays as the executable lowering of registered patterns (no behaviour change to existing plans with no pin). `MaintenanceConfig` cells gain `write: Option<String>` in `crates/smelt-core/src/metadata.rs` (MetadataError exhaustiveness gate forces the new diagnostic arms). `BackendCapabilities` (`crates/smelt-dialect/src/dialect.rs:29`) gains `supports_column_scoped_merge` + conditional-write flags; the `Backend` trait method (`crates/smelt-backend/src/lib.rs:324`) is deleted and call sites read the struct.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/{mod,choice}.rs` — registry + available-addressings rule.
- `crates/smelt-core/src/metadata.rs` — `write:` pin parse + errors.
- `crates/smelt-dialect/src/dialect.rs`, `crates/smelt-backend/src/lib.rs`, `crates/smelt-backend-duckdb/`, `crates/smelt-backend-spark/` — capability migration.
- `crates/smelt-db/src/queries/maintenance.rs`, `crates/smelt-db/src/lib.rs`, `crates/smelt-db/src/diagnostics_types.rs` — the two diagnostics folded into `file_diagnostics()`.
- `crates/smelt-cli/src/explain.rs` — admissible-set + pin rows.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: the grain-demotion/write-pin/registry entry narrows to whatever S-group surface remains (behavioural).
- `docs/specs/diagnostics.md` — catalogue entries for `MaintenanceWriteAddressingRefused` / `MaintenanceWritePatternUnavailable`.
- `docs-site/docs/reference/smelt-yml.md` — the `maintenance.cells[].write` key, documented as an open name with fail-loud refusal.

**Review checklist** (material findings only):
- [ ] The pin selects among admissible mechanisms only — it never widens the set, and refusals name cell + pattern + reason.
- [ ] No silent downgrade path exists (asserted: a refused pin yields a diagnostic and no substituted technique).
- [ ] Registry admission reproduces today's plans exactly when no pin is set (no behaviour change — assert on existing fixtures' explain output).
- [ ] Capability check reads the struct; the trait method is gone (no dual source of truth).
- [ ] `diagnostics.md` and spec semantics agree for both new codes.

**Commit.** `feat(maintenance): open write-pattern registry, available-addressings rule, and the validated cells[].write pin`

---

## Group D — observed output deltas (M3-output)

### Phase D1: Spec diff — landed-delta refinement + observed-delta storage/trust

**Goal.** Spec-first gate for the group: refine `sources.md`'s landed-delta notion from whole-table/interval to a **changed-row set with a partition projection**, and settle where recorded deltas live and how they commit. Default posture to confirm or overturn: warehouse-resident beside the merge ledger (the `_smelt_ledger` precedent in `smelt-state`), written **in the consuming write's transaction**, smelt-state-owned bookkeeping in the same excluded class as the ledger. State the observed-delta trust boundary: recorded deltas are trusted because the state is smelt-owned; no out-of-band-edit tripwire in v1 (recorded as an Open Question, not silently assumed away).

**Pre-conditions.** C4 (the conditional write exists, so there is a changed-row set to talk about).

**TDD tests to write first.** None — spec-diff phase; D2/D3 carry the tests. `/smelt:validate` cleanliness of the edited sections is the phase's check.

**Implementation shape.** Edit `sources.md` (landed-delta refinement; the delta a model edge hands downstream is the observed changed-row set where recorded, else the run's written window — widen-never-narrow), and `incremental_models.md` (§"The graph layer" consumption of observed deltas; storage/transactionality; the trust boundary; the settle-bound × observed-delta composition already named in §"What the composed shape uniquely enables"). Known Divergences updated to record what D2–D4 will build.

**Critical files (allowed to touch in this phase).**
- `docs/specs/sources.md` — landed-delta refinement; trust boundary.
- `docs/specs/incremental_models.md` — observed-delta semantics, storage home, transactionality; Known Divergences.

**Docs touched.**
- Spec edits above (this phase **is** the docs work); all wording timeless — the observed delta has always been part of the contract, its absence is a divergence.

**Review checklist** (material findings only):
- [ ] The refinement is widen-never-narrow: absent a recorded delta, the written window remains the edge's delta (no consumer may assume the narrower form exists).
- [ ] Transactionality is stated as a requirement, not an implementation note (a delta visible without its write, or vice versa, breaks propagation soundness).
- [ ] The trust boundary + missing tripwire is an explicit Open Question with the v1 assumption named.
- [ ] No phase vocabulary in spec body.

**Commit.** `spec(sources+incremental): landed-delta refinement to changed-row sets; observed-delta storage, transactionality, and trust boundary`

---

### Phase D2: T5 — record the observed output delta

**Goal.** The conditional write (C4/C5) already computes the changed-row set — record it. Key-level v1 record (the model's `unique_key` values plus the touched partition value where the output is composed), **comparable columns only**: a `plausible` or otherwise Incomparable column's flutter must never appear in (or dirty) the recorded delta — C2's verdict decides membership. Written in the same backend transaction as the suppressed write, per D1's ruling.

**Pre-conditions.** D1 ruled; C4 landed (C5 extends the record to the staged-candidate path when both are present).

**TDD tests to write first.**
- `crates/smelt-state/tests/` (or the module tests beside `ddl_duckdb.rs`) — observed-delta table DDL/DML per D1's ruling: create-if-absent, upsert of `(model, run window, key set, partition values)`; re-running an idempotent window replaces, never duplicates.
- `crates/smelt-runtime/tests/` — after a conditional MERGE where 3 of 100 candidate rows differed, the recorded delta holds exactly those 3 keys; a fully-suppressed run records an **empty** delta (present-and-empty, distinct from absent); the record commits atomically with the write (a failed write leaves no delta row — assert via an injected failure).
- `crates/smelt-runtime/tests/` — an Incomparable column's change alone (e.g. a `plausible` audit stamp) records nothing.
- `crates/smelt-runtime/tests/statement_parity.rs` — **if** D1 ruled the DML emitter-authored, a parity leg for the recording statement; if ruled smelt-state bookkeeping, extend the structural no-authoring gate's allowlist with the classified justification instead.
- Conformance leg (`crates/smelt-cli/tests/maintenance_conformance/`) — at fixed `S`, the recorded delta equals the actual before/after row diff of the write (computed independently by the harness).

**Implementation shape.** Follow the ledger pattern: DDL/DML builders beside `generate_ledger_table_ddl`/`generate_ledger_insert_sql` in `crates/smelt-state/src/ddl_duckdb.rs` (Spark variant deferred, fail-loud like the ledger), invoked through a `Backend` hook analogous to `fold_ledger_delta` inside the same transaction the conditional write runs in. The changed-key SELECT is derived from the same comparison predicate C4 emits — one comparison, two consumers (the write's suppression arm and the delta record), never two divergent predicates.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/ddl_duckdb.rs` (+ `ddl_spark.rs` fail-loud stub) — observed-delta DDL/DML.
- `crates/smelt-backend/src/lib.rs`, `crates/smelt-backend-duckdb/src/lib.rs` — the transactional hook.
- `crates/smelt-runtime/src/cumulative.rs` / `maintenance_driver.rs` — wiring off the conditional write.
- `crates/smelt-logical/src/maintenance/emit.rs` — only if D1 ruled emitter-authored.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: observed-delta recording built for the conditional families (behavioural).

**Review checklist** (material findings only):
- [ ] One comparison predicate feeds both suppression and recording (no drift between "what wrote" and "what was recorded").
- [ ] Empty-vs-absent distinction is explicit (empty = ran and nothing changed; absent = never recorded — consumers must not conflate them).
- [ ] Same-transaction commit proven, including the failure-injection case.
- [ ] Incomparable columns provably excluded (C2's verdict is the only membership authority).
- [ ] Non-DuckDB backends fail loudly, never receive DuckDB DDL.

**Commit.** `feat(maintenance): record the observed output delta of conditional writes, transactionally with the write`

---

### Phase D3: Key→partition projection of observed deltas — exact `--landed` for model edges

**Goal.** Project a composed model's key-level observed delta to partition intervals via its locality route (exact under routes 1–2; widened backward by `r` + margins under route 3) and feed forward propagation's per-edge dirt, so a composed model edge propagates **observed** dirt instead of derived-clamp-widened dirt. An empty observed delta propagates nothing — the graph half of the no-op cascade. Bare keyed nodes still propagate nothing (refused, Group B). Resolves the research's open question 3 for the composed case with no key-level dirt representation in the graph.

**Pre-conditions.** D2; B1–B2 (composed nodes are graph-admissible with the projection seam in place).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (propagate unit) — projection per route: route-1/2 deltas project to exactly the keys' own partitions; route-3 projection widens backward by `r` + margins; the projection never narrows below the observed partitions (widen-never-narrow asserted as a property).
- `crates/smelt-runtime/tests/since_upstream_propagation.rs` — a composed upstream with a recorded 3-key delta dirties exactly the 3 keys' partitions downstream; with an **empty** recorded delta, zero downstream regions are scheduled; with an **absent** record (pre-conditional-write run), the edge falls back to the written window (the D1 widen rule).
- `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs` — the adjoint law `forward(backward(P)) ⊇ P` extended over observed-delta-fed composed edges.
- Real fixture e2e: web-analytics — a run of `events_deduped` that suppresses everything schedules no downstream `sessions`/identity regions under `--since-upstream`.

**Implementation shape.** A pure projection function beside the locality types (`crates/smelt-logical/src/maintenance/locality.rs` or `propagate.rs`): `project_observed_delta(route, delta) -> Vec<DayInterval>`. `crates/smelt-runtime/src/propagation.rs` (`plan_since_upstream`) consults the observed-delta store for a model-edge origin before falling back to the written window; `propagate.rs` consumes the resulting intervals unchanged (the graph stays interval-typed — no keyed dirt-sets).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/{locality,propagate}.rs` — the projection.
- `crates/smelt-runtime/src/propagation.rs` — observed-delta consultation in `plan_since_upstream`.
- `crates/smelt-state/src/` — read API for the delta store.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: exact dirt projection live for composed edges (behavioural; the §"What the composed shape uniquely enables" second bullet's status).

**Review checklist** (material findings only):
- [ ] The graph representation stays interval-only — no key-level dirt leaks into `Edge`/`Propagation`.
- [ ] Empty/absent handled per D1 (empty ⇒ nothing; absent ⇒ written-window fallback), asserted separately.
- [ ] Adjointness holds over the new edge kind.
- [ ] Route-3 widening includes the margins, not just `r`.

**Commit.** `feat(propagation): project observed output deltas to exact partition dirt on composed model edges`

---

### Phase D4: Observed-delta explain surface + docs

**Goal.** Surface the machinery: `smelt explain` prints, per cell, whether an observed delta is recorded and how it projects (route, exactness, widening); the docs-site guide gains the observed-deltas + no-op-cascades section.

**Pre-conditions.** D2–D3.

**TDD tests to write first.**
- `crates/smelt-cli/tests/explain_model.rs` — a composed model's report shows the recording status per conditional cell and the projection form ("exact (key-embedded)" / "widened by `r` + margins"); a bare keyed model shows no projection row.
- `crates/smelt-cli/tests/explain_show_sql.rs` — unchanged statement rendering (no regression from the report additions).
- Docs-site build green with the new guide section.

**Implementation shape.** Extend `build_maintenance_plan_report` (`crates/smelt-cli/src/explain.rs`) with the recording/projection rows read off the derived plan + locality verdict (pure data — no re-derivation in the CLI). Guide prose under `docs-site/docs/guide/` describing: what a run records, how a stable upstream chain degenerates to empty-delta no-ops, and how settle bounds compose with observed deltas.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/explain.rs` — report rows.
- `docs-site/docs/guide/` — the observed-deltas section.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences narrowed (explain surface live).
- `docs-site/docs/guide/` + `docs-site/docs/reference/cli.md` — timeless feature prose; no plan vocabulary.

**Review checklist** (material findings only):
- [ ] Explain reads the derived plan only (maintenance-plan purity — no CLI-side re-derivation).
- [ ] The guide states the widen-never-narrow fallback honestly (absent record ⇒ written window).
- [ ] Settle × observed-delta composition described as one story (the fourth composed-shape capability).
- [ ] All prose timeless.

**Commit.** `feat(explain)+docs: observed-delta recording and projection surface; no-op cascade guide`

---

## Group E — delta-restricted compute over model edges (M2)

### Phase E1: Spec diff — P1 skeleton-source closure + referential-integrity world-fact

**Goal.** Spec-first gate: define P1 (**skeleton-source closure**) in `model_properties.md` — the row skeleton is provably owned by the driving source alone, via skeleton-role extraction × per-column provenance × one-to-one join contribution × row preservation × **no membership predicates on enrichment columns**; fail-closed to `Open`; v1 restricted to non-aggregating enrichment scopes (join-below-aggregation ⇒ `Open`). Define the `referential_integrity` world-fact in `sources.md` under the trust rule: a narrowing declaration admitted only paired with a count-preservation runtime tripwire over the touched region.

**Pre-conditions.** None (may run in parallel with Group D).

**TDD tests to write first.** None — spec-diff phase; E2–E4 carry the tests.

**Implementation shape.** `model_properties.md` gains the P1 verdict definition (inputs, lattice, the five conjuncts, the v1 aggregation restriction, fail-closed rule) in the derived-proofs table; `sources.md` gains the world-fact YAML shape, its trust-rule classification (narrowing ⇒ tripwire), and the `SourceCountPreservationViolated`-class diagnostic row; `incremental_models.md` names the closure as the licence the delta-restricted transform consumes (transform catalogued in `model_transforms.md` in the same diff).

**Critical files (allowed to touch in this phase).**
- `docs/specs/model_properties.md` — P1.
- `docs/specs/sources.md` — the RI world-fact + tripwire.
- `docs/specs/model_transforms.md`, `docs/specs/incremental_models.md` — the T3 transform entry + consumption note; Known Divergences.
- `docs/specs/diagnostics.md` — catalogue rows for the new codes.

**Docs touched.**
- Spec edits above; timeless.

**Review checklist** (material findings only):
- [ ] The five conjuncts are individually named with their owning proofs (no monolithic "is safe" predicate).
- [ ] Join-below-aggregation ⇒ `Open` is stated as a v1 restriction in Known Divergences, not smuggled into the verdict definition.
- [ ] The RI tripwire follows the trust rule's shape exactly (violation fails the consuming run loudly; past outputs declared suspect).
- [ ] No phase vocabulary.

**Commit.** `spec(properties+sources): P1 skeleton-source closure; referential-integrity world-fact with count-preservation tripwire`

---

### Phase E2: P1 skeleton-source-closure proof

**Goal.** Implement P1 walk-composed in `smelt-logical`: fold the five conjuncts over the query tree, fail-closed to `Open` on any unproven leg or any aggregation above the enrichment join. The proof must **discriminate**: a bare inner-JOIN enrichment with no RI declaration and no `LEFT JOIN` fails the closure — pinned as a lasting negative test.

**Pre-conditions.** E1.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (closure unit) — closes for: driving fact `LEFT JOIN` dimension, payload-only projection, no enrichment-column predicates; stays `Open` for: inner JOIN without the RI declaration; a `WHERE dim.col = …` membership predicate; enrichment feeding a `GROUP BY`; a fan-out (non-1:1) join contribution.
- **Pinned discriminating test** — `crates/smelt-logical/tests/skeleton_closure_pinned.rs::bare_inner_join_enrichment_stays_open` over the real `examples/timeseries/models/daily_events_enriched.sql` body: must remain `Open` (this pin survives until F5 flips only the RI-declared variant).
- `crates/smelt-logical/tests/walk_coverage.rs` — the new fold's leaf classifiers are classified; no raw-text scan on an admission path.
- Inner-join **with** the source's `referential_integrity` declaration closes, and the derived plan records the paired tripwire obligation on the consuming cell.

**Implementation shape.** New `Transfer` impl (or a conjunct-vector fold beside `PropertyTransfer`) in `crates/smelt-logical/src/analysis/` (e.g. `skeleton_closure.rs`), consuming `maintenance/skeleton.rs` skeleton-role extraction, `analysis/join_shape.rs`, per-column provenance (`maintenance/grouping.rs`), and the one-to-one contribution logic generalized from `maintenance_driver::dimension_join_contribution` (hoisted into `smelt-logical` if still runtime-resident — single ownership). Verdict lands on the plan cell as an admission input.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/skeleton_closure.rs` (new) + `walk.rs` touchpoints.
- `crates/smelt-logical/src/maintenance/{derive,mod}.rs` — the verdict on the cell.
- `crates/smelt-core/src/sources.rs` — parse the `referential_integrity` declaration (fail-loud malformed cases).

**Docs touched.**
- `docs/specs/model_properties.md` — the P1 row moves from not-yet to built (Known Divergences narrows, behavioural).

**Review checklist** (material findings only):
- [ ] Fail-closed on every conjunct independently (each negative case names which leg failed).
- [ ] The pinned discriminating test is marked as load-bearing (comment naming F5's flip condition).
- [ ] One-to-one contribution logic has a single owner (no duplicate in runtime).
- [ ] `walk_coverage` green; no ad hoc SQL-text scans.

**Commit.** `feat(properties): skeleton-source-closure proof (P1), walk-composed and fail-closed`

---

### Phase E3: T3 — delta-restricted compute over model edges

**Goal.** Where P1 closes and an exact upstream delta exists (a maintained-model edge carrying a Group-D observed delta — the free-delta case, no sidecar), restrict the enrichment recompute to rows whose enrichment inputs changed: a semi-join on the delta keys replaces the widened scan for that cell. This restricts **recompute breadth under an exact delta**, never what is scanned into `S` — suppression (Group C) and restriction (this phase) compose but are separately licensed. Demo: the web-analytics chain — a redelivered event flows through `events_deduped`'s conditional write; the downstream recompute touches only the affected keys' rows.

**Pre-conditions.** E2; D2–D3 (the exact delta and its projection exist).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (emit unit) — the delta-restricted statement variant: the recompute's driving scan gains the semi-join on the delta key set; with closure `Open` or no recorded delta, the emitted statement is byte-identical to the unrestricted form (no partial restriction).
- `crates/smelt-runtime/tests/statement_parity.rs` — parity leg for the delta-restricted statement group; structural no-authoring gate still green.
- `crates/smelt-runtime/tests/` (e2e, DuckDB) — 3-model chain: upstream conditional write changes 2 keys → the downstream enrichment recompute reads exactly those keys' rows (assert scan predicate + row counts) → end state equals a full refresh.
- Real fixture e2e: web-analytics `events_deduped` → sessions-side consumer — a single redelivered-then-changed event recomputes only its session's rows.
- Fallback: an absent observed delta (pre-D2 upstream) runs the ordinary widened scan (widen-never-narrow).

**Implementation shape.** New emitter variant in `crates/smelt-logical/src/maintenance/emit.rs` (the restricted scan is part of the emitted statement, single-author); technique admission in `derive.rs`/`choice.rs` gated on `(P1 closed) ∧ (exact delta present)`; `crates/smelt-runtime` threads the delta key set from the store into the emitter inputs. The restriction is per-cell — other cells of the same model keep their techniques.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/{emit,derive,choice}.rs` — the T3 variant + admission.
- `crates/smelt-runtime/src/{execute,maintenance_driver}.rs` — delta threading.
- `examples/web_analytics/` — only if the demo needs a consuming-model tweak (no new models).

**Docs touched.**
- `docs/specs/incremental_models.md` + `docs/specs/model_transforms.md` — Known Divergences: T3 live for model edges (behavioural).

**Review checklist** (material findings only):
- [ ] Restriction licensed only by P1 ∧ exact delta — either absent ⇒ byte-identical unrestricted statement (asserted).
- [ ] `S` is unchanged by restriction (the equivalence oracle's input set is identical either way).
- [ ] Emitter single-author; parity leg present.
- [ ] The fallback path is the widened scan, never a skip.

**Commit.** `feat(maintenance): delta-restricted enrichment recompute over model edges under skeleton-source closure`

---

### Phase E4: Conformance — restriction equivalence + the no-op cascade

**Goal.** Net the group in the generative oracle: delta-restricted and widened-scan variants produce bit-identical state at fixed `S` over the closure-admitted recipe subset; and the end-to-end payoff — an upstream run that changes nothing produces zero writes and zero scheduled downstream regions across a 3-model chain.

**Pre-conditions.** E3.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/` — recipe extension: a closure-admissible enrichment shape (fact + RI-declared or LEFT-JOIN dimension, payload-only) and its closure-failing sibling; the pool records which recipes expect restriction.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::delta_restricted_equals_widened_scan_at_fixed_s` — generative, deterministic seed: run each admitted recipe both ways over the same schedule, assert bit-identical output state.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::empty_delta_cascade_is_a_no_op` — a 3-model chain, an upstream run over already-processed input: zero rows written at every stage (via observed deltas), zero regions scheduled by `--since-upstream`, and the full-refresh oracle still matches.
- An admission-rate floor for the closure-admitted subset (restriction must actually engage in the sample).

**Implementation shape.** Testkit + gate only; no production code. The both-ways run reuses the technique-pin surface (`maintenance.cells[].technique`) to force the widened-scan variant.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/{recipe,render,oracle*}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/{gate,registry}.rs`

**Docs touched.**
- `docs/specs/incremental_models.md` — References: conformance coverage note gains the restriction + cascade legs.

**Review checklist** (material findings only):
- [ ] Bit-equality asserted at fixed `S`, both directions of the pin.
- [ ] The cascade test asserts *zero writes* (not just zero diffs) — the suppression is observable, not incidental.
- [ ] Admission-rate floor present for the restricted subset.
- [ ] Deterministic seed; runtime bounded.

**Commit.** `test(conformance): delta-restriction equivalence at fixed S; empty-delta no-op cascade end-to-end`

---

## Group F — input fingerprint sidecars (M3-input)

### Phase F1: Spec diff — the fingerprint sidecar

**Goal.** Spec-first gate for the most state-heavy group: specify the row-content fingerprint sidecar that synthesizes a change feed for an external `mutable_snapshot` source — naming/namespace, storage home, transactionality with the consuming write, GC, `--full-refresh` behaviour, multi-consumer sharing, invalidation rules — and the digest stance: SHA-256-class digests with the collision-soundness invariant stated explicitly and oracle-gated (exact `IS DISTINCT FROM` remains the write-suppression compare; digests are sidecar-only). Reconcile explicitly with `output_fingerprint.md`'s "ephemeral, never persisted" principle: the sidecar is a **different artifact class** — cross-run by definition — and the spec must say why the principle doesn't apply rather than silently coexisting with it. P4 (fingerprint projection) is defined in `model_properties.md`.

**Pre-conditions.** None (parallel with D/E permitted; F2+ depend on this ruling).

**TDD tests to write first.** None — spec-diff phase; F2–F5 carry the tests.

**Implementation shape.** New spec section in `incremental_models.md` (or a dedicated sidecar section referenced from `sources.md`) covering the artifact's full lifecycle; `model_properties.md` gains P4; `output_fingerprint.md` gains the boundary paragraph naming the two artifact classes; `multi_backend.md` capability note if the sidecar needs backend features. Open questions that stay open (e.g. cross-project sharing) are recorded as such.

**Critical files (allowed to touch in this phase).**
- `docs/specs/incremental_models.md`, `docs/specs/sources.md` — the sidecar lifecycle.
- `docs/specs/model_properties.md` — P4.
- `docs/specs/output_fingerprint.md` — the artifact-class boundary.

**Docs touched.**
- Spec edits above; timeless.

**Review checklist** (material findings only):
- [ ] Every lifecycle question the skeleton listed (namespace, GC, `--full-refresh`, multi-consumer) has a ruling or a named Open Question — none silently unaddressed.
- [ ] The digest soundness invariant is stated as an assumption with its oracle gate named (not folded into "obviously fine").
- [ ] The `output_fingerprint.md` reconciliation names both artifact classes and why each rule applies to its own class only.
- [ ] Invalidation is widen-never-narrow by construction.

**Commit.** `spec(incremental+properties): the row-content fingerprint sidecar — lifecycle, digest stance, and P4 projection`

---

### Phase F2: P4 — fingerprint-projection derivation

**Goal.** Derive, per (model × external source), **which columns feed the model** — the projection the fingerprint digests — so an irrelevant-column churn in the source never dirties the consumer. Fail-closed: an unprojectable consumption (SELECT *, opaque function over the source, unresolvable provenance) ⇒ full-row digest, never a guessed subset.

**Pre-conditions.** F1.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` — a model reading `dim.name, dim.tier` derives exactly that projection; `SELECT *` from the dimension ⇒ full-row; an opaque `smelt.extern` call over the dimension ⇒ full-row; a CTE-composed read resolves through the walk or fails closed to full-row.
- `crates/smelt-logical/tests/walk_coverage.rs` — the projection derivation is walk-composed/classified.
- Two consumers of one source with different projections derive **distinct** projections (per-consumer, not unioned silently — sharing policy per F1's ruling).

**Implementation shape.** A projection fold in `crates/smelt-logical/src/analysis/` reusing per-column provenance; verdict carried on the plan cell that consumes the source. Pure data — no digest computation here.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/` (projection fold) + `maintenance/{derive,mod}.rs` (verdict on the cell).

**Docs touched.**
- `docs/specs/model_properties.md` — Known Divergences: P4 built (behavioural).

**Review checklist** (material findings only):
- [ ] Fail-closed cases each land on full-row digest with the reason recorded (observable in explain later, not silent).
- [ ] Projection is per-consumer per F1's sharing ruling.
- [ ] Walk-composed; no raw-text scans.
- [ ] No digesting/state logic leaks into this phase.

**Commit.** `feat(properties): fingerprint-projection derivation (P4), fail-closed to full-row`

---

### Phase F3: T4 — sidecar build + synthesized external change feed

**Goal.** Build the sidecar and consume it: DDL/DML emitted per F1's ruling, upserted **in the consuming write's transaction**; each run snapshot-diffs the external `mutable_snapshot` source against the sidecar (digest compare over the P4 projection) to yield the changed-key set — the synthesized change feed that makes the source's delta exact instead of whole-table.

**Pre-conditions.** F1–F2; D2 (the delta-store read/write patterns to mirror); C4 (the conditional write the sidecar upsert rides with).

**TDD tests to write first.**
- `crates/smelt-state/tests/` / emitter unit — sidecar DDL (create-if-absent, keyed by source key + digest over the P4 projection), upsert DML; first run against an absent sidecar yields the whole-table delta and populates it.
- `crates/smelt-runtime/tests/` (DuckDB e2e) — run 1 populates the sidecar; a source edit touching 2 of 1000 rows makes run 2's derived changed-key set exactly those 2 keys; an edit to a column **outside** the P4 projection yields an empty changed set; the upsert commits atomically with the consuming write (failure injection).
- `crates/smelt-runtime/tests/statement_parity.rs` — parity leg (or classified allowlist entry, per F1's authoring ruling) for the sidecar statements + diff query.
- Conformance leg — a sidecar-fed run sequence equals the full-refresh oracle after every step (the digest soundness oracle gate F1 names).

**Implementation shape.** Sidecar DDL/DML beside the ledger builders (`crates/smelt-state/src/ddl_duckdb.rs`; Spark fail-loud), transactional hook via `smelt-backend` like `fold_ledger_delta`; the diff query is emitter-authored; the changed-key set feeds the same per-cell delta seam D2/E3 established, so T3-over-external is only a licence change away (F5).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/ddl_duckdb.rs` (+ Spark stub), `crates/smelt-backend*/src/lib.rs` — sidecar storage + hook.
- `crates/smelt-logical/src/maintenance/emit.rs` — the diff query emitter.
- `crates/smelt-runtime/src/maintenance_driver.rs` — the per-run diff step.

**Docs touched.**
- `docs/specs/incremental_models.md` / `sources.md` — Known Divergences: sidecar built for DuckDB; Spark fail-loud (behavioural).

**Review checklist** (material findings only):
- [ ] Absent sidecar ⇒ whole-table delta + populate (never an error, never a skip).
- [ ] Out-of-projection churn provably produces an empty changed set.
- [ ] Same-transaction upsert proven with failure injection.
- [ ] The conformance leg is the digest-soundness oracle gate F1 promised.
- [ ] Non-DuckDB backends fail loudly.

**Commit.** `feat(maintenance): fingerprint sidecar with transactional upsert; synthesized change feed for mutable_snapshot sources`

---

### Phase F4: Sidecar invalidation

**Goal.** Invalidate correctly: a model-definition change affecting the consumption, a source schema evolution, or a P4 projection change ⇒ the sidecar is declared stale and the next run treats **everything** as changed (widen-never-narrow) while rebuilding it. A stale or absent sidecar always degrades to the unconditional/whole-table path — never to a silent skip.

**Pre-conditions.** F3.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/` — changing the model's projection over the source invalidates (next run: whole-table delta + sidecar rebuilt under the new projection); a source column add that enters the P4 projection invalidates; one that doesn't, doesn't.
- `crates/smelt-runtime/tests/` — a hand-corrupted/version-mismatched sidecar is detected (stored projection/digest-version stamp) and treated as absent, with a loud log — never trusted, never a skip.
- Conformance: an invalidation mid-schedule (recipe edit) still matches the full-refresh oracle on every subsequent step.

**Implementation shape.** Stamp the sidecar with its identity (projection hash, digest version, model-definition provenance); compare on every run; mismatch ⇒ treat-as-absent + rebuild. Reuses the schema-tracking precedent in `crates/smelt-state/src/schema_tracking.rs`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/{ddl_duckdb,schema_tracking}.rs`, `crates/smelt-runtime/src/maintenance_driver.rs`.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: invalidation rules live (behavioural).

**Review checklist** (material findings only):
- [ ] Every invalidation trigger F1 lists has a test; none default to "still valid".
- [ ] Degradation target is always the unconditional path (asserted: full delta, all rows recomputed, oracle still matches).
- [ ] The identity stamp covers projection *and* digest version.
- [ ] No narrow-on-invalidate path exists.

**Commit.** `feat(maintenance): fingerprint-sidecar invalidation — widen to everything-changed, never skip`

---

### Phase F5: T3 over external sources — the point-lookup enrichment recompute

**Goal.** Close the loop on the motivating fixture: extend the real `examples/timeseries/models/daily_events_enriched.sql` (fact + mutable user-dimension enrichment — the model already exists and is exercised by `technique_lowering.rs`; extend, don't duplicate) so one renamed user out of the full dimension yields a point-lookup enrichment recompute: sidecar diff (F3) → changed-key set → P1-licensed delta-restricted recompute (E3) over the external edge. The fixture **deliberately** needs the `referential_integrity` declaration (or a `LEFT JOIN`) to pass the closure — E2's pinned discriminating test flips here for the declared variant only; the undeclared inner-join variant stays `Open` and keeps its pin.

**Pre-conditions.** E2–E3; F3–F4.

**TDD tests to write first.**
- `crates/smelt-logical/tests/skeleton_closure_pinned.rs` — the RI-declared (or LEFT-JOIN) variant of `daily_events_enriched` now closes; the bare inner-join variant **still fails** (the pin from E2 stays red-guarded, proving the proof discriminates).
- `crates/smelt-runtime/tests/technique_lowering.rs` (extend the existing e2e) — with the declaration added to the users source: rename 1 user of N → run → the enrichment recompute's scan touches only that user's fact rows (assert emitted predicate + row counts); end state equals full refresh; the RI count-preservation tripwire passes.
- A violated RI declaration (inject a dangling fact key) fails the run loudly via the E1 tripwire.
- `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/timeseries/` stays clean with the source-YAML addition.

**Implementation shape.** Fixture + source-YAML work plus the final admission wiring: E3's technique gate accepts (P1 closed ∧ sidecar-derived exact delta) for an external edge — the licence union, no new mechanism. The tripwire query is emitter-authored and runs in the consuming transaction.

**Critical files (allowed to touch in this phase).**
- `examples/timeseries/models/daily_events_enriched.sql` + `examples/timeseries/sources/` — the declaration.
- `crates/smelt-logical/src/maintenance/{derive,choice}.rs` — external-edge licence union.
- `crates/smelt-logical/src/maintenance/emit.rs` — tripwire emitter (if not landed in E1's group).

**Docs touched.**
- `docs/specs/incremental_models.md` / `sources.md` — Known Divergences: T3 live over external sources (behavioural).
- `docs-site/docs/guide/` — the "one renamed user" walk-through in the enrichment section.

**Review checklist** (material findings only):
- [ ] The discriminating pair (declared closes / undeclared stays `Open`) is asserted in one test file, side by side.
- [ ] Point-lookup verified by observed scan breadth, not just end-state equality.
- [ ] Tripwire fires on a genuine violation and fails the run transactionally.
- [ ] No duplicate fixture — the existing model/tests are extended.

**Commit.** `feat(maintenance): delta-restricted recompute over external sources — the one-renamed-user point lookup`

---

## Group G — choice and closing sweep

### Phase G1: Conditional variants in per-cell technique choice

**Goal.** Conditional variants enter per-cell technique choice as **proven-interchangeable in the strongest sense** (bit-identical at fixed `S` — the C4/E4 conformance legs are the proof): `resolve_cell_choice`/`EffectiveOverride` rank suppressed vs unconditional under the `defaults.prefer` → `cells[].prefer` → `cells[].technique` ladder; first-build and definition-change backfill **admit but do not prefer** the conditional variant (there is no prior state to compare against — the compare is pure cost). `smelt bakeoff` remains unwired (tracked by `docs/plans/20260707-maintenance-plan-impl.md` MP13) — this phase is the choice-ladder integration only, and the spec divergence stays honest about it.

**Pre-conditions.** C4–C6, E3 (the variants exist); R1 (the pin vocabulary the ladder resolves).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (choice unit) — with no pins, an admitted conditional variant is preferred over unconditional for a steady-state cell; a first-build cell resolves unconditional even where conditional is admitted; `cells[].technique` pins either way and an inadmissible pin refuses (never falls back silently).
- `crates/smelt-logical/tests/` — the `prefer` soft-bias ladder orders as specified (defaults < cell < technique), narrower scope winning.
- `crates/smelt-runtime/tests/` (e2e) — a pinned-unconditional model and its unpinned sibling produce bit-identical state over the same schedule (the interchangeability claim exercised through the real pipeline).
- `crates/smelt-cli/tests/explain_model.rs` — explain shows the chosen variant per cell and why (pin / preference / default).

**Implementation shape.** Extend `ChosenTechnique`/`resolve_cell_choice` (`crates/smelt-logical/src/maintenance/choice.rs`) with the conditional-variant dimension; the first-build/backfill posture is a rule in the resolver, not a runtime special case. No cost *model* (measured statistics) in this phase — preference order is structural; a statistics-fed cost model is recorded as an open question (does it want region-level change-ratio statistics from prior observed deltas?).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/choice.rs` + `mod.rs`.
- `crates/smelt-runtime/src/maintenance_driver.rs` — consume the resolution.
- `crates/smelt-cli/src/explain.rs` — the chosen-variant row.

**Docs touched.**
- `docs/specs/incremental_models.md` — §Known Divergences: choice live for conditional variants; `smelt bakeoff` still unwired (honest, with tracking link); the cost-model open question recorded.

**Review checklist** (material findings only):
- [ ] Choice can change freshness/cost only, never bits at fixed `S` (validator-not-chooser upheld; e2e leg proves it).
- [ ] First-build/backfill posture is admit-not-prefer, asserted both ways.
- [ ] Inadmissible pins refuse loudly.
- [ ] Bakeoff honesty: no phantom surface documented as live.

**Commit.** `feat(maintenance): conditional variants enter per-cell technique choice; first-build stays unconditional by preference`

---

### Phase G2: Docs sweep + drift report

**Goal.** Close the plan: run `/smelt:validate incremental_models` and drive the drift report to zero findings — or convert each remaining finding into a Known Divergences entry with a tracking link. Sweep `docs-site/` so every surface this plan shipped is documented (`key_recurrence`, the composed shape and its three routes, settle bounds, `write:` pins, suppression, observed deltas, restriction, the sidecar) and the "partitioned or keyed is a category error" framing is present in the guide and tutorial — kill any surviving prose that frames the axes as exclusive alternatives.

**Pre-conditions.** All prior phases `done` (or explicitly deferred with divergences recorded).

**TDD tests to write first.**
- `crates/smelt-cli/tests/tutorial_freshness.rs` — all web-analytics pages fresh after the sweep.
- `crates/smelt-cli/tests/example_diagnostics.rs` + `crates/smelt-lsp/tests/example_workspaces.rs` — every guide code block's backing fixture clean.
- Docs-site build green; `/smelt:validate incremental_models` output attached to the phase commit (zero findings, or each finding cross-linked to a divergence entry).

**Implementation shape.** `rg` the docs-site + specs for exclusive-axes phrasing ("either partitioned or keyed", "keyed models cannot have a time axis", mode-era vocabulary); rewrite against §"The two axes are orthogonal". Fill reference gaps (`docs-site/docs/reference/{smelt-yml,sources-yml,cli}.md`). Update `docs/ROADMAP.md` with the completed groups and dates.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/**` — the sweep.
- `docs/specs/*.md` — divergence reconciliation only (no new semantics).
- `docs/ROADMAP.md`.

**Docs touched.**
- Everything above — the phase is the docs work; all prose timeless.

**Review checklist** (material findings only):
- [ ] Zero unexplained validate findings.
- [ ] No exclusive-axes phrasing survives anywhere in docs-site or spec bodies (`rg` evidence in the review).
- [ ] Every shipped surface has a reference entry and a guide mention.
- [ ] ROADMAP updated with dates, no commit hashes.

**Commit.** `docs: composed-axes + conditional-maintenance sweep; drive incremental_models drift report to zero`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- `key_per_partition`'s real trajectory profile (backfill-cascade discipline, lateness truncation — `models.md` names both). A0 only makes it refuse honestly; building the grain waits for demand and gets its own plan.
- Retiring the `batched:` sub-block surface (`batched.unique_key` / `batched.safety_overrides` → top-level keys) — S1 lands the top-level `unique_key:` and derived-label logic without breaking the sub-block; the sub-block's retirement + `smelt migrate` assist is a follow-up surface cut.

## Verification

How to confirm the spec is satisfied at the end:

- **The tracer stands.** `examples/web_analytics` builds clean with the composed `events_deduped` stage; the redelivery-storm e2e shows (a) end state equals a full refresh, (b) an unchanged-input re-run writes zero rows, (c) an empty delta propagates zero downstream regions. Tutorial freshness gate green (`cargo test -p smelt-cli --test tutorial_freshness`).
- **Generative equivalence net.** `cargo test -p smelt-cli --test maintenance_conformance` — composed-shape recipes across all three routes; suppressed, staged-candidate, delta-restricted, and sidecar-fed variants all equal the full-refresh oracle after every step.
- **Statement single-ownership.** `cargo test -p smelt-runtime --test statement_parity` — parity legs for every emitter this plan added or changed (slice-predicated fold, checked route-3 probe, suppressed MERGE variants, staged-candidate group, delta-restricted statement, sidecar DDL/DML), plus the structural no-authoring gate over the new statement shapes.
- **Walk composition.** `cargo test -p smelt-logical --test walk_coverage` — P1/P3/P4 land walk-composed and classified.
- **Adjointness.** `cargo test -p smelt-logical --test maintenance_propagation_adjoint` — the law holds over composed nodes and observed-delta projections.
- `bash .claude/scripts/verify-phase.sh` green; `cargo test -p smelt-lsp --test example_workspaces` green.
- `/smelt:validate incremental_models` reports zero drift (or every finding is a recorded Known Divergence with a tracking link).
