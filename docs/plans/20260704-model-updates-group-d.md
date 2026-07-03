# Plan: Model updates — Group D (New keyed modes)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — Group D (phases D1–D3)
**Specs (oracles)**:
- [`docs/specs/latest_value_models.md`](../specs/latest_value_models.md) — D1: the `refresh: latest_value` mode, one-row-per-key output, ordering-column monoid vs last-processed, derived input consumption.
- [`docs/specs/versioned_models.md`](../specs/versioned_models.md) — D2: the `refresh: versioned` mode, interval-keyed output, close-old/open-new, source-event-time-stamped validity columns.
- [`docs/specs/materialized_view.md`](../specs/materialized_view.md) — D3: engine-owned IVM, no-silent-fallback, no smelt-side eligibility analysis.
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis", §"Constraint violations" (the keyed-mode `timeseries:`/`batched:` forbids; the `refresh: materialized_view`-without-native-IVM hard error).
- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — §"IVM capabilities": the `supports_native_ivm` gate.
- [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — the keyed end-state contract, the `merge_into` execution model, and the driving-source + per-partition step machinery D1/D2 reuse.
**Research**: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Parts 13–17 (keyed/stateful modes, emulation vs delegation, the user surface); Part 19 (input-consumption derived from source shape; §19.4 the ordering-column monoid; §19.8 the two open questions carried below).
**Spec diff**: the 2026-07-04 spec edits that added the three keyed-mode specs (`latest_value_models.md`, `versioned_models.md`, `materialized_view.md`) and the Part-19 follow-through (input consumption derived from the source; the keyed-mode `timeseries:` forbid scoped to the model itself; `latest_value`'s ordering-column preferred direction). Those specs are `status: experimental — not yet implemented`; this group lands the code so each spec's §Known Divergences "Not implemented" note is removed as its phase completes.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Key dependency (do not re-order).** D1 (`latest_value`) and D2 (`versioned`) build on **C1** — the keyed-mode `merge_into` + state-table + presentation-view plumbing that Group C's C1 phase introduces (`cumulative_aggregate.md` §"The maintenance boundary"). D3 (`materialized_view` emit) builds on **A4** — the IVM capability flags and the `refresh: materialized_view` hard-error path. If C1 (for D1/D2) or A4 (for D3) has not landed when a phase is picked up, set the phase `blocked` per the block rule; do not re-implement the dependency here.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (registered
by a human once Groups A and C are far enough along — see the Key-dependency note above).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The keyed-mode specs are `experimental`; the §Semantics and §"Constraint violations" rows are what you are implementing.
2. Confirm you are on branch `worktree-incremental`.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. If every row is `done`, run §Verification, flip this sub-plan's registry Status to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this phase's own red target) → implementer subagent (red-green TDD on the listed tests, real fixtures in `examples/`) → reviewer subagent (material findings only) → iterate → set the row `done` → commit + push with the phase's `Commit.` line. A phase's row lists a **spec increment** where one is pre-authorised (D2); making the cited edits is expected, not scope creep.

**Ordering.** D1 → D2 → D3. D2 reuses the windowed-keyed executor decision D1 makes (the §19.8 shared-executor question — see Open decisions); sequence D2 after D1 so the decision is already recorded. D3 is independent of D1/D2 (it depends only on A4) and may be taken at any point once A4 has landed, but is listed last because it is the thinnest.

**Open decisions this sub-plan must make (not answered by the spec).** Two research §19.8 questions are carried in-plan; resolve them in the phase that owns them and record the choice under §"Deferred during implementation" (or inline in the phase) so the next phase inherits it:
- **Shared executor vs per-rule copies (D1, then D2 follows).** The windowed-input path now has three members (`cumulative`, `latest_value`, `versioned`). Reuse cumulative's driving-source + per-partition-step machinery under one shared executor, or keep a per-rule copy per the narrow-composable-rules posture? D1 decides; D2 adopts D1's choice.
- **Snapshot-diff mechanics (D1/D2).** What the `--event-time` flags mean for a snapshot-diff run (there is no window), and how `--auto` staleness fires for a source with no monotone clock. The spec permits shipping snapshot-diff as always-full-rescan first and deferring the staleness question; if you do, record it under §Deferred and narrow the spec §Known-Divergence note rather than claiming it settled.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits describe the feature as if it always existed; as each phase lands, **remove** the matching §Known-Divergence "Not implemented" note (or narrow it to the still-open sub-behaviour) rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec, an unmet dependency (C1 not landed for D1/D2; A4 not landed for D3), or a pre-flight red unrelated to this phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape added three keyed-output refresh modes to the peer enum
(`models.md` §"Refresh axis"): two smelt-owned (`latest_value`, `versioned`) and one engine-owned
(`materialized_view`). Today only `full`, `batched`, and `cumulative` are wired; declaring any of the
three new values produces an unknown-refresh-value error. Group D lands them.

The two smelt-owned modes are close relatives of `cumulative`: they occupy the same
*window-forward-consumption* input cell (research §19.2–19.3) and reuse the same keyed maintenance
boundary — a state/presentation split and a `merge_into` upsert. That boundary is exactly what
Group C's **C1** builds for the decomposed-monoid rung (`cumulative_aggregate.md` §"The maintenance
boundary"; state table + presentation view as one atomically-swapped unit). D1 and D2 consume that
machinery rather than re-deriving it — the reason they depend on C1, not merely on A1.

The engine-owned mode is deliberately thin: smelt emits the backend's native maintained object and
relays the engine's accept/reject verbatim, hard-erroring when the backend has no native IVM
(`materialized_view.md` §"No silent fallback"). A4 already parses `refresh: materialized_view` and
lands that hard error; D3 adds the *emit* path behind the capability gate — no smelt-side eligibility
analysis (`materialized_view.md` §Design "Delegation inherits the engine's correctness").

## Scope

### In scope
- **D1** — `refresh: latest_value` (SCD Type 1): a classifier (natural key + attributes, no partition column on the model itself) and upsert-overwrite execution via `merge_into`. Encodes the two Part-19 requirements: the ordering-column monoid (max-by-ordering-key, order-independent merges) with last-processed as the derived-ordered fallback (§19.4), and input consumption derived from the source shape (window-forward over a `timeseries:` source vs whole re-scan of a mutable snapshot source; §Semantics).
- **D2** — `refresh: versioned` (SCD Type 2): a classifier and version-maintenance (close-old / open-new via `merge_into`) with smelt-managed validity columns stamped from the **source's event time** (not the run clock) so replays are end-state-equivalent. Carries the pre-authorised spec increment promoting the settled validity-column + change-tracking surface from `versioned_models.md` Open Questions into §Surface.
- **D3** — `refresh: materialized_view` emit: create the backend's native maintained object for the model's SQL; relay the engine's accept/reject verbatim; keep the A4 hard error when `supports_native_ivm = false`. No smelt-side eligibility analysis. Emit path exercised against a mock / `supports_native_ivm = true` fixture (no real IVM backend exists yet).

### Explicitly deferred
- **Deletions and late corrections** for D1/D2 (a key vanishing from the incoming set; a correction to an already-closed interval) — the shared keyed-mode retraction question (`latest_value_models.md` / `versioned_models.md` §Open Questions; research §18.2). Stays an Open Question; not a phase here.
- **Snapshot-diff `--auto` staleness** for a source with no monotone clock (§19.8). D1/D2 may ship snapshot-diff as always-full-rescan and defer the staleness firing; record under §Deferred.
- **Richer native-IVM pre-flight** (predicting incrementalisability before submission) and the **per-engine physical-strategy modifier** for D3 — out of scope by design until a concrete IVM backend motivates them (`materialized_view.md` §Known Divergences; research §17.8, §18.2).
- **The `accumulating_snapshot` / maintained-trajectory peer** (research §19.6 cell 1) and the **observation-series named rejection** (§19.6 cell 2) — no new refresh values here; both are master-level Open Questions.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| D1    | pending |        |      |
| D2    | pending |        |      |
| D3    | pending |        |      |

---

### Phase D1: `refresh: latest_value` (SCD Type 1) — classifier + upsert-overwrite execution

**Goal.** Select the mode with `refresh: latest_value` (implying stored `table`). Classify the model
(natural key + attribute columns; no partition column on the model itself), then maintain **one row
per key** by upserting incoming rows over stored ones via `merge_into`. Encode both Part-19
requirements: (a) prefer an **ordering column derived from the SQL** — the combiner is max-by-ordering-key,
a commutative monoid, so merges are order-independent (out-of-order / parallel backfill licensed);
last-processed is the fallback and *derives* strictly-sequential window execution. (b) Input
consumption is **derived from the source**: a `timeseries:` source is consumed window-forward via the
same `--event-time` driving-source machinery as `cumulative`; a mutable snapshot source is re-scanned
and upserted whole.

**Pre-conditions.** **C1 landed** — the keyed-mode `merge_into` + state-table + presentation-view
plumbing (`cumulative_aggregate.md` §"The maintenance boundary"). If C1 is not yet `done`, block. A1
landed (`RefreshStrategy` exists to add the `LatestValue` value to). `RefreshStrategy` today is
`{Full, Batched, Cumulative, MaterializedView}` after Group A (`config.rs:26`, Deserialize `:34-58`);
`cumulative` classification lives in `crates/smelt-logical/src/rules/cumulative.rs` (`classify_cumulative`,
`CumulativeClassification`, `DrivingSource`, `SourceTimeseriesMap`) and its execution in
`crates/smelt-runtime/src/cumulative.rs` (`execute_cumulative_aggregate:34`, `build_cumulative_merge_sql:218`,
`classify_cumulative_sql:358`), dispatched from `crates/smelt-runtime/src/execute.rs:735-946` via
`ModelStrategy::Cumulative` (`crates/smelt-runtime/src/types.rs:150-172`).

**Open decision (record the choice).** §19.8 shared-executor-vs-per-rule-copies: does the windowed
`latest_value` path reuse cumulative's driving-source + per-partition-step executor (one shared
executor under the keyed umbrella) or keep its own copy? Decide here; D2 adopts the same. Also decide
whether snapshot-diff ships as always-full-rescan (deferring `--auto` staleness) and record under
§Deferred.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` (or `metadata.rs`) unit — `refresh: latest_value` deserialises to `RefreshStrategy::LatestValue`; the keyed-mode constraint violations fire: `refresh: latest_value` + `timeseries:` **on the model** is a hard error, and `refresh: latest_value` + a `batched:` block is a hard error (`models.md` §"Constraint violations"). A bare `refresh: foo` still errors listing `latest_value` among the valid values.
- `crates/smelt-logical/src/rules/` unit (new `latest_value.rs`, mirroring `cumulative.rs`) — the classifier accepts a SELECT of natural key + attributes and identifies the ordering column when the SQL projects one (an `updated_at`); a model whose SELECT declares no natural key is rejected with a named diagnostic.
- `crates/smelt-runtime/src/` unit — the upsert-overwrite merge SQL keeps exactly one row per key and, **with an ordering column**, retains the max-by-ordering-key value under an out-of-order merge (the §19.4 footgun: replaying an old run window must **not** clobber a newer value already stored).
- `examples/` real fixture — a new `examples/latest_value_snapshot/` (mutable snapshot source) and/or `examples/latest_value_stream/` (a `timeseries:` update-events source): `smelt build` yields one row per key, always the most-recent value; changing an attribute overwrites in place; `cargo test -p smelt-cli --test example_diagnostics` is clean. End-state-equivalence harness: the maintained table equals a full rebuild over the processed inputs, including an **out-of-order-merge** case for the ordering-column form.

**Implementation shape.**
- Add `RefreshStrategy::LatestValue` (`config.rs:26`) + its Deserialize/Serialize arms (`"latest_value"`); add the keyed-mode constraint-violation rows (forbid `timeseries:` and `batched:` block on the model) mirroring the `cumulative` validation branch (`metadata.rs`).
- New classifier `crates/smelt-logical/src/rules/latest_value.rs` (sibling of `cumulative.rs`): derive natural key + attributes from the SELECT; derive the ordering column from the SQL where one is projected; resolve the driving source (window-forward) vs mutable-snapshot input via the source's `timeseries:` shape (`SourceTimeseriesMap`, the same signal `cumulative` reads).
- Execution: `merge_into` upsert-overwrite. Per the shared-executor decision, either extend the `cumulative` driving-source/per-partition loop to carry a last-writer combiner (max-by-ordering-key or last-processed) or add a sibling `latest_value` executor. Add `ModelStrategy::LatestValue` (`types.rs:150-172`) and its dispatch arm in `execute.rs`.
- Ordering-column monoid: with an ordering column, the per-partition merge is order-independent — do not force sequential windows. Without one, mark the model ordered (derived sequential execution, the §11.4 posture) — reuse the self-referential/ordered-execution gating if D-adjacent Group-B work (B6) has landed it, else gate windows sequentially locally.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy` (`:26-58`); `crates/smelt-core/src/metadata.rs` — keyed-mode constraint violations.
- `crates/smelt-logical/src/rules/latest_value.rs` (new) + `rules/mod.rs`; read-only reference `rules/cumulative.rs`.
- `crates/smelt-runtime/src/cumulative.rs` (reference for the merge loop) + a `latest_value` executor or a shared extension; `crates/smelt-runtime/src/execute.rs:735-946` (dispatch); `crates/smelt-runtime/src/types.rs:150-172` (`ModelStrategy`).
- `crates/smelt-backend/src/lib.rs:286` — `merge_into` trait (reused as-is); DuckDB impl `crates/smelt-backend-duckdb/src/lib.rs:618`.
- `examples/latest_value_*` (new fixtures).

**Docs touched.**
- `docs/specs/latest_value_models.md` §Known Divergences — remove the "Not implemented" note; narrow "Definition of 'latest' is unsettled" to record the *decided* ordering-column-preferred direction (the algebra it cites is now realised), leaving only the tie-break/deletion sub-questions open.
- `docs-site/docs/guide/` — add the `refresh: latest_value` mode to the refresh-modes guide (one-row-per-key; derived input consumption).

**Review checklist.**
- [ ] `refresh: latest_value` deserialises; both keyed-mode constraint violations enforced (`models.md` §"Constraint violations").
- [ ] Classifier derives the natural key and (where projected) the ordering column from the SQL — not from a strategy block.
- [ ] End-state-equivalence harness passes, including the ordering-column out-of-order-merge case (the §19.4 footgun test is green: an old-window replay does not clobber a newer value).
- [ ] The shared-executor-vs-per-rule-copies decision is recorded (§Deferred or inline) for D2 to adopt.
- [ ] Example fixture builds with zero diagnostics; spec/docs edits are timeless.

**Commit.** `feat(refresh): add refresh: latest_value keyed mode — classifier + upsert-overwrite via merge_into`

---

### Phase D2: `refresh: versioned` (SCD Type 2) — classifier + version maintenance + validity columns

**Goal.** Select the mode with `refresh: versioned` (implying stored `table`). Classify the model
(natural key + tracked attributes; no partition column on the model itself), then maintain **version
history**: compare each incoming row to the stored current version per key and, where a tracked
attribute changed, **close the prior version and open a new one** via `merge_into`. smelt manages the
validity columns (`valid_from` / `valid_to` / `is_current`) and stamps them from the **source's event
time**, not the run clock, so replays are end-state-equivalent. Input consumption is derived from the
source exactly as D1: a `timeseries:` update-events / CDC source is consumed window-forward with
windows applied in temporal order (close/open is inherently ordered); a mutable snapshot source is
re-scanned and compared.

**Pre-conditions.** **C1 landed** (keyed-mode `merge_into` + state/presentation plumbing); if not,
block. D1 landed — D2 **adopts D1's shared-executor-vs-per-rule-copies decision** rather than
re-opening it (§19.8). `RefreshStrategy::LatestValue` exists (D1).

**Spec increment (pre-authorised).** Promote the settled validity-column + change-tracking surface
from `versioned_models.md` §Open Questions into §Surface as it is decided: fix the exact
`valid_from` / `valid_to` / `is_current` names & types, the open-interval representation (NULL vs
far-future sentinel), and the tracked-attribute rule (all projected non-key columns by default vs an
explicit subset, and how a column is marked untracked). Make the `versioned_models.md` §Surface prose
agree with the code in the same commit.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — `refresh: versioned` deserialises to `RefreshStrategy::Versioned`; the keyed-mode constraint violations fire (`+ timeseries:` on the model, `+ batched:` block → hard error).
- `crates/smelt-logical/src/rules/versioned.rs` (new) unit — the classifier derives the natural key + tracked attributes from the SELECT; a change in a tracked attribute is detected; a change confined to an *untracked* column opens no new version.
- `crates/smelt-runtime/src/` unit — the close-old/open-new merge produces, for a key with three successive states, **two closed intervals + one open** (`is_current = true`); validity intervals are stamped from the source event time (a replayed window reproduces the same intervals). Invariant: non-overlapping validity intervals per key, at most one open version.
- `examples/versioned_snapshot/` (or `versioned_stream/`) real fixture — `smelt build` yields the interval history; **non-overlapping snapshots merged in any order converge to the same history** (order-independence). Interval-keyed end-state-equivalence harness vs a full rebuild; `example_diagnostics` clean.

**Implementation shape.**
- Add `RefreshStrategy::Versioned` (`config.rs:26`) + Deserialize/Serialize (`"versioned"`) + the two keyed-mode constraint-violation rows (`metadata.rs`).
- New classifier `crates/smelt-logical/src/rules/versioned.rs` (sibling of `cumulative.rs`/`latest_value.rs`): natural key + tracked attributes; driving-source vs snapshot input via `SourceTimeseriesMap`.
- Execution: version-maintenance merge (close prior current version — set its `valid_to` / clear `is_current` — and insert the new open version) built on the D1-decided executor. Validity columns computed from the source event-time expression, not the run clock. Add `ModelStrategy::Versioned` (`types.rs`) + dispatch (`execute.rs`).
- Output schema: augment the model's projected columns with the managed validity columns per the promoted §Surface.

**Critical files.**
- `crates/smelt-core/src/config.rs` (`RefreshStrategy`), `crates/smelt-core/src/metadata.rs` (constraint violations).
- `crates/smelt-logical/src/rules/versioned.rs` (new) + `rules/mod.rs`.
- `crates/smelt-runtime/` — a `versioned` executor or the shared extension; `execute.rs` dispatch; `types.rs` `ModelStrategy`.
- `crates/smelt-backend/src/lib.rs:286` — `merge_into` (the close/open uses it; if the close-old step needs a different match/update shape than the plain upsert, extend the trait rather than hand-rolling raw SQL in the runtime).
- `examples/versioned_*` (new fixtures).

**Docs touched.**
- `docs/specs/versioned_models.md` — §Surface: the promoted validity-column + change-tracking surface (spec increment above). §Known Divergences: remove the "Not implemented" note and the "Validity-column surface is unsettled" / "Tracked-attribute selection is unsettled" notes for the parts now decided; keep only the deletions/late-corrections retraction question open.
- `docs-site/docs/guide/` — add `refresh: versioned` (keep-every-version; validity interval; source-event-time stamping).

**Review checklist.**
- [ ] `refresh: versioned` deserialises; keyed-mode constraint violations enforced.
- [ ] Validity columns stamped from the source event time (replay reproduces identical intervals); non-overlapping-per-key invariant holds; ≤ one open version per key.
- [ ] Interval-keyed end-state-equivalence harness passes, including the order-independent-merge case.
- [ ] Spec §Surface promotion matches the code exactly in the same commit; timeless.
- [ ] D2 adopted D1's executor decision (no second copy unless D1 chose per-rule copies).

**Commit.** `feat(refresh): add refresh: versioned keyed mode — version maintenance + smelt-managed validity columns`

---

### Phase D3: `refresh: materialized_view` — emit the native maintained object (thin, delegated)

**Goal.** For a `refresh: materialized_view` model, emit the backend's native maintained object for the
model's SQL when `supports_native_ivm = true`; relay the engine's accept/reject **verbatim**; keep the
A4 hard error when `supports_native_ivm = false` (the common case today — so on DuckDB it always
errors). **No** smelt-side eligibility analysis: eligibility is exactly what the engine
incrementalises (`materialized_view.md` §Design "Delegation inherits the engine's correctness").

**Pre-conditions.** **A4 landed** — `supports_native_ivm` / `supports_retraction` flags and the
`refresh: materialized_view` parse + hard error (`multi_backend.md` §"IVM capabilities";
`materialized_view.md` §"No silent fallback"). If A4 is not yet `done`, block. Independent of D1/D2.
Today the emit path can only be exercised against a mock backend: no real backend advertises
`supports_native_ivm = true` (`crates/smelt-dialect/src/dialect.rs:67`).

**TDD tests to write first.**
- `crates/smelt-cli` (or `smelt-db`) real-fixture test — on DuckDB, `refresh: materialized_view` produces the A4 hard error exactly (`materialized_view.md` §"No silent fallback": *"requires native incremental-view maintenance; this engine has none — use `refresh: cumulative`…"*); it does **not** silently become `cumulative` or a full table. (Regression-guards that D3 does not accidentally introduce a silent fallback.)
- `crates/smelt-backend`/`smelt-runtime` unit against a **mock backend with `supports_native_ivm = true`** — a `refresh: materialized_view` model routes to `create_materialized_view_as` (not `create_table_as`); when the mock's IVM runtime *rejects* the query, the engine's own reason is surfaced verbatim (a hard error carrying the backend message, e.g. an Enzyme-style `MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`), not masked or downgraded.
- `examples/` — a `refresh: materialized_view` fixture that asserts the DuckDB hard error via `example_diagnostics` (the emit-success path stays in the mock unit test, since no real IVM backend exists).

**Implementation shape.**
- Route `RefreshStrategy::MaterializedView` at execution: when the resolved backend's `supports_native_ivm` is `true`, call `create_materialized_view_as` (`crates/smelt-backend/src/lib.rs:309`) / `drop_materialized_view_if_exists` (`:325`); when `false`, emit the A4 hard error. This replaces the A3-removed / A4-relocated storage-value branch at `crates/smelt-backend/src/lib.rs:134` (which today gates on the old `supports_materialized_views` capability).
- Remove the **silent** `create_materialized_view_as` default-fallback-to-`create_table_as`-with-warning (`crates/smelt-backend/src/lib.rs:309-317`) semantics for this path: the no-native-IVM case is a hard error surfaced before emit, not a warn-and-degrade. (Leave the trait default only if it is unreachable for the refresh path.)
- Relay engine rejection: propagate the backend's error text as the diagnostic reason rather than a generic message.
- Add `ModelStrategy::MaterializedView` (or reuse a delegated-emit marker) + its dispatch arm; no state table, no combiner, no per-partition loop — smelt owns no maintenance state for this mode (`materialized_view.md` §Constraint 4).

**Critical files.**
- `crates/smelt-backend/src/lib.rs` — the `:134` branch (now `refresh`-driven, gated on `supports_native_ivm`), `create_materialized_view_as:309` / `drop_materialized_view_if_exists:325`.
- `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB advertises `supports_native_ivm = false`; ensure no MV emit path is reachable there.
- `crates/smelt-dialect/src/dialect.rs:67` — `supports_native_ivm` gate (read-only; set by A4).
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/types.rs` — `ModelStrategy` + dispatch.
- `crates/smelt-backend` test module / a mock backend fixture with `supports_native_ivm = true` — the emit + engine-reject unit tests.

**Docs touched.**
- `docs/specs/materialized_view.md` §Known Divergences — narrow the "Not implemented; no backend advertises native IVM" note: the emit path now exists behind the capability gate; the residual gap is that no *shipping* backend sets `supports_native_ivm = true`, so the hard error is still the only reachable outcome on real backends. Keep the eligibility-surfacing and per-engine-strategy items deferred.
- `docs-site/` — note that `refresh: materialized_view` emits a native maintained object on engines with native IVM and hard-errors elsewhere; smelt performs no eligibility analysis of its own.

**Review checklist.**
- [ ] DuckDB `refresh: materialized_view` is the exact A4 hard error; **no** silent fallback to `cumulative` or a full table.
- [ ] Against a mock `supports_native_ivm = true` backend, the model routes to `create_materialized_view_as` and engine rejection is relayed verbatim.
- [ ] No smelt-side eligibility analysis added; smelt keeps no maintenance state for this mode.
- [ ] `materialized_view.md` §Known-Divergence note narrowed to the emit gap; edits timeless.

**Commit.** `feat(refresh): emit native materialized_view via backend IVM behind supports_native_ivm; relay engine errors`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Record here: the D1 shared-executor-vs-per-rule-copies decision, any snapshot-diff
always-full-rescan / `--auto`-staleness deferral, and the shared deletions/late-corrections retraction
question if it surfaces.)

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — the new `latest_value` / `versioned` / `materialized_view` example fixtures build with the expected diagnostics (zero for the valid keyed-mode fixtures; the exact hard error for the DuckDB `materialized_view` fixture).
- End-state-equivalence harnesses: `latest_value` (incl. ordering-column out-of-order merge) and `versioned` (interval-keyed, order-independent merge) each equal a full rebuild over the processed inputs. `materialized_view` has the hard-error path test plus the mock-backend emit + engine-reject tests.
- `/smelt:validate latest_value_models`, `/smelt:validate versioned_models`, `/smelt:validate materialized_view` report zero drift for the surfaces this group lands; each spec's "Not implemented" §Known-Divergence note is removed (or narrowed to the still-open sub-behaviour) as its phase completes.
