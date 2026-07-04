# Plan: Model updates — Fundamentals (L1 proofs + L2 transforms)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the fundamentals layer (L1 + L2) of the re-cut master.
**Specs (oracles)**:
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — PRIMARY for L1. §"Derived proofs" (the maturity table), §Semantics (event-time trace, nullability gate, unified bound/reach, algebraic discriminants, driving-fact resolution, determinism predicate), §Constraints & Invariants (fail-closed; escape hatches may only widen), §"Known Divergences" (the six code duplications + the heuristic text-scanning layer).
- [`docs/specs/model_transforms.md`](../specs/model_transforms.md) — PRIMARY for L2. §Surface (the transform catalogue + maturity), §Semantics (the load-bearing mechanics), §Design ("Rejected: auto-widening the write window"), §Constraints & Invariants ("Equivalence or refusal"; "Write window = output window"), §"Known Divergences".
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — the framework these capabilities serve. §"The equivalence invariant" (per-partition + end-state) is the invariant **every** L2 transform preserves and every L1 proof is proven in service of; §"The algebraic maintenance ladder" reads the L1 discriminants as its ordering criterion; §"Validator, not chooser".
- [`docs/specs/models.md`](../specs/models.md) — §"Input-consumption axis" (the cross-cutting axis whose proof stage is input-delta discovery); the three-state declaration law; the litmus rule.
**Research (the "why" + the L-decomposition)**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Target plan architecture (the re-cut master)" (L0–L4; this sub-plan is **L1 + L2**), §"Mapping the current master onto the layers", and the inventory tables (proofs / world-facts / transforms with the six duplications enumerated).
**Spec diff**: none new — L0 (spec authoring) is `done`; `model_properties.md` / `model_transforms.md` / `model_maintenance.md` already exist and are normative. Each phase **flips a maturity cell** (`built, duplicated`/`partial`/`not-yet`/`unbuilt` → `built`) in the relevant §Surface table and **removes or narrows** the matching §Known-Divergence note as its behaviour ships; no phase authors a spec.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Oracle reframe (2026-07-04 — read before F1).** `model_maintenance.md` was reframed after this plan was drafted, so re-read it:
- **One invariant, addressing is the axis.** There is a single invariant (`incremental == full refresh over processed inputs`). Order/set-determinacy is a *corollary* holding for **every** mode (`batched` included); **per-partition equivalence is a *strengthening* of the one invariant, not a peer of "end-state"**. The load-bearing distinction is **output addressing** — partition-addressed (identity-free, whole-partition rewrite) vs key-addressed (identity-requiring `merge_into`, writes reaching back by key outside the input window; SCD2's close-out is the canonical case). Where a phase parenthetical below says "per-partition + end-state", read it as "the one invariant, per-partition being `batched`'s strengthening of it".
- **Windowed by default; horizon derived.** New §"Windowed maintenance and the horizon": maintenance is windowed wherever the model is clocked (full scan is the fallback for a clockless snapshot source); the **horizon is derived** from the model's reach — a declared value is a *warning ceiling only* and never relaxes the clamp; a late arrival beyond the derived horizon is **silently clamped, not diagnosed** (surfacing lateness is a model-author + data-check concern, not a maintenance guarantee).
- **Phase consequences.** F1's unified reach now also feeds a **maintained-window / horizon derivation** proof (`model_properties.md`, `not-yet`); the **two-layer widened-scan + exact-clamp** transform (≈F13) and **dimension-driven horizon MERGE** (≈F15) are joined by a newly-catalogued **horizon settled-delay / tail-rewrite** transform (`model_transforms.md`, `unbuilt`), promoted from the deferred pile. No phase numbering changed; these are additive maturity cells the relevant phases flip.

*(A prior interrupted F1 attempt — `interval.rs` + the interval-reach/parser/orchestration consolidation — is patch-saved in the session scratchpad; it was **not** committed and F1 re-derives cleanly against this reframed oracle.)*

**Scope boundary (read first).** This sub-plan implements **L1 (derived proofs)** and **L2 (transforms)** — the reusable capabilities the refresh modes later compose. It does **not** cover **L3** (the model-scoped declaration *surfaces* — `nondeterministic_columns`, declared monotonicity, functional dependency, bounded-domain budget) or **L4** (mode compositions: batched relaxations, cumulative rungs, the new keyed modes). Those are separate sub-plans (existing Group B / Group C / Group D, and later L3/L4 sub-plans). Where an L1 proof or L2 transform is *consumed* by a mode, this sub-plan builds and unit-tests the capability against its capability spec; wiring it into a mode's surface is the mode sub-plan's job.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when
this fundamentals layer is scaffolded into the registry — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant oracle for every phase is the **processed-input equivalence invariant**
   (`model_maintenance.md` §"The equivalence invariant": per-partition for `batched`, end-state for the
   keyed modes). Every L1 proof is proven *in service of* it; every L2 transform is licensed *because it
   preserves* it and is **refused with a diagnostic** when it cannot (`model_transforms.md` §Constraints
   "Equivalence or refusal"). Every proof is **fail-closed** (`model_properties.md` §Constraints): an
   undecidable construct yields the reject verdict, never an optimistic default.
2. Confirm you are on branch `worktree-incremental` and that L0 (the three capability/framework specs)
   is landed.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status
   to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names
a fail-closed reject test, and every phase that changes emitted SQL adds an equivalence-harness test) →
reviewer subagent (material findings only) → iterate → set the row `done` → commit + push with the
phase's `Commit.` line.

**Consolidations must not regress (F1–F3, and the driver-lift F11).** The three consolidation phases and
the driver lift **refactor** live analysis that the shipped `batched` and `cumulative` paths depend on.
Their acceptance gate includes the existing `smelt-logical` unit tests **and** the `smelt-cli` batched +
cumulative integration/equivalence tests staying green (`crates/smelt-cli/tests/incremental*`,
`.../cumulative*`). A consolidation that flips a behavioural test is a bug in the consolidation, not a
spec change — do not update the equivalence expectations to match new output.

**Equivalence-harness tests need DuckDB.** Phases that change emitted SQL (F1, F11, F13, F14, F15) assert
full-refresh / per-partition equivalence via the DuckDB harnesses; those require `DUCKDB_LIB_DIR` set
(and `LD_LIBRARY_PATH`) per `CLAUDE.md`. Pure classifier phases (F4–F10) are `smelt-logical` unit tests
with no DuckDB dependency.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/`
edits describe the feature as if it always existed; as each phase lands, **flip the §Surface maturity
cell** to `built` and **remove/narrow** the matching §Known-Divergence note rather than annotating it
with a phase number.

**Block rule.** On a design decision not answered here or by the spec, or a pre-flight red unrelated to
this phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a
clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape re-cut the maintained-model family "fundamentals-first"
(`docs/research/20260704-maintenance-fundamentals.md`): the refresh modes are compositions of a small,
shared set of **derived proofs** (`model_properties.md`) and **physical transforms**
(`model_transforms.md`), each proven/licensed against the one **processed-input equivalence invariant**
(`model_maintenance.md`). L0 authored those three specs; the implementation lags. Two problems this layer
fixes: (a) the shared spine currently exists as **six live code duplications** in `crates/smelt-logical`
(two interval-reach analyses, three interval-literal parsers, three copies of the nondeterminism-function
list, two driving-fact resolvers, two bound-derivation orchestration sites, aggregate-name extraction
done twice), so every mode-vertical phase that lands adds another private copy; and (b) several proofs and
every "new" transform are unbuilt. This sub-plan pays down the duplications **first** (they must not
regress the shipped batched/cumulative paths), then builds the remaining proof classifiers, then the five
mode-agnostic transforms the research's L2 list names. Modes (L4) compose these by name; this layer never
wires a mode surface.

## Scope

### In scope (L1 + L2)

**L1 — derived proofs.** Consolidations first (pay down the six duplications), then the new classifiers:
- **F1** — Unified bound/reach derivation (dups 1, 2, 5): merge `temporal.rs` `EffectiveWindow`
  (day-granular) into `source_bounds.rs` `BoundResult` (second-granular) behind one verdict; collapse the
  three interval-literal parsers into one; unify the two bound-derivation orchestration sites.
- **F2** — Driving-fact / anchor resolution (dup 4): one resolver replacing `cumulative.rs` ref-count and
  `source_bounds.rs` alias-scoped trace.
- **F3** — Shared nondeterminism predicate + typed aggregate classifier (dups 3, 6): a single
  function-name list replacing the three copies; fold the twice-done aggregate-name extraction into the
  typed `SqlFunction::is_aggregate`.
- **F4** — Algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) as
  a shared classifier generalising the cumulative monoid set.
- **F5** — Scoped partition-alignment signal exposed per scope (`GROUP BY` / `DISTINCT` / window `OVER`).
- **F6** — Join-contribution monotonicity + fan-out / cardinality.
- **F7** — Presentation-map purity.
- **F8** — Additive-only model-diff.
- **F9** — Input-delta discovery (window-forward / snapshot-diff / change-feed) — the proof stage of the
  input-consumption axis.
- **F10** — Window-independence / ordered-execution (self-edge detection).

**L2 — transforms** (the five the research L2 list names):
- **F11** — Windowed-keyed-maintenance driver: lift the cumulative-orchestration loop into a mode-agnostic
  driver.
- **F12** — Hidden decomposed state + presentation view mechanism.
- **F13** — Two-layer widened-scan / exact-clamp redesign (read the margin, write only the output window).
- **F14** — Targeted column backfill (licensed by additive-only model-diff).
- **F15** — Dimension-driven horizon-bounded MERGE (target-as-replica + join-contribution monotonicity +
  horizon `H`).

### Explicitly deferred (to later layers)

These transforms/surfaces are named in the catalogues but are **not** L1/L2 fundamentals — the research
L-decomposition homes them elsewhere; they are **not silently dropped**:

- **Retraction via delta history** and **explicit bounded-domain multiset state** — the research L4
  cumulative residue (`(retraction, multiset, reprocessing, presentation purity)`); land with the
  cumulative mode work (existing master Group **C**: C3 retraction, C4 multiset). They are licensed by the
  **group** / **bounded-domain** rungs, which F4 makes *derivable*, but their state machinery is
  mode-local to `cumulative_aggregate.md`.
- **Compile-time pinning** (run-deterministic `NOW`/`CURRENT_*` → one literal per run) — inventory home
  `local:ba`; lands with the batched non-determinism work (Group **B**, B3). F3 ships the *predicate* that
  classifies `NOW`/`CURRENT_*` as run-deterministic; the pinning *transform* is batched-local.
- **UNION-branch wrap-and-filter** — inventory home `local:ba`; lands with the batched UNION/subquery
  consumers (Group **B**, B1). F5's scoped partition-alignment and the existing set-operation-distribution
  proof feed it; the wrap-and-filter emit is batched-local.
- **Idempotent window re-scan vs delta-driven probe** — its proof stage (input-delta discovery) is F9;
  the *transform* is the input-consumption axis's re-scan/probe mechanism, wired per consuming mode (L4),
  not a standalone fundamental here.
- **Watermark settled-delay / tail-rewrite** — open (`model_transforms.md` §Known Divergences, the
  forward-reach "unworked mirror"); not yet specified as a transform. F1 derives the forward `after`/`H`
  reach it would consume; the settled-delay transform itself is deferred with its spec.

L3 (declaration surfaces) and L4 (mode compositions) are out of scope entirely — separate sub-plans.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| F1    | done    | 09927bc6 | 2026-07-04 |
| F2    | done    | dcbdccab | 2026-07-05 |
| F3    | done    | 787f9bc0 | 2026-07-05 |
| F4    | done    | aa815fb7 | 2026-07-05 |
| F5    | done    | 7af45aba | 2026-07-05 |
| F6    | done    | 0a2ad09c | 2026-07-05 |
| F7    | done    | 143781b2 | 2026-07-05 |
| F8    | done    | 09f44b42 | 2026-07-05 |
| F9    | done    | 79a21f8e | 2026-07-05 |
| F10   | done    | 86c5b3d1 | 2026-07-05 |
| F11   | done    | a286d520 | 2026-07-05 |
| F12   | done    | d1384d4c | 2026-07-05 |
| F13   | done    |        | 2026-07-05 |
| F14   | done    | b54ba654 | 2026-07-05 |
| F15   | pending |        |      |

---

### Phase F1: Unified bound / reach derivation (dups 1, 2, 5)

**Goal.** Collapse the two independent interval-reach analyses into **one** verdict: fold
`analysis/temporal.rs` (`EffectiveWindow`, day-granular, backfill classification) into
`analysis/source_bounds.rs` (`BoundResult`, second-granular, pushdown pruning) so a single walk computes
the finite backward (`before`) / forward (`after`) reach per source. Collapse the three interval-literal
parsers (`monotonicity.rs::parse_interval_value`, `source_bounds.rs::parse_interval_value_str`,
`temporal.rs::extract_interval_days`) into one parser with consistent unit handling (month = `Symbolic`,
not two divergent ≈30d rules). Unify the two bound-derivation orchestration sites
(`rules/incremental.rs` and `smelt-runtime/src/compile.rs`) onto one entry point.

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Unified bound / reach derivation"**
(`built, duplicated` → `built`); §Semantics "Unified bound / reach derivation" (the `before`/`after`
split, computation-reach vs source-lateness); §Known Divergences duplication items **(1), (2), (5)**.

**Pre-conditions.** L0 landed. Consumes the W1 monotonicity primitive (`trace_event_time`) read-only.

**Depends on.** — (first consolidation).

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit — the day-granular case `temporal.rs` used to
  own (a `DATE_TRUNC('day', …)` backfill window) now derives the same reach through `derive_model_bounds`;
  assert the `BoundResult::Bounded{before, after}` seconds equal the former `EffectiveWindow` day span.
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit — one interval parser: `'1 month'` folds to
  `Symbolic` (not ≈30d) and `'90 minutes'`/`'1.5 hours'` fold to the same `Seconds` magnitude, from the
  single parser; the two former string parsers' call sites now route through it.
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit (fail-closed) — a `NotDerivable` source still
  refuses (returns the reject verdict), naming the construct; `Unbounded` still forbids a pushed filter.
- `crates/smelt-cli/tests/incremental*` + `crates/smelt-cli/tests/cumulative*` (real fixtures under
  `examples/timeseries/`) — **existing** batched + cumulative equivalence tests stay green (no regression
  from the merge). Requires `DUCKDB_LIB_DIR`.

**Implementation shape.** Make `source_bounds.rs::derive_model_bounds` the single spine; delete
`temporal.rs`'s parallel `EffectiveWindow` walk and re-express its one consumer (backfill classification)
on top of `BoundResult` at second granularity, rendering to days only at the display boundary. Extract one
`parse_interval` returning `Offset` (`Seconds`/`Symbolic`) and route `monotonicity.rs`,
`source_bounds.rs`, and the former `temporal.rs` call sites through it. Point both orchestration sites
(`rules/incremental.rs`, `smelt-runtime/src/compile.rs`) at one wrapper.

**Critical files.**
- `crates/smelt-logical/src/analysis/source_bounds.rs` — `BoundResult`, `derive_model_bounds` (`:204`),
  `parse_interval_value_str` (`:579`) → the unified parser.
- `crates/smelt-logical/src/analysis/temporal.rs` — `EffectiveWindow` (`:465`), `extract_interval_days`
  folded away.
- `crates/smelt-logical/src/analysis/monotonicity.rs` — `parse_interval_value` (`:493`) routed through the
  shared parser (primitive logic otherwise untouched).
- `crates/smelt-runtime/src/compile.rs`, `crates/smelt-logical/src/rules/incremental.rs` — the two
  orchestration sites unified.

**Docs touched.**
- `model_properties.md` §Surface — flip "Unified bound / reach derivation" to `built`; §Known Divergences
  — remove duplication items (1), (2), (5) (or the whole "six duplications" note's clauses for 1/2/5,
  leaving 3/4/6 for F2/F3).
- `model_transforms.md` §Known Divergences — narrow the "Duplicated licensing analyses" note (interval-reach
  + bound-derivation sites now single).
- `docs-site/` — no user-facing surface change (internal analysis consolidation); verify prose unaffected.

**Review checklist.**
- [ ] One walk (`derive_model_bounds`) produces the reach; `EffectiveWindow` is gone, its consumer
      re-expressed on `BoundResult`.
- [ ] One interval parser; month = `Symbolic`; both former string parsers routed through it.
- [ ] Two orchestration sites unified onto one entry point.
- [ ] `NotDerivable`/`Unbounded` still fail-closed.
- [ ] Existing batched + cumulative equivalence tests green (no regression).
- [ ] §Surface cell flipped; duplication notes 1/2/5 removed; edits timeless.

**Commit.** `refactor(logical): unify bound/reach derivation — one walk, one interval parser, one orchestration site`

---

### Phase F2: Driving-fact / anchor resolution (dup 4)

**Goal.** Replace the two unrelated driving-fact resolvers — `cumulative.rs`'s ref-count-based selection
and `source_bounds.rs::resolve_join_driving_fact`'s alias-scoped monotonicity trace — with **one**
resolver: among the *joined* inputs of a scope, exactly-one-`Traceable` input is the anchor (alias-scoped
leaf disambiguation); zero or two-or-more is fail-closed. Route `cumulative`'s driving-source selection
through it.

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Driving-fact / anchor resolution"**
(`built, duplicated` → `built`); §Semantics "Driving-fact / anchor resolution"; §Known Divergences
duplication item **(4)**.

**Pre-conditions.** L0 landed. Reuses the W1 trace read-only.

**Depends on.** — (independent consolidation; may run alongside F1/F3).

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit — the alias-scoped resolver disambiguates two
  joined inputs sharing a partition-column *name* (via FROM/alias scope), returning the single `Traceable`
  anchor; zero-`Traceable` and two-`Traceable` both fail-closed, naming the ambiguity.
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `cumulative`'s "single driving source" selection
  (formerly ref-count) now delegates to the shared resolver and yields the same verdict for the existing
  fixtures (one driving source admitted; zero / multiple refused — the current
  `test_zero_driving_sources_refused` / `test_multiple_driving_sources_refused` stay green).
- `crates/smelt-cli/tests/cumulative*` (real fixtures) — cumulative equivalence unchanged. Requires
  `DUCKDB_LIB_DIR`.

**Implementation shape.** Make `source_bounds.rs::resolve_join_driving_fact` the single resolver; adapt
`cumulative.rs`'s candidate-source loop (`:356`+) to build the alias→source map and call it, deleting the
ref-count path. Keep the fail-closed zero/two behaviour identical.

**Critical files.**
- `crates/smelt-logical/src/analysis/source_bounds.rs` — `resolve_join_driving_fact` (`:778`).
- `crates/smelt-logical/src/rules/cumulative.rs` — driving-source selection (`:356`+, `driving_source`).

**Docs touched.**
- `model_properties.md` §Surface — flip "Driving-fact / anchor resolution" to `built`; §Known Divergences
  — remove duplication item (4).
- `model_maintenance.md` §Known Divergences — remove the "two driving-fact resolvers" clause from the
  six-duplications note.
- `docs-site/` — none (internal).

**Review checklist.**
- [ ] One resolver; `cumulative` delegates to it; ref-count path deleted.
- [ ] Exactly-one-`Traceable` admitted; zero / two fail-closed, naming the ambiguity.
- [ ] Cumulative fixtures + equivalence green.
- [ ] §Surface cell flipped; duplication note (4) removed; edits timeless.

**Commit.** `refactor(logical): single driving-fact/anchor resolver; route cumulative through the alias-scoped trace`

---

### Phase F3: Shared nondeterminism predicate + typed aggregate classifier (dups 3, 6)

**Goal.** Replace the `NONDETERMINISTIC_FUNCTIONS` list copied across `rules/incremental.rs`,
`rules/cumulative.rs`, and the inline set in `analysis/monotonicity.rs` with **one** predicate that also
encodes the run-deterministic (`NOW`/`CURRENT_*`, pinnable) vs row-nondeterministic
(`RANDOM`/`RAND`/`UUID`/`GEN_RANDOM_UUID`/`SETSEED`) split. Fold the twice-done aggregate-name extraction
(`cumulative.rs`'s string re-parse vs the typed `SqlFunction::is_aggregate`) into the single typed
classifier.

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Determinism (run vs row) + nondeterminism
predicate"** (`built, duplicated` → `built`); §Semantics "Determinism (run vs row) and the nondeterminism
predicate"; §Known Divergences duplication items **(3), (6)**.

**Pre-conditions.** L0 landed.

**Depends on.** — (independent consolidation).

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/…` unit — one predicate: `RANDOM`/`UUID`/`SETSEED` classify
  row-nondeterministic; `NOW`/`CURRENT_TIMESTAMP`/`CURRENT_DATE` classify run-deterministic; an ordinary
  function is neither. The three former call sites (`incremental.rs`, `cumulative.rs`, `monotonicity.rs`)
  route through it (assert no remaining private list).
- `crates/smelt-logical/src/rules/cumulative.rs` unit — the nondeterminism reject
  (`test_nondeterministic_refused`) stays green through the shared predicate; the aggregate-name check now
  uses `SqlFunction::is_aggregate` (no string re-parse), and its existing classifier tests stay green.
- `crates/smelt-logical` unit (fail-closed) — an unknown / unresolved function name is treated
  conservatively (not silently "deterministic"): the taint/skeleton check rejects, per fail-closed.

**Implementation shape.** Add the predicate (function-name → `RunDeterministic | RowNondeterministic |
Neither`) beside the trace; delete the two `const NONDETERMINISTIC_FUNCTIONS` copies and the inline set in
`monotonicity.rs`. Replace `cumulative.rs`'s string aggregate re-parse with `SqlFunction::from_name(..).
is_some_and(|f| f.is_aggregate())` (the spelling already used in `analysis/mod.rs:84`).

**Critical files.**
- `crates/smelt-logical/src/rules/incremental.rs`, `crates/smelt-logical/src/rules/cumulative.rs`
  (`NONDETERMINISTIC_FUNCTIONS` `:207`, `:346`), `crates/smelt-logical/src/analysis/monotonicity.rs`
  (inline set) — collapse to the shared predicate.
- `crates/smelt-types/src/functions.rs` — `SqlFunction::is_aggregate` (the single typed classifier).

**Docs touched.**
- `model_properties.md` §Surface — flip "Determinism (run vs row) + nondeterminism predicate" to `built`;
  §Known Divergences — remove duplication items (3), (6).
- `model_maintenance.md` §Known Divergences — remove the "non-deterministic-function list … copied" and
  "aggregate-name extraction done twice" clauses (all six duplications now resolved across F1–F3).
- `docs-site/` — none (internal).

**Review checklist.**
- [ ] One nondeterminism predicate (run vs row); three copies deleted; no private list remains.
- [ ] Aggregate-name extraction uses typed `SqlFunction::is_aggregate`; string re-parse deleted.
- [ ] Unknown function name is conservatively rejected (fail-closed).
- [ ] Cumulative + incremental nondeterminism/aggregate tests green.
- [ ] §Surface cell flipped; duplication notes (3), (6) removed; all six duplications now gone.

**Commit.** `refactor(logical): single nondeterminism predicate + typed aggregate classifier; retire the copies`

---

### Phase F4: Algebraic discriminants classifier

**Goal.** Build one classifier returning the raw algebraic facts of a combiner — **is-monoid**,
**needs-inverse**, **decomposable** (additive vs holistic), **value-monotone vs order-monotone** —
generalising the cumulative monoid allowlist into a shared, reusable classifier. These are *discriminants*,
**not** the ladder (the ordering + maintainable/delegated cutoff stays in `model_maintenance.md`).

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Algebraic discriminants"**
(`partial (monoid set built in cumulative; others not-yet)` → `built`); §Semantics "Algebraic
discriminants (the raw facts, not the ladder)"; §Design "Discriminants here, ladder in maintenance".
Consumed-by (not wired here): `model_maintenance.md` §"The algebraic maintenance ladder".

**Pre-conditions.** F3 landed (the typed `SqlFunction::is_aggregate` classifier the discriminants build on).

**Depends on.** F3.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/…` unit — `SUM`/`COUNT` → monoid **and** invertible (group);
  `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` → monoid, **not** invertible (needs-inverse = true, cannot un-see);
  `AVG`/variance/approx-distinct → decomposable; `MEDIAN`/`MODE`/exact-`COUNT(DISTINCT)` → holistic.
- `crates/smelt-logical/src/analysis/…` unit — `MIN`/`MAX`/`EXISTS` → value-monotone; `MAX_BY` →
  order-monotone (value may switch).
- `crates/smelt-logical/src/analysis/…` unit (fail-closed) — an unrecognised / UDF aggregate is **not**
  optimistically classified as a monoid: it yields the reject/`unknown` discriminant, per fail-closed.
- `crates/smelt-logical/src/rules/cumulative.rs` unit — cumulative's existing monoid gate now reads the
  shared discriminant (its classifier tests stay green).

**Implementation shape.** A pure function `combiner_discriminants(fn) -> Discriminants{ is_monoid,
needs_inverse, decomposable, monotone: Value|Order|None }` keyed on `SqlFunction`; migrate cumulative's
inline allowlist to consume it. No mode selection — a classifier only.

**Critical files.**
- `crates/smelt-logical/src/analysis/` (new discriminants module or an addition to `mod.rs`).
- `crates/smelt-types/src/functions.rs` — combiner metadata if the fact is best carried on `SqlFunction`.
- `crates/smelt-logical/src/rules/cumulative.rs` — monoid gate reads the shared discriminant.

**Docs touched.**
- `model_properties.md` §Surface — flip "Algebraic discriminants" to `built`; §Known Divergences — remove
  the "algebraic discriminants beyond the monoid set" clause from the unbuilt-classifiers note.
- `docs-site/` — none (internal; no standalone user page for proofs).

**Review checklist.**
- [ ] All four discriminants classified for the monoid/group/decomposable/holistic + value/order sets.
- [ ] Unrecognised aggregate is not optimistically a monoid (fail-closed).
- [ ] Cumulative monoid gate delegates to the shared discriminant; ladder **not** duplicated here.
- [ ] §Surface cell flipped; edits timeless.

**Commit.** `feat(logical): shared algebraic-discriminant classifier (is-monoid / needs-inverse / decomposable / value-vs-order-monotone)`

---

### Phase F5: Scoped partition-alignment signal (GROUP BY / DISTINCT / window OVER)

**Goal.** Expose the partition-alignment verdict (`Aligned` / `NotAligned{reason}`) **per scope** —
computed for each `GROUP BY`, `DISTINCT`, and window `OVER` scope independently — as one shared signal.
The `GROUP BY`/`DISTINCT` scopes are already AST-based (`scope_group_by_alignment`,
`scope_distinct_alignment`); this phase adds the window-`OVER` scope on the same AST footing (replacing the
text scan) and packages the three into one per-scope signal callers consume with either polarity (batched
*admits* on containment, keyed modes *reject* on it).

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Partition alignment (scoped)"**
(`built (AST); window OVER text-scanned` → `built`); the "consumed with opposite polarity" note; §Known
Divergences "Heuristic text-scanning layer" (the window-`OVER` scan clause).

**Pre-conditions.** L0 landed. Independent of the discriminants.

**Depends on.** —.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/mod.rs` unit — a window `OVER (PARTITION BY … )` scope's alignment is
  judged on the AST (the parsed partition-by list), matching the `GROUP BY`/`DISTINCT` scope verdicts; a
  scope whose key ⊇ `partition_column` → `Aligned`, else `NotAligned{reason}`.
- `crates/smelt-logical/src/analysis/mod.rs` unit — the per-scope signal is computed for a subquery body's
  *own* `GROUP BY` (not only the outermost), so a caller can read alignment at any scope.
- `crates/smelt-logical/src/analysis/mod.rs` unit (fail-closed) — an unparseable / unresolved `OVER` scope
  yields `NotAligned{reason}` (never optimistic `Aligned`).

**Implementation shape.** Add `scope_over_alignment` beside `scope_group_by_alignment` /
`scope_distinct_alignment` (`mod.rs:202`, `:237`), sharing the containment test; return the trio as one
per-scope `PartitionAlignment` the callers (batched, cumulative) read. Do **not** wire the consumers here
— just expose the signal (mode wiring is L4).

**Critical files.**
- `crates/smelt-logical/src/analysis/mod.rs` — `PartitionAlignment` (`:154`), `scope_group_by_alignment`
  (`:202`), `scope_distinct_alignment` (`:237`), new `scope_over_alignment`.

**Docs touched.**
- `model_properties.md` §Surface — flip "Partition alignment (scoped)" to `built` (drop the "window OVER
  text-scanned" qualifier); §Known Divergences — remove the window-`OVER`-scan clause from the
  text-scanning note.
- `docs-site/` — none (internal).

**Review checklist.**
- [ ] Window `OVER` alignment is AST-based, matching `GROUP BY`/`DISTINCT` verdicts; text scan gone.
- [ ] The signal is per-scope (any nesting), one shared verdict callers read with either polarity.
- [ ] Unresolved scope fails closed to `NotAligned`.
- [ ] §Surface cell flipped; text-scan clause removed; edits timeless.

**Commit.** `feat(logical): scoped partition-alignment signal over GROUP BY / DISTINCT / window OVER (AST)`

---

### Phase F6: Join-contribution monotonicity + fan-out / cardinality

**Goal.** Two related proofs. **Fan-out / cardinality**: does a join multiply rows / change target
cardinality, or enrich in place. **Join-contribution monotonicity**: a semi-/dimension-join's per-key
contribution folds without an inverse (value- or order-monotone) *and* does not fan into a decrementing
aggregate — composed from the inverse-free discriminant (F4) + fan-out. This licenses the dimension-driven
horizon MERGE (F15).

**Spec anchor.** `model_properties.md` §"Derived proofs" → rows **"Fan-out / cardinality"** (`not-yet`) and
**"Join-contribution monotonicity"** (`not-yet`) → both `built`; the "composed from the inverse-free
discriminant + fan-out/cardinality" note.

**Pre-conditions.** F4 landed (inverse-free / value-vs-order discriminants).

**Depends on.** F4.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/…` unit (fan-out) — an equi-join on a non-unique key → `OneToMany`
  (row-multiplying); a join on a key proven unique (or a dimension lookup) → `OneToOne` (enriches in
  place); an *unknown* cardinality → fail-closed `OneToMany` (the conservative verdict).
- `crates/smelt-logical/src/analysis/…` unit (join-contribution) — a dimension enrichment whose
  contribution is value-/order-monotone and does not feed a decrementing aggregate → monotone contribution
  admitted; a contribution feeding a `SUM` that can decrease, or a fan-out into a decrementing aggregate →
  refused, naming the reason.
- `crates/smelt-logical/src/analysis/…` unit (fail-closed) — an undecidable join shape yields the reject
  verdict, never an optimistic "monotone".

**Implementation shape.** A `fan_out(join, ctx) -> OneToOne | OneToMany` proof (unknown → `OneToMany`), and
`join_contribution_monotone(join, ctx) -> bool/verdict` composing `fan_out` + F4's needs-inverse /
value-vs-order discriminants. Pure `smelt-logical`; no transform emitted here (that is F15).

**Critical files.**
- `crates/smelt-logical/src/analysis/` (new join-analysis module or additions to `source_bounds.rs`).

**Docs touched.**
- `model_properties.md` §Surface — flip both rows to `built`; §Known Divergences — remove
  "fan-out/cardinality" from the unbuilt-classifiers note.
- `docs-site/` — none (internal).

**Review checklist.**
- [ ] Fan-out returns `OneToOne`/`OneToMany`; unknown → fail-closed `OneToMany`.
- [ ] Join-contribution monotonicity composes fan-out + inverse-free discriminant; decrementing-aggregate
      fan-in refused.
- [ ] Undecidable join shape fails closed.
- [ ] Both §Surface cells flipped; edits timeless.

**Commit.** `feat(logical): join-contribution monotonicity + fan-out/cardinality proofs`

---

### Phase F7: Presentation-map purity

**Goal.** A proof that a hidden-state presentation map `π(state)` is a **pure** function of a single
consistent state row — it reads no other rows, tables, or windows. This is the soundness condition for a
decomposed-state presentation view (F12).

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Presentation-map purity"** (`not-yet` →
`built`); §Constraints note that presentation-map purity is *not* mode-only (its verdict is stateable
without naming a mode). `model_transforms.md` §Semantics "Hidden decomposed state + presentation view"
(sound iff `π` is a pure function of one consistent state row).

**Pre-conditions.** L0 landed.

**Depends on.** —.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/…` unit — `sum / count` over the state row's own columns → pure
  (`Pure`); an expression referencing another table / a window / a subquery over other rows → impure
  (`Impure{reason}`).
- `crates/smelt-logical/src/analysis/…` unit (fail-closed) — an unresolved reference or an opaque UDF in
  the presentation expression yields `Impure` (never optimistic `Pure`).

**Implementation shape.** `presentation_map_purity(expr, state_columns, ctx) -> Pure | Impure{reason}`:
walk the expression, admit only references to the state row's columns + pure scalar ops; reject any
cross-row / cross-table / windowed / opaque reference.

**Critical files.**
- `crates/smelt-logical/src/analysis/` (new purity proof).

**Docs touched.**
- `model_properties.md` §Surface — flip "Presentation-map purity" to `built`.
- `docs-site/` — none (internal).

**Review checklist.**
- [ ] Pure single-state-row maps admitted; cross-row / cross-table / windowed maps rejected.
- [ ] Unresolved / opaque reference fails closed to `Impure`.
- [ ] §Surface cell flipped; edits timeless.

**Commit.** `feat(logical): presentation-map purity proof (π reads one consistent state row)`

---

### Phase F8: Additive-only model-diff

**Goal.** A proof that a model edit **only adds** columns derivable from `{existing target} ∪ {monotone
dimension}` — so an in-place backfill is admissible rather than a full rebuild. The column/dependency-set
diff is derivable; "did an existing column's semantics change" is **not** and falls to a declared
migration intent (out of scope — L3).

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Additive-only model-diff"** (`not-yet` →
`built`); §Design "Derive where decidable, declare where not"; §Known Divergences "Additive-only model-diff
vs semantic change".

**Pre-conditions.** L0 landed.

**Depends on.** —.

**TDD tests to write first.**
- `crates/smelt-logical/src/…` unit — an edit adding a new column whose expression references only existing
  target columns + a monotone dimension → `AdditiveOnly`; an edit changing an existing column's expression
  → `NotAdditive{reason}` (a rebuild / declared migration is required).
- `crates/smelt-logical/src/…` unit — an edit adding a column that references a *new* non-monotone
  dependency → `NotAdditive`.
- `crates/smelt-logical/src/…` unit (fail-closed) — an unresolvable diff (renamed column, ambiguous
  lineage) yields `NotAdditive` (never optimistic `AdditiveOnly`).

**Implementation shape.** `additive_only_diff(old_model, new_model, ctx) -> AdditiveOnly | NotAdditive`:
compare the two column sets + dependency sets; admit only pure additions derivable from
`{existing} ∪ {monotone dim}`; any change to an existing column or a non-monotone new dependency refuses.

**Critical files.**
- `crates/smelt-logical/src/…` (new model-diff proof; likely reads the model graph / schema).

**Docs touched.**
- `model_properties.md` §Surface — flip "Additive-only model-diff" to `built`; §Known Divergences — narrow
  the "Additive-only model-diff vs semantic change" note to the residual declared-migration-intent gap.
- `docs-site/` — none (internal; the transform it licenses gets user docs in F14).

**Review checklist.**
- [ ] Pure column-additions from `{existing} ∪ {monotone dim}` → `AdditiveOnly`; existing-column edits →
      `NotAdditive`.
- [ ] Non-monotone new dependency → `NotAdditive`.
- [ ] Unresolvable diff fails closed.
- [ ] §Surface cell flipped; semantic-change note narrowed; edits timeless.

**Commit.** `feat(logical): additive-only model-diff proof (licenses targeted backfill)`

---

### Phase F9: Input-delta discovery

**Goal.** The proof stage of the input-consumption axis: derive *which* rows are new from the source's
shape — **window-forward** (a `timeseries:` source), **snapshot-diff** (a mutable snapshot), or
**change-feed** (a CDF-bearing source). It never changes what the stored relation means (`models.md`
§"Input-consumption axis"); it pairs with the source mutation-profile world-fact (catalogued, declared in
`sources.md`) and the re-scan/probe transform (per-mode, L4).

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Input-delta discovery"** (`partial` →
`built`); §Semantics "Interactions" → "Input-consumption axis"; §"Catalogued inputs" (the source mutation
profile). `model_maintenance.md` §Interactions "Input-consumption".

**Pre-conditions.** L0 landed. Reads the catalogued source mutation profile (does not re-home it).

**Depends on.** —.

**TDD tests to write first.**
- `crates/smelt-logical/src/…` unit — a `timeseries:` (append-only clock) source → `WindowForward`; a
  mutable snapshot source → `SnapshotDiff`; a CDF-bearing source → `ChangeFeed`.
- `crates/smelt-logical/src/…` unit (fail-closed) — a source whose mutation profile is unknown /
  underivable yields the conservative verdict (whole-relation re-scan, i.e. no unsound delta), never an
  optimistic window-forward that would silently drop rows.

**Implementation shape.** `input_delta_discovery(source, ctx) -> WindowForward | SnapshotDiff |
ChangeFeed` keyed on the source's mutation profile + clock presence. Proof only — the re-scan vs
delta-probe *transform* is wired per consuming mode (L4), not here.

**Critical files.**
- `crates/smelt-logical/src/…` (new input-delta proof); reads `sources.md`'s mutation profile via core
  metadata.

**Docs touched.**
- `model_properties.md` §Surface — flip "Input-delta discovery" to `built`.
- `models.md` §"Input-consumption axis" — verify the proof-stage prose matches (no phase vocabulary).
- `docs-site/` — none (internal; consuming modes document the behaviour).

**Review checklist.**
- [ ] The three delta kinds derived from source shape; proof only (no transform wired).
- [ ] Unknown mutation profile fails closed to a whole-relation re-scan.
- [ ] §Surface cell flipped; edits timeless.

**Commit.** `feat(logical): input-delta discovery proof (window-forward / snapshot-diff / change-feed)`

---

### Phase F10: Window-independence / ordered-execution

**Goal.** A proof that a window reads only its sources (parallelisable) versus its own prior output
(ordered) — via self-edge detection in the model DAG. The verdict is the orchestrator signal that gates
parallel vs strictly-sequential backfill; a self-reference the planner cannot prove converges
partition-by-partition is refused.

**Spec anchor.** `model_properties.md` §"Derived proofs" → row **"Window-independence / ordered-execution"**
(`not-yet` → `built`).

**Pre-conditions.** L0 landed.

**Depends on.** —.

**TDD tests to write first.**
- `crates/smelt-logical/src/…` unit — a model reading only external sources → `WindowIndependent`
  (parallelisable); a model with a DAG self-edge reading its own prior partitions → `Ordered`.
- `crates/smelt-logical/src/…` unit (fail-closed) — a self-reference reading *forward* / whole-history
  (non-converging) → refused, naming the non-convergent self-edge (never silently `Ordered`).

**Implementation shape.** `window_independence(model, graph, ctx) -> WindowIndependent | Ordered |
Refused{reason}`: detect the self-edge in `ModelGraph`; admit `Ordered` only for a backward-bounded
self-reference (prior partitions), refuse forward / whole-history. Proof/signal only — the ordered-backfill
chunker is batched-local (L4).

**Critical files.**
- `crates/smelt-logical/src/graph.rs` — `ModelGraph`, self-edge detection.
- `crates/smelt-logical/src/…` — the window-independence proof.

**Docs touched.**
- `model_properties.md` §Surface — flip "Window-independence / ordered-execution" to `built`; §Known
  Divergences — remove "window-independence/ordered-execution" from the unbuilt-classifiers note.
- `docs-site/` — none (internal; the ordered-backfill behaviour is documented by the batched mode).

**Review checklist.**
- [ ] Source-only → `WindowIndependent`; backward-bounded self-edge → `Ordered`.
- [ ] Forward / whole-history self-reference refused (fail-closed).
- [ ] §Surface cell flipped; edits timeless.

**Commit.** `feat(logical): window-independence / ordered-execution proof via DAG self-edge detection`

---

### Phase F11: Windowed-keyed-maintenance driver

**Goal.** Lift the cumulative-orchestration loop into a **mode-agnostic** driver: `classify → step over
driving partitions in temporal order → per-partition pushdown → create-or-merge`. Cumulative stays the
reference path and must not regress; the driver is the reusable mechanism the other keyed modes (L4) will
sequence `merge_into` through.

**Spec anchor.** `model_transforms.md` §Surface → row **"Windowed-keyed-maintenance driver"**
(`partial (cumulative-orchestration today)` → `built`); §Semantics "Keyed `merge_into`" (the step-loop
paragraph); §Known Divergences "the windowed-keyed-maintenance driver … only partially built". Invariant:
`model_maintenance.md` §"The equivalence invariant" (end-state equivalence).

**Pre-conditions.** F2 (driving-fact resolver) and F4 (monoid rung discriminant) landed.

**Depends on.** F2, F4.

**TDD tests to write first.**
- `crates/smelt-logical/src/…` (or `smelt-runtime`) unit — the driver sequences `merge_into` across
  driving partitions in temporal order for a monoid combiner; the classify → step → pushdown →
  create-or-merge stages are exercised independently of `cumulative`'s rule module.
- `crates/smelt-cli/tests/cumulative*` (real fixtures) — cumulative's end-state equivalence is **unchanged**
  after re-expressing its loop on the shared driver (no regression). Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-logical/src/…` unit (fail-closed) — a non-monoid combiner is **refused** by the driver
  (routed to full refresh / diagnostic), never merged approximately (`model_transforms.md` §Constraints).

**Implementation shape.** Extract cumulative's per-partition orchestration into a driver parameterised by
`{driving-fact resolver (F2), rung discriminant (F4), per-partition pushdown, create-or-merge}`; re-express
`cumulative` on it. No new mode wired — the driver is the mechanism; `latest_value`/`versioned` compose it
later (L4).

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — the orchestration loop, re-expressed on the driver.
- `crates/smelt-runtime/src/compile.rs` — the create-or-merge emit path.
- new driver module (`smelt-logical` or `smelt-runtime`).

**Docs touched.**
- `model_transforms.md` §Surface — flip "Windowed-keyed-maintenance driver" to `built`; §Known Divergences
  — remove/narrow the "only partially built … reference path" note (the driver is now the normative
  mechanism, cumulative one consumer).
- `docs-site/` — none (internal; the mode pages document their own behaviour).

**Review checklist.**
- [ ] Driver is mode-agnostic (classify/step/pushdown/create-or-merge), parameterised by F2 + F4.
- [ ] Cumulative re-expressed on it; end-state equivalence unchanged (no regression).
- [ ] Non-monoid combiner refused, never merged approximately (fail-closed).
- [ ] §Surface cell flipped; driver note narrowed; edits timeless.

**Commit.** `refactor(logical): lift the cumulative loop into a mode-agnostic windowed-keyed-maintenance driver`

---

### Phase F12: Hidden decomposed state + presentation view mechanism

**Goal.** The transform that stores a monoid element that is *not* the user value (`(sum,count)` / Welford
triple / HLL vector) and exposes the user value through a pure presentation view `π(state)`. `merge_into`
maintains the state element; `π` never touches history. Sound iff `π` is pure over one consistent state
row (F7).

**Spec anchor.** `model_transforms.md` §Surface → row **"Hidden decomposed state + presentation view"**
(`unbuilt` → `built`); §Semantics "Hidden decomposed state + presentation view"; §Design "The ladder is
the maintainable/delegated boundary" (rung 2). Licensed by the decomposed-monoid discriminant (F4) +
presentation-map purity (F7).

**Pre-conditions.** F4 (decomposable discriminant) and F7 (presentation-map purity) landed.

**Depends on.** F4, F7.

**TDD tests to write first.**
- `crates/smelt-logical/src/…` / `smelt-runtime` unit — a decomposable aggregate (`AVG`) lowers to a state
  column `(sum,count)` maintained by componentwise `+` plus a presentation view emitting `sum/count`; the
  `(state, view)` pair is one atomically-swapped unit.
- `crates/smelt-logical/src/…` unit (fail-closed) — a holistic aggregate (`MEDIAN`) or an **impure** `π`
  (per F7) is **refused** (routed elsewhere), never decomposed approximately.

**Implementation shape.** A transform that, given a decomposable combiner (F4) and a pure `π` (F7), emits
the hidden-state column + presentation view; the state is maintained by `merge_into` (built). This is the
*mechanism*; which mode drives it (cumulative rung-2) is L4 (existing Group C C1/C2).

**Critical files.**
- `crates/smelt-runtime/src/transformer.rs` / `compile.rs` — state-column + view emit.
- `crates/smelt-logical/src/…` — the decompose-to-state lowering keyed on F4/F7.

**Docs touched.**
- `model_transforms.md` §Surface — flip "Hidden decomposed state + presentation view" to `built`; §Known
  Divergences — remove it from the "Unbuilt" list.
- `docs-site/` — none here (the cumulative rung-2 page documents the user-facing behaviour at L4).

**Review checklist.**
- [ ] Decomposable combiner → hidden state + pure view, `(state,view)` atomic.
- [ ] Holistic combiner / impure `π` refused (fail-closed).
- [ ] `merge_into` maintains state; `π` never reads history.
- [ ] §Surface cell flipped; unbuilt entry removed; edits timeless.

**Commit.** `feat(runtime): hidden decomposed-state column + pure presentation view mechanism`

---

### Phase F13: Two-layer widened-scan / exact-clamp redesign

**Goal.** Fix the over-widened *written* window. Redesign so a finite-frame-reach transform **scans**
`[start − k − offset, end)` (wide enough to compute the window at the left edge) while the output clamp
**writes** only `[start, end)` — the margin is read but never re-written. This replaces the runtime's
current behaviour, which over-widens the write window (double-counting at partition edges) and under-reads
the scan.

**Spec anchor.** `model_transforms.md` §Surface → row **"Two-layer widened-scan + exact output clamp"**
(`partial (redesign)` → `built`); §Semantics "Source-filter pushdown + the two clamps"; §Design "Rejected:
auto-widening the write window"; §Constraints "Write window = output window; scan window ⊇ output window";
§Known Divergences "Two-layer widened-scan is a *partial* redesign". Invariant: `model_maintenance.md`
§"The equivalence invariant" (per-partition).

**Pre-conditions.** F1 landed (the unified bound/reach `k`/`offset`).

**Depends on.** F1.

**TDD tests to write first.**
- `crates/smelt-runtime/src/transformer.rs` unit — a model with finite frame reach `k` emits a scan filter
  over `[start − k − offset, end)` and an output clamp over `[start, end)`; assert the two windows differ
  and the write window equals the output window exactly.
- `crates/smelt-cli/tests/incremental*` (real fixture, e.g. a bounded-`RANGE`/interval-join model under
  `examples/timeseries/`) — the redesigned transform matches full refresh per partition at the left edge
  (the former over-widen double-count is gone). Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/transformer.rs` unit — the transparent single-source zero-margin fast path
  (`is_transparent_single_source`) still drops the redundant outer clamp (pushdown filter *is* the clamp).

**Implementation shape.** Split the widened-scan and the output clamp into two independently-derived
windows from F1's `BoundResult`; the scan reads the margin, the clamp writes the output window. Remove the
old write-window widening; keep the transparent fast path.

**Critical files.**
- `crates/smelt-runtime/src/transformer.rs` — `inject_source_filters`, `inject_time_filter`,
  `is_transparent_single_source`.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — the `k`/`offset` read (via F1).

**Docs touched.**
- `model_transforms.md` §Surface — flip "Two-layer widened-scan + exact output clamp" to `built`; §Known
  Divergences — remove the "partial redesign … over-widens the written window" note (keep the transparent
  fast-path statement as a plain fact).
- `docs-site/docs/guide/incremental-models.md` — verify the lookback/margin prose matches the read-margin /
  write-window split (no user-facing surface *change*, but the described behaviour must be accurate).

**Review checklist.**
- [ ] Scan window ⊇ output window; write window = output window exactly (no over-widen).
- [ ] Left-edge per-partition equivalence holds (double-count gone).
- [ ] Transparent zero-margin fast path preserved.
- [ ] §Surface cell flipped; redesign note removed; edits timeless.

**Commit.** `refactor(runtime): two-layer widened-scan / exact output-clamp — read the margin, write only the window`

---

### Phase F14: Targeted column backfill

**Goal.** The transform that, for an **additive-only model diff** (F8), edits only the added columns in
place (`UPDATE` / dimension-merge) instead of a full rebuild. Refuses (falls back to rebuild) the moment
the additive-only licence does not hold.

**Spec anchor.** `model_transforms.md` §Surface → row **"Targeted column backfill"** (`unbuilt (new)` →
`built`); §Semantics "Targeted column backfill and dimension-driven horizon MERGE"; §Constraints
"Equivalence or refusal". Licensed by additive-only model diff (F8).

**Pre-conditions.** F8 landed (additive-only model-diff proof).

**Depends on.** F8.

**TDD tests to write first.**
- `crates/smelt-runtime/src/…` unit — an `AdditiveOnly` diff (F8) emits an in-place `UPDATE`/dimension-merge
  of only the added columns, not a full `CREATE ... AS SELECT` rebuild.
- `crates/smelt-cli/tests/…` (real fixture) — after an additive column edit, a targeted backfill produces
  the same target state a full rebuild would (equivalence), touching only the new columns. Requires
  `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/…` unit (fail-closed) — a `NotAdditive` diff **refuses** the targeted backfill
  and falls back to rebuild (never an approximate in-place edit).

**Implementation shape.** A transform gated on F8's verdict emitting the in-place column `UPDATE` /
dimension-merge; on `NotAdditive` it declines and the caller rebuilds. The *when* (schema evolution flow)
is a consumer concern; this phase builds the mechanism + its licence check.

**Critical files.**
- `crates/smelt-runtime/src/transformer.rs` / `compile.rs` — the targeted-backfill emit.
- `crates/smelt-logical/src/…` — F8 verdict consumed as the licence.

**Docs touched.**
- `model_transforms.md` §Surface — flip "Targeted column backfill" to `built`; §Known Divergences — remove
  it from the "Unbuilt" list.
- `docs-site/` — a schema-evolution / backfill note if a user-facing surface exists; otherwise verify the
  guide prose matches (mechanism is licensed by an additive edit).

**Review checklist.**
- [ ] `AdditiveOnly` → in-place UPDATE/dimension-merge of only added columns; equivalence-tested.
- [ ] `NotAdditive` refuses, falls back to rebuild (fail-closed, no approximate edit).
- [ ] §Surface cell flipped; unbuilt entry removed; edits timeless.

**Commit.** `feat(runtime): targeted column backfill for an additive-only model diff`

---

### Phase F15: Dimension-driven horizon-bounded MERGE

**Goal.** The transform that merges a dimension batch straight into the target replica slice
`[conv_ts − H, conv_ts]` **without re-reading the fact** — licensed by target-as-replica (`merge_into`,
built) **plus** join-contribution monotonicity (F6) **plus** a bounded horizon `H` (F1's forward `after`
reach). Refuses (falls back to rebuild) the moment any licence does not hold.

**Spec anchor.** `model_transforms.md` §Surface → row **"Dimension-driven horizon-bounded MERGE"**
(`unbuilt (new)` → `built`); §Semantics "Targeted column backfill and dimension-driven horizon MERGE".
Licensed by target-as-replica + join-contribution monotonicity (F6) + horizon `H`.

**Pre-conditions.** F6 (join-contribution monotonicity + fan-out) and F1 (forward `after`/`H` reach) landed.

**Depends on.** F1, F6.

**TDD tests to write first.**
- `crates/smelt-runtime/src/…` unit — given a monotone join contribution (F6) + a bounded `H`, a dimension
  batch merges into the target slice `[conv_ts − H, conv_ts]` via `merge_into`, and the fact table is not
  re-read.
- `crates/smelt-cli/tests/…` (real fixture) — the horizon MERGE produces the same target state a full
  refresh would over the affected slice (equivalence). Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/…` unit (fail-closed) — a non-monotone contribution (F6 refuses) or an
  **unbounded** horizon (no `H` from F1) **refuses** the MERGE and falls back to rebuild, never merging
  approximately.

**Implementation shape.** A transform composing built `merge_into` with F6's contribution-monotonicity
licence and F1's forward reach `H`; emits the horizon-bounded MERGE into the target slice; declines on any
missing licence. Consumers (`accumulating_snapshot`, batched enrichment) wire it at L4.

**Critical files.**
- `crates/smelt-runtime/src/transformer.rs` / `compile.rs` — the horizon-MERGE emit.
- `crates/smelt-logical/src/…` — F6 licence + F1 `H` consumed.

**Docs touched.**
- `model_transforms.md` §Surface — flip "Dimension-driven horizon-bounded MERGE" to `built`; §Known
  Divergences — remove it from the "Unbuilt" list.
- `docs-site/` — none here (the consuming mode documents the user-facing behaviour at L4).

**Review checklist.**
- [ ] Monotone contribution + bounded `H` → MERGE into `[conv_ts − H, conv_ts]`; fact not re-read;
      equivalence-tested.
- [ ] Non-monotone contribution or unbounded horizon refuses, falls back to rebuild (fail-closed).
- [ ] §Surface cell flipped; unbuilt entry removed; edits timeless.

**Commit.** `feat(runtime): dimension-driven horizon-bounded MERGE (target-as-replica + monotone contribution + H)`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this sub-plan.)

- Retraction via delta history, bounded-domain multiset, compile-time pinning, UNION-branch
  wrap-and-filter, watermark settled-delay/tail-rewrite — deferred to later layers per §Scope
  "Explicitly deferred (to later layers)".

## Verification

How to confirm L1 + L2 are satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Consolidations did not regress.** The existing `smelt-cli` batched + cumulative integration/equivalence
  tests (`crates/smelt-cli/tests/incremental*`, `.../cumulative*`) stay green after F1–F3 and F11 — the
  per-partition / end-state equivalence oracle is the net (`model_maintenance.md` §"The equivalence
  invariant"). Requires `DUCKDB_LIB_DIR`.
- The generative monotonicity soundness oracle stays green throughout — no consolidation or new proof
  admits an unsound verdict.
- **Each L2 transform is equivalence-or-refusal.** F11/F13/F14/F15 each have a full-refresh /
  per-partition equivalence real-fixture test **and** a fail-closed refusal unit test (an unlicensed model
  is refused with a diagnostic, never applied approximately — `model_transforms.md` §Constraints).
- `cargo test -p smelt-cli --test example_diagnostics` and `-p smelt-lsp --test example_workspaces` —
  example workspaces build with zero diagnostics.
- `/smelt:validate model_properties` and `/smelt:validate model_transforms` report zero drift for the
  surfaces this layer touches; every §Surface maturity cell this plan flips to `built` is `built`, and
  every §Known-Divergence duplication note (the six) is gone from `model_properties.md` /
  `model_maintenance.md`.
