# Plan: Model updates — L4 composition for `refresh: batched`

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L4 mode-composition** layer for the `batched` member of the refresh axis. It **wires the fundamentals L1/L2 capabilities** ([`docs/plans/20260704-model-updates-fundamentals.md`](20260704-model-updates-fundamentals.md)) into the batched surface and re-expresses the **batched-local residue**.
**Specs (oracles)**:
- [`docs/specs/batched_models.md`](../specs/batched_models.md) — PRIMARY. §Surface → **§"Composition"** (the composition table is the charter of this plan: {properties required, world-facts consumed, transforms driven, output shape} + the batched-**local** machinery enumerated after it). §Semantics: "Execution model", "Run window vs partition granularity", "Batch safety classification", "First-run and backfill" (backfill chunking), "Per-partition equivalence" (+ column-locality), "Safety checks", "Non-determinism and the payload rule", "Event-time outer-visibility", "Observing the per-source clamp", "Window independence and self-referential models". §Constraints & Invariants 1–12. §Known Divergences.
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — the framework batched composes. §"The equivalence invariant" (**one** invariant; `batched` is **partition-addressed** — identity-free, whole-partition DELETE+INSERT; **per-partition equivalence is a *strengthening*** of the one invariant, available because each output slice is partition-local); §"Windowed maintenance and the horizon" (maintenance is **windowed by default** — batched is clocked; the **horizon is derived**, a declared value is a warning ceiling only, a late arrival beyond the horizon is silently clamped not diagnosed); §"Validator, not chooser" (no silent downgrade to full-refresh).
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — the properties batched *requires*, cited by exact §"Derived proofs" row name: **event-time monotonicity trace**, **column nullability gate**, **unified bound / reach derivation**, **maintained-window / horizon derivation**, **injection-point / pushdown-depth**, **frame-reach taxonomy**, **partition alignment (scoped)**, **driving-fact / anchor resolution**, **determinism (run vs row) + nondeterminism predicate**, **body-structure classifier**, **set-operation distribution**, **static-seed detection**, **window-independence / ordered-execution**. Model-scoped declaration: **`nondeterministic_columns`**.
- [`docs/specs/model_transforms.md`](../specs/model_transforms.md) — the transforms batched *drives*, cited by exact §Surface row name: **source-filter pushdown (window-an-input)**, **partition DELETE+INSERT**, **outer output-clamp**, **two-layer widened-scan + exact output clamp**, **UNION-branch wrap-and-filter**, **compile-time pinning**.
- [`docs/specs/timeseries.md`](../specs/timeseries.md) — §"Granularity values", §"Granularity arithmetic", partition-column projection & monotonicity.
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis", §"Constraint violations", §"Input-consumption axis".
**Research (the "why" + the L-decomposition)**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Target plan architecture (the re-cut master)" (L0–L4; this sub-plan is **L4/batched**) and §"Mapping the current master onto the layers" (the row-by-row re-home of Group B: B0/B1/B2 → L1/L2 fundamentals; B3 taint / B4 alignment-consumer / B5 granularity / B6 self-referential / B7 integer-key / B8 observability → this L4/batched-local layer). Also `docs/research/20260703-model-updates.md` Parts 3, 5–11, §18.3.
**Spec diff**: none new — every surface this plan lands was made normative by the 2026-07-04 spec reshape. Each phase **wires** a shared capability (already spec'd in `model_properties.md`/`model_transforms.md`) into the batched composition, or ships a **batched-local** mechanism the composition table names; as a phase lands it **removes or narrows** the matching `batched_models.md` §Known-Divergence note. No phase authors a spec.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary — this supersedes Group B (read first).** This sub-plan **replaces** the mode-vertical
[`docs/plans/20260704-model-updates-group-b.md`](20260704-model-updates-group-b.md). Group B's B0 (filter-placement
classifier + unified bound derivation), B1 (monotonicity-primitive consumers: UNION / subquery-CTE / join
driving-fact) and B2 (bounded-`RANGE` frames + the `LAG`/`LEAD` two-layer widened-scan + the symmetric
`after_secs` forward-reach walk) are **no longer batched-local** — the fundamentals-first re-cut homes them in
L1/L2 (`docs/research/20260704-maintenance-fundamentals.md` §"Mapping the current master onto the layers"):
B0 = the **unified bound / reach derivation** consolidation (fundamentals **F1**) + the **injection-point /
pushdown-depth** and **two-layer widened-scan + exact output clamp** redesign (fundamentals **F13**); B1's
driving-fact resolution = **F2**, its body-structure / set-op / static-seed pieces are shared proofs; B2's
frame-reach + widened-scan = **F1/F13**. This plan **cites those F-phases as prerequisites and does not
re-plan them.** What remains L4/batched-local — the **batch-safety roll-up**, **event-time outer-visibility**,
**backfill chunking**, the **`nondeterministic_columns` payload taint + compile-time pinning** composition
(B3 residue), the **group-aligned `HAVING`/`DISTINCT`** consumer of scoped partition alignment (B4 residue),
**run/partition granularity alignment** (B5), **self-referential ordered execution** (B6),
**monotone-integer partition keys** (B7), and **per-source clamp observability** (B8) — is what this plan
re-cuts, each pinned to per-partition equivalence and each fail-closed. Group C (keyed rungs) and Group D
(new keyed modes) are untouched — this plan is only the `batched` (partition-addressed) member.

**Oracle reframe (2026-07-04 — read before BL1).** `model_maintenance.md` and `batched_models.md` were
reframed by L0; re-read them:
- **One invariant; batched is partition-addressed.** There is a single invariant
  (`incremental == full refresh over processed inputs`). `batched`'s output is **partition-addressed** —
  identity-free, each source partition maps to an output partition rewritten wholesale (DELETE+INSERT), no
  row identity needed. **Per-partition equivalence is a *strengthening*** of the one invariant that batched
  enjoys because each output slice is partition-local; it is **not** a peer of a separate "end-state"
  contract. Every batched phase's oracle is therefore per-partition equivalence *as the strengthening of the
  one invariant* — never approximate, refused with a diagnostic when a construct breaks it.
- **Windowed by default; horizon derived.** Batched is clocked (`timeseries:`), so maintenance is windowed:
  only `[run_start − before, run_end + after)` per source is scanned and the write is **clamped** to the
  exact write window (the two-layer scan ⊇ write split, **F13**). The horizon (`after` forward reach) is
  **derived** from the model's reach, never trusted from a declaration; a late arrival beyond it is silently
  clamped, not diagnosed (surfacing lateness is a model-author + data-check concern).
- **Validator, not chooser.** A model the batch-safety classifier rejects, or whose bound derivation is
  `NotDerivable`, is **refused at planning time** (`BatchedNotSafe`), never silently downgraded to
  full-refresh (Constraint 10).

**Prerequisite maturity note (honest).** The DuckDB DELETE+INSERT batched path — the batch-safety roll-up,
source-filter pushdown, the outer clamp, and backfill chunking — is **already built and tested today**
(`batched_models.md` status). This L4 layer's early phases (BL1, BL2) **re-express that shipped behaviour on
the consolidated fundamentals substrate** (F1's single `BoundResult` walk replaces the two parallel
derivations; F13's read-margin/write-clamp split replaces the over-widened write window) — they are
**refactor-and-wire** phases whose acceptance gate is *no regression* of the existing batched equivalence
harness. The later phases (BL3–BL8) admit genuinely new forms (payload non-determinism, group-aligned
`HAVING`/`DISTINCT`, granularity validation, ordered self-edges, integer keys, observability). Do not treat
BL1/BL2 as greenfield.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when this
L4/batched layer is scaffolded into the registry, **superseding** the Group B row — the loop never scaffolds
it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The invariant
   oracle for every phase is **per-partition equivalence as the strengthening of the one processed-input
   equivalence invariant** (`model_maintenance.md` §"The equivalence invariant"; `batched_models.md`
   §"Per-partition equivalence", Constraint 6). Every relaxation only *widens* what is admitted and must
   **fail closed** (`batched_models.md` Constraint 10 "No silent downgrade", Constraint 12; every shared
   proof it wires is fail-closed in `model_properties.md` §Constraints).
2. Confirm you are on branch `worktree-incremental`, that **Group A** (A1/A2, `done`) is landed, and that the
   **fundamentals F-phase(s)** this phase's *Depends on* field names are `done` in
   `20260704-model-updates-fundamentals.md`'s Progress table. **Do not start a phase whose F-prerequisite is
   still `pending`** — it would re-derive a capability the fundamentals layer owns. If the F-phase is not yet
   landed, set this row `blocked` (reason: "awaits F<n>") and move on.
3. **First action of every phase: `rg` for the identifier you are about to touch and confirm its current
   spelling.** The fundamentals consolidations (F1 merged `temporal.rs` into `source_bounds.rs` and unified
   the interval parser + orchestration; F2 unified the driving-fact resolver; F3 the nondeterminism predicate;
   F5 the scoped alignment signal; F13 redesigned the runtime clamp) will have **moved or renamed** the
   file:line anchors below — they were taken against the pre-fundamentals tree. Confirm before editing.
4. Find the next `pending` row in the Progress-tracking table below. Honour its **Depends on** field. If every
   row is `done`, run §Verification, flip this sub-plan's registry Status to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this phase's
own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names a
**fail-closed reject test** — an ineligible construct refused with a diagnostic — AND every phase carries a
**"must not regress batched equivalence"** note) → reviewer subagent (material findings only) → iterate → set
the row `done` → commit + push with the phase's `Commit.` line.

**The batched per-partition equivalence harness is the regression net.** Do **not** re-implement the
monotonicity primitive (`crates/smelt-logical/src/analysis/monotonicity.rs`, `trace_event_time`) or the
shared proofs — the fundamentals layer owns them; this layer *composes* them. Every phase keeps the shipped
batched full-refresh-equivalence tests (`crates/smelt-cli/tests/incremental_*.rs`,
`.../incremental/backfill.rs`) green; a phase that flips one is a bug in the wiring, not a spec change — do
not edit the equivalence expectations to match new output.

**Equivalence-harness tests need DuckDB.** Every phase that changes emitted SQL or execution shape
(BL1–BL7) asserts per-partition / full-refresh equivalence via the DuckDB harness; that requires
`DUCKDB_LIB_DIR` set (and `LD_LIBRARY_PATH`) per `CLAUDE.md`. Pure classifier-wiring assertions are
`smelt-logical` unit tests with no DuckDB dependency.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits
describe the feature as if it always existed; as each phase lands, **remove/narrow** the matching
`batched_models.md` §Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (the per-phase "Open decision" callouts
flag the known ones), an F-prerequisite still `pending`, or a pre-flight red unrelated to this phase's target:
set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit
`<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape recast batched as a **composition** (`batched_models.md` §"Composition"): a table
of the shared **properties** it requires, the **world-facts** it consumes, the **transforms** it drives, and
its **output shape** (partitioned), plus the machinery that is batched-*local*. The fundamentals sub-plan
(L1 proofs + L2 transforms) builds and consolidates the shared capabilities; **this L4 sub-plan wires them
into the batched surface** and re-expresses the residue the composition table names as batched-local. It
**supersedes Group B**, whose mode-vertical decomposition mixed shared capabilities (now F-phases) with
batched-local machinery. Where Group B derived a bound, resolved a driving fact, or classified partition
alignment privately, this layer *reads the fundamentals verdict*. What is left here is exactly the
batched-*local* content: the three-class **batch-safety roll-up** of the per-source bound map, **event-time
outer-visibility**, **backfill chunking** from the class, the `nondeterministic_columns` **payload taint +
run-clock pinning** composition, the group-aligned `HAVING`/`DISTINCT` **consumer** of scoped alignment,
**run/partition granularity alignment**, **self-referential ordered execution**, **monotone-integer partition
keys**, and **per-source clamp observability**.

## Scope

### In scope (L4/batched-local)

- **BL1** — Batched composition core: wire F1's unified per-source `BoundResult` map into the batched-local
  **batch-safety roll-up** (`FullyBatchSafe`/`BoundedSafe(n)`/`PerPartitionOnly`) and the
  `NotDerivable → BatchedNotSafe` refusal; drive the four transforms (source-filter pushdown, partition
  DELETE+INSERT, outer output-clamp, two-layer widened-scan + exact clamp from F13) off it; **drop the outer
  clamp on the transparent slice**; finalize **event-time outer-visibility**
  (`EventTimeColumnNotVisibleAtOuterSelect`) on set-operation distribution + static-seed.
- **BL2** — **Backfill chunking** from the batch-safety class (single pair / auto-sized 3× clamped 7–90 /
  per-partition; calendar-aligned Month/Quarter/Year; per-chunk transaction boundary), the batched-local
  transform.
- **BL3** — **Non-determinism composition** (B3 residue): wire F3's run-vs-row nondeterminism predicate into
  the batched **payload taint flow** with the three hard exclusions, and the model-scoped
  `nondeterministic_columns` declaration; ship the batched-local **compile-time pinning** of run-deterministic
  clocks (`NOW`/`CURRENT_*`).
- **BL4** — **Group-aligned `HAVING` / `DISTINCT`** (B4 residue): consume F5's scoped partition-alignment
  signal to admit `HAVING` when `GROUP BY ⊇ partition_column` and `DISTINCT` when its key ⊇ `partition_column`;
  keep `LIMIT` unconditionally rejected.
- **BL5** — **Run-window ↔ partition-granularity alignment** (`g_run ≥ g_part`, aligned boundaries; B5).
- **BL6** — **Monotone-integer partition keys** (non-temporal `partition_column`: sequence id / offset /
  watermark; B7), the batched admission + integer clamp/lookback on F1's generalised `Offset`.
- **BL7** — **Self-referential ordered execution** (B6): derive the ordered property from F10's window-
  independence / self-edge proof and constrain the BL2 chunker to strictly-sequential temporal backfill;
  refuse a non-converging (forward / whole-history) self-reference.
- **BL8** — **Per-source clamp observability** (B8): run-relative `explain --json` scan window + LSP hover
  readout, rendered from F1's bound map (ISO-8601 for temporal, integer for BL6 keys).

### Explicitly deferred

- **Keyed modes** — Group C (maintenance rungs) and Group D (`latest_value`/`versioned`/`materialized_view`).
  This plan touches only the partition-addressed `batched` member.
- **Batched Open Questions not yet settled** (research §18.2): scalar subqueries over bounded sources,
  `GROUPING SETS`/`ROLLUP`/`CUBE`, membership/grouping non-determinism, aggregating-branch unions. Each stays
  **rejected (fail-closed)**; none is a phase.
- **Deeper flow/taint indirection** (BL3): the direct-projection taint check ships first; following a
  non-deterministic value through CTEs/subqueries is deferred (fail-closed on any unresolvable indirection).
- **CTE-only `event_time_column` non-visibility** (`batched_models.md` §Known Divergences): a CTE alias that
  does not project `event_time_column` is tracked separately (`20260616-smelt-feedback-fixes.md`); BL1 closes
  the direct-subquery/set-operation cases only.
- **Function-body `OVER` invisibility** (`batched_models.md` §Known Divergences): the window-safety scan of
  unexpanded outer SQL is tracked in `20260530-thread-fn-registry-classification.md`; out of scope here.
- **Per-column `data_latency`** late-data automation — deferred; the two interim mitigations remain the only
  options.

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| BL1 | F1, F13, Group A (A1) | `batched_models.md` §"Composition", §"Batch safety classification", §"Execution model", §"Event-time outer-visibility" | done |
| BL2 | BL1 | `batched_models.md` §"First-run and backfill"; `model_transforms.md` §"Transforms that stay in a mode spec" | done |
| BL3 | F3, Group A (A1) | `batched_models.md` §"Non-determinism and the payload rule", Constraint 12; `model_transforms.md` "Compile-time pinning" | done |
| BL4 | F5 | `batched_models.md` §"Safety checks" (`HAVING`/`DISTINCT`/`LIMIT`) | done |
| BL5 | F1, Group A (A1) | `batched_models.md` §"Run window vs partition granularity"; `timeseries.md` §"Granularity arithmetic" | done (2026-07-05) |
| BL6 | F1 | `batched_models.md` §Surface (monotone integer `partition_column`), §"Observing the per-source clamp" | pending |
| BL7 | F10, BL2 | `batched_models.md` §"Window independence and self-referential models" | pending |
| BL8 | F1, BL1, BL6 | `batched_models.md` §"Observing the per-source clamp" | pending |

---

### Phase BL1: Batched composition core — batch-safety roll-up + transforms on the unified bound map; drop the outer clamp on the transparent slice; event-time outer-visibility

**Goal.** Re-express the batched DELETE+INSERT path as a **composition** on the fundamentals substrate. Roll
F1's unified per-source `BoundResult` map into the batched-local three-class **batch-safety** verdict
(`FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly`), refuse a `NotDerivable` source at planning time
(`BatchedNotSafe`, no silent downgrade), and drive the four transforms off one derivation: **source-filter
pushdown**, **partition DELETE+INSERT**, **outer output-clamp**, and F13's **two-layer widened-scan + exact
output clamp**. **Drop the outer clamp on the transparent slice** (exactly one timeseries source, zero-margin
`Bounded(_, 0, 0)`) — the per-source pushdown filter *is* the clamp (Injection-point / pushdown-depth
property); keep it for a genuine lookback margin or multiple timeseries sources. Finalize **event-time
outer-visibility**: reject `EventTimeColumnNotVisibleAtOuterSelect` for a plain `UNION`/`INTERSECT`/`EXCEPT`
or a subquery FROM that hides `event_time_column`, exempting a `UNION ALL` whose every branch traces
`Traceable` (set-operation distribution + static-seed).

**Spec anchor.** `batched_models.md` §"Composition" (this is the composition-table wiring); §"Batch safety
classification" (the roll-up + `BatchedNotSafe`); §"Execution model" (the four transforms; the transparent-slice
drop, step 2); §"Event-time outer-visibility" + Constraint 11. Transforms: `model_transforms.md`
"Source-filter pushdown", "Partition DELETE+INSERT", "Outer output-clamp", "Two-layer widened-scan + exact
output clamp". Properties: `model_properties.md` "Unified bound / reach derivation", "Injection-point /
pushdown-depth", "Set-operation distribution", "Static-seed detection".

**Pre-conditions.** Group A (A1/A2) landed. **F1** (unified bound/reach — one `BoundResult` walk, one interval
parser, one orchestration entry) and **F13** (two-layer widened-scan / exact output clamp redesign) landed.
Consumes the W1 primitive read-only.

**Depends on.** F1, F13, A1.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — the batch-safety roll-up reads F1's `BoundResult` map:
  all-`Bounded(_,0,0)` → `FullyBatchSafe`; any `Bounded` with `before+after>0` → `BoundedSafe(n)` with
  `n = max(before+after)`; any `Unbounded` → `PerPartitionOnly`; any `NotDerivable` → refused
  (`BatchedNotSafe`, naming the construct — **fail-closed reject test**).
- `crates/smelt-logical/src/rules/incremental.rs` unit — a transparent single-source subquery
  (`Bounded(_,0,0)`, single timeseries source) emits **one** source-level filter and **no** outer wrap; a
  lookback margin or a second timeseries source keeps the outer clamp (assert the injection shape).
- `crates/smelt-logical/src/rules/incremental.rs` unit (**fail-closed reject**) — a plain `UNION` (or a
  subquery FROM not projecting `event_time_column`) is rejected `EventTimeColumnNotVisibleAtOuterSelect`; a
  `UNION ALL` whose every branch traces `Traceable` is exempt; a `StaticSeed` branch is named and rejected.
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental_*.rs` +
  `.../incremental/backfill.rs` (real `examples/timeseries/` fixtures) — the shipped batched per-partition /
  full-refresh equivalence tests stay green after re-expressing the roll-up + injection on F1/F13. Requires
  `DUCKDB_LIB_DIR`.

**Implementation shape.** Replace the batched batch-safety derivation (the text scans in `analyze_batch_safety`
and the parallel outer-clamp derivation) with a roll-up over F1's `BoundResult` map — one map in, one class +
one injection plan out. Drive pushdown / DELETE+INSERT / outer-clamp / F13's widened-scan off that single plan;
the transparent-slice path drops the outer wrap (F13's `is_transparent_single_source` fast path is the emit
side). Route event-time outer-visibility through the set-operation-distribution + static-seed verdicts rather
than re-classifying branches privately. Keep `NotDerivable → BatchedNotSafe` refusal exactly as today.

**Open decision (for the implementer).** *What carries late-data re-writes after the transparent-slice drop.*
Narrowing the output clamp on the transparent slice adopts an exact source clamp; the widened **DELETE write
window** (`filter_range`, `crates/smelt-cli/src/temporal.rs`) must still cover the late-data re-write window.
**Prefer**: narrow only the *output clamp*, keep the DELETE write-window widening (idempotence contract,
`batched_models.md` §"Execution model" step 1, untouched). State the choice in the commit.

**Critical files** (confirm post-fundamentals spellings first).
- `crates/smelt-logical/src/rules/incremental.rs` — `analyze_batch_safety`, `BatchSafety`, the roll-up entry.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — F1's `BoundResult` / `derive_and_classify_bounds`
  (read-only consumer).
- `crates/smelt-runtime/src/transformer.rs` — `inject_source_filters`, `inject_time_filter`,
  `is_transparent_single_source` (F13 emit side).

**Docs touched.**
- `batched_models.md` §"Composition" — verify the composition table matches the wired reality; §Known
  Divergences — narrow the "Two-layer widened-scan … redesign not yet emitted" note where it references the
  transparent fast path (F13 owns the full removal); leave the CTE-only and function-body `OVER` notes intact
  (out of scope).
- `docs-site/docs/guide/incremental-models.md` — verify the lookback / transparent-slice prose matches; no
  user-facing surface change.

**Review checklist.**
- [ ] One derivation (F1's map) feeds both the batch-safety class and every injected filter; no second walk.
- [ ] Transparent slice emits a single source filter, no outer wrap; lookback / multi-source keeps the clamp.
- [ ] `NotDerivable → BatchedNotSafe` refuses fail-closed, naming the construct (no silent downgrade).
- [ ] Event-time outer-visibility rejects the hidden-column cases; `Traceable` UNION ALL exempt; `StaticSeed`
      named-and-rejected.
- [ ] Shipped batched equivalence tests green (no regression).
- [ ] Edits timeless; late-data re-write path stated in the commit.

**Commit.** `refactor(logical): compose batched batch-safety roll-up + transforms on the unified bound map; drop the outer clamp on the transparent slice`

---

### Phase BL2: Backfill chunking from the batch-safety class

**Goal.** The batched-local **backfill-chunking** transform: pick the chunk shape from the BL1 batch-safety
class — `FullyBatchSafe` → one DELETE+INSERT pair for any `[start, end)`; `BoundedSafe(n)` → auto-sized
sub-ranges (3× context, clamped 7–90 partitions), each one pair, sequential in temporal order;
`PerPartitionOnly` → one partition per iteration. Per-partition batching is **calendar-aligned** for
`Month`/`Quarter`/`Year` (advance by true calendar units); `Day`/`Week` step 1-day/7-day. Each chunk's
DELETE+INSERT is **one transaction**; a failure halts at the first failed chunk (earlier committed chunks are
idempotent and do not roll back).

**Spec anchor.** `batched_models.md` §"First-run and backfill" (the class → chunking table; calendar
alignment; per-chunk transaction boundary; failure mode). `model_transforms.md` §"Transforms that stay in a
mode spec" (backfill chunking is batched-local, not catalogued). Invariant: `batched_models.md`
§"Per-partition equivalence".

**Pre-conditions.** BL1 landed (the batch-safety class + injection plan).

**Depends on.** BL1.

**TDD tests to write first.**
- `crates/smelt-cli/src/executor.rs` (or the chunker module) unit — `FullyBatchSafe` yields one pair for a
  60-day range; `BoundedSafe(n)` yields auto-sized sub-ranges clamped to 7–90 partitions; `PerPartitionOnly`
  yields one partition per iteration, temporal order.
- `crates/smelt-cli/…` unit — `Month`/`Quarter`/`Year` per-partition batches land on calendar boundaries
  regardless of month length; `Day`/`Week` step fixed 1/7 days.
- `crates/smelt-cli/…` unit (**fail-closed reject**) — a range that is not a whole-granularity multiple, or
  a class that could not be derived (BL1 refused), does **not** silently chunk — it surfaces BL1's refusal /
  the granularity error, never an approximate split.
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental/backfill.rs` (real fixture) —
  a multi-chunk backfill converges to the same table state a single-pass full refresh would, per partition;
  a failure mid-backfill leaves earlier chunks committed and re-running the same range resumes correctly.
  Requires `DUCKDB_LIB_DIR`.

**Implementation shape.** Read the BL1 class in the backfill dispatch; map it to the chunk-shape table; drive
the DELETE+INSERT loop with per-chunk transaction boundaries. Reuse the existing calendar arithmetic in
`Granularity`. This is largely re-expressing shipped chunking on BL1's class — keep the equivalence net green.

**Critical files** (confirm spellings first).
- `crates/smelt-cli/src/executor.rs` — `execute_*_incremental` backfill dispatch / the chunker.
- `crates/smelt-cli/src/temporal.rs` — `filter_range`, chunk-boundary arithmetic.
- `crates/smelt-core/src/config.rs` — `Granularity` calendar helpers.

**Docs touched.**
- `batched_models.md` §"First-run and backfill" — verify the chunking / calendar-alignment / transaction-
  boundary prose matches; no §Known-Divergence note dedicated to chunking (none removed).
- `docs-site/docs/guide/incremental-models.md` — verify the backfill/chunk-size guidance matches.

**Review checklist.**
- [ ] Chunk shape derives from the batch-safety class; sub-ranges clamped 7–90; per-partition temporal order.
- [ ] Calendar alignment for Month/Quarter/Year; fixed step for Day/Week.
- [ ] Per-chunk transaction boundary; failure halts, earlier chunks idempotent (resume-correct).
- [ ] A non-derivable class / non-multiple range surfaces the refusal, never an approximate chunk (fail-closed).
- [ ] Backfill equivalence fixture green (no regression); edits timeless.

**Commit.** `feat(cli): drive backfill chunk shape from the batch-safety class (calendar-aligned, per-chunk transaction)`

---

### Phase BL3: Non-determinism composition — `nondeterministic_columns` payload taint + compile-time pinning

**Goal.** Compose F3's **determinism (run vs row) + nondeterminism predicate** into the batched surface. Two
distinct admissions: (1) **compile-time pinning** (the batched-local transform) freezes a run-deterministic
clock (`NOW`/`CURRENT_*`) to one literal per run, so a **direct SELECT-list projection** of it is admitted
even into an unlisted column; (2) a **row-nondeterministic** value (`RANDOM`/`UUID`) is admitted only when it
flows **exclusively** into a column listed in the `batched:` block's `nondeterministic_columns`, gated by the
payload **taint flow** with three **hard exclusions** — `event_time_column`/`partition_column`, any
`unique_key` column, and any membership/grouping position (`WHERE`/`HAVING`/`JOIN … ON`/`DISTINCT`/`GROUP BY`/
window `PARTITION BY`/`ORDER BY`/frame). Listing an excluded column is a **configuration error**.

**Spec anchor.** `batched_models.md` §"Non-determinism and the payload rule" + Constraint 12; §Surface
(`nondeterministic_columns`). `model_properties.md` "Determinism (run vs row) + nondeterminism predicate",
`nondeterministic_columns` declaration. `model_transforms.md` "Compile-time pinning" (`unbuilt` → the pinning
emit ships here, batched-local; F3 owns the predicate).

**Pre-conditions.** Group A (A1) landed (the `batched:` block carries `nondeterministic_columns`). **F3**
(shared run-vs-row nondeterminism predicate + typed aggregate classifier) landed — do **not** re-derive the
predicate; consume it.

**Depends on.** F3, A1.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — `nondeterministic_columns` parses into
  `BatchedConfig`; listing `event_time_column` / `partition_column` / a `unique_key` column is a
  **configuration error** (Constraint 12; **fail-closed reject test**).
- `crates/smelt-logical/src/rules/incremental.rs` unit — `inserted_at = NOW()` (direct projection) **builds**
  via compile-time pinning even without a listed column; `stamp = RANDOM()` builds **only** when `stamp ∈
  nondeterministic_columns`; `RANDOM()` in `WHERE`/`GROUP BY`/`PARTITION BY` **refuses** (fail-closed), naming
  the offending position.
- `crates/smelt-logical/…` unit (taint flow) — a row-nondeterministic value reaching a listed payload column
  is admitted; reaching a non-listed column or an excluded role is refused; an **unresolvable indirection**
  refuses (fail-closed, per the deferred-depth open decision).
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental_*.rs` (new
  `nondeterministic_columns` fixture) — the model builds and its **deterministic skeleton** matches full
  refresh; the pinned `NOW()` value is one literal across a chunked backfill. Requires `DUCKDB_LIB_DIR`.

**Implementation shape.** Split the batched non-determinism gate into a **run-clock pinner** (compile-time
freeze of `NOW`/`CURRENT_*`, emitted before compile) and a **payload taint flow** over F3's predicate: from
each row-nondeterministic call follow its value; admit iff every sink is a listed payload column; reject the
three hard exclusions naming the role. Add `nondeterministic_columns: Vec<String>` handling + the
excluded-column config-error validation. The blunt `safety_overrides.allow_nondeterministic` remains but
discouraged.

**Open decision (for the implementer).** *Depth of the taint flow.* Ship the **direct-projection** analysis
first (fail-closed on any indirection through CTEs/subqueries); record deferred indirection depth under
§Deferred. **Membership/grouping non-determinism stays out of scope even with the opt-in** — keep it rejected.

**Critical files** (confirm spellings first).
- `crates/smelt-core/src/config.rs`, `metadata.rs` — `BatchedConfig` field + excluded-column validation.
- `crates/smelt-logical/src/rules/incremental.rs` — the non-determinism gate, new pinner + taint flow
  (consuming F3's predicate).
- `crates/smelt-runtime/src/transformer.rs` (or `compile.rs`) — the compile-time pinning emit.
- `examples/` — a `nondeterministic_columns` fixture (audit-stamp column).

**Docs touched.**
- `batched_models.md` §Known Divergences — **remove** the "Compile-time pinning of run-deterministic clocks
  not yet built" note and the payload-opt-in clause of the non-determinism note; keep the
  membership/grouping-out-of-scope clause as a plain out-of-scope statement.
- `docs-site/docs/guide/incremental-models.md` — document `nondeterministic_columns`, the three hard
  exclusions, and run-clock pinning.

**Review checklist.**
- [ ] Run-clock pinning (direct projection, unlisted OK) and payload opt-in (listed only) are distinct paths.
- [ ] Excluded-column listing is a config error; membership/grouping non-determinism stays rejected.
- [ ] Taint flow consumes F3's predicate (no private list); fail-closed on indirection.
- [ ] Deterministic skeleton equivalence-tested; pinned `NOW()` constant across a chunked backfill.
- [ ] §Known-Divergence pinning + payload-opt-in notes removed; edits timeless.

**Commit.** `feat(batched): pin run-deterministic clocks + admit payload non-determinism via nondeterministic_columns taint`

---

### Phase BL4: Group-aligned `HAVING` / `DISTINCT` on the scoped partition-alignment signal

**Goal.** Consume F5's **partition alignment (scoped)** signal to admit `HAVING` when the enclosing scope's
own `GROUP BY` key ⊇ `partition_column`, and `DISTINCT` when its projected key ⊇ `partition_column`; keep
`LIMIT` **unconditionally rejected** (a row-count cap never commutes with the partition filter). Every
alignment check is **per scope** (a `UNION` branch / subquery body judged against its own key set, never
inheriting a sibling's), reading F5's shared signal rather than a private whole-model text gate.

**Spec anchor.** `batched_models.md` §"Safety checks" (`HAVING`/`DISTINCT`/`LIMIT`; per-scope evaluation).
`model_properties.md` "Partition alignment (scoped)" (consumed with *admit-on-containment* polarity here).

**Pre-conditions.** **F5** (scoped partition-alignment signal over `GROUP BY` / `DISTINCT` / window `OVER`,
AST-based, per scope) landed — consume it; do not re-classify alignment.

**Depends on.** F5.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — a group-aligned `HAVING` (`GROUP BY ⊇
  partition_column`) **builds**; a `DISTINCT` whose key ⊇ `partition_column` **builds**.
- `crates/smelt-logical/src/rules/incremental.rs` unit (**fail-closed reject**) — a non-aligned `HAVING`, a
  non-aligned `DISTINCT`, and any `LIMIT` are **refused**, naming the reason.
- `crates/smelt-logical/src/rules/incremental.rs` unit — the alignment verdict is read **per scope** (a
  subquery body's own `GROUP BY`, not only the outer), consistent with F5's per-scope signal.
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental_parity.rs` (real fixture) — a
  group-aligned `HAVING` model matches full refresh across partitions. Requires `DUCKDB_LIB_DIR`.

**Implementation shape.** Replace the whole-model `allow_having` / `DISTINCT` text gates with a superset test
read from F5's per-scope `PartitionAlignment` verdict at each scope; `LIMIT` stays unconditionally rejected.
No alignment logic is authored here — this is a consumer of F5.

**Critical files** (confirm spellings first).
- `crates/smelt-logical/src/rules/incremental.rs` — the `HAVING`/`DISTINCT`/`LIMIT` gates, now reading F5.
- `crates/smelt-logical/src/analysis/mod.rs` — F5's `PartitionAlignment` signal (read-only).

**Docs touched.**
- `batched_models.md` §"Safety checks" — verify the `HAVING`/`DISTINCT`/`LIMIT` prose matches the shipped
  superset rule (no dedicated §Known-Divergence note; none removed).
- `docs-site/docs/guide/incremental-models.md` — document the group-aligned `HAVING`/`DISTINCT` admission and
  that `LIMIT` is never admitted.

**Review checklist.**
- [ ] Group-aligned `HAVING`/`DISTINCT` admitted and equivalence-tested; non-aligned refused (fail-closed).
- [ ] `LIMIT` unconditionally rejected.
- [ ] Alignment read per scope from F5's shared signal; no private re-classification.
- [ ] Edits timeless.

**Commit.** `feat(logical): admit group-aligned HAVING/DISTINCT in batched via the scoped partition-alignment signal`

**Verification note (2026-07-05).** Already shipped pre-re-cut (commit `8241900b`, before this plan's L1/L2
consolidation) — `check_having_alignment_all_scopes`/`check_distinct_alignment_all_scopes` in
`crates/smelt-logical/src/rules/incremental.rs` already consume F5's per-scope `scope_group_by_alignment` /
`scope_distinct_alignment` (`analysis/mod.rs`), `LIMIT` stays unconditionally rejected, and the equivalence
fixture (`crates/smelt-cli/tests/incremental/having_distinct_alignment.rs`) plus the fail-closed reject unit
tests already exist. `batched_models.md` §"Safety checks" and `docs-site/docs/guide/incremental-models.md`
already document the superset rule. Re-verified green (`cargo test -p smelt-logical --lib incremental::` — 64
passed; `cargo test -p smelt-cli --test incremental` — 46 passed, including
`having_distinct_alignment::test_group_aligned_having_matches_full_refresh_per_partition`). No code/doc change
made; row flipped to `done` as a verification-only phase.

---

### Phase BL5: Run-window ↔ partition-granularity alignment (`g_run ≥ g_part`)

**Goal.** Enforce that the CLI run window `[--event-time-start, --event-time-end)` is a positive integer
multiple of `timeseries.granularity` aligned to granularity boundaries, and additionally that
`g_run ≥ g_part` with aligned boundaries. Derive `g_part` from the partition-column transform unit
(`DATE_TRUNC('day', …) → day`) via the monotonicity trace / F1's unified interval unit, not a re-parse;
validate the CLI run window against it.

**Spec anchor.** `batched_models.md` §"Run window vs partition granularity"; §CLI. `timeseries.md`
§"Granularity arithmetic". Invariant: `batched_models.md` Constraint 8 (granularity closed under partition
arithmetic).

**Pre-conditions.** Group A (A1) landed (run-window plumbing in the CLI). **F1** landed (the unified interval
unit `g_part` derives from — consistent month=`Symbolic` handling).

**Depends on.** F1, A1.

**TDD tests to write first.**
- `crates/smelt-cli/src/temporal.rs` unit — a run window finer than `g_part` (hourly window, daily-partitioned
  model) is **rejected** with a message stating the minimum window (**fail-closed reject test**); a
  `g_run ≥ g_part` aligned window passes.
- `crates/smelt-cli/src/temporal.rs` unit — `g_part` derived from the partition-column transform unit matches
  the model's declared `timeseries.granularity`.
- `crates/smelt-cli/tests/incremental/backfill.rs` (real fixture) — an incomplete final partition is handled
  per §"Run window vs partition granularity" (the run window's last partial partition). Requires
  `DUCKDB_LIB_DIR`.
- **Must not regress batched equivalence.** The existing run-window / backfill equivalence fixtures stay green.

**Implementation shape.** In run-window validation (`temporal.rs`), add the `g_run ≥ g_part` comparison with
boundary alignment using the `Granularity` arithmetic; derive `g_part` from the partition-column transform unit
via the trace's many-to-one (`is_strict=false`) form rather than re-parsing.

**Open decision (for the implementer).** *Hard-validate vs auto-coarsen.* **Ship hard-validation first**
(reject a sub-`g_part` window with the minimum-window message) — the fail-closed choice; record auto-coarsen
as a deferred enhancement under §Deferred. Do not silently coarsen.

**Critical files** (confirm spellings first).
- `crates/smelt-cli/src/temporal.rs` — `filter_range`, run-window validation.
- `crates/smelt-core/src/config.rs` — `Granularity` arithmetic.
- `crates/smelt-logical/src/analysis/monotonicity.rs` — read `g_part` from the trace (call site only).

**Docs touched.**
- `batched_models.md` §"Run window vs partition granularity" — verify prose matches hard-validation (no
  §Known-Divergence removed; new enforcement of an already-specified invariant).
- `docs-site/docs/guide/incremental-models.md` — document the `g_run ≥ g_part` requirement and the too-fine
  run-window error.

**Review checklist.**
- [ ] Sub-`g_part` run window rejected with a clear minimum-window message (fail-closed).
- [ ] `g_part` derived from the partition-column transform, matching declared granularity.
- [ ] Incomplete final partition handled; equivalence fixtures green.
- [ ] Edits timeless.

**Commit.** `feat(cli): enforce run-window ≥ partition-granularity alignment for batched runs`

---

### Phase BL6: Monotone-integer partition keys

**Goal.** Generalise the batched machinery from a time-typed `partition_column` to a non-temporal **monotone
integer** key (sequence id / offset / watermark): admit integer offset arithmetic (`batch_id + <const>`,
integer bands) as `Traceable` on F1's generalised `Offset` (past `Seconds`), derive integer `g_part` and
lookback margins, and inject integer source filters (`WHERE c >= run_start − k AND c < run_end + k`).

**Spec anchor.** `batched_models.md` §Surface ("`partition_column` must be monotone — a timestamp *or* an
ever-increasing integer"), §"Observing the per-source clamp" (integer bound rendering). `model_properties.md`
"Event-time monotonicity trace" (`offset` = `Seconds` | `Symbolic`, generalised to carry an integer
magnitude), "Unified bound / reach derivation".

**Pre-conditions.** **F1** landed (the unified bound/reach; its `Offset` is where the integer magnitude lives).
*If F1 did not generalise `Offset` past `Seconds`, the generalisation is an F1 extension — record a block
"awaits F1 Offset generalisation" rather than re-deriving it here; this phase wires the batched admission +
integer clamp on top.*

**Depends on.** F1.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/monotonicity.rs` unit — the whitelist admits monotone integer offset
  arithmetic (`batch_id + <const>`, integer bands) as `Traceable`.
- `crates/smelt-logical/src/analysis/monotonicity.rs` unit (**fail-closed reject**) — a non-monotone integer
  transform (`batch_id % N`, `batch_id * -1`) is `NotTraceable`, naming the construct.
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit — `Offset` carries an integer magnitude; a
  `Bounded(c, k, 0)` integer lookback derives.
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental_parity.rs` (real fixture) — a
  model partitioned by a monotone `batch_id` integer builds and backfills, matching full refresh. Requires
  `DUCKDB_LIB_DIR`.

**Implementation shape.** Extend the trace whitelist to admit integer monotone forms and reject non-monotone
integer transforms; carry an integer magnitude in `Offset`; thread integer `g_part` / lookback through the
bound derivation and the source-filter injection.

**Open decision (for the implementer).** *Integer clamp readout.* An integer `Offset` has no ISO-8601 form;
decide the BL8 readout string (a bare integer count) and keep BL8's rendering polymorphic over `Offset`. Flag
if this forces a BL8 change.

**Critical files** (confirm spellings first).
- `crates/smelt-logical/src/analysis/monotonicity.rs` — `Offset`, the classifier whitelist for integer keys.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — integer `BoundResult` / `Offset`.
- `crates/smelt-runtime/src/transformer.rs` — integer source-filter injection.

**Docs touched.**
- `batched_models.md` §Surface / §"Composition" — verify the monotone-integer prose matches (no dedicated
  §Known-Divergence note; remove none unless one was added earlier).
- `docs-site/docs/guide/incremental-models.md` — document integer `partition_column` support.

**Review checklist.**
- [ ] Integer monotone forms admitted; non-monotone integer transforms rejected (fail-closed).
- [ ] `Offset` carries integer magnitudes; integer source filters inject correctly.
- [ ] Integer-partitioned fixture backfills to the full-refresh state (no regression).
- [ ] Edits timeless.

**Commit.** `feat(logical): support monotone-integer partition columns in batched eligibility and bound derivation`

---

### Phase BL7: Self-referential batched models — ordered execution

**Goal.** Consume F10's **window-independence / ordered-execution** proof (self-edge detection in the model
DAG): a batched model reading its own prior partitions via `smelt.<self>` is marked **ordered**, and the BL2
backfill chunker builds its windows **strictly sequentially in temporal order** (no parallel / out-of-order
dispatch); a window-independent model keeps auto-chunking. A self-reference F10 cannot prove converges
partition-by-partition (reads *forward* or across whole history) is **refused at planning time**, naming the
non-convergent self-edge. The model stays a partitioned `batched` table (it does **not** become
`refresh: cumulative`).

**Spec anchor.** `batched_models.md` §"Window independence and self-referential models". `model_properties.md`
"Window-independence / ordered-execution" (consumed here; verdict = `WindowIndependent` | `Ordered` |
`Refused`).

**Pre-conditions.** **F10** (window-independence / ordered-execution proof via DAG self-edge) landed. **BL2**
landed (the chunker BL7 constrains).

**Depends on.** F10, BL2.

**TDD tests to write first.**
- `crates/smelt-planner/…` (or `smelt-cli` executor) unit — a batched model whose SQL reads `smelt.<self>`
  prior partitions is `Ordered` and its chunker runs strictly sequentially in temporal order; a model with no
  self-edge is `WindowIndependent` (parallelisable / auto-chunked).
- `crates/smelt-planner/…` unit (**fail-closed reject**) — a self-reference reading *forward* / whole-history
  is **refused** at planning time with a diagnostic naming the non-convergent self-edge (never silently
  `Ordered`, never mis-parallelised).
- **Must not regress batched equivalence.** `crates/smelt-cli/tests/incremental/backfill.rs` (real fixture) —
  a running-balance model reading yesterday's close backfills correctly in temporal order, is **never**
  parallelised, and its end state matches a strictly-sequential reference build. Requires `DUCKDB_LIB_DIR`.

**Implementation shape.** Read F10's verdict in the backfill dispatch; thread the `Ordered` flag into the BL2
chunker so ordered models chunk one window at a time in temporal order (no parallel dispatch, no out-of-order
sub-ranges); window-independent models keep BL2's auto-chunking. Refuse a non-converging self-reference. No
self-edge detection is authored here — this consumes F10.

**Critical files** (confirm spellings first).
- `crates/smelt-logical/src/graph.rs` — `ModelGraph` self-edge (F10, read-only).
- `crates/smelt-cli/src/executor.rs` — backfill dispatch honours `ordered`.
- `crates/smelt-planner/src/rules/incremental.rs` (re-export) — the planning-time refusal.

**Docs touched.**
- `batched_models.md` §Known Divergences — **remove** the "Self-referential (ordered) batched models are
  specified but not yet enforced" note.
- `docs-site/docs/guide/incremental-models.md` — document that a self-referential batched model runs ordered
  and cannot be parallelised, and that a forward self-reference is refused.

**Review checklist.**
- [ ] Self-edge verdict read from F10; `Ordered` models chunk strictly sequentially (never parallelised).
- [ ] Forward / whole-history self-reference refused at planning time (fail-closed).
- [ ] Running-balance fixture backfills to the strictly-sequential reference state (no regression).
- [ ] §Known-Divergence self-referential note removed; edits timeless.

**Commit.** `feat(planner): enforce ordered batched backfill from F10's self-edge verdict; refuse non-converging self-references`

---

### Phase BL8: Per-source clamp observability

**Goal.** Finish the two batched-local observability surfaces (`batched_models.md` §"Observing the per-source
clamp"): `smelt explain --json` resolves the run-relative scan window `[run_start − before, run_end + after)`
per source when a run window is supplied; LSP hover on a `smelt.<path>` reference inside a batched model shows
its derived clamp alongside the existing schema/column readout. The four bound outcomes
(`Bounded(c,0,0)` / `Bounded(c,before,after)` / `Unbounded` / lookup) render distinctly; a `NotDerivable`
source surfaces its refusal, not a window. Temporal bounds render ISO-8601 (`Seconds::to_iso8601`); BL6
integer bounds render a bare integer count.

**Spec anchor.** `batched_models.md` §"Observing the per-source clamp" (both surfaces + the four-outcome
render table). Reads F1's bound map.

**Pre-conditions.** **F1** landed (the bound map + ISO-8601 rendering). **BL1** landed (the batch-safety
labels). Sequence after **BL6** so the readout covers integer bounds.

**Depends on.** F1, BL1, BL6.

**TDD tests to write first.**
- `crates/smelt-cli/src/explain.rs` unit — `explain --json --event-time-start/--event-time-end` reports the
  concrete `[run_start − before, run_end + after)` per source; without a run window it reports symbolic
  offsets only.
- `crates/smelt-cli/src/explain.rs` unit — the four bound outcomes render distinctly per the §"Observing the
  per-source clamp" table (including a BL6 integer-bound rendering).
- `crates/smelt-lsp/tests/…` (hover integration) — hovering a `smelt.<path>` reference inside a batched model
  shows its derived clamp + window alongside the schema/column readout.
- (**fail-closed / refusal render**) — a `NotDerivable` model shows the refusal diagnostic, **not** a per-source
  window (neither in `explain` nor hover).
- **Must not regress batched equivalence.** Observability is read-only; the batched execution fixtures stay
  green (no emit change).

**Implementation shape.** In `build_explain_output` / `ExplainIncremental`, resolve the concrete window from
F1's bound map when a run window is present; keep the symbolic-only path when absent. Extend LSP hover with a
batched-clamp branch for `smelt.<path>` references, reusing the bound map + the ISO-8601 / integer rendering;
append to (do not replace) the existing schema/column hover.

**Critical files** (confirm spellings first).
- `crates/smelt-cli/src/explain.rs` — `ExplainIncremental`, `SourceBoundJson`, `map_source_bounds`.
- `crates/smelt-lsp/src/hover.rs` — new batched-clamp hover branch.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — the bound map + rendering (read-only).

**Docs touched.**
- `batched_models.md` §Known Divergences — **remove** the "Per-source clamp observability partly emitted"
  note (both surfaces now ship); leave the "no clamp *warning*" parenthetical as a plain out-of-scope
  statement.
- `docs-site/docs/guide/incremental-models.md` — document `explain --json` run-relative windows and the
  editor-hover clamp readout.

**Review checklist.**
- [ ] `explain --json` with a run window reports the concrete window; without one, symbolic offsets only.
- [ ] Four bound outcomes render distinctly; hover shows the clamp alongside schema.
- [ ] BL6 integer bounds render sensibly; `NotDerivable` shows the refusal, not a window (fail-closed).
- [ ] §Known-Divergence observability note removed; edits timeless.

**Commit.** `feat(cli,lsp): resolve run-relative source clamp in explain --json + surface it in LSP hover`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this sub-plan.)

- Deeper taint indirection through CTEs/subqueries (BL3), auto-coarsen the run window (BL5), CTE-only
  `event_time_column` non-visibility, function-body `OVER` invisibility, per-column `data_latency` — deferred
  per §Scope "Explicitly deferred".
- **Pre-existing stale fixture (found in BL2 pre-flight, 2026-07-05):**
  `crates/smelt-cli/tests/e2e/incremental_refusal.rs::test_outer_having_refused` fails on current `main`
  (confirmed red already at `e03f42d9`, before BL1) — its `bad_having` fixture's `GROUP BY 1, 2` already
  contains the declared `partition_column` (`event_date` at position 1), so under the group-aligned `HAVING`
  semantics this HAVING should be **admitted**, not refused; the fixture predates that semantics and needs a
  genuinely non-aligned `GROUP BY` to still exercise a refusal. Left red and untouched by BL2 (unrelated to
  backfill chunking) — **BL4 owns the fix** (its own TDD list already covers "a non-aligned HAVING refuses");
  BL4 should update this fixture's `GROUP BY` to omit `event_date` as part of landing group-aligned admission.

## Verification

How to confirm the L4/batched composition is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **No regression of the batched contract.** The shipped batched per-partition / full-refresh equivalence
  tests (`crates/smelt-cli/tests/incremental_*.rs`, `.../incremental/backfill.rs` under
  `examples/timeseries/`) stay green after every phase — the per-partition equivalence oracle (the
  strengthening of the one invariant, `model_maintenance.md` §"The equivalence invariant") is the net.
  Requires `DUCKDB_LIB_DIR`.
- The generative monotonicity soundness oracle stays green throughout — no batched relaxation admits an
  unsound push.
- **Each phase is equivalence-or-refusal.** BL1–BL7 each have a full-refresh / per-partition equivalence
  real-fixture test **and** a fail-closed reject unit test (an ineligible construct refused with a
  diagnostic, never applied approximately — `batched_models.md` Constraint 10/12).
- `cargo test -p smelt-cli --test example_diagnostics` and `-p smelt-lsp --test example_workspaces` — example
  workspaces build with zero diagnostics.
- `/smelt:validate batched_models` reports zero drift for the surfaces this layer touches; every
  §Known-Divergence note this plan lists as removed (BL3 pinning + payload-opt-in; BL7 self-referential; BL8
  observability) is gone from `batched_models.md`, and the composition table matches the wired reality.
