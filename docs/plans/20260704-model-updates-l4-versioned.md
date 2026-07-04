# Plan: Model updates — L4 composition for `refresh: versioned` (SCD Type 2)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L4 mode-composition** layer for the keyed `versioned` mode of the re-cut master.
**Specs (oracles)**:
- [`docs/specs/versioned_models.md`](../specs/versioned_models.md) — PRIMARY. §Surface (the `refresh: versioned` opt-in + output shape); §Semantics §"Composition table" (the by-name capability references) + §"End-state equivalence (interval-keyed)" + §"Input consumption is derived from the source"; §"Versioned-local machinery" (close-old / open-new interval combiner; smelt-managed validity columns `[valid_from, valid_to)` + `is_current`; tracked-attribute selection; validity stamped from source event-time; deletion handling); §Constraints & Invariants (1–5); §Known Divergences (does-not-parse; validity-column surface unsettled; tracked-attribute unsettled; late-correction retraction).
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — the framework this mode composes into. §"The equivalence invariant" — **ONE invariant**; `versioned` is **key-addressed (identity-requiring `merge_into`)**, and is the spec's **canonical proof that output addressing, not the source clock, is the axis**: admitting a new value for a key requires **closing the previously-open version — a row whose timestamp lies arbitrarily far outside the current input window** — so `versioned` can never be a per-partition rewrite; its *scan* may still be windowed (if clocked) but its *write* reaches back by key. §"The algebraic maintenance ladder" (the close-old/open-new is a value/order-monotone keyed fold on the smelt-maintained side); §"Windowed maintenance and the horizon" (scan ⊇ write; derived horizon); §"Validator, not chooser".
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — the proofs consumed **by exact name**: value-monotone vs order-monotone discriminants; event-time monotonicity trace; driving-fact / anchor resolution; window-independence / ordered-execution. All fail-closed.
- [`docs/specs/model_transforms.md`](../specs/model_transforms.md) — the transforms driven **by exact name**: keyed **`merge_into`** sequenced by the **windowed-keyed-maintenance driver** + **source-filter pushdown** on the driving source. The close-old / open-new combiner stays **local** to `versioned_models.md` (§"Transforms that stay in a mode spec").
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis" (the peer enum); §"Constraint violations" (the keyed-mode `timeseries:`/`batched:` forbids on the model itself); §"Input-consumption axis" (window-forward vs snapshot-diff, derived).

**Research (the "why" + the L-decomposition)**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Target plan architecture (the re-cut master)" (L0–L4; this sub-plan is **L4 for `versioned`**), §"Mapping the current master onto the layers" (Group D re-cut as compositions). Prior mode-vertical design: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) Parts 17 + 19 (the user surface; naming; the input-consumption axis; §19.8 shared-executor question).

**Spec diff**: no new spec **file**. Two normative reconciles land as the mode ships (pre-authorised, made in the same commit as the code that realises them):
- `crates/smelt-core/src/config.rs` `RefreshStrategy` gains the **`Versioned`** enum variant (`"versioned"` de/serialisation) — today it accepts only `Full`/`Batched`/`Cumulative`/`MaterializedView`, so `refresh: versioned` fails deserialization (`versioned_models.md` §Known Divergences "Not implemented — does not parse").
- `models.md` **line 292** ("Keyed refresh modes beyond `cumulative` are not fully built") is reconciled as each phase lands: remove `versioned` from the "do **not** parse" list once V1 lands the variant; the `versioned`-specific clauses are dropped as the mode reaches end-state equivalence.
- `versioned_models.md` §Surface / §"Versioned-local machinery" promotions: the **validity-column surface** (exact `valid_from`/`valid_to`/`is_current` names & types, open-interval representation) and the **tracked-attribute rule** move from §Known Divergences → normative §Surface as V3/V4 decide them; each §Known-Divergence "unsettled" note is removed or narrowed to the still-open sub-behaviour in the same commit.

**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan is the **L4 composition for `refresh: versioned`** only. It **supersedes the D2 portion** of the mode-vertical Group D sub-plan ([`docs/plans/20260704-model-updates-group-d.md`](20260704-model-updates-group-d.md) §"Phase D2"): where D2 planned to *re-derive* the driving-source loop, the validity maintenance, and the classifier inside the mode, this plan **wires the L1/L2 fundamentals by name** — the classifier consumes the shared driving-fact resolver and value/order-monotone discriminants, and the maintenance folds through the shared windowed-keyed-maintenance driver + keyed `merge_into` — rather than growing a private copy. Group D's **D1** (`latest_value`) and **D3** (`materialized_view`) are **not** superseded here; they are re-cut as their own L4 sub-plans (D1 as an L4 overwrite composition; D3 as the thin delegated-emit path). When this sub-plan is registered in the master, mark the Group D registry row's D2 obligation as **superseded by** this file so the loop does not run both against the same branch.

**Maturity honesty.** `refresh: versioned` is **not built and does not parse today** — declaring it produces an unknown-refresh-value error. Every phase below flips a concrete piece of that gap from unbuilt → built and narrows the matching §Known-Divergence note; no phase claims a behaviour it has not landed a green test for.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when this
L4 layer is scaffolded into the registry — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant oracle for every phase is the **end-state equivalence invariant in its interval-keyed
   specialisation** (`model_maintenance.md` §"The equivalence invariant"): the user-visible set of
   `(key, version, validity interval)` rows equals a full rebuild over the processed snapshots, independent
   of merge order. `versioned` is **key-addressed** — its close-out write reaches a stored row **by key,
   arbitrarily far outside the current input window** (the canonical proof that addressing, not the source
   clock, is the axis). Every proof this mode consumes is **fail-closed** (`model_properties.md`
   §Constraints): an undecidable construct yields the reject verdict, never an optimistic default. Every
   transform is licensed **because it preserves** the invariant and is **refused with a diagnostic** when it
   cannot (`model_transforms.md` §Constraints "Equivalence or refusal").
2. Confirm you are on branch `worktree-incremental`, that Group A (A1 — the `RefreshStrategy` enum exists to
   add `Versioned` to) is landed, and that this phase's **Depends on** F-phases (below) are `done` in
   [`docs/plans/20260704-model-updates-fundamentals.md`](20260704-model-updates-fundamentals.md)'s
   Progress-tracking table. If a dependency is not `done`, set the row `blocked` per the block rule.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status to
   `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names a
fail-closed reject test, and every phase from V2 on adds an **interval-keyed end-state-equivalence** test
that exercises the **out-of-window close-out** — a new value for a key closing a version whose timestamp
lies outside the current run window) → reviewer subagent (material findings only) → iterate → set the row
`done` → commit + push with the phase's `Commit.` line.

**Equivalence-harness tests need DuckDB.** Phases that emit maintenance SQL (V2–V5) assert interval-keyed
end-state equivalence (and order-independence) via the DuckDB harness; those require `DUCKDB_LIB_DIR` set
(and `LD_LIBRARY_PATH`) per `CLAUDE.md`. The pure classifier phase (V1) is a `smelt-core` /
`smelt-logical` unit phase, but its example fixture still builds under DuckDB.

**Compose, do not re-derive (supersedes D2).** This mode is built by **wiring fundamentals by name**. The
classifier reads the shared **driving-fact / anchor resolution** (F2) and **value/order-monotone
discriminants** (F4); the maintenance folds through the shared **windowed-keyed-maintenance driver** (F11)
emitting keyed **`merge_into`**; the scan is bounded by the shared **bound/reach** (F1) + **widened-scan /
exact-clamp** source-filter pushdown (F13); ordered execution is gated by **window-independence /
ordered-execution** (F10). A phase that finds itself copying a fundamentals analysis into
`rules/versioned.rs` is a bug in the composition — call the shared capability instead, or block on the
missing F-phase.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary (V1–V5) lives in *this file only*. Spec +
`docs-site/` edits describe the feature as if it always existed; as each phase lands, **promote** the
settled surface into `versioned_models.md` §Surface / §"Versioned-local machinery" and **remove/narrow** the
matching §Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec, an unmet dependency (a required
F-phase or Group A not `done`), or a pre-flight red unrelated to this phase's target: set the row `blocked`
with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit
`<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape re-cut the maintained-model family "fundamentals-first"
(`docs/research/20260704-maintenance-fundamentals.md`): the refresh modes are **compositions** of shared
**derived proofs** (`model_properties.md`) and **physical transforms** (`model_transforms.md`), each proven
against the one equivalence invariant (`model_maintenance.md`). The mode-vertical Group D planned each new
keyed mode as a self-contained vertical that re-derived the driving-source loop and maintenance path. This
L4 sub-plan re-cuts the `versioned` vertical (Group D's D2) as a composition: it lands the mode's **local**
machinery — the close-old / open-new interval combiner, the smelt-managed validity columns, tracked-attribute
selection, event-time-stamped validity, deletion handling — and **wires the fundamentals by name** for
everything else.

`versioned` is the spec's sharpest case for one framework claim: **output addressing, not the source clock,
is the axis that drives the physical transform** (`model_maintenance.md` §"The equivalence invariant",
§Design "One invariant, not two"). Admitting a new value for a key means **closing the previously-open
version** — writing `valid_to` / clearing `is_current` on a stored row whose `valid_from` may lie arbitrarily
far in the past, outside any bounded input window. That retroactive close-out is intrinsic to the mode: it
cannot be expressed as a whole-partition rewrite, so `versioned` is **key-addressed** regardless of whether
its source is clocked. The mode may still *window its scan* (a `timeseries:` update-events / CDC source is
consumed window-forward via source-filter pushdown), but its *write reaches back by key*. This plan makes
that split explicit in the maintenance emit: the driver folds the combiner over the scanned window in event
order, and the `merge_into` writes the close-and-reopen wherever the matched key lives.

## Scope

### In scope (L4 — the `versioned` composition)

- **V1** — `refresh: versioned` parse/selector + classifier. Add the `Versioned` `RefreshStrategy` variant
  (it does not parse today); classify the model (natural key + tracked attributes; no partition column on
  the model itself) by **composing** the driving-fact / anchor resolution (F2) and value/order-monotone
  discriminants (F4) — not a private copy. Enforce the keyed-mode constraint violations.
- **V2** — Close-old / open-new interval-maintenance combiner via keyed `merge_into` — **the out-of-window
  keyed write**. Fold the combiner through the shared windowed-keyed-maintenance driver (F11); the
  `merge_into` closes the matched key's open version and opens a new one, reaching back by key outside the
  input window. This is the phase that proves the mode key-addressed.
- **V3** — smelt-managed validity columns (`valid_from` / `valid_to` / `is_current`) + the interval
  invariant: non-overlapping per key, closed intervals abutting at shared boundaries, ≤ one `is_current`
  row per key. Promote the settled column surface into `versioned_models.md` §Surface.
- **V4** — Tracked-attribute selection (which columns' changes open a new version) + deletion handling
  (soft-close on retraction; CDC delete-event as close signal).
- **V5** — Event-time-stamped validity (`valid_from`/`valid_to` from the **source's event time**, never the
  run clock) + window-independence / ordered-execution enforcement (F10): windows applied in temporal order
  because close/open is inherently ordered; scan bounded by F1/F13 while the write stays key-addressed.

### Explicitly deferred (out of this sub-plan)

- **Late corrections to an already-closed interval** and an **opt-in hard-delete surface** — the shared
  keyed-mode retraction question (`versioned_models.md` §Known Divergences; `cumulative_aggregate.md`
  §"Reprocessing semantics"). Deletion as a **soft-close** *is* in scope (V4); correcting a *closed*
  interval is not. Stays an Open Question.
- **Snapshot-diff `--auto` staleness** for a clock-less mutable snapshot source (`models.md` §Known
  Divergences; research §19.8). Snapshot-diff ships as **always-full-rescan-and-compare** first (identical
  end-state contract, only scan cost differs — `versioned_models.md` §"Input consumption"); the staleness
  firing is recorded under §Deferred, not claimed settled.
- **Group D's D1 (`latest_value`) and D3 (`materialized_view`)** — separate L4 sub-plans. This file
  supersedes **only** D2.
- The **shared-executor-vs-per-rule-copies** question (research §19.8): in the L4 re-cut this is **settled**
  — the windowed-keyed-maintenance driver (F11) *is* the shared executor, and `versioned` composes it by
  name. No per-rule copy of the driving-source loop is created (that was the D2 posture this plan supersedes).

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| V1 | Group A (A1); F2; F4 | `versioned_models.md` §Surface, §"Composition table"; `models.md` §"Constraint violations" | pending |
| V2 | V1; F11 (driver); F4 | `versioned_models.md` §"Close-old / open-new interval maintenance"; `model_maintenance.md` §"The equivalence invariant" (key-addressed close-out) | pending |
| V3 | V2 | `versioned_models.md` §"Validity columns (smelt-managed)", §Constraints 3 | pending |
| V4 | V2; V3 | `versioned_models.md` §"Tracked-attribute selection", §"Deletion handling" | pending |
| V5 | V2; F1; F10; F13 | `versioned_models.md` §"Validity stamped from source event-time", §"Input consumption"; `model_maintenance.md` §"Windowed maintenance and the horizon" | pending |

---

### Phase V1: `refresh: versioned` parse/selector + classifier

**Goal.** Make `refresh: versioned` parse and select the mode (implying stored `table`), and classify an
eligible model. Add the `Versioned` variant to `RefreshStrategy` (it does **not** parse today). Classify
the model — natural key + tracked attribute columns, **no** partition column on the model itself — by
**composing** the shared **driving-fact / anchor resolution** (F2, to resolve the driving source of a
window-forward `timeseries:` source) and the **value/order-monotone discriminants** (F4, to confirm the
close-old / open-new fold is a value/order-monotone keyed step). Enforce the keyed-mode constraint
violations: `refresh: versioned` + `timeseries:` on the model and `refresh: versioned` + a `batched:` block
are both hard errors (`models.md` §"Constraint violations"). **No** private driving-fact or monotonicity
analysis inside `rules/versioned.rs` — call the fundamentals.

**Spec anchor.** `versioned_models.md` §Surface (the opt-in + output shape), §"Composition table" (properties
required: value/order-monotone discriminants + event-time trace + driving-fact resolution; the by-name
references); §Constraints 1–2. `models.md` §"Refresh axis", §"Constraint violations". Reconcile: `config.rs`
`RefreshStrategy` gains `Versioned`; `models.md` line 292 drops `versioned` from the "do not parse" list.

**Depends on.** Group A (A1 — `RefreshStrategy` exists to extend); **F2** (driving-fact / anchor resolution);
**F4** (value/order-monotone discriminants). Consumes the W1 event-time trace read-only (already `done`).

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` (or `metadata.rs`) unit — `refresh: versioned` deserialises to
  `RefreshStrategy::Versioned`; a bare `refresh: foo` still errors listing `versioned` among the valid
  values.
- `crates/smelt-core/src/metadata.rs` unit (fail-closed) — the keyed-mode constraint violations fire:
  `refresh: versioned` + `timeseries:` **on the model** is a hard error, and `refresh: versioned` + a
  `batched:` block is a hard error (`models.md` §"Constraint violations"), each naming the offending block.
- `crates/smelt-logical/src/rules/versioned.rs` (new) unit — the classifier accepts a SELECT projecting a
  natural key + tracked attributes and resolves the driving source **via the shared F2 resolver** (assert
  it delegates, not a local ref-count); the close-old/open-new fold is confirmed value/order-monotone **via
  the shared F4 discriminant**.
- `crates/smelt-logical/src/rules/versioned.rs` unit (fail-closed reject) — a model with **no derivable
  natural key**, or an **ambiguous driving fact** (F2 returns zero / two `Traceable` inputs), or a combiner
  F4 classifies as non-monotone, is **refused** with a named diagnostic — never optimistically admitted.
- `examples/versioned_snapshot/` real fixture — a `refresh: versioned` model builds with zero diagnostics
  (`cargo test -p smelt-cli --test example_diagnostics`). (End-state maintenance lands V2; V1's fixture
  asserts the model classifies and builds an initial version.)

**Implementation shape.** Add `RefreshStrategy::Versioned` (`config.rs:26`) + its Deserialize/Serialize arms
(`"versioned"`); add the keyed-mode constraint-violation rows (forbid `timeseries:` and a `batched:` block
on the model) mirroring the `cumulative` validation branch (`metadata.rs`). New classifier
`crates/smelt-logical/src/rules/versioned.rs` (sibling of `cumulative.rs`) that **calls** the F2 resolver and
F4 discriminant rather than re-deriving them; derive natural key + tracked attributes from the SELECT; resolve
window-forward vs mutable-snapshot input via the source's `timeseries:` shape (`SourceTimeseriesMap`, the same
signal `cumulative` reads). No maintenance emit yet.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy` (`:26-58`); `crates/smelt-core/src/metadata.rs` —
  keyed-mode constraint violations.
- `crates/smelt-logical/src/rules/versioned.rs` (new) + `rules/mod.rs`; read-only reference
  `rules/cumulative.rs`; the shared F2 resolver (`analysis/source_bounds.rs::resolve_join_driving_fact`) and
  F4 discriminant module.
- `examples/versioned_snapshot/` (new fixture).

**Docs touched.**
- `versioned_models.md` §Known Divergences — narrow the "Not implemented — does not parse" note to the
  still-unbuilt maintenance (the variant now parses and classifies).
- `models.md` line 292 — remove `versioned` from the "do **not** parse" list.
- `docs-site/docs/guide/` — add `refresh: versioned` to the refresh-modes guide (keyed + validity interval;
  keep-every-version); timeless.

**Review checklist.**
- [ ] `refresh: versioned` deserialises; both keyed-mode constraint violations enforced, each naming the block.
- [ ] Classifier **composes** F2 (driving fact) + F4 (value/order-monotone) — no private resolver or
      monotonicity list in `rules/versioned.rs`.
- [ ] No-key / ambiguous-driving-fact / non-monotone-combiner all fail closed with a named diagnostic.
- [ ] Example fixture classifies + builds clean; `config.rs`/`models.md`:292 reconciles landed; edits timeless.

**Commit.** `feat(refresh): parse + classify refresh: versioned by composing driving-fact + monotone discriminants`

---

### Phase V2: Close-old / open-new interval combiner via keyed `merge_into` (the out-of-window keyed write)

**Goal.** Land the versioned-**local** maintenance combiner and emit it through the shared
**windowed-keyed-maintenance driver** (F11) as a keyed **`merge_into`**. For each incoming row, keyed by
natural key: look up the key's current (open) version; if none, **open** a new version; if a tracked
attribute differs, **close** the old version (set `valid_to`, clear `is_current`) and **open** a new one at
that boundary; if nothing tracked differs, do nothing. **This is the out-of-window keyed write** — the
close-out sets `valid_to` / clears `is_current` on a stored row whose `valid_from` lies arbitrarily far
outside the current input window, so the `merge_into` reaches back **by key, wherever the row lives**. This
phase is the concrete proof that `versioned` is key-addressed and can never be a per-partition rewrite.

**Spec anchor.** `versioned_models.md` §"Close-old / open-new interval maintenance (the combiner)"; §"End-state
equivalence (interval-keyed)". `model_maintenance.md` §"The equivalence invariant" (the key-addressed close-out
— "a row whose timestamp lies arbitrarily far outside the current input window"), §"The algebraic maintenance
ladder" (value/order-monotone keyed fold). `model_transforms.md` §"Keyed `merge_into`", §"Windowed-keyed-
maintenance driver".

**Depends on.** V1 (the classifier); **F11** (the windowed-keyed-maintenance driver the combiner folds
through); **F4** (the value/order-monotone rung the driver gates on).

**TDD tests to write first.**
- `crates/smelt-runtime/src/…` unit — the close-old/open-new emit, given a matched open version and a
  differing tracked attribute, produces one `merge_into` that (a) closes the matched key's open version and
  (b) opens a new one at the shared boundary; an unmatched key opens only. No wholesale history re-read.
- `crates/smelt-runtime/src/…` unit — the combiner folds through the **F11 driver** (assert it is sequenced
  by the shared driver, not a private per-partition loop copied from `cumulative.rs`).
- `crates/smelt-cli/tests/versioned*` (real fixture, **out-of-window close-out equivalence**) — a key whose
  open version was created in an earlier, now-out-of-window run gets a new value in the current window; the
  maintained table closes the **old** (out-of-window) version and opens the new one, and equals a full
  rebuild over the processed snapshots. This is the load-bearing end-state-equivalence test: the write
  reaches outside the input window by key. Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/…` unit (fail-closed reject) — if the driver cannot sequence the combiner
  (F4 classifies the fold as non-monotone, or the driving fact is ambiguous), the maintenance is **refused**
  with a diagnostic and falls back to full refresh — never a partial/approximate merge (`model_transforms.md`
  §Constraints "Equivalence or refusal").

**Implementation shape.** Implement the close-old/open-new combiner as a versioned-local step (the boundary
timestamp shared between the close and the open so intervals abut). Register it with the F11 driver so the
driver's `classify → step over driving partitions in temporal order → per-partition pushdown →
create-or-merge` loop sequences it. Emit the maintenance as keyed `merge_into` (`smelt-backend`
`merge_into` trait, `:286`; DuckDB impl `:618`). If the plain upsert `merge_into` shape cannot express
close-and-reopen (matched → update the old row's `valid_to`/`is_current` **and** insert a new row), extend
the trait rather than hand-rolling raw SQL in the runtime. Add `ModelStrategy::Versioned` (`types.rs:150-172`)
+ its dispatch arm in `execute.rs`.

**Critical files.**
- `crates/smelt-runtime/src/…` — the versioned maintenance path (a `versioned` executor composing the F11
  driver, mirroring `cumulative.rs` structurally but delegating the loop to the shared driver);
  `execute.rs` dispatch; `types.rs:150-172` `ModelStrategy`.
- `crates/smelt-backend/src/lib.rs:286` — `merge_into` (extend for close-and-reopen if needed); DuckDB impl
  `crates/smelt-backend-duckdb/src/lib.rs:618`.
- `crates/smelt-logical/src/rules/versioned.rs` — hands the combiner spec to the driver.
- `examples/versioned_snapshot/` — extended with a multi-state key.

**Docs touched.**
- `versioned_models.md` §Known Divergences — remove the maintenance clause from the "Not implemented" note
  (close-old / open-new via `merge_into` now built); the combiner §"Versioned-local machinery" prose already
  normative, verify it matches.
- `model_transforms.md` §Known Divergences — narrow any "windowed-keyed-maintenance driver only partially
  built" clause now that `versioned` is a second consumer.
- `docs-site/` — verify the guide's keep-every-version prose matches.

**Review checklist.**
- [ ] Close-old/open-new emitted as one keyed `merge_into`; matched key closes + reopens, unmatched opens; no
      history re-read.
- [ ] The out-of-window close-out equivalence test is green: a new value closes a version created outside the
      current window and the table equals a full rebuild.
- [ ] The combiner folds through the **F11 driver** — no private per-partition loop.
- [ ] Non-monotone / ambiguous-driving-fact maintenance is refused (fail-closed), never partially merged.
- [ ] `ModelStrategy::Versioned` dispatched; edits timeless.

**Commit.** `feat(refresh): versioned close-old/open-new combiner via keyed merge_into — the out-of-window keyed write`

---

### Phase V3: smelt-managed validity columns + the interval invariant

**Goal.** Make `valid_from`, `valid_to`, and `is_current` **smelt-managed** — appended to the model's
projected columns, not projected by the user's SELECT — and enforce the interval invariant: validity
intervals are **non-overlapping per key**, closed intervals **abut at shared boundaries** with no gaps, and
there is **at most one `is_current` (open) version per key** at any time. Settle and **promote** the exact
column names/types and the open-interval representation (NULL vs far-future sentinel) from
`versioned_models.md` §Known Divergences into normative §Surface.

**Spec anchor.** `versioned_models.md` §"Validity columns (smelt-managed)", §Output shape, §Constraints 3
("Validity intervals are non-overlapping per key"). §Known Divergences "Validity-column surface is unsettled"
(promoted here).

**Depends on.** V2 (the combiner that writes the columns).

**TDD tests to write first.**
- `crates/smelt-runtime/src/…` unit — the maintained output schema is the user's projected columns **plus**
  the three managed validity columns; the user's SELECT does **not** project them (a user column named
  `valid_from` is a conflict diagnostic, not silently overwritten).
- `crates/smelt-cli/tests/versioned*` (real fixture) — a key with **three successive states** yields exactly
  **two closed intervals + one open** (`is_current = true`, `valid_to` open); the closed intervals abut
  (each `valid_to` equals the next `valid_from`), no gap, no overlap. Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/…` unit (fail-closed / invariant) — an emit that would produce **two** open
  versions for one key, or overlapping intervals, is caught (assertion / refusal), never written — the
  ≤ one-`is_current`-per-key invariant is enforced, not assumed.

**Implementation shape.** Define the managed validity-column set once (names + types + open-interval
representation, per the promoted §Surface decision); append them in the maintenance emit; make `is_current`
the indexed "current-version" flag the combiner's per-run lookup uses (equivalent to "`valid_to` is open").
Add a conflict check when a user column collides with a managed name.

**Critical files.**
- `crates/smelt-runtime/src/…` — the maintained-schema augmentation + the close/open column writes.
- `crates/smelt-logical/src/rules/versioned.rs` — the managed-column conflict check at classify time.
- `examples/versioned_snapshot/` — three-state key fixture.

**Docs touched.**
- `versioned_models.md` §Surface / §"Validity columns (smelt-managed)" — promote the settled names/types +
  open-interval representation from §Known Divergences (spec increment); remove the "Validity-column surface
  is unsettled" note.
- `docs-site/docs/guide/` — document the three managed columns + the interval semantics.

**Review checklist.**
- [ ] The three managed columns are appended, not user-projected; a name collision is a diagnostic.
- [ ] Three-state key → two closed + one open; intervals abut, no gap/overlap.
- [ ] ≤ one `is_current` per key enforced (never two open versions).
- [ ] §Surface promotion matches the code in the same commit; "unsettled" note removed; edits timeless.

**Commit.** `feat(refresh): smelt-managed valid_from/valid_to/is_current + non-overlapping interval invariant`

---

### Phase V4: Tracked-attribute selection + deletion handling

**Goal.** Two versioned-local rules. **Tracked-attribute selection**: a new version opens for a key only
when a **tracked attribute** changes between the stored current version and the incoming row — by default
every projected non-key column is tracked; a change confined to an **untracked** column opens no new
version. **Deletion handling**: a key present in the store but absent from the incoming set is a
**retraction**, settled as a **soft-close** — the current version is closed (`valid_to` set, `is_current =
false`) with no new version opened; a CDC feed's explicit delete event is the close signal. Settle and
promote the tracked-attribute rule (all projected non-key columns by default vs a marked subset; derive from
SQL where unambiguous) from §Known Divergences into normative surface.

**Spec anchor.** `versioned_models.md` §"Tracked-attribute selection", §"Deletion handling"; §Design "Derive
from SQL where possible". §Known Divergences "Tracked-attribute selection is unsettled" (promoted here); the
late-correction/hard-delete retraction question stays **open** (deferred).

**Depends on.** V2 (the combiner), V3 (the validity columns the close/soft-close write).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/versioned.rs` unit — the classifier derives the tracked set (all projected
  non-key columns by default); a change in a **tracked** attribute is detected as a version boundary; a
  change confined to an **untracked** column opens **no** new version.
- `crates/smelt-cli/tests/versioned*` (real fixture) — a snapshot where a key's untracked field drifts but
  no tracked attribute changes produces **no** spurious version; a tracked change opens one. Requires
  `DUCKDB_LIB_DIR`.
- `crates/smelt-runtime/src/…` unit — a key absent from the incoming set is **soft-closed** (current version
  `valid_to` set, `is_current = false`, no new open version); a CDC delete event closes the same way.
- `crates/smelt-logical/src/rules/versioned.rs` unit (fail-closed reject) — an **ambiguous** tracked set (a
  column whose tracked/untracked status cannot be derived and is not declared) is **refused** with a named
  diagnostic — never silently defaulted to tracked-or-untracked. A **late correction to an already-closed
  interval** is **refused** (the deferred retraction case), not silently applied.

**Implementation shape.** Derive the tracked set from the SELECT's non-key projected columns; if the design
lands an untracked-marking surface, wire it (prefer derive-from-SQL over a strategy block per §Design). In the
combiner's compare step, only tracked-attribute differences open a version. Add the soft-close path: a key in
the store but not the incoming set (window-forward: absent at the window boundary; snapshot-diff: absent from
the re-scan) closes its current version with the event-time boundary, no reopen. Refuse (diagnostic) a
correction targeting a closed interval.

**Critical files.**
- `crates/smelt-logical/src/rules/versioned.rs` — tracked-set derivation + the ambiguity refusal.
- `crates/smelt-runtime/src/…` — the tracked-attribute compare in the combiner; the soft-close emit.
- `examples/versioned_snapshot/` — untracked-drift + deletion fixtures.

**Docs touched.**
- `versioned_models.md` §"Tracked-attribute selection" — promote the settled default + marking rule from
  §Known Divergences (spec increment); remove the "Tracked-attribute selection is unsettled" note; keep the
  "Late corrections to a closed interval" note **open** (narrowed to just the closed-interval correction +
  hard-delete surface).
- `docs-site/docs/guide/` — document tracked vs untracked columns and soft-close-on-delete.

**Review checklist.**
- [ ] Tracked change opens a version; untracked-only change does not; the tracked set is derived (or declared)
      unambiguously, ambiguity refused.
- [ ] Deletion soft-closes the current version (no reopen); CDC delete event is the close signal.
- [ ] Late correction to a closed interval is refused (fail-closed, deferred retraction case).
- [ ] §Surface promotion matches the code; "unsettled" note removed, late-correction note narrowed; timeless.

**Commit.** `feat(refresh): versioned tracked-attribute selection + soft-close deletion handling`

---

### Phase V5: Event-time-stamped validity + window-independence / ordered-execution enforcement

**Goal.** Stamp `valid_from` / `valid_to` from the **source's event time** — the update-events feed's
event-time column, or the snapshot's as-of timestamp — **never the run clock** — so re-running a window, or
backfilling windows out of order, reproduces byte-identical interval boundaries and end-state equivalence
survives replays. Enforce **window-independence / ordered-execution** (F10): because close/open is inherently
ordered, a window-forward feed's windows are applied in **temporal order**; a self-referential / non-converging
shape is refused. Bound the **scan** by the shared bound/reach (F1) + widened-scan / exact-clamp source-filter
pushdown (F13) — the scan is windowed while the **write stays key-addressed** (the close-out reaches outside
the scan window by key).

**Spec anchor.** `versioned_models.md` §"Validity stamped from source event-time (not run clock)",
§"Input consumption is derived from the source", §Constraints 4–5. `model_maintenance.md` §"Windowed
maintenance and the horizon" (scan ⊇ write; a clocked key-addressed mode windows its scan yet writes back by
key). `model_properties.md` §"Window-independence / ordered-execution", §"Event-time monotonicity trace".
`model_transforms.md` §"Source-filter pushdown".

**Depends on.** V2 (the combiner); **F1** (bound/reach for the scan window); **F10** (window-independence /
ordered-execution); **F13** (widened-scan / exact-clamp source-filter pushdown). Consumes the W1 event-time
trace read-only.

**TDD tests to write first.**
- `crates/smelt-runtime/src/…` unit — `valid_from` / `valid_to` are stamped from the source event-time
  expression, **not** `NOW()` / the run clock (assert the emitted SQL references the source event-time
  column, and that no run-clock literal is injected into the validity boundary).
- `crates/smelt-cli/tests/versioned*` (real fixture, **replay + order-independence**) — re-running a window,
  and backfilling non-overlapping windows **in reversed order**, both converge to **byte-identical** interval
  boundaries and the same history; equals a full rebuild. Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-cli/tests/versioned*` (real fixture, **out-of-window close-out under windowed scan**) — with a
  `timeseries:` source consumed window-forward, the scan is bounded to the run window (F1/F13 pushdown) yet a
  new value still closes an out-of-window prior version by key; the maintained table equals a full rebuild.
  This exercises scan-windowed-but-write-key-addressed explicitly. Requires `DUCKDB_LIB_DIR`.
- `crates/smelt-logical/src/rules/versioned.rs` unit (fail-closed reject) — ordered execution is required for
  a window-forward feed: a source whose event-time is **not `Traceable`** (F10 cannot prove ordered
  convergence), or a run-clock-stamped validity, is **refused** with a diagnostic — never silently stamped
  from the run clock or run out of order.

**Implementation shape.** Compute the validity boundaries from the source event-time expression resolved via
the W1 trace; forbid a run-clock stamp (reject if no source event-time is derivable for a window-forward
feed). Gate the driver's window sequencing on F10: a window-forward feed is marked **ordered** (windows in
temporal order); a non-converging self-reference is refused. Bound the scan via F1's reach + F13's
widened-scan / exact-clamp pushdown, leaving the write key-addressed (the close-out is not clamped to the
scan window). Snapshot-diff ships as full-rescan-and-compare (no window; `--auto` staleness deferred —
record under §Deferred).

**Critical files.**
- `crates/smelt-runtime/src/…` — the event-time-stamped validity emit; the F1/F13 scan pushdown; the F10
  ordered-window gate in the driver invocation.
- `crates/smelt-logical/src/rules/versioned.rs` — the run-clock-stamp / non-ordered refusal.
- `examples/versioned_stream/` (new `timeseries:` update-events fixture) alongside `versioned_snapshot/`.

**Docs touched.**
- `versioned_models.md` §Known Divergences — remove the residual "Not implemented" note (the mode is now
  end-state equivalent and replay-safe); leave only the deferred late-correction/hard-delete and
  snapshot-diff-`--auto`-staleness questions open.
- `models.md` line 292 — drop the remaining `versioned` clauses (it now parses **and** is fully built).
- `docs-site/docs/guide/` — document source-event-time stamping + why replays are safe; the window-forward
  vs snapshot-diff derived consumption.

**Review checklist.**
- [ ] Validity stamped from the source event-time, never the run clock; run-clock stamp refused.
- [ ] Replay + reversed-order backfill converge to byte-identical intervals (order-independence); equals full
      rebuild.
- [ ] Scan bounded (F1/F13) while the write stays key-addressed — the out-of-window close-out under a windowed
      scan is green.
- [ ] Non-`Traceable` / non-ordered window-forward feed refused (fail-closed).
- [ ] `models.md`:292 fully reconciled; §Known-Divergence notes narrowed to the deferred items; edits timeless.

**Commit.** `feat(refresh): event-time-stamped versioned validity + ordered-execution enforcement (replay-safe)`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this sub-plan.)

- **Late corrections to an already-closed interval** and an **opt-in hard-delete surface** — the shared
  keyed-mode retraction question (`versioned_models.md` §Known Divergences; `cumulative_aggregate.md`
  §"Reprocessing semantics"). V4 refuses a closed-interval correction fail-closed; the surface is deferred.
- **Snapshot-diff `--auto` staleness** for a clock-less mutable snapshot source (research §19.8). V5 ships
  snapshot-diff as always-full-rescan-and-compare; the staleness firing is deferred (record the choice here
  and narrow the `models.md` §Known-Divergence note rather than claiming it settled).
- **Shared-executor question (§19.8)** — settled *in this plan*: the windowed-keyed-maintenance driver (F11)
  is the shared executor and `versioned` composes it by name; no per-rule copy is created (this is the D2
  posture the plan supersedes).

## Verification

How to confirm the `versioned` L4 composition is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Composition, not re-derivation.** `rules/versioned.rs` contains **no** private driving-fact resolver,
  monotonicity list, or per-partition loop — it calls F2 / F4 and folds the combiner through the F11 driver;
  the scan is bounded by F1 / F13 and ordered by F10. (A reviewer grep for a copied `NONDETERMINISTIC_*` list
  or a hand-rolled per-partition loop in `versioned.rs` finds none.)
- **Interval-keyed end-state equivalence + the out-of-window close-out.** The DuckDB harness shows the
  maintained `(key, version, validity interval)` set equals a full rebuild over the processed snapshots, and
  the **out-of-window close-out** test (a new value closing a version whose timestamp is outside the current
  input/scan window, reached by key) is green under both snapshot-diff and windowed-scan
  (`model_maintenance.md` §"The equivalence invariant"). Requires `DUCKDB_LIB_DIR`.
- **Order-independence / replay-safety.** Re-running a window and backfilling non-overlapping windows in any
  order converge to byte-identical intervals (validity stamped from source event-time).
- **Invariants.** Non-overlapping validity per key; closed intervals abut; ≤ one `is_current` per key.
- **Every phase fail-closed.** V1 (no key / ambiguous fact / non-monotone), V2 (non-monotone / ambiguous
  refuse the merge), V4 (ambiguous tracked set / late closed-interval correction), V5 (run-clock stamp /
  non-ordered feed) each have a reject test that emits a diagnostic rather than an approximate build.
- `cargo test -p smelt-cli --test example_diagnostics` and `-p smelt-lsp --test example_workspaces` — the new
  `versioned_snapshot` / `versioned_stream` fixtures build with zero diagnostics.
- `/smelt:validate versioned_models` reports zero drift for the surface this sub-plan lands; the "Not
  implemented — does not parse" note is gone, the validity-column and tracked-attribute surfaces are promoted
  into §Surface, and `models.md` line 292 no longer lists `versioned` as unbuilt.
</content>
</invoke>
