# Plan: Model updates — L4 composition for `refresh: latest_value` (SCD Type 1)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L4 mode-composition** layer for `refresh: latest_value`. This sub-plan **supersedes the D1 portion** of the mode-vertical [`docs/plans/20260704-model-updates-group-d.md`](20260704-model-updates-group-d.md) (see §"Scope boundary").
**Specs (oracles)**:
- [`docs/specs/latest_value_models.md`](../specs/latest_value_models.md) — PRIMARY. §Surface (the composition table + output shape); §Semantics ("End-state equivalence", "Upsert-overwrite (the local combiner)", "Definition of 'latest' and the preferred direction", "The classifier", "Input consumption is derived from the source"); §Design; §Constraints & Invariants; §"Known Divergences" (the "does not parse" note, the unsettled "latest" definition, deletions).
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — the framework this mode composes against. §"The equivalence invariant" (**one** invariant; `latest_value` is **key-addressed** — identity-requiring `merge_into`, one current row per natural key, upsert-overwrite; discharged on the **end-state**); §"Windowed maintenance and the horizon" (windowed-by-default; derived horizon); §"Validator, not chooser".
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — the proofs this mode requires **by exact name**: **value-monotone vs order-monotone discriminant** (§"Algebraic discriminants"), **driving-fact / anchor resolution**, **input-delta discovery**.
- [`docs/specs/model_transforms.md`](../specs/model_transforms.md) — the transform this mode drives **by exact name**: keyed **`merge_into`** (target-as-replica) sequenced by the **windowed-keyed-maintenance driver**; upsert-overwrite catalogued as "stays in a mode spec".
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis" (the keyed-output peer), §"Input-consumption axis", §"Constraint violations" (the keyed-mode `timeseries:`/`batched:` forbids).
**Research (the "why" + the L-decomposition)**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Target plan architecture (the re-cut master)" (L0–L4; this sub-plan is an **L4 mode composition**), §"Mapping the current master onto the layers". Mode-surface origin: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) Part 17 (naming), Part 19 (§19.4 the ordering-column monoid; §19.8 the shared-executor / snapshot-diff open questions).
**Spec diff**: no new spec authored. Two normative edits land as their phases complete:
- **`RefreshStrategy` enum addition** — `RefreshStrategy` (`crates/smelt-core/src/config.rs`) today accepts only `full | batched | cumulative | materialized_view`; this plan adds the `LatestValue` variant + its `"latest_value"` Deserialize/Serialize arm. As that lands, `latest_value_models.md` §Known Divergences "Not implemented — the mode does not parse" note is **removed**.
- **`models.md` line 292 reconcile** — the §"Known Divergences" note "Keyed refresh modes beyond `cumulative` are not fully built" enumerates `latest_value` among the values that **do not parse**; narrow it (drop `latest_value` from the does-not-parse list) as LV1 lands, leaving `versioned`/`accumulating_snapshot` there.
No phase authors a spec; each phase **removes or narrows** a §Known-Divergence note and flips no §Surface maturity cell (the mode spec has no maturity table — its "Not implemented" note is the status carrier).
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan is the **L4 composition** for the single mode `refresh: latest_value`. It **supersedes the D1 phase** of [`docs/plans/20260704-model-updates-group-d.md`](20260704-model-updates-group-d.md): where D1 was cut mode-vertically (re-deriving its own keyed `merge_into` + driving-source loop on top of Group C's C1), this re-cut **wires the fundamentals (L1+L2) capabilities by name** — it builds *no* proof and *no* transform of its own. The order-monotone discriminant, the anchor resolver, input-delta discovery, and the windowed-keyed-maintenance driver are owned by the fundamentals sub-plan ([`docs/plans/20260704-model-updates-fundamentals.md`](20260704-model-updates-fundamentals.md), F-phases below); this plan composes them into the mode's surface, classifier, and execution. It does **not** cover `versioned` (D2 — its own L4 sub-plan) or `materialized_view` (D3 — engine-owned, unrelated to these fundamentals). When Group D is re-scaffolded as L4 sub-plans, its D1 row is retired in favour of this file.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when the
L4 `latest_value` composition is scaffolded into the registry — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant oracle for every phase is the **end-state equivalence invariant** for key-addressed output
   (`model_maintenance.md` §"The equivalence invariant": the stored row per key equals `full_refresh`
   restricted to the processed inputs — the last-writer value). This mode is **key-addressed**: one
   current row per natural key, maintained by keyed `merge_into` whose write reaches stored rows by key,
   not by input window. The mode is **windowed-by-default** where its source is clocked
   (`model_maintenance.md` §"Windowed maintenance and the horizon"). Every classifier verdict is
   **fail-closed** and **validator-not-chooser** (`model_maintenance.md` §"Validator, not chooser"): an
   un-classifiable model — an undecidable "latest", an ambiguous key set, a non-key-addressable shape — is
   **refused with a diagnostic**, never silently downgraded to full refresh.
2. Confirm you are on branch `worktree-incremental`. Confirm the **fundamentals F-phase dependencies** for
   your phase (Depends-on column) are `done` in
   [`docs/plans/20260704-model-updates-fundamentals.md`](20260704-model-updates-fundamentals.md)'s
   Progress-tracking table, and that Group A is `done` in the master. If a dependency is not landed, set
   the row `blocked` per the block rule — do **not** re-implement the fundamental here (that is the L4/L1
   layering this re-cut exists to enforce).
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status to
   `done` in the master, retire the master's D1 row (§"Scope boundary"), and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names a
fail-closed **reject** test, and every phase that changes emitted SQL adds an **end-state-equivalence**
harness test) → reviewer subagent (material findings only) → iterate → set the row `done` → commit + push
with the phase's `Commit.` line.

**End-state-equivalence tests need DuckDB.** Phases that emit `merge_into` (LV2, LV3, LV4) assert the
stored table equals a full rebuild over processed inputs via the DuckDB harness; those require
`DUCKDB_LIB_DIR` set (and `LD_LIBRARY_PATH`) per `CLAUDE.md`. The parse/classifier phase (LV1) is
`smelt-core` + `smelt-logical` unit tests with no DuckDB dependency.

**Wire the fundamentals by name — do not re-derive.** This is an L4 composition. LV1's classifier reads the
**value-vs-order-monotone discriminant** (F4) and **driving-fact / anchor resolution** (F2); LV2/LV3 drive
keyed `merge_into` through the **windowed-keyed-maintenance driver** (F11); LV4 reads **input-delta
discovery** (F9). If a phase finds a needed fundamental missing or wrong, block on the F-phase — never
add a private copy (that reintroduces the six duplications the fundamentals layer paid down).

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits
describe the feature as if it always existed; as each phase lands, **remove/narrow** the matching
§Known-Divergence note (the "does not parse" note, the `models.md` line-292 does-not-parse list) rather
than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (e.g. the §19.8 shared-executor
choice if F11 did not settle it, or the deletions Open Question), an unmet F-phase / Group-A dependency, or
a pre-flight red unrelated to this phase's target: set the row `blocked` with a one-line reason, append to
§"Blocked phases", restore a clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit
`<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape added `refresh: latest_value` to the peer refresh enum
(`models.md` §"Refresh axis") — the Type-1 slowly-changing-dimension pattern, keeping **only the current
row** per natural key, overwriting in place with no history. Today it **does not parse**: `RefreshStrategy`
(`crates/smelt-core/src/config.rs`) accepts only `full`/`batched`/`cumulative`/`materialized_view`, so
`refresh: latest_value` produces an *Invalid refresh strategy* error before any classifier runs
(`latest_value_models.md` §Known Divergences).

Under the fundamentals-first re-cut (`docs/research/20260704-maintenance-fundamentals.md`), `latest_value`
is **not** a from-scratch mode build — it is a **composition** (`latest_value_models.md` §Surface
composition table) of capabilities the fundamentals layer (L1+L2) already owns:

| Composition facet | Capability (owner) | F-phase |
|---|---|---|
| Property — the retained value's algebra | **value-monotone vs order-monotone discriminant** resolving to the order-monotone `MAX_BY`-by-key case (`model_properties.md`) | F4 |
| Property — anchor when the source is joined | **driving-fact / anchor resolution** (`model_properties.md`) | F2 |
| Property — how new rows are found | **input-delta discovery** (`model_properties.md`) | F9 |
| Transform — the physical maintenance | keyed **`merge_into`** (target-as-replica) sequenced by the **windowed-keyed-maintenance driver** (`model_transforms.md`) | F11 |
| Output shape | **keyed** — one current row per natural key (`models.md`) | — |

This sub-plan lands (1) the parse/selector + the order-monotone keyed-overwrite classifier, (2) the
upsert-overwrite combiner via keyed `merge_into` on the driver, (3) the "latest" semantics — max-by-ordering
monoid vs last-processed fallback and its eligibility proof, and (4) the input-consumption derivation
(window-forward for a clocked source vs snapshot-diff for a mutable one). Every phase is fail-closed with a
reject test; the two smelt-driven representations discharge the *same* end-state invariant, differing only
in *which* is order-independent.

## Scope

### In scope (L4 composition for one mode)

- **LV1** — `refresh: latest_value` parse/selector + the keyed-mode constraint violations + the
  order-monotone keyed-overwrite **classifier** (natural key + attributes; reads F4 + F2). Fail-closed on
  an ambiguous key set / non-key-addressable shape.
- **LV2** — the **upsert-overwrite combiner** via keyed `merge_into` sequenced by the
  windowed-keyed-maintenance driver (F11). One row per key; end-state equivalence.
- **LV3** — the **definition of "latest"**: the max-by-ordering-key monoid (order-independent, parallel /
  out-of-order backfill) vs the last-processed fallback (order-dependent, sequential), and the
  eligibility proof selecting between them. Fail-closed when "latest" is undecidable.
- **LV4** — **input-consumption derivation**: window-forward for a clocked (`timeseries:`) source vs
  snapshot-diff (whole re-scan) for a mutable snapshot source, via input-delta discovery (F9). No
  `strategy:` knob. Fail-closed (conservative whole-relation re-scan) on an unknown mutation profile.

### Explicitly deferred (not this sub-plan)

- **Deletions / late corrections** — a key vanishing from the incoming set: retain or delete? The shared
  keyed-mode retraction Open Question (`latest_value_models.md` §Known Divergences; research §18.2). Stays
  open; no phase here.
- **Deterministic tie-break on the ordering key** — how ties on the max-by-ordering key break
  deterministically (`latest_value_models.md` §"Definition of 'latest'"). LV3 admits the ordering-column
  monoid; the tie-break rule is recorded as a residual Open Question, not settled here unless F4's
  discriminant already fixes it.
- **Snapshot-diff `--auto` staleness** for a clock-less source (research §19.8). LV4 may ship snapshot-diff
  as always-full-rescan and defer the staleness firing; record under §Deferred and narrow the spec note.
- **`versioned` (D2)** and **`materialized_view` (D3)** — separate L4 / engine-owned sub-plans.

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| LV1 | Group A (A1); **F2**, **F4** | `latest_value_models.md` §Surface + §"The classifier"; `models.md` §"Constraint violations" | pending |
| LV2 | LV1; **F11** | `latest_value_models.md` §"Upsert-overwrite (the local combiner)"; `model_transforms.md` keyed `merge_into` + driver | pending |
| LV3 | LV2; **F4** | `latest_value_models.md` §"Definition of 'latest' and the preferred direction"; `model_properties.md` order-monotone discriminant | pending |
| LV4 | LV2; **F9** | `latest_value_models.md` §"Input consumption is derived from the source"; `models.md` §"Input-consumption axis" | pending |

---

### Phase LV1: `refresh: latest_value` parse/selector + keyed-mode constraints + order-monotone classifier

**Goal.** Make the mode **parse** and **classify**. Add `RefreshStrategy::LatestValue` (implying stored
`table`); enforce the keyed-mode constraint violations (`refresh: latest_value` forbids a `timeseries:`
block **and** a `batched:` block *on the model itself*). Add the order-monotone keyed-overwrite classifier:
the projection carries a **natural key** + attribute columns and the per-key fold is a keyed overwrite;
the classifier reads the **value-vs-order-monotone discriminant** (F4) and **driving-fact / anchor
resolution** (F2) — it builds neither. Fail-closed per validator-not-chooser: an ambiguous key set, a
non-overwrite fold, or an unresolvable anchor is **refused with a diagnostic**, never downgraded.

**Spec anchor.** `latest_value_models.md` §Surface (parse + composition table), §"The classifier",
§Constraints 1–3, 5; `models.md` §"Constraint violations" (the keyed-mode `timeseries:`/`batched:` forbids)
and §Known Divergences line 292 (narrow the does-not-parse list). Invariant: `model_maintenance.md`
§"Validator, not chooser".

**Depends on.** Group A (`RefreshStrategy` exists to extend). **F2** (anchor resolution) and **F4** (the
order-monotone discriminant) landed — the classifier *reads* them; if either is `pending`, block.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` unit — `refresh: latest_value` deserialises to
  `RefreshStrategy::LatestValue`; round-trips through Serialize; a bare `refresh: foo` still errors and the
  message now lists `latest_value` among the valid values.
- `crates/smelt-core/src/metadata.rs` unit — the two keyed-mode constraint violations fire:
  `refresh: latest_value` + a `timeseries:` block **on the model** is a hard error, and
  `refresh: latest_value` + a `batched:` block is a hard error (`models.md` §"Constraint violations").
- `crates/smelt-logical/src/rules/latest_value.rs` (new) unit — a SELECT projecting a natural key + attribute
  columns whose per-key fold is a keyed overwrite classifies as admitted, reading the F4 order-monotone
  discriminant + (when the source is joined) the F2 anchor resolver.
- **(fail-closed reject)** `crates/smelt-logical/src/rules/latest_value.rs` unit — a model declaring no
  derivable natural key, or a non-key-addressable shape (a non-overwrite fold), or an unresolvable join
  anchor is **refused with a named diagnostic** (not silently full-refreshed).

**Implementation shape.** Add `RefreshStrategy::LatestValue` (`config.rs`) + its `"latest_value"`
Deserialize/Serialize arm; extend the deserialize error string. Add the keyed-mode constraint-violation
rows in `metadata.rs`, mirroring the existing `cumulative`/`materialized_view` forbid branch. New classifier
`crates/smelt-logical/src/rules/latest_value.rs` (sibling of `cumulative.rs`): derive natural key +
attributes from the SELECT; call F4's discriminant to confirm the order-monotone keyed-overwrite shape and
F2's resolver for the joined-source anchor. Classifier only — no execution yet.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy` enum + Deserialize/Serialize.
- `crates/smelt-core/src/metadata.rs` — keyed-mode constraint violations.
- `crates/smelt-logical/src/rules/latest_value.rs` (new) + `rules/mod.rs`; read-only reference
  `rules/cumulative.rs`; consumes F2 (`analysis/source_bounds.rs` anchor resolver) + F4 (discriminants).

**Docs touched.**
- `latest_value_models.md` §Known Divergences — remove the "Not implemented — the mode does not parse" note
  (the mode now parses + classifies; execution lands in LV2–LV4, so narrow rather than fully delete if
  execution is not yet wired — state "parses and classifies; upsert-overwrite execution lands with the
  driver wiring").
- `models.md` §Known Divergences (line 292) — drop `latest_value` from the "do not parse" list, leaving
  `versioned`/`accumulating_snapshot`.
- `docs-site/` — no user-facing execution surface yet; hold the guide addition for LV2 (when build works).

**Review checklist.**
- [ ] `refresh: latest_value` deserialises; error string lists it; round-trips.
- [ ] Both keyed-mode constraint violations enforced (`timeseries:` + `batched:` on the model).
- [ ] Classifier reads F4 + F2 by name; derives natural key + attributes from the SQL, not a strategy block.
- [ ] Ambiguous key / non-key-addressable / unresolvable anchor refused with a diagnostic (fail-closed).
- [ ] `latest_value_models.md` "does not parse" note removed/narrowed; `models.md` line 292 reconciled; edits timeless.

**Commit.** `feat(refresh): parse refresh: latest_value + order-monotone keyed-overwrite classifier (composes F2/F4)`

---

### Phase LV2: Upsert-overwrite combiner via keyed `merge_into` on the windowed-keyed-maintenance driver

**Goal.** Execute the mode: maintain **one row per natural key** by folding each incoming row over the
stored one with the **upsert-overwrite** combiner — the mode-local realisation of keyed `merge_into`
(target-as-replica), sequenced by the **windowed-keyed-maintenance driver** (F11). Matched keys overwrite,
unmatched insert. This phase wires the driver + emit; the *choice* of "latest" algebra (monoid vs
last-processed) is LV3 and the input window is LV4 — LV2 uses the last-processed overwrite over a
whole-input scan as the baseline both later phases specialise.

**Spec anchor.** `latest_value_models.md` §"Upsert-overwrite (the local combiner)", §"End-state
equivalence", §Constraints 3–4; `model_transforms.md` keyed `merge_into` + windowed-keyed-maintenance
driver. Invariant: `model_maintenance.md` §"The equivalence invariant" (end-state, key-addressed).

**Depends on.** LV1 (classifier admits the model). **F11** (the windowed-keyed-maintenance driver) landed —
LV2 sequences `merge_into` *through* it; if F11 is `pending`, block. (This is the D1→C1 dependency re-cut
onto the fundamentals driver — LV2 does not build a keyed loop of its own.)

**TDD tests to write first.**
- `crates/smelt-runtime/src/` unit — the upsert-overwrite `merge_into` emitted for a `latest_value` model
  keeps **exactly one row per key**; a second run with a changed attribute overwrites in place (no version
  row accumulates); the classify → step → merge stages route through the F11 driver, not a private loop.
- **(end-state equivalence)** `crates/smelt-cli/tests/…` real fixture (new `examples/latest_value_snapshot/`)
  — after multiple `smelt build`s the maintained table **equals a full rebuild** over the processed inputs
  (last-writer value per key). Requires `DUCKDB_LIB_DIR`.
- **(fail-closed reject)** `crates/smelt-runtime/src/` unit — a shape the LV1 classifier refuses (or a
  non-key-addressable model reaching execution) is **not** merged approximately: execution declines and the
  diagnostic surfaces (never a silent partial merge).

**Implementation shape.** Add `ModelStrategy::LatestValue` (`types.rs`) + its dispatch arm (`execute.rs`).
Route it through the F11 windowed-keyed-maintenance driver parameterised with the upsert-overwrite combiner
(last-writer overwrite as the baseline); emit keyed `merge_into` via the existing backend trait
(`crates/smelt-backend/src/lib.rs` `merge_into`; DuckDB impl reused as-is). No new transform — this composes
F11 + the built `merge_into`.

**Critical files.**
- `crates/smelt-runtime/src/execute.rs` (dispatch), `crates/smelt-runtime/src/types.rs` (`ModelStrategy`).
- the F11 driver module (consumed, not modified); `crates/smelt-backend/src/lib.rs` `merge_into` (reused).
- `crates/smelt-logical/src/rules/latest_value.rs` (classifier verdict feeds the driver config).
- `examples/latest_value_snapshot/` (new fixture).

**Docs touched.**
- `latest_value_models.md` §Known Divergences — narrow the parse/execution status note (the mode now builds
  and maintains one row per key).
- `docs-site/docs/guide/` — add `refresh: latest_value` to the refresh-modes guide (one row per key,
  overwrite-in-place, no history).

**Review checklist.**
- [ ] One row per key maintained; attribute change overwrites in place; no version rows.
- [ ] Execution routes through the F11 driver + built `merge_into`; **no** private keyed loop added.
- [ ] End-state-equivalence harness passes (maintained table = full rebuild over processed inputs).
- [ ] Un-classifiable / non-key-addressable model refused, never merged approximately (fail-closed).
- [ ] Example fixture builds with zero diagnostics; spec/guide edits timeless.

**Commit.** `feat(refresh): latest_value upsert-overwrite via keyed merge_into on the windowed-keyed-maintenance driver`

---

### Phase LV3: Definition of "latest" — max-by-ordering monoid vs last-processed fallback + eligibility proof

**Goal.** Fix *which* representation of "latest" the combiner uses, derived from the SQL. When the source
carries an **ordering column** (an updated-at the projection derives), the per-key combiner is
**max-by-ordering-key** — a commutative, associative, idempotent semilattice fold — so `merge_into` is
order-independent and the driver may step partitions in any order / backfill in slices. Absent an ordering
column, "latest" is **last-processed** — order-dependent, not a monoid — forcing strictly sequential window
application (the derived-ordered fallback). The eligibility proof selects between them via the
**order-monotone discriminant** (F4); it is fail-closed when "latest" is undecidable.

**Spec anchor.** `latest_value_models.md` §"Definition of 'latest' and the preferred direction",
§"Upsert-overwrite" (the two combiner algebras), §Semantics "End-state equivalence" (order-independence up
to the definition of latest); `model_properties.md` §"Algebraic discriminants" (order-monotone `MAX_BY`
case). Invariant: `model_maintenance.md` §"The equivalence invariant".

**Depends on.** LV2 (the merge path exists). **F4** (the value-vs-order-monotone discriminant) landed —
LV3 reads it to prove the ordering-column monoid; if `pending`, block.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/latest_value.rs` unit — a projection carrying an ordering column
  (`updated_at`) is classified **max-by-ordering monoid** (order-independent); a projection with no
  derivable ordering column is classified **last-processed** (order-dependent, sequential). The verdict is
  read from F4's order-monotone discriminant.
- **(end-state equivalence, out-of-order)** `crates/smelt-cli/tests/…` real fixture — for the ordering-column
  form, **replaying an old run window does not clobber a newer stored value** (the §19.4 footgun); an
  out-of-order / sliced backfill converges to the same end-state as a full rebuild. Requires `DUCKDB_LIB_DIR`.
- **(fail-closed reject)** `crates/smelt-logical/src/rules/latest_value.rs` unit — a model whose "latest" is
  **undecidable** (an ambiguous / multi-candidate ordering key that F4 cannot resolve to a single
  order-monotone fold) is **refused with a diagnostic**, never silently defaulted to last-processed or to
  the monoid.

**Implementation shape.** In the classifier, consume F4's order-monotone discriminant to detect the ordering
column and pick the combiner: `MaxByOrdering{ordering_col}` (a commutative monoid) or `LastProcessed`.
Thread the choice into LV2's driver config: the monoid form lifts the sequential-execution constraint
(any-order stepping / sliced backfill); the last-processed form marks the model **ordered** (strictly
sequential windows — reuse the ordered-execution gating if Group-B B6 has landed it, else gate sequentially
locally). Fail-closed on an undecidable ordering key.

**Critical files.**
- `crates/smelt-logical/src/rules/latest_value.rs` — combiner selection reading F4.
- `crates/smelt-runtime/src/` — driver config: order-independent vs strictly-sequential stepping.

**Docs touched.**
- `latest_value_models.md` §Known Divergences — narrow "Definition of 'latest' is unsettled" to record the
  **decided** ordering-column-preferred direction (the max-by-ordering monoid is now realised), leaving only
  the deterministic tie-break sub-question open (§Deferred).
- `docs-site/docs/guide/` — note that an ordering column licenses out-of-order / parallel backfill; without
  one, backfill is strictly sequential.

**Review checklist.**
- [ ] Ordering-column form classified max-by-ordering monoid (order-independent); absent → last-processed (ordered).
- [ ] The choice is read from F4's discriminant, not declared in a block.
- [ ] Out-of-order-merge equivalence passes; old-window replay does not clobber a newer value (§19.4 footgun green).
- [ ] Undecidable "latest" refused with a diagnostic (fail-closed).
- [ ] "Definition of 'latest'" note narrowed to the decided direction; tie-break stays Open; edits timeless.

**Commit.** `feat(refresh): latest_value "latest" algebra — max-by-ordering monoid vs last-processed fallback (composes F4)`

---

### Phase LV4: Input-consumption derivation — window-forward (clocked) vs snapshot-diff (mutable)

**Goal.** Derive *how new input is discovered* from the **source's shape**, never from a model declaration —
the mode-local application of the input-consumption axis via **input-delta discovery** (F9). A source
carrying a `timeseries:` clock is consumed **window-forward** in `--event-time` run windows applied to the
*source's* `partition_column` (exactly as `cumulative` consumes its driving source), reading only the new
tail; a mutable snapshot source (no monotone clock) is **snapshot-diffed** — re-scanned whole and upserted.
The end-state contract is identical; only the scan cost differs. Fail-closed: an unknown mutation profile
falls to the conservative whole-relation re-scan, never an optimistic window-forward that could silently
drop rows.

**Spec anchor.** `latest_value_models.md` §"Input consumption is derived from the source"; `models.md`
§"Input-consumption axis"; `model_maintenance.md` §"Windowed maintenance and the horizon" (windowed-by-
default; full scan is the fallback) and §Interactions "Input-consumption". Invariant: `model_maintenance.md`
§"The equivalence invariant".

**Depends on.** LV2 (the merge path exists). **F9** (input-delta discovery) landed — LV4 reads its
window-forward / snapshot-diff verdict; if `pending`, block.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/latest_value.rs` unit — a source carrying a `timeseries:` clock yields the
  **window-forward** consumption cell (via F9); a mutable snapshot source yields **snapshot-diff**. Derived
  from the source, never a `strategy:` knob (assert no model-level input-consumption declaration is read).
- **(end-state equivalence)** `crates/smelt-cli/tests/…` — a new `examples/latest_value_stream/` (a
  `timeseries:` update-events source): a windowed run reads **only the covered partitions**, and the
  maintained table equals a full rebuild over the processed inputs; the `latest_value_snapshot` fixture
  (LV2) confirms the whole-rescan path yields the identical end-state. Requires `DUCKDB_LIB_DIR`.
- **(fail-closed reject)** `crates/smelt-logical/src/rules/latest_value.rs` unit — a source whose mutation
  profile is **unknown / underivable** falls to the conservative whole-relation re-scan (snapshot-diff),
  **never** an optimistic window-forward; a model that can resolve *neither* a clock *nor* a snapshot-diffable
  source is **refused with a diagnostic**.

**Implementation shape.** In the classifier/executor, consume F9's `WindowForward | SnapshotDiff` verdict
keyed on the source's `timeseries:` shape (the same `SourceTimeseriesMap` signal `cumulative` reads). For
window-forward, drive the F11 loop over the source's `partition_column` under `--event-time` (reuse the
cumulative driving-source machinery per the F11 driver — no per-rule copy). For snapshot-diff, scan the
whole source and upsert. Ship snapshot-diff as **always-full-rescan** first; defer `--auto` staleness for a
clock-less source (§Deferred). No `strategy:` knob.

**Critical files.**
- `crates/smelt-logical/src/rules/latest_value.rs` — consumption verdict via F9.
- `crates/smelt-runtime/src/` — window-forward driver step (through F11) vs whole-rescan upsert.
- `examples/latest_value_stream/` (new fixture).

**Docs touched.**
- `latest_value_models.md` §Known Divergences — narrow the snapshot-diff note to the deferred `--auto`
  staleness sub-question (window-forward + whole-rescan snapshot-diff now both realised).
- `models.md` §"Input-consumption axis" — verify the derived-cell prose matches (no phase vocabulary; no
  surface change, accuracy only).
- `docs-site/docs/guide/` — note the derived consumption: a clocked source is read window-forward, a mutable
  snapshot whole; no knob.

**Review checklist.**
- [ ] Consumption derived from the source via F9 (window-forward clocked vs snapshot-diff mutable); no `strategy:` knob.
- [ ] Window-forward reads only covered partitions; both paths equal a full rebuild (end-state equivalence).
- [ ] Unknown mutation profile falls to conservative whole-rescan; neither-resolvable source refused (fail-closed).
- [ ] snapshot-diff `--auto` staleness deferral recorded under §Deferred; spec note narrowed; edits timeless.

**Commit.** `feat(refresh): latest_value input-consumption derivation — window-forward vs snapshot-diff (composes F9)`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Record here: the deterministic ordering-key tie-break rule if it surfaces during LV3; the
snapshot-diff always-full-rescan / `--auto`-staleness deferral from LV4; the shared deletions / late-
corrections retraction Open Question if it surfaces.)

- Deletions / late corrections (a key vanishing from the incoming set), deterministic tie-break on the
  ordering key, and snapshot-diff `--auto` staleness — deferred per §Scope "Explicitly deferred".

## Verification

How to confirm the L4 `latest_value` composition is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Parse + classify.** `refresh: latest_value` deserialises; the two keyed-mode constraint violations fire;
  the order-monotone keyed-overwrite classifier admits a valid model and **refuses** (with a diagnostic) an
  ambiguous key set / non-key-addressable shape / undecidable "latest" (`cargo test -p smelt-core -p smelt-logical`).
- **End-state equivalence.** The `examples/latest_value_snapshot/` (whole-rescan) and
  `examples/latest_value_stream/` (window-forward) fixtures each maintain a table equal to a full rebuild
  over the processed inputs, including the **out-of-order-merge** case for the ordering-column monoid form
  (the §19.4 footgun). Requires `DUCKDB_LIB_DIR`.
- **Composed, not re-derived.** The classifier reads F2/F4/F9 and execution routes through the F11 driver +
  built `merge_into`; no private keyed loop, discriminant, resolver, or delta-discovery copy is added
  (guards against reintroducing the six fundamentals duplications).
- `cargo test -p smelt-cli --test example_diagnostics` and `-p smelt-lsp --test example_workspaces` — the new
  `latest_value` fixtures build with zero diagnostics.
- `/smelt:validate latest_value_models` reports zero drift; the "Not implemented — does not parse" note is
  gone, the "Definition of 'latest'" note is narrowed to the decided ordering-column direction, and
  `models.md` line 292's does-not-parse list no longer lists `latest_value`.
</content>
</invoke>
