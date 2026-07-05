# Plan: Time-partitioned keyed output (key temporal locality)

**Date**: 2026-07-05
**Spec**: [`docs/specs/keyed_models.md`](../specs/keyed_models.md)
**Spec diff**: commit `42da901d` (working tree at authoring); companion edits in `model_maintenance.md`, `sources.md`, `models.md`, `timeseries.md`, `diagnostics.md`
**Tracking PR / branch**: `worktree-incremental` (master: `docs/plans/20260704-model-updates.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/keyed_models.md` — it is the correctness oracle, in particular §"Key temporal locality". Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. **Pre-condition on the keyed collapse.** This plan builds on `refresh: keyed` as delivered by `docs/plans/20260705-keyed-collapse.md` (K1–K6): `RefreshStrategy::Keyed`, the unified column-family classifier (overwrite + once-write + plain-overwrite families), the transactional merge ledger, and the windowed-keyed-maintenance driver must all be landed and green before Phase 1. If K1–K6 are not `done`, stop and tell the user — do not start against the interim `cumulative` seed.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the "Verification" section and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update the spec first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` (the keyed dedupe/enrichment fixtures under `examples/timeseries/`).
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Salsa purity, fail-loud discipline, layered single-ownership).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Spec and `docs-site/` edits describe the feature as if it has always existed — no phase headings/labels/callouts. Behavioural gaps go under the spec's **Known Divergences**; "what landed when" lives in this plan's Progress table.

---

## Context

The keyed refresh mode's output is keyed state (one row per key). The spec's §"Key temporal locality" admits an **optional `timeseries:` block** on that output when a run's writes are provably confined to a computable slice of the output's time axis — the (key, time)-addressed cell. This unblocks event-grain dedupe over a bounded redelivery window (which partition-local `batched` cannot dedup across partitions, and which an unpruned keyed merge cannot afford at volume), per-(key, period) aggregates, and the clock-sink problem where a keyed stage strips the timeseries property from the DAG and forces every downstream consumer into full scans. Design derivation: `docs/research/20260705-keyed-time-superset.md`.

## Scope

### In scope (spec coverage)
- §"Key temporal locality" — the admission gate; the three routes (key-embedded, key-determined, recurrence-bounded); structural preconditions; per-slice equivalence; row-movement rule.
- §"Key temporal locality" — the slice-pruned merge target (target-scan pruning, not a write clamp) and the route-3 transactional runtime check.
- §Semantics "The output as a clocked source" — registering an admitted keyed output as clocked; the derived settle bound in `smelt explain`.
- §Surface — the `timeseries:` admission on keyed; the narrowed `KeyedForbidsTimeseries` and `KeyedGroupByContainsPartitionColumn`; the new `KeyedRecurrenceBoundViolated`.
- `sources.md` §"Source YAML shape" — the `key_recurrence` declaration (parse + fail-loud).
- User docs: the keyed-models guide's time-partitioned recipe.

### Explicitly deferred
- **Scope-map `smelt explain` surface** (the per-input dispatch rows, `model_maintenance.md` §composition contract). Mode-agnostic; its own plan.
- **Granularity relaxation** (driver granularity ≠ output granularity, e.g. daily driver → weekly output). Spec keeps granularity equality; open question recorded in the spec.
- **Snapshot-reconcile locality** (a derived recurrence bound pruning a diff-merge). v1 is window-forward only, per spec preconditions.
- **Slice-scoped deletion** (`NOT MATCHED BY SOURCE` over a provably complete slice). Interacts with the open key-deletion divergence.
- **Hourly driver granularity.** Inherited driver limitation (`maintenance_driver.rs::driving_steps`), tracked in the keyed spec's Known Divergences; orthogonal to this plan.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | pending |        |      |
| 2     | pending |        |      |
| 3     | pending |        |      |
| 4     | pending |        |      |
| 5     | pending |        |      |
| 6     | pending |        |      |
| 7     | pending |        |      |
| 8     | pending |        |      |

---

### Phase 1: Admit the block behind a fail-closed locality gate

**Goal.** Stop treating `refresh: keyed` + `timeseries:` as an unconditional hard error; route it to a single locality-gate entry point that (until later phases wire the routes) still refuses via `KeyedForbidsTimeseries` with the three-route message. Narrow `KeyedGroupByContainsPartitionColumn`. This is the seam — everything stays fail-closed and spec-consistent.

**Pre-conditions.** K1–K6 landed. `refresh: keyed` parses; the classifier resolves the driving source and column families.

**TDD tests to write first.**
- `crates/smelt-core/tests/*` (frontmatter validation) — `refresh: keyed` + `timeseries:` no longer rejected at the models.md frontmatter-combination layer; it reaches the keyed rule instead of `deny`-listing at parse.
- `crates/smelt-logical/src/rules/cumulative.rs` (locality unit tests) — a keyed model with a `timeseries:` block returns a `KeyedForbidsTimeseries` diagnostic whose message names all three routes and the nearest missing fact (assert on message substrings), not a blanket "keyed forbids timeseries".
- `crates/smelt-logical/src/rules/cumulative.rs` — `KeyedGroupByContainsPartitionColumn` fires **only** when the model has no `timeseries:` block; with a block present, control passes to the locality gate (which refuses for a different, named reason in this phase).
- Real fixture: `examples/timeseries/` gains a keyed model declaring `timeseries:`; `cargo test -p smelt-cli --test example_diagnostics` sees the specific `KeyedForbidsTimeseries` three-route message (fixture asserted as an intentional-diagnostic case until Phase 2 admits it).

**Implementation shape.** In `config.rs` frontmatter validation, drop the `keyed + timeseries ⇒ hard error` combination row for `keyed` (keep it for `versioned`/`materialized_view`). Add a `establish_locality(model, classifier_output, driving_source) -> Result<LocalitySlice, LocalityRefusal>` entry point in the keyed rule; Phase 1 body returns `Err(NoRouteImplemented)` rendered as `KeyedForbidsTimeseries`. Gate `KeyedGroupByContainsPartitionColumn` on `timeseries.is_none()`.

**Critical files.**
- `crates/smelt-core/src/config.rs` — frontmatter-combination validation.
- `crates/smelt-logical/src/rules/cumulative.rs` — the gate entry point; the narrowed partition-column diagnostic.
- `crates/smelt-logical/src/rules/rule_diagnostics.rs` — the three-route `KeyedForbidsTimeseries` message.

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: narrow "every `timeseries:` block is refused" to "refused pending route implementation; the gate and message exist". (Describe behaviour, not the phase.)

**Review checklist.**
- [ ] TDD tests exist and assert the three-route message + the narrowed partition-column trigger.
- [ ] Spec rules from §"Key temporal locality" preconditions honored (gate is the single entry point).
- [ ] Fail-loud discipline: no silent admission; refusal is a classified diagnostic.
- [ ] No scope creep — no route logic yet.
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): admit timeseries: on keyed behind a fail-closed locality gate; narrow partition-column diagnostic`

---

### Phase 2: Route 1 (key-embedded) + structural preconditions

**Goal.** Implement the structural preconditions (window-forward run shape; `partition_column` a projection, NOT NULL, family-admissible; granularity equality) and route 1 — `partition_column` is a `unique_key` column. Admit the block when route 1 holds; derive the slice = scan window widened by the derived lateness/skew margins. Emit per-slice equivalence in the model's derived properties.

**Pre-conditions.** Phase 1 gate in place.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` — precondition checks: snapshot-reconcile model with a `timeseries:` block is refused (no locality without a clock); a nullable partition projection is refused (`MalformedTimeseries` path); driver-granularity ≠ output-granularity is refused.
- `crates/smelt-logical/src/rules/cumulative.rs` — route 1 admits when `partition_column ∈ unique_key`; the returned `LocalitySlice` equals `[scan_start − lateness, scan_end]` on the output clock.
- Real fixture: `examples/timeseries/` per-(key, day) keyed aggregate keyed on `(entity_id, day)` compiles clean (no diagnostics) via `example_diagnostics`.

**Implementation shape.** Flesh `establish_locality` route 1: check the preconditions, then `partition_column ∈ unique_key`. Return `LocalitySlice { start, end, margins }` from the scan window and the driving source's `source_lateness`. Fold per-slice equivalence into the derived-postures surface (`smelt explain`).

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — preconditions + route 1; `LocalitySlice`.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — reuse the interval/margin derivation for the slice width.

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: route 1 now admitted (behavioural).
- `docs-site/docs/guide/` — the keyed guide gains the per-(key, period) example (draft; expanded in Phase 8).

**Review checklist.**
- [ ] Preconditions match the spec's bulleted list exactly.
- [ ] Route 1 slice derivation is derived, never declared.
- [ ] Per-slice equivalence surfaced.
- [ ] No pruning yet (Phase 3) — admission only.
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): key-embedded locality route + structural preconditions for time-partitioned output`

---

### Phase 3: Slice-pruned merge target

**Goal.** Wire the physical payoff: the `merge_into` target scan carries `WHERE target.<partition_column> ∈ slice`, derived per window step. Pruning is no-op elimination on the target read; every scanned delta row still merges (no write clamp).

**Pre-conditions.** Phase 2 admits route 1 and yields a `LocalitySlice`.

**TDD tests to write first.**
- `crates/smelt-runtime/src/cumulative.rs` (merge SQL builder) — `build_merge_sql` for a locality-admitted model emits a target-side partition predicate over the slice bounds; for a non-time-partitioned keyed model the SQL is unchanged (no predicate).
- `crates/smelt-runtime/src/cumulative.rs` — the predicate bounds equal the `LocalitySlice`; a delta row whose key lives in an out-of-slice partition still appears in the `USING (...)` delta (merge is not dropped) — asserts pruning is read-side only.
- Real fixture (runtime, DuckDB): a keyed per-(key, day) model over a two-window `examples/timeseries/` fixture where window 2 does not re-scan window 1's partition (assert via row counts / the emitted SQL), yet end state equals a full refresh.

**Implementation shape.** Thread `Option<LocalitySlice>` into the merge builder; when present, add the target predicate. Keep the delta SELECT unchanged.

**Critical files.**
- `crates/smelt-runtime/src/cumulative.rs` — `build_merge_sql` target predicate.
- `crates/smelt-runtime/src/maintenance_driver.rs` — pass the per-step slice through `WindowedKeyedRule`.

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: slice-pruned merge target built for the derived routes.

**Review checklist.**
- [ ] Pruning is target-read-side only; delta rows are never dropped.
- [ ] End-state equivalence holds under pruning (real-fixture oracle).
- [ ] No predicate emitted for non-time-partitioned keyed models (no regression).
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): slice-pruned merge target scan under established locality`

---

### Phase 4: Route 2 (key-determined) via the functional-dependency proof

**Goal.** Admit locality when the partition projection is a per-key constant under the once-write provenance proof — a key-derived expression or a declared functional dependency over a non-null column. Slice = the delta's own partition values; pruning is exact regardless of key age. Partition value never moves under this route.

**Pre-conditions.** Phases 2–3. The functional-dependency proof (`analysis/functional_dependency.rs`) exists (delivered by the fundamentals work).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` — route 2 admits when the partition projection is key-derived (e.g. `date(MIN(event_ts))` on a once-write key); the slice is the delta's partition values, not the scan window.
- `crates/smelt-logical/src/analysis/functional_dependency.rs` — a declared FD over a nullable column is refused; over a non-null column admitted.
- Real fixture: an `examples/timeseries/` dedupe-by-`event_id` model whose `first_seen_date` is key-determined; an old key's partition is pruned exactly (assert the emitted slice is a single partition, not the full scan window).

**Implementation shape.** Route 2 in `establish_locality`: call the once-write/FD proof; on success return a slice tied to the delta's partition values (a per-step point/narrow interval), not the scan window.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — route 2.
- `crates/smelt-logical/src/analysis/functional_dependency.rs` — the proof consumed here (read-only; no new proof logic unless a gap surfaces).

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: route 2 admitted.

**Review checklist.**
- [ ] Route 2 slice is exact (delta partition values), not the widened scan window.
- [ ] Nullable-column FD refused (fail-closed).
- [ ] Row movement is impossible under this route (asserted).
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): key-determined locality route via the functional-dependency proof`

---

### Phase 5: `key_recurrence` source declaration (parse + fail-loud)

**Goal.** Add the `key_recurrence: { key: [...], window: '<interval>' }` field to the source YAML surface: parse it, validate it (unknown column / missing `window:` / bad interval → `MalformedSource`), carry it on `SourceInfo`. Consumed by nothing until Phase 6.

**Pre-conditions.** None beyond the current source loader.

**TDD tests to write first.**
- `crates/smelt-core/tests/source_yaml.rs` — a well-formed `key_recurrence` parses onto `SourceInfo`; an unknown key column, a missing `window:`, and a malformed interval each produce `MalformedSource`.
- `crates/smelt-core/tests/source_yaml.rs` — absent `key_recurrence` leaves the field `None` (no default bound).

**Implementation shape.** Add `key_recurrence: Option<KeyRecurrence>` to `SourceInfo` and the raw deserialization struct in `sources.rs`; reuse the fail-loud interval grammar (`source_bounds::parse_interval`). Validate key columns against the declared `columns:`.

**Critical files.**
- `crates/smelt-core/src/sources.rs` — `SourceInfo`, `parse_source_yaml`, the new struct + validation.

**Docs touched.**
- `docs/specs/sources.md` — already specifies the field; verify the parse matches (no new prose unless drift).
- `docs-site/docs/reference/sources-yml.md` — add the `key_recurrence` key to the reference.

**Review checklist.**
- [ ] All three malformed cases produce `MalformedSource` (fail-loud).
- [ ] Field is `Option`, absent ⇒ `None` (no silent default).
- [ ] Interval grammar reused, not re-implemented.
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(sources): key_recurrence declaration — parse, validate, carry on SourceInfo`

---

### Phase 6: Route 3 (recurrence-bounded) + transactional runtime check

**Goal.** Admit locality when a key-recurrence bound `r` holds — derived from SQL where statically decidable, else read from the driving source's declared `key_recurrence`. Slice = scan window widened backward by `r` plus margins. A **declared** `r` is admitted only **checked**: the run verifies at merge time that no delta row matched (or would duplicate) a stored key outside the slice; a violation fails the run transactionally (`KeyedRecurrenceBoundViolated`). Row movement (an extremal/overwrite partition projection superseded by a late row) is admitted in-slice. This delivers the flagship event-grain dedupe over a bounded window.

**Pre-conditions.** Phases 2–5. The merge ledger (K4) provides the transactional boundary the check rides in.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` — a derivable `r` (e.g. an explicit `BETWEEN event_ts AND event_ts + INTERVAL`) admits route 3 with a proof-backed slice; a declared-only `r` admits with a `checked` flag on the slice.
- `crates/smelt-runtime/src/cumulative.rs` — the emitted merge for a checked route-3 model includes the out-of-slice-match count query; a fixture with an in-bound redelivery merges cleanly; a fixture with an out-of-bound redelivery rolls back with `KeyedRecurrenceBoundViolated` (count + sample keys in the message).
- `crates/smelt-runtime/*` — a **derived** route-3 slice never emits the runtime check (proofs don't need checking).
- Real fixture (DuckDB): the 3-day dedupe model over `examples/timeseries/` — day-by-day window-forward, `MAX_BY(payload, event_ts)` supersede, `key_recurrence: { key: [event_id], window: '3 days' }`; end state equals a full refresh; an injected 4-day-late duplicate trips the check.

**Implementation shape.** Route 3 in `establish_locality`: try the SQL derivation of `r`; else read `key_recurrence` (matched to the model's `unique_key`) and mark the slice `checked`. In the merge builder, when `checked`, add the out-of-slice match count and wrap the window's `merge_into` + check in the ledger transaction; non-zero ⇒ roll back + `KeyedRecurrenceBoundViolated`.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — route 3 derivation + declared-bound read.
- `crates/smelt-runtime/src/cumulative.rs` — the checked-merge path.
- `crates/smelt-runtime/src/maintenance_driver.rs` — transactional wrapping with the ledger.
- `crates/smelt-logical/src/rules/rule_diagnostics.rs` — `KeyedRecurrenceBoundViolated`.

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: route 3 + the runtime check built.
- `docs-site/docs/guide/` — the dedupe recipe (expanded in Phase 8).

**Review checklist.**
- [ ] Derived `r` prunes without a runtime check; declared `r` always emits the check.
- [ ] A violated declared bound fails transactionally — never silently drops (spec Constraint 15).
- [ ] Row movement admitted in-slice; overwrite columns still force sequential order.
- [ ] End-state equivalence holds on the real dedupe fixture.
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): recurrence-bounded locality route + transactional out-of-slice check`

---

### Phase 7: Output as a clocked source + settle bound

**Goal.** Register an admitted keyed output's `timeseries:` so downstream batched models receive source-filter pushdown against it and a downstream keyed model may take it as its clocked driving source — the clock propagates through the DAG instead of stopping at the keyed stage. Derive and surface the output's **settle bound** in `smelt explain` (route 1: source lateness; route 3: `r` + margins; route 2: never settles).

**Pre-conditions.** Phases 2–6 (at least one route admits).

**TDD tests to write first.**
- `crates/smelt-logical/*` (pushdown / source-shape resolution) — a keyed model with an admitted `timeseries:` output resolves as a clocked source for a downstream model; a downstream batched model gets a source-filter pushdown predicate against it; a downstream keyed model selects it as driving source (window-forward), not snapshot-reconcile.
- `crates/smelt-logical/src/rules/cumulative.rs` — the derived settle bound matches the route (assert per route).
- Real fixture: a two-stage `examples/timeseries/` pipeline — keyed dedupe → downstream daily batched aggregate — where the downstream model receives pushdown (assert on the compiled SQL / `smelt explain`), i.e. the clock-sink is gone.

**Implementation shape.** Emit the admitted keyed output's `TimeseriesConfig` into the same source-shape/timeseries registry a declared source populates, so existing pushdown and driving-source resolution consume it unchanged. Add the settle bound to the explain surface.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — publish the output timeseries + settle bound.
- `crates/smelt-logical/src/analysis/*` — feed the model-output timeseries into the shape registry consumed by pushdown/driving-source resolution.

**Docs touched.**
- `docs/specs/keyed_models.md` — Known Divergences: output-as-clocked-source + settle bound built (clears the clock-sink divergence).
- `docs-site/docs/guide/` — document that a time-partitioned keyed output feeds downstream window-forward (Phase 8 prose).

**Review checklist.**
- [ ] Downstream batched gets pushdown; downstream keyed selects window-forward.
- [ ] Settle bound matches the route (route 2 = never settles).
- [ ] A re-written slice is treated as changed input by staleness (no stale downstream).
- [ ] Spec/docs edits are timeless.

**Commit.** `feat(keyed): publish time-partitioned keyed output as a clocked source; derive the settle bound`

---

### Phase 8: User docs — the time-partitioned keyed recipe

**Goal.** Document the feature end-to-end in the keyed-models guide: the two motivating pipelines (event-grain dedupe over a bounded window; daily/period enrichment), the three routes and when each applies, the `key_recurrence` declaration, and when to reach for `refresh: batched` instead (keyless/multiset bodies).

**Pre-conditions.** Phases 1–7 landed.

**TDD tests to write first.**
- `cargo test -p smelt-cli --test example_diagnostics` — the guide's worked examples exist as clean fixtures under `examples/timeseries/` (every code block that claims to compile does).
- Doc-link check (if the docs-site link checker runs in CI) passes for the new page.

**Implementation shape.** Write/extend the keyed guide page; ensure each SQL block is backed by a compiling fixture.

**Critical files.**
- `docs-site/docs/guide/` — the keyed-models guide (time-partitioned section).
- `examples/timeseries/` — the backing fixtures (already added incrementally in earlier phases; consolidate).

**Docs touched.**
- `docs-site/docs/guide/` — the recipe.
- `docs/specs/keyed_models.md` — References → User docs points at the new/updated guide page.

**Review checklist.**
- [ ] Every SQL block is backed by a compiling fixture.
- [ ] The three routes and the batched-instead guidance are documented as timeless feature description.
- [ ] Spec References → User docs updated.

**Commit.** `docs(keyed): time-partitioned keyed output guide — dedupe + enrichment recipes`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- Real-fixture: the 3-day dedupe model and the keyed→batched two-stage pipeline under `examples/timeseries/` build clean and match a full refresh (end-state equivalence harness).
- `cargo test -p smelt-cli --test example_diagnostics` — zero diagnostics on the keyed time-partitioned fixtures.
- `cargo test -p smelt-lsp --test example_workspaces` — same via the LSP backend.
- An injected out-of-bound redelivery trips `KeyedRecurrenceBoundViolated` (declared route) and rolls back.
- `cargo test` and `cargo clippy --all-targets` green.
- `/smelt:validate keyed_models` reports zero drift against §"Key temporal locality".
