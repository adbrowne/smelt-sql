# Plan: Accumulating snapshot refresh mode

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — Group D (new keyed refresh modes). This is the accumulating-snapshot member.
**Spec (oracle)**: [`docs/specs/accumulating_snapshot.md`](../specs/accumulating_snapshot.md) — PRIMARY, normative. Every phase is pinned to a spec section; the invariant oracle is the **once-write end-state equivalence contract** (§"Once-write end-state equivalence", Constraint 10) and the **fail-closed discipline** (Constraints 12–16).
**Also consumes**:
- [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — the structural template + the windowed-keyed step loop this mode shares (§"Cross-partition equivalence", execution model).
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis", §"Constraint violations".
- [`docs/specs/timeseries.md`](../specs/timeseries.md) — the driving-source declaration (partition column, granularity, source-lateness) the classifier reads.
- [`docs/specs/architecture.md`](../specs/architecture.md) — §"Layered single-ownership" (classifier is pure rule-data in `smelt-logical`), §"Backend primitives" (`merge_into`), §"Fail-loud discipline" (the hot-key cap).
**Spec diff**: the 2026-07-04 spec commit `d3d67ac8` (settled the open questions into normative decisions) on top of the initial draft. This plan cites that settled spec; it authors no new surface.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Engine dependency on Group B (partial).** The *derived* horizon path (P4) reads the `after_secs` half of
the source bound, landed by [`docs/plans/20260704-model-updates-group-b.md`](20260704-model-updates-group-b.md)
phase **B2**. Until B2 lands, `H` is **declaration-only** (on the source) and P4 ships only the declared
path — P1–P3 and P5 do **not** depend on B2 and can land first. The runtime injection layer already carries
`SourceBound { before_secs, after_secs }` (`crates/smelt-runtime/src/transformer.rs`), so no plumbing is new;
P4 wires the *derived* value into `after_secs` once B2 emits it.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan of
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) (Group D).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant oracle for every phase is the **once-write end-state equivalence contract**
   (`accumulating_snapshot.md` §"Once-write end-state equivalence", Constraint 10) and **fail-closed
   discipline** (Constraints 12–16). Every classifier admission must **fail closed**: an unprovable
   construct is refused at planning time, never silently downgraded.
2. Confirm you are on branch `worktree-incremental`.
3. **First action of every phase: `rg` for the identifier you are about to touch and confirm its current
   spelling and location.** This plan's file:line anchors were taken against the tree at `d3d67ac8`;
   Group A/B and the cumulative work may have shifted exact strings. In particular, verify where
   `classify_cumulative` / `CumulativeClassification` live *now* (imported from `smelt_planner` in
   `cumulative.rs` today, but the layering invariant puts pure rule-data classifiers in `smelt-logical`)
   — place the accumulating-snapshot classifier as their sibling, honouring
   `architecture.md` §"Layered single-ownership".
4. Find the next `pending` row in the Progress-tracking table. That is your phase. Honour its **Depends
   on** field. If every row is `done`, run §Verification, flip this plan's registry Status to `done`
   in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** consumer
phase needs a *full-refresh end-state-equivalence* test AND a *fail-closed* stays-rejected unit test) →
reviewer subagent (material findings only) → iterate → set the row `done` → commit + push with the
phase's `Commit.` line.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/`
edits describe the feature as if it always existed; as each phase lands, **narrow or remove** the
matching `accumulating_snapshot.md` §Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (the per-phase "Open decision"
callouts flag the known ones), or a pre-flight red unrelated to this phase's target: set the row
`blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit
`<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The `accumulating_snapshot` mode is a smelt-maintained **keyed-output** refresh mode for retroactive
enrichment: one row per key, milestone columns filled *once-write* as later facts arrive, combined
across driving-source windows via idempotent monoids (`LEAST`/`GREATEST`/`COALESCE`/max-by-ordering).
The design is fully settled in `docs/specs/accumulating_snapshot.md` (research
[`20260703-model-updates.md`](../research/20260703-model-updates.md) Part 20). The §20.9 sizing table is
the reality check: the windowed step loop and keyed `merge_into` **already ship** (via `cumulative`), so
this plan is largely *composition* — the genuinely new pieces are the once-write classifier (with the
`COALESCE` provenance prover), the horizon derivation, and the fail-loud hot-key cap.

**What already exists** (verified at `d3d67ac8`):
- `RefreshStrategy` enum with `Batched`/`Cumulative` (`crates/smelt-core/src/config.rs`); frontmatter
  forbid-validation pattern (`CumulativeForbidsBatched` in `crates/smelt-core/src/metadata.rs`).
- `crates/smelt-runtime/src/cumulative.rs` (457 lines) — the window-forward executor:
  `execute_cumulative_aggregate` → `generate_partitions` → per-partition `inject_source_filters` →
  create-or-`build_cumulative_merge_sql`. **This is the loop P2 generalises.**
- `SourceBound { partition_col, before_secs, after_secs }` and `inject_source_filters`
  (`crates/smelt-runtime/src/transformer.rs`) — injection is already `(before, after)`-shaped.
- `merge_into` on the `Backend` trait (`crates/smelt-backend/src/lib.rs`) + DuckDB/Spark impls.
- The cumulative classifier (`classify_cumulative`, `CumulativeClassification`, `AggregatorColumn`,
  `SourceTimeseriesMap`, `CumulativeDiagnostic`) — the accumulating-snapshot classifier's sibling.

## Scope

### In scope
- **P1** — Refresh-axis value + frontmatter forbid-validation + the 11 diagnostic codes (scaffold; no
  admission yet — a declared `accumulating_snapshot` model is *recognised* and its forbid/`GROUP BY`
  diagnostics fire, but full classification lands in P3).
- **P2** — Extract the shared **windowed-keyed-maintenance driver** from `cumulative.rs`, parameterised
  by `(classifier, merge-SQL builder)`; `cumulative` re-consumes it with zero behaviour change.
- **P3** — The once-write **classifier**: `unique_key` from `GROUP BY`, single-driving-source resolution,
  the combiner allowlist, the **`COALESCE` once-write provenance prover** (key-derived / source-declared
  FD), and every fail-closed diagnostic.
- **P4** — The **attribution horizon `H`**: declared-on-source path (always) + derived-from-predicate
  path (reads B2's `after_secs`); unbounded → `AccumulatingSnapshotUnboundedHorizon`.
- **P5** — The **merge-SQL builder + windowed execution + fail-loud hot-key cap**: once-write combiners
  into `merge_into`; the run-window clamp `[run_start − H, run_end]`; the per-run hot-key working-set cap.
- **P6** — Spec de-drift + `docs-site` user docs.

### Explicitly deferred (genuine deferrals, per spec §Known Divergences)
- **Settled-key GC / a hot-state store.** v1 keeps every key in the lookup and never GCs; only the
  per-run *working set* is capped (P5). A §14.4 space-budget GC needs a persistent-watermark store —
  out of scope.
- **`COALESCE` provenance breadth beyond the two provable forms** (tracing per-key constancy through
  CTEs/subqueries). P3 ships the key-derived + source-declared-FD prover and fails closed on the rest.
- **Non-determinism run-pinning.** v1 hard-rejects `NOW()`/`RANDOM()`; adopting `batched`'s B3
  run-pinning is a later alignment.
- **Granularity beyond the shared driver's** (`day`/`week` today) — a property of the shared driver
  (P2), widened there if ever, not here.
- **A per-model horizon override.** `H` is derived-or-source-declared only (Constraint 16).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| P1    | pending |        |      |
| P2    | pending |        |      |
| P3    | pending |        |      |
| P4    | pending |        |      |
| P5    | pending |        |      |
| P6    | pending |        |      |

---

### Phase P1: Refresh-axis value + frontmatter forbids + diagnostic scaffold

**Goal.** Make `refresh: accumulating_snapshot` a recognised value that no longer errors as
"unknown refresh", wire the two forbid-validations (`timeseries:` / `batched:`) and the `GROUP BY`
presence check, and register the 11 owned diagnostic codes (Surface §"Diagnostic codes"). Full milestone
classification is P3; P1 lands the value + the structural forbids + `AccumulatingSnapshotRequiresGroupBy`
/ `AccumulatingSnapshotForbidsTimeseries` / `AccumulatingSnapshotForbidsBatched`.

**Depends on.** — (foundation).

**TDD tests to write first.**
- `crates/smelt-core/tests/refresh_axis.rs` — `"accumulating_snapshot"` parses to
  `RefreshStrategy::AccumulatingSnapshot` and round-trips through serialize; an unknown value still
  errors (fail-loud).
- `crates/smelt-core/src/metadata.rs` unit — a model with `refresh: accumulating_snapshot` **and** a
  `timeseries:` block is a config error (`AccumulatingSnapshotForbidsTimeseries`); **and** a `batched:`
  block → `AccumulatingSnapshotForbidsBatched`. Mirror the existing `CumulativeForbidsBatched` test.
- `crates/smelt-core/src/metadata.rs` unit — `refresh: accumulating_snapshot` implies stored `table`
  (no `materialization:` restated), matching `Cumulative`.

**Implementation shape.**
- Add `AccumulatingSnapshot` to `RefreshStrategy` (`config.rs`), its `from_str`/`Serialize` arms, and
  `implies table` in the materialization resolution (mirror `Cumulative`).
- Add the forbid-validations in `metadata.rs` alongside the cumulative ones; add the diagnostic-code
  variants and their `map_metadata_error_to_diagnostic` arms (the exhaustiveness gate in
  `smelt-db/src/lib.rs` will force this — see CLAUDE.md §"MetadataError exhaustiveness gate").

**Critical files.**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy`, `get_refresh`, materialization implication.
- `crates/smelt-core/src/metadata.rs` — forbid-validations, `MetadataError` variants.
- `crates/smelt-db/src/lib.rs` — `map_metadata_error_to_diagnostic` exhaustive arms.

**Docs touched.**
- `docs/specs/accumulating_snapshot.md` §Known Divergences — narrow the "Not implemented" note (the
  refresh value + forbids now exist; classifier/execution still unbuilt). Do not delete.

**Review checklist.**
- [ ] `refresh: accumulating_snapshot` recognised; implies `table`; unknown values still fail-loud.
- [ ] `timeseries:`/`batched:` forbids fire with the owned diagnostics; exhaustiveness gate satisfied.
- [ ] No admission logic yet (P3) — a valid-shaped model still refuses with a "not yet classifiable"
      path, not a silent build.

**Commit.** `feat(core): recognise refresh: accumulating_snapshot + forbid timeseries:/batched: blocks`

---

### Phase P2: Extract the shared windowed-keyed-maintenance driver

**Goal.** Factor `cumulative.rs`'s step loop — classify → `generate_partitions` in temporal order →
per-partition `inject_source_filters(SourceBound)` → create-table-or-`merge_into` → sum results — into a
single reusable driver parameterised by `(classifier, merge-SQL builder)`, per
`accumulating_snapshot.md` Design §"One windowed executor". `cumulative` re-consumes the driver with
**zero behaviour change** (its existing tests are the regression net).

**Depends on.** — (independent of P1; can run in parallel, but sequence after P1 for a clean base).

**TDD tests to write first.**
- **Regression, not new behaviour.** The existing cumulative suite
  (`crates/smelt-cli/tests/incremental/…`, `crates/smelt-runtime` cumulative unit tests,
  `crates/smelt-cli/tests/merge_parity.rs`) must stay green after the extraction — that *is* the P2
  test. Add no accumulating-snapshot behaviour here.
- `crates/smelt-runtime/…` unit — the extracted driver, given cumulative's classifier + merge-SQL
  builder, produces byte-identical injected SQL and the same partition sequence as before (a
  characterization test pinning the refactor).

**Implementation shape.**
- Introduce a `WindowedKeyedDriver` (working name) in `smelt-runtime` that owns the loop skeleton and
  takes two collaborators: a classifier producing `{ driving_source, timeseries, unique_key, per-column
  combiner map, bounds }` and a merge-SQL builder producing the `CREATE TABLE AS` / `MERGE INTO` text
  from that classification + the per-window pushed SQL.
- Re-express `execute_cumulative_aggregate` as a thin caller of the driver with cumulative's classifier
  + `build_cumulative_merge_sql`.
- Keep `SourceBound { before_secs, after_secs }` as the per-source bound the driver injects; cumulative
  passes `(0, 0)` as today.

**Critical files.**
- `crates/smelt-runtime/src/cumulative.rs` — `execute_cumulative_aggregate` (`:34`),
  `build_cumulative_merge_sql` (`:218`), `generate_partitions` (`:256`), `single_partition_range`
  (`:300`) → moved/generalised into the new driver module.
- `crates/smelt-runtime/src/transformer.rs` — `inject_source_filters`, `SourceBound`, `TimeRange`
  (unchanged; the driver calls them).
- `crates/smelt-runtime/src/lib.rs` — module wiring.

**Open decision (for the implementer).** *Where the driver's `(classifier, merge-builder)` seam is
typed* — a trait vs two `Fn` params. Prefer a small **trait** (`WindowedKeyedRule`) so `cumulative` and
`accumulating_snapshot` are named impls and `latest_value`/`versioned` can join later; record the seam
in the commit.

**Docs touched.**
- None user-facing (internal refactor). Verify `cumulative_aggregate.md` execution-model prose still
  matches (it should — behaviour is unchanged).

**Review checklist.**
- [ ] Cumulative suite fully green; injected SQL + partition sequence byte-identical (characterization).
- [ ] Driver seam is `(classifier, merge-builder)`; no accumulating-snapshot logic leaked in yet.
- [ ] `SourceBound` still `(before, after)`; cumulative passes `(0,0)`.

**Commit.** `refactor(runtime): extract shared windowed-keyed-maintenance driver from the cumulative executor`

---

### Phase P3: The once-write classifier + COALESCE provenance prover

**Goal.** Build the pure classifier (`smelt-logical`, per the layering invariant) that turns an
`accumulating_snapshot` model's expanded outer SELECT into `{ unique_key, milestone_columns:
name→(aggregator, combiner), driving_source }` and rejects every non-conforming construct fail-closed,
per `accumulating_snapshot.md` §"Classifier checks", §"Driving source", Surface §"Milestone combiner
allowlist". Includes the **`COALESCE` once-write provenance prover** (key-derived / source-declared FD).

**Depends on.** P1 (refresh value + diagnostics). Sibling of the cumulative classifier.

**TDD tests to write first.**
- `smelt-logical` classifier unit — the Surface example (`MIN`/`MAX`/`MAX_BY` milestones, single
  timeseries source, `GROUP BY event_id`) classifies to the expected `unique_key` + combiner map.
- Fail-closed units, one per rejection (each names the offending construct):
  no `GROUP BY` (`AccumulatingSnapshotRequiresGroupBy`); non-allowlisted aggregate
  (`AccumulatingSnapshotUnknownCombiner`); a composite-over-aggregate projection (rejected);
  `GROUP BY` contains the driving `partition_column`
  (`AccumulatingSnapshotGroupByContainsPartitionColumn`); 0 timeseries sources
  (`AccumulatingSnapshotNoDrivingSource`); ≥2 (`AccumulatingSnapshotMultipleDrivingSources`);
  a fact-to-dimension join (`AccumulatingSnapshotJoinExpressedEnrichment`); `NOW()`/`RANDOM()`
  (`AccumulatingSnapshotForbidsNondeterministic`).
- **`COALESCE` provenance units** — a `COALESCE`-first-non-null whose argument is **key-derived**
  (function of the `GROUP BY` key) is **admitted**; one backed by a **source-declared FD** `key→col` is
  **admitted**; an arbitrary `COALESCE(<free_col>)` with no provable per-key constancy is
  **rejected** (`AccumulatingSnapshotCorrectableMilestone`), naming the column. `MIN`/`MAX`/`MIN_BY`/
  `MAX_BY` are **never** rejected by this check (regression assertion).
- `crates/smelt-db/…` integration — classification runs on the **expanded** CST (function expansion
  before the classifier, §"Functions inside bodies"); a `smelt.define`-resolved milestone aggregator
  admitted iff its expanded body is an allowlisted combiner at the outermost position.

**Implementation shape.**
- Model the classifier on `classify_cumulative` (same driving-source resolution + `SourceTimeseriesMap`
  input), swapping the aggregator algebra for the once-write allowlist (`MIN→LEAST`, `MAX→GREATEST`,
  `COALESCE→COALESCE`, `MAX_BY/MIN_BY→max/min-by-ordering`).
- The `COALESCE` prover: admit iff (a) the argument's leaves are all functions of the `unique_key`
  columns, or (b) the driving source declares a functional dependency `unique_key → column`. Otherwise
  `AccumulatingSnapshotCorrectableMilestone`. `MIN/MAX/*_BY` skip the prover (unconditional monoids).
- Join-expressed enrichment detection: a FROM with a second events-like relation joined on the key (not
  a keyed union over one driving stream) → `AccumulatingSnapshotJoinExpressedEnrichment`.

**Critical files.**
- `crates/smelt-logical/src/rules/` — new `accumulating_snapshot` classifier module (pure rule-data);
  verify against where `classify_cumulative` currently lives and match the layer.
- `crates/smelt-logical/src/analysis/` — the once-write provenance prover (key-derivation walk;
  source-FD lookup).
- `crates/smelt-core/src/metadata.rs` / `timeseries.rs` — surface the source-declared FD the prover
  reads (if not already present, add it as a `timeseries:`/source declaration read — **flag if this
  needs a source-side surface addition**, which would be a spec touch).

**Open decision (for the implementer).** *Whether the source-declared FD `key→column` already has a
surface.* If `timeseries.md`/source config has no functional-dependency declaration today, form (b) of
the `COALESCE` prover has no input — ship **form (a) only** (key-derived) and record form (b) under
§Deferred, or block pending a source-FD surface decision. Do **not** invent a source surface silently;
form (a) alone still admits the common case (first-touch derived from the key).

**Docs touched.**
- `docs/specs/accumulating_snapshot.md` §Known Divergences — narrow the "Not implemented" note
  (classifier now exists); if form (b) is deferred, note that `COALESCE` admission is form-(a)-only for
  now under the existing provenance-breadth divergence.

**Review checklist.**
- [ ] All rejections fail closed and name the construct; `MIN/MAX/*_BY` never hit the once-write prover.
- [ ] `COALESCE` admitted only via a provable form; unprovable → `AccumulatingSnapshotCorrectableMilestone`.
- [ ] Single-driving-source resolution matches cumulative's; runs on the expanded CST.
- [ ] Classifier is pure rule-data in the correct layer (architecture.md §Layered single-ownership).

**Commit.** `feat(logical): accumulating_snapshot once-write classifier + COALESCE provenance prover`

---

### Phase P4: The attribution horizon `H`

**Goal.** Resolve `H` per `accumulating_snapshot.md` §"The attribution horizon": **declared** on the
driving source (always available, default 0) and **derived** from a `BETWEEN col AND col + INTERVAL`
forward predicate (reads B2's `after_secs`). Form the run-window clamp `[run_start − H, run_end]`. An
unbounded horizon → `AccumulatingSnapshotUnboundedHorizon`. No per-model override (Constraint 16).

**Depends on.** P3 (classifier structures). The **derived** path additionally depends on Group B **B2**
(`after_secs` walk in `source_bounds.rs`); the **declared** path does not.

**TDD tests to write first.**
- `smelt-logical` unit — a source with a declared lateness `H` produces that clamp; default 0 → clamp
  `[run_start, run_end]`.
- `smelt-logical` unit (derived) — the Surface example's `conversion_ts BETWEEN event_ts AND event_ts +
  INTERVAL '30 days'` derives `H = 30 days` via `after_secs` (**gated on B2**; if B2 not yet landed,
  this test is `#[ignore]`d with a note and P4 ships declared-only — record under §Deferred).
- `smelt-logical` unit (fail-closed) — no forward predicate and no source-lateness declaration →
  `AccumulatingSnapshotUnboundedHorizon`, naming the missing bound.

**Implementation shape.**
- Read the source-lateness declaration from the driving source's `timeseries:`/source config; when a
  forward `BETWEEN … + INTERVAL` predicate is present on the driving source, take `H = after_secs` from
  `source_bounds.rs` (B2). Prefer the derived value where both exist? — **derived wins** (derive-don't-
  declare), matching Surface §CLI; record this in the commit.
- Compute the clamp as a pure function of `(run_window, H)` — never data-dependent (Constraint 8).

**Critical files.**
- `crates/smelt-logical/src/analysis/source_bounds.rs` — `after_secs` (B2); the horizon read.
- `crates/smelt-logical/src/rules/` — the accumulating-snapshot classifier's horizon field.

**Open decision (for the implementer).** *Derived-vs-declared precedence when both are present.* Spec
says derived is preferred (§"The attribution horizon" resolution order). Ship derived-wins; if a source
declares a *larger* lateness than the predicate reach, flag it (the predicate bounds computation reach,
the declaration bounds world-lateness — they can legitimately differ). Record the chosen rule.

**Docs touched.**
- `docs/specs/accumulating_snapshot.md` §Known Divergences — if the derived path lands (B2 ready),
  **remove** the "Derived `H` waits on the `after_secs` walk" note; else narrow it to "declared path
  shipped; derived path waits on B2".

**Review checklist.**
- [ ] Declared path always works (default 0); derived path reads `after_secs` when B2 present.
- [ ] Unbounded horizon refused fail-closed; clamp is a pure function of `(run_window, H)`.
- [ ] No per-model override surface introduced.

**Commit.** `feat(logical): derive/declare the accumulating_snapshot attribution horizon + run-window clamp`

---

### Phase P5: Merge-SQL builder + windowed execution + fail-loud hot-key cap

**Goal.** Complete the execution path: a merge-SQL builder emitting the once-write `MERGE INTO`
(matched: per-column combiner; unmatched: insert) or the first-window `CREATE TABLE AS`, driven by P2's
shared driver over the clamped run window, with the **fail-loud per-run hot-key working-set cap**
(`accumulating_snapshot.md` §"Execution model", §"The hot-key set and its space cap", Constraint 13).

**Depends on.** P2 (driver), P3 (classifier), P4 (horizon/clamp).

**TDD tests to write first.**
- `crates/smelt-runtime/…` unit — `build_accumulating_snapshot_merge_sql` emits `LEAST(target.c,
  delta.c)` / `GREATEST(…)` / `COALESCE(…)` / max-by-ordering per the combiner map; first window with no
  target table emits `CREATE TABLE AS SELECT`.
- **End-state-equivalence real fixture** (`crates/smelt-cli/tests/…`, an enrichment model under
  `examples/timeseries/`) — running the driving-source windows **in order**, **reversed**, and **with a
  re-run/overlap** all converge to the **same** stored table, and that table equals a **full refresh**
  over the same window set (§"Once-write end-state equivalence", the core contract).
- Fail-loud unit — a run whose delta would touch more keys than the cap **errors** with a message
  steering to a narrower window / full refresh (§"space cap"); a run under the cap succeeds.

**Implementation shape.**
- Add the accumulating-snapshot merge-SQL builder as the driver's merge collaborator; combiners are the
  fixed lookup from the classification's per-column map.
- Wire `execute_accumulating_snapshot` as a thin caller of P2's driver with the P3 classifier + this
  builder + P4's clamped window.
- The hot-key cap: after the per-window delta SELECT, assert the delta's distinct-key count ≤ cap;
  exceed → `anyhow::bail!` with the steering message. **Do not** silently truncate.

**Critical files.**
- `crates/smelt-runtime/src/` — the new merge-SQL builder + `execute_accumulating_snapshot`; the driver
  from P2.
- `crates/smelt-backend/src/lib.rs` / `crates/smelt-backend-duckdb/src/lib.rs` — `merge_into` (reused;
  verify the once-write combiner SQL renders on DuckDB).
- `crates/smelt-cli/src/run.rs` / `crates/smelt-runtime/src/execute.rs` — dispatch the new refresh mode
  to `execute_accumulating_snapshot` (mirror the cumulative dispatch; note the two parallel
  incremental paths per memory `project_incremental_execution_paths`).

**Open decision (for the implementer).** *The cap default + tunability.* Spec leaves the concrete value
open (§Known Divergences). Ship a conservative default (e.g. a large fixed bound) that errors loudly;
decide whether to expose an operator override now or defer — record under §Deferred. Do not make the cap
a *silent* soft limit.

**Docs touched.**
- `docs/specs/accumulating_snapshot.md` §Known Divergences — narrow the "Not implemented" note
  (execution now ships); pin the chosen cap default in the "hot-key cap default is unspecified" note.
- `docs-site/docs/guide/materializations.md` — begins in P6.

**Review checklist.**
- [ ] Any-order / overlap / re-run all converge and match full refresh (equivalence contract).
- [ ] Once-write combiners render correctly on DuckDB; first window creates, rest merge.
- [ ] Hot-key cap errors loudly (never silent truncation); dispatch wired for both execution paths.

**Commit.** `feat(runtime): windowed once-write merge execution for accumulating_snapshot + fail-loud hot-key cap`

---

### Phase P6: Spec de-drift + user docs

**Goal.** Bring `accumulating_snapshot.md` §Known Divergences to its shipped state and add the
user-facing documentation. Per `/smelt:validate accumulating_snapshot`, zero drift on the shipped
surface.

**Depends on.** P1–P5.

**TDD tests to write first.**
- `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test
  example_workspaces` — an `accumulating_snapshot` example workspace builds with **zero diagnostics**.
  Add a small enrichment example under `examples/timeseries/` (event + conversion keyed stream).

**Implementation shape.**
- `docs/specs/accumulating_snapshot.md` §Known Divergences — remove the "Not implemented" note; leave
  only the genuine deferrals (settled-key GC, `COALESCE` provenance breadth, run-pinning alignment,
  granularity, and — if still unlanded — the derived-`H`/B2 dependency). Update §References Code anchors
  to the real shipped locations (the shared driver's final module path).
- `docs-site/docs/guide/materializations.md` — document `refresh: accumulating_snapshot` on the refresh
  axis alongside `batched`/`cumulative`: the once-write milestone pattern, the combiner allowlist, the
  bounded horizon, the keyed-union (not join) modelling rule, and the fail-loud cap.

**Docs touched.** (this phase is docs.)

**Review checklist.**
- [ ] Example workspace builds with zero diagnostics.
- [ ] §Known Divergences reflects shipped state; only genuine deferrals remain; timeless-oracle intact.
- [ ] `docs-site` materializations page documents the mode.
- [ ] `/smelt:validate accumulating_snapshot` reports zero drift on the shipped surface.

**Commit.** `docs(accumulating_snapshot): de-drift the spec + document the mode on the refresh axis`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only.)

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Once-write end-state equivalence, the load-bearing contract.** P5's real-fixture test asserts
  any-order / overlap / re-run convergence **and** full-refresh equivalence over the same window set
  (`accumulating_snapshot.md` §"Once-write end-state equivalence", Constraint 10). This is the master
  §"Post-implementation verification" oracle for this mode.
- **Fail-closed, per consumer phase.** P3–P5 each carry a stays-rejected unit test naming the construct
  (Constraints 12–16): unknown combiner, correctable `COALESCE`, join-expressed enrichment, unbounded
  horizon, and an over-cap run all refuse loudly.
- The cumulative suite stays green throughout (P2's extraction is behaviour-preserving).
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test
  example_workspaces` — the enrichment example builds with zero diagnostics.
- `/smelt:validate accumulating_snapshot` reports zero drift for the shipped surface; every
  §Known-Divergence note this plan lists as removed is gone from the spec.
