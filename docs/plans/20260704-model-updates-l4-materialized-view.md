# Plan: Model updates — L4 composition for `refresh: materialized_view`

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L4 mode-composition** layer, `materialized_view` vertical.
**Specs (oracles)**:
- [`docs/specs/materialized_view.md`](../specs/materialized_view.md) — PRIMARY. §"Composition" (L1 proofs = **none**; transform = **delegate-to-native-IVM**; world-fact = `supports_native_ivm`; output = keyed, **freshness owner = the engine (push)**; equivalence discharged by the engine's native IVM, not the smelt oracle); §Semantics "Engine-incrementalizability", "No smelt-side eligibility", "No silent fallback"; §Constraints (esp. 3 no-silent-fallback, 4 smelt owns no state, 5 no smelt-side eligibility); §Design "Delegation inherits the engine's correctness".
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — §"The equivalence invariant" (the equivalence reframe: for this mode the single invariant is discharged by the **engine's** native IVM, smelt runs no combiner) and §"Validator, not chooser" (smelt never chooses a mode for the user, so `materialized_view` cannot silently degrade).
- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — §"Incremental-view-maintenance capabilities": the `supports_native_ivm` flag Group A's A4 already wired the hard error against; `supports_native_ivm = false` on every current backend.
**Research**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Target plan architecture (the re-cut master)" (L0–L4; this sub-plan is the L4 `materialized_view` composition).
**Spec diff**: **none new** — `materialized_view.md` already exists and is normative; this sub-plan lands the code so its §Known-Divergence "emit path silently falls back / not exercised" clauses are removed or narrowed as each phase completes. No phase authors a spec.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan is the L4 composition for **`refresh: materialized_view`** and is **deliberately minimal**. It **supersedes the D3 portion** of Group D (`docs/plans/20260704-model-updates-group-d.md` §"Phase D3"); take the emit + no-silent-fallback work from there and re-cut it here (Group D's remaining D1/D2 keyed modes are untouched). For this mode smelt **proves nothing** about the SQL and does **no** native-IVM eligibility analysis of its own — it delegates to the engine (`materialized_view.md` §"No smelt-side eligibility"). The composition wires **zero** fundamentals: L1 proofs required = **none**; its one input is the `supports_native_ivm` world-fact; its transform is *delegate-to-native-IVM* (emit the backend's own maintained object; hard-error if the engine rejects). Equivalence is discharged by the engine's native IVM, **not** smelt's generative oracle. Because no shipping backend advertises `supports_native_ivm = true`, the mode **parses today but hard-errors on every current backend**; the happy path is only reachable against a mock/`true` fixture (a real Databricks-Enzyme-class backend is needed to exercise it for real — see §Deferred). Broader native-IVM eligibility analysis (predicting incrementalisability before submission) and the per-engine physical-strategy modifier are **out of scope by design** (§Deferred).

**Fail-closed discipline (every phase).** An engine rejection — no native IVM, or an IVM runtime that refuses the query — is a **hard error surfaced verbatim**, never a silent downgrade to a plain table or to another refresh mode (`materialized_view.md` §Constraint 3; §"No silent fallback"). No phase may introduce a warn-and-degrade path.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (registered by a
human; the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The invariant
   for this mode is the single processed-input equivalence invariant, **discharged by the engine's native
   IVM** (`model_maintenance.md` §"The equivalence invariant"): smelt runs no combiner, keeps no maintenance
   state, and does no eligibility analysis — it emits the native object and relays the engine's verdict.
2. Confirm you are on branch `worktree-incremental` and that **A4 is landed** (the `supports_native_ivm` /
   `supports_retraction` flags and the `refresh: materialized_view` compile-time hard error — Group A, `done
   2026-07-04`).
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status to
   `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests — **every** phase names a
fail-closed hard-error/reject test) → reviewer subagent (material findings only) → iterate → set the row
`done` → commit + push with the phase's `Commit.` line.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits
describe the feature as if it always existed; as each phase lands, **remove or narrow** the matching
§Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec, an unmet dependency (A4 not landed), or
a pre-flight red unrelated to this phase's target: set the row `blocked` with a one-line reason, append to
§"Blocked phases", restore a clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape added `refresh: materialized_view` to the peer refresh enum
(`models.md` §"Refresh axis") as the **engine-owned** freshness mode: smelt hands the model's logical SQL to
the backend's native IVM runtime, which keeps the result current continuously. It is the delegation target
where the algebraic maintenance ladder ends — smelt runs no combiner and keeps no state
(`model_maintenance.md` §"The algebraic maintenance ladder"; `materialized_view.md` §"Composition").

Group A's **A4** already parses `refresh: materialized_view` (`RefreshStrategy::MaterializedView`), added the
`supports_native_ivm` / `supports_retraction` capability flags (both `false` on every current backend), and
lands the compile-time hard error: `crates/smelt-runtime/src/compile.rs` refuses a `materialized_view` model
when `supports_native_ivm = false` with the §"No silent fallback" message, asserted by
`test_materialized_view_hard_errors_without_native_ivm`. Because every backend sets the flag `false`, that
gate always fires first, so the mode never reaches emit today.

Two gaps remain, and this sub-plan closes them minimally:
1. **The emit path is unbuilt.** When a backend *does* advertise `supports_native_ivm = true`, no execution
   route calls the backend's native maintained-object emitter for a `materialized_view` model.
2. **The backend-level emitter silently falls back.** `Backend::create_materialized_view_as`
   (`crates/smelt-backend/src/lib.rs:293`) defaults to `create_table_as` **with a warning** — a silent
   downgrade to a plain table, inconsistent with `materialized_view.md` Constraint 3. It is latent today (the
   A4 compile gate shields it) but must become a hard error before any backend sets `supports_native_ivm =
   true`, else a mis-capable backend would silently emit a stale table.

This vertical is thin by design: smelt emits the native object, relays the engine's accept/reject verbatim,
and hard-errors when there is no native IVM. It performs **no** smelt-side eligibility analysis
(`materialized_view.md` §Design "Delegation inherits the engine's correctness").

## Scope

### In scope
- **MV1** — the delegate-to-native-IVM **emit path**: route `RefreshStrategy::MaterializedView` at execution
  to the backend's `create_materialized_view_as` when `supports_native_ivm = true`; **remove the silent
  `create_table_as` fallback** in the backend default (make no-native-IVM a hard error, never a warn-and-
  degrade). Exercised against a mock `supports_native_ivm = true` backend fixture (no real IVM backend
  exists). smelt owns no state, runs no combiner, does no eligibility analysis for this route.
- **MV2** — **engine-error verbatim relay + the no-silent-fallback regression**: when the (mock) native IVM
  runtime *rejects* the query, surface the engine's own reason verbatim as the diagnostic (hard error, not
  masked); and confirm the A4 hard-error path holds on DuckDB (`supports_native_ivm = false`) — the model is
  the exact A4 message, **never** a silent switch to `cumulative` or a full table. A4 already lands the gate;
  this adds the regression guard that MV1's emit path did not open a fallback hole.
- **MV3 (optional)** — a `smelt explain` readout showing **freshness owner = engine (push)** for a
  `materialized_view` model (`materialized_view.md` §Semantics "Freshness owner"), so the operator contract
  the mode exists to surface is legible. Deferrable if `smelt explain` has no mode-readout surface to extend.

### Absorbed from the keyed-collapse decision record (no phase needed)

- **D16 — output-shape wording fix.** The keyed-collapse decision record
  (`docs/research/20260705-keyed-collapse-application.md` D16) assigned this sub-plan the
  textual correction of `materialized_view.md`'s output-shape wording from "keyed" to
  **engine-defined**, plus the matching `models.md` refresh-table cell. This landed as a
  companion spec edit in the keyed-collapse plan's Phase K1
  (`docs/plans/20260705-keyed-collapse.md`), not as a phase of this sub-plan — it is a
  purely textual, blocking-free change with no code dependency, so it shipped ahead of
  MV1–MV3 rather than waiting on them. No phase here needs to touch that wording again.
  The consumer-facing `timeseries:`-on-output direction D16 also raised is **not**
  absorbed here or there — it remains deferred pending pushdown wiring
  (`materialized_view.md` §Known Divergences).

### Explicitly deferred (minimal by design)
- **The real happy path against a shipping native-IVM backend.** No current backend advertises
  `supports_native_ivm = true` (`multi_backend.md`; `crates/smelt-dialect/src/dialect.rs`), so MV1's emit
  path is only reachable against a mock/`true` fixture. Exercising it end-to-end against Databricks Enzyme
  (the reference target) is gated on that backend existing — deferred, not dropped. Until then the hard error
  is the only outcome reachable on real backends.
- **Richer native-IVM pre-flight** (predicting incrementalisability before submission) and the **per-engine
  physical-strategy modifier** — out of scope by design until a concrete IVM backend motivates them
  (`materialized_view.md` §Known Divergences). This mode is minimal: smelt relays the engine's verdict, it
  does not re-derive it.

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| MV1 | A4 (done); no fundamentals — this mode wires none | `materialized_view.md` §"Composition" (delegate-to-native-IVM); §"No silent fallback" case 1 | pending |
| MV2 | MV1; A4 (done) | `materialized_view.md` §"Engine-incrementalizability", §"No silent fallback" case 2; §Constraint 3 | pending |
| MV3 (optional) | MV1 | `materialized_view.md` §Semantics "Freshness owner: the engine (push)" | pending |

---

### Phase MV1: delegate-to-native-IVM emit path + remove the silent fallback

**Goal.** For a `refresh: materialized_view` model, when the resolved backend's `supports_native_ivm` is
`true`, emit the backend's **native maintained object** (`create_materialized_view_as`) rather than a plain
table; when `false`, the A4 compile-time hard error fires (unchanged). Remove the backend default's silent
`create_table_as`-with-a-warning fallback in `create_materialized_view_as` so the no-native-IVM case can only
be a hard error, never a warn-and-degrade. smelt keeps **no** maintenance state, runs **no** combiner, and
does **no** eligibility analysis for this route (`materialized_view.md` §Constraints 4, 5).

**Spec anchor.** `materialized_view.md` §"Composition" (transform = **delegate-to-native-IVM**; output =
keyed; smelt owns no state); §"No silent fallback" case 1 (no-native-IVM → hard error, never a table);
§Design "Delegation inherits the engine's correctness". Invariant discharged by the engine's native IVM
(`model_maintenance.md` §"The equivalence invariant").

**Pre-conditions.** **A4 landed** — `supports_native_ivm` flag + the `refresh: materialized_view` compile
hard error (`crates/smelt-runtime/src/compile.rs`, `test_materialized_view_hard_errors_without_native_ivm`).
If A4 is not `done`, block. No fundamentals dependency — this mode composes none.

**TDD tests to write first.**
- `crates/smelt-backend`/`smelt-runtime` unit against a **mock backend with `supports_native_ivm = true`** —
  a `refresh: materialized_view` model routes to `create_materialized_view_as` (the native maintained
  object), **not** `create_table_as`. No state table, no `merge_into`, no per-partition loop is emitted for
  this mode.
- `crates/smelt-backend` unit (fail-closed) — the backend default `create_materialized_view_as` **no longer**
  silently calls `create_table_as` with a warning; a backend that reaches it without native IVM hard-errors
  (or the trait default is provably unreachable for the refresh path). Assert no plain-table DDL is emitted.
- `crates/smelt-cli` (or `smelt-db`) real-fixture regression — on DuckDB (`supports_native_ivm = false`),
  `refresh: materialized_view` still produces the exact A4 hard error; MV1's emit route did **not** open a
  silent-fallback hole.

**Implementation shape.**
- Route `RefreshStrategy::MaterializedView` at execution: when the resolved backend's `supports_native_ivm`
  is `true`, call `create_materialized_view_as` (`crates/smelt-backend/src/lib.rs:293`) /
  `drop_materialized_view_if_exists` (`:306`); when `false`, the A4 compile gate already refused it before
  emit. Add `ModelStrategy::MaterializedView` (or a delegated-emit marker) + its dispatch arm; **no** state
  table, combiner, or per-partition loop.
- Remove the silent `create_table_as`-with-a-warning default in `create_materialized_view_as`
  (`:292-300`): the no-native-IVM case is a hard error surfaced before emit, not a warn-and-degrade. Leave a
  trait default only if it is provably unreachable for the refresh path.

**Critical files.**
- `crates/smelt-backend/src/lib.rs` — `create_materialized_view_as` (`:293`) / `drop_materialized_view_if_exists`
  (`:306`); the storage-emit branch (`:125`/`:185`/`:198`) now `refresh`-driven, gated on `supports_native_ivm`.
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/types.rs` — `ModelStrategy` + dispatch arm.
- `crates/smelt-runtime/src/compile.rs` — the A4 `supports_native_ivm` compile gate (read-only; `:1020`).
- `crates/smelt-dialect/src/dialect.rs` — `supports_native_ivm` flag (read-only; `:72`, `false` everywhere).
- `crates/smelt-backend` test module / a mock backend fixture with `supports_native_ivm = true` — the emit unit test.

**Docs touched.**
- `materialized_view.md` §Known Divergences — narrow the "compile-time gate hard-errors; the backend emit
  path silently falls back" note: the backend silent `create_table_as` fallback is **removed** and the emit
  path now routes to the native object behind the capability gate; keep only the residual "no shipping
  backend advertises `supports_native_ivm = true`" gap. `model_transforms.md` §Known Divergences — narrow the
  matching `create_materialized_view_as` *partial* delegate-to-native-IVM note.
- `docs-site/` — note that `refresh: materialized_view` emits a native maintained object on engines with
  native IVM and hard-errors elsewhere (no smelt-side eligibility analysis).

**Review checklist.**
- [ ] Against a mock `supports_native_ivm = true` backend, the model routes to `create_materialized_view_as`, not `create_table_as`.
- [ ] The silent `create_table_as`-with-warning fallback is removed; no-native-IVM is a hard error, never a table.
- [ ] No state table / combiner / per-partition loop emitted for this mode (smelt owns no state).
- [ ] DuckDB `refresh: materialized_view` is still the exact A4 hard error (regression green).
- [ ] §Known-Divergence notes narrowed; edits timeless.

**Commit.** `feat(refresh): emit native materialized_view via backend IVM behind supports_native_ivm; drop the silent table fallback`

---

### Phase MV2: relay engine rejection verbatim + no-silent-fallback regression

**Goal.** When the (mock) native IVM runtime **rejects** the query, surface the engine's own reason verbatim
as the diagnostic — a hard error carrying the backend's message (e.g. an Enzyme-style
`MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`), never masked, downgraded, or replaced with a generic message
(`materialized_view.md` §"Engine-incrementalizability", §"No silent fallback" case 2). Confirm the A4
hard-error path (case 1, no native IVM) as a standing regression: smelt never rescues the user into
`cumulative` or a full table. smelt performs **no** eligibility analysis of its own — it relays the engine's
accept/reject verbatim (§"No smelt-side eligibility").

**Spec anchor.** `materialized_view.md` §"Engine-incrementalizability" (the engine's verdict, relayed
verbatim); §"No silent fallback" case 2 (engine reject → hard error carrying the engine's reason) and case 1
(no native IVM → hard error); §Constraint 3. `model_maintenance.md` §"Validator, not chooser" (smelt never
chooses a mode for the user, so no silent degrade).

**Pre-conditions.** MV1 landed (the emit route exists). A4 landed (the compile gate). Independent of any
fundamentals.

**TDD tests to write first.**
- `crates/smelt-backend`/`smelt-runtime` unit against a **mock `supports_native_ivm = true` backend whose IVM
  runtime rejects the query** — the model fails with a hard error whose diagnostic text **contains the mock
  engine's own reason verbatim** (not a generic "materialized view failed"); the error is not masked or
  downgraded to a plain table.
- `crates/smelt-cli` (or `smelt-db`) real-fixture regression — on DuckDB, `refresh: materialized_view`
  produces the exact A4 hard error message (`materialized_view.md` §"No silent fallback" case 1: *"requires
  native incremental-view maintenance; this engine has none — use `refresh: cumulative`…"*); it does **not**
  become `cumulative` or a full table. (Regression-guards that no fallback was introduced.)
- `examples/` — a `refresh: materialized_view` fixture asserting the DuckDB hard error via
  `example_diagnostics` (the emit-success / reject paths stay in the mock unit tests — no real IVM backend
  exists).

**Implementation shape.**
- In the MV1 emit route, propagate the backend's rejection error **text** as the diagnostic reason rather
  than a generic message; do not catch-and-degrade. No smelt-side pre-flight of incrementalisability is
  added — smelt submits and relays (`materialized_view.md` §Constraint 5).
- Add the `examples/` fixture + the DuckDB hard-error assertion (a regression net around A4 + MV1).

**Critical files.**
- `crates/smelt-runtime/src/execute.rs` — the MV emit route's error propagation (relay backend text verbatim).
- `crates/smelt-backend` mock fixture — a `supports_native_ivm = true` backend whose IVM emitter returns a rejection.
- `examples/materialized_view_*` (new fixture) + `crates/smelt-cli/tests/example_diagnostics` expectation.

**Docs touched.**
- `materialized_view.md` §Known Divergences — narrow the remaining note to the sole residual gap (no shipping
  backend advertises `supports_native_ivm = true`, so the hard error is the only outcome reachable on real
  backends); the emit + verbatim-relay behaviour now exists behind the capability gate.
- `docs-site/` — note that on an IVM backend a rejected query surfaces the engine's own reason; smelt does no
  eligibility analysis of its own.

**Review checklist.**
- [ ] Mock engine rejection surfaces the backend's reason **verbatim** in the diagnostic; not masked/generic.
- [ ] DuckDB `refresh: materialized_view` is the exact A4 hard error; **no** silent fallback to `cumulative`/full table.
- [ ] No smelt-side eligibility analysis added; smelt relays the engine verdict.
- [ ] Example fixture asserts the DuckDB hard error via `example_diagnostics`; edits timeless.

**Commit.** `feat(refresh): relay native-IVM engine rejection verbatim; regression-guard the no-silent-fallback rule`

---

### Phase MV3 (optional): `smelt explain` freshness-owner readout

**Goal.** Surface, in `smelt explain` (or the equivalent model-readout), that a `refresh: materialized_view`
model's **freshness owner is the engine (push)** — the engine keeps it current continuously between runs —
distinguishing it from the smelt-owned pull modes (`materialized_view.md` §Semantics "Freshness owner: the
engine (push)"). This makes the operational contract the mode exists to surface legible. **Optional**: if
`smelt explain` has no mode-readout surface to extend cleanly, block this phase and record the readout as
deferred rather than inventing a surface.

**Spec anchor.** `materialized_view.md` §Semantics "Freshness owner: the engine (push)" (the `cumulative`
pull vs `materialized_view` push contrast; the "who is responsible for freshness" answer).

**Pre-conditions.** MV1 landed. Independent of MV2.

**TDD tests to write first.**
- `crates/smelt-cli` (or wherever `explain` lives) unit/fixture — `smelt explain` for a `materialized_view`
  model reports freshness owner = **engine (push)**; a `cumulative` model reports freshness owner = **smelt
  (pull)**, confirming the two are distinguished.
- (fail-closed) the readout does not claim engine-push for a mode smelt owns.

**Implementation shape.** Extend the existing `explain`/model-readout with a freshness-owner field derived
from the refresh mode (`materialized_view` → engine/push; the smelt-owned modes → smelt/pull). Read-only over
the resolved model; no execution change.

**Critical files.**
- `crates/smelt-cli/src/` — the `explain` command / model-readout renderer.
- `crates/smelt-core/src/config.rs` — `RefreshStrategy` (read-only, for the owner mapping).

**Docs touched.**
- `docs-site/` — document the freshness-owner line in the `explain` output (engine-push for
  `materialized_view`).
- `materialized_view.md` — no change (the §Semantics contract already describes freshness ownership).

**Review checklist.**
- [ ] `explain` reports engine (push) for `materialized_view`, smelt (pull) for the smelt-owned modes.
- [ ] Read-only; no execution behaviour change.
- [ ] Edits timeless; or, if no clean surface exists, blocked + recorded in §Deferred.

**Commit.** `feat(cli): smelt explain shows freshness owner = engine (push) for refresh: materialized_view`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only.) This vertical is **minimal by design**. Broader native-IVM eligibility analysis (smelt-side
prediction of incrementalisability) and the per-engine physical-strategy modifier are out of scope
(`materialized_view.md` §Known Divergences). The **real** happy path is gated on a shipping native-IVM
backend (Databricks Enzyme is the reference target); until one exists, MV1/MV2 exercise the emit + reject
paths against a mock `supports_native_ivm = true` fixture, and the hard error is the only outcome reachable
on real backends. If MV3 finds no clean `explain` surface, record the freshness-owner readout here.

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces`
  — the `materialized_view` example fixture builds with the expected **hard error** on DuckDB (no silent
  fallback); no valid-build fixture exists for this mode until a native-IVM backend does.
- Mock-backend coverage: the emit path routes to `create_materialized_view_as` on `supports_native_ivm =
  true`; an engine rejection is relayed **verbatim**; the backend silent `create_table_as` fallback is gone.
- `/smelt:validate materialized_view` reports zero drift for the surface this vertical lands; the
  §Known-Divergence "emit path silently falls back / not exercised" clauses are removed or narrowed to the
  sole residual "no shipping backend advertises native IVM" gap.
