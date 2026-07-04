# Plan: Model updates — L3 model-scoped declaration surfaces

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L3 layer**
(model-scoped declaration surfaces) of the re-cut master. L0 (specs) is `done`; L1 (derived proofs) + L2
(transforms) are the running sub-plan
[`docs/plans/20260704-model-updates-fundamentals.md`](20260704-model-updates-fundamentals.md)
(phases F1–F15). This sub-plan owns the **world-facts smelt cannot derive** — the ones the modeller (or the
source owner) states — each of which may only **widen** what a proof admits and never substitute for a
proof's fail-closed reject.
**Specs (oracles)**:
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — PRIMARY. §"Model-scoped declarations"
  (the four-row declaration table: declared monotonicity guarantee, `nondeterministic_columns`, functional
  dependency `key → column`, bounded-domain / space budget); §"Catalogued inputs (owned elsewhere)" (the
  world-facts that live on the source/backend/core, referenced not re-homed); §Constraints "Declared escape
  hatches may only widen"; §Design "Derive where decidable, declare where not"; §Known Divergences ("the
  model-scoped declaration surfaces are tracked by …").
- [`docs/specs/models.md`](../specs/models.md) — §"Input-consumption axis (derived, not declared)" (the
  vertical-declared / horizontal-derived split; the mutation-profile is the one non-derivable world-fact on
  this axis, declared *on the source*); §"The declaration law: declared, derived, implied by the mode" (the
  list of *assertions* that bound/widen — source-lateness, `nondeterministic_columns`, bounded-domain
  budget, cost ceiling, declared-monotonicity, source mutation profile); §Known Divergences "Source mutation
  profile is inferred, not yet a first-class source declaration".
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — §"Windowed maintenance and the
  horizon" (the **horizon is derived**; a declared horizon is a *warning ceiling only* and never relaxes the
  clamp; a late arrival beyond the derived horizon is silently clamped, not diagnosed); §Design "The horizon
  is derived, not declared"; §Constraints (the derived-horizon bullet); §Known Divergences "Windowed-by-
  default and the derived horizon are contract, not yet fully built" (the ceiling declaration + its warning
  are not yet surfaced).
- [`docs/specs/sources.md`](../specs/sources.md), [`docs/specs/timeseries.md`](../specs/timeseries.md) —
  the catalogue-by-reference homes for the **source mutation profile** and **source-lateness margin**.
**Research (the "why" + the L-decomposition)**:
[`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) —
§"Target plan architecture (the re-cut master)" bullet **L3 — Declaration surfaces** (this sub-plan);
§"Resolved decisions" → "Static/declared line — decided: derive-else-declare" (a property is a derived proof
where statically decidable and a declared world-fact otherwise); the mapping-table rows **B3** (`+ L3
nondeterministic_columns`) and **C4** (`L3 assertion + L2 multiset state`).
**Spec diff**: two declaration surfaces are *named concretely* by this sub-plan (the spec's §"Model-scoped
declarations" table describes each declaration abstractly but does not fix its frontmatter key; each phase
that lands a declaration names the concrete key in §Surface). One phase (DC5) adds a **new** declaration
surface — the first-class source mutation-profile + source-lateness home in `sources.md`/`timeseries.md`
(today inferred from clock presence, `models.md` §Known Divergences). Those spec edits are called out
per-phase; no phase authors a whole spec. Every other change flips/narrows a §Known-Divergence note as the
declaration ships.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan implements **L3 — the model-scoped declaration *surfaces***
and the **catalogue-by-reference** reconciliation for the source/backend world-facts. A declaration here is
a *validated input surface* plus the **widening semantics + fail-closed guard** on the proof it feeds; it is
**not** the taint flow, the transform, or the mode that consumes it. Specifically out of scope:
- The **`nondeterministic_columns`** declaration already exists (Group A) and its **taint-flow enforcement**
  is completed under the fundamentals/L4-batched work — this sub-plan does **not** rebuild either; it is the
  worked example of the widening contract every other declaration here mirrors, referenced not duplicated.
- The **transforms** each declaration licenses — once-write enrichment (F14 targeted backfill /
  `accumulating_snapshot`), the bounded-domain multiset state (Group C C4 / F-layer multiset), the
  widened-scan/exact-clamp horizon transform (F13/F15) — are L2/L4; this sub-plan builds the *declaration +
  its widening/guard*, and unit-tests that the declaration widens the right verdict and is **refused** when
  it tries to narrow. Wiring the widened verdict into a transform's emit is the transform/mode phase's job.
- The **derived horizon proof** (`model_properties.md`, `not-yet`, fed by F1's unified reach) is L1/L2; DC4
  builds only the *ceiling* declaration + its warning atop whatever derived horizon exists.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when this
L3 layer is scaffolded into the registry — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The single
   governing rule for every phase is `model_properties.md` §Constraints **"Declared escape hatches may only
   widen"**: a model-scoped declaration may only *widen* the set a proof admits; it may **never** substitute
   for a proof's default reject on a construct the proof itself cannot decide, and **never** narrow
   eligibility. Concretely — a declaration supplies a world-fact the proof *lacks* (the proof was
   undecidable → widen with the fact), but it can never override a *positive* disproof (a construct the proof
   affirmatively rejects). The taint-flow guard on `nondeterministic_columns` (Group A) is the reference
   shape: a listed payload column widens; an `event_time`/`partition`/`unique_key` column is **refused**.
2. Confirm you are on branch `worktree-incremental`, that L0 (the capability/framework specs) is landed, and
   that this phase's **Depends on** F-phase(s) are `done` in
   `docs/plans/20260704-model-updates-fundamentals.md`.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status to
   `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names a
**fail-closed reject test** — a declaration that tries to *narrow* or *substitute* for a proof's reject must
be refused with a diagnostic) → reviewer subagent (material findings only) → iterate → set the row `done` →
commit + push with the phase's `Commit.` line.

**Fail-closed reject is the acceptance gate, not a nicety.** Every declaration phase's minimum bar is two
tests: (a) the *widening* test — the declaration admits a construct the proof alone rejected as
*undecidable*; and (b) the *fail-closed reject* test — the same declaration applied to a construct the proof
*positively* disproves, or used to *narrow* eligibility, is **refused with a diagnostic** and never silently
honoured. A phase without both is incomplete.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary (DC1…, F1…) lives in *this file only*. Spec +
`docs-site/` edits describe each declaration as if it always existed; as each phase lands, name the concrete
frontmatter key in §Surface and **narrow** the matching §Known-Divergence note rather than annotating it
with a phase number.

**Block rule.** On a design decision not answered here or by the spec (e.g. the exact frontmatter key spelling
where the spec leaves it open, or a Depends-on F-phase not yet landed), or a pre-flight red unrelated to this
phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean
tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape sorts every fact about a model by *who fixes it* — **declared**, **derived**, or
**implied by the mode** (`models.md` §"The declaration law"). The **derived** facts are L1 proofs; the
**declared** facts are a short list of *assertions* that bound or widen what the machinery may do without ever
picking a strategy. `model_properties.md` §"Model-scoped declarations" names four of them
(declared-monotonicity, `nondeterministic_columns`, functional dependency, bounded-domain budget); the
maintenance horizon adds a fifth (a warning **ceiling**, `model_maintenance.md`); and the input-consumption
axis names a sixth that lives *on the source* rather than the model (the mutation profile, plus the
source-lateness margin). This sub-plan builds those declaration surfaces and — crucially — the **widening
semantics with a fail-closed guard** on each: a declaration is the honest fallback where smelt cannot derive
a world-fact (`derive-else-declare`), but it may only *widen* a proof, never override a positive reject or
narrow eligibility. Only `nondeterministic_columns` exists today (Group A) with its taint-flow enforcement
completed elsewhere; it is the reference shape. The other five are `not-yet`.

## Scope

### In scope (L3)

- **DC1** — Declared monotonicity guarantee escape hatch: the frontmatter surface + the widening of the
  event-time trace's `NotTraceable{undecidable}` verdict (a UDF / opaque body), with a fail-closed guard that
  refuses to override a *positive* disproof (`StaticSeed`; a row-nondeterministic value in a skeleton
  position).
- **DC2** — Functional dependency (`key → column`) declaration: the surface + validation, feeding once-write
  enrichment; fail-closed when the SQL *proves* the column varies within a key (F6 fan-out) or when the FD is
  used to skip a dedup the proof requires.
- **DC3** — Bounded-domain / space budget declaration: the surface + validation, feeding the cumulative
  rung-4 exact-holistic multiset; **fail-loud with a cap, never a default** — an absent/implicit cap is an
  error, and the declaration cannot be applied to a non-holistic misuse.
- **DC4** — Horizon-ceiling declaration: the surface + a **warning-only** comparison against the *derived*
  maintained-window/horizon; the ceiling never relaxes the clamp.
- **DC5** — Catalogue-by-reference pass: add/confirm the **source mutation-profile** + **source-lateness
  margin** declaration homes in `sources.md`/`timeseries.md`, and confirm `model_properties.md` §"Catalogued
  inputs" references them; where a home does not yet exist first-class, land it (the honest derive-else-
  declare fallback) or record it deferred with a plan link.

### Explicitly out of scope (referenced, not rebuilt)

- **`nondeterministic_columns` declaration + its taint flow** — the declaration is Group A (`built`); its
  taint-flow enforcement (no row-nondeterministic value in a skeleton position) is fundamentals/L4-batched.
  This sub-plan references it as the reference widening shape and does not touch it.
- **The transforms/modes each declaration licenses** — once-write enrichment (F14 / `accumulating_snapshot`),
  bounded-domain multiset state (Group C C4), the horizon-bounded MERGE / widened-scan clamp (F13/F15). L2/L4.
- **The derived-horizon proof** (`model_properties.md`, `not-yet`, from F1's reach) — L1/L2; DC4 layers only
  the ceiling declaration + warning on top of it.

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| DC1   | Group A (done); F1 (done) | `model_properties.md` §"Model-scoped declarations" (declared monotonicity); §Constraints "may only widen" | done (2026-07-05) |
| DC2   | Group A (done); F6 | `model_properties.md` §"Model-scoped declarations" (functional dependency `key → column`) | done (2026-07-05) |
| DC3   | Group A (done); F4 | `model_properties.md` §"Model-scoped declarations" (bounded-domain / space budget) | done (2026-07-05) |
| DC4   | Group A (done); F1 (done) + derived-horizon proof | `model_maintenance.md` §"Windowed maintenance and the horizon" | pending |
| DC5   | Group A (done); F9 | `models.md` §Known Divergences "Source mutation profile …"; `model_properties.md` §"Catalogued inputs" | pending |

---

### Phase DC1: Declared monotonicity guarantee escape hatch

**Goal.** Add the model-scoped **declared monotonicity** surface — the modeller's assertion that the model's
event-time expression is monotone where static proof is *undecidable* (a UDF, an opaque body). The static
default is always reject-the-push (`trace_event_time` → `NotTraceable`); this declaration **widens** that one
verdict to admit the pushdown. It may **not** override a *positive* disproof: a `StaticSeed` (constant/`NULL`
in the event-time slot — provably not a stream) or a row-nondeterministic value in a skeleton position is
still refused. Proposed frontmatter key: `assert_monotonic: <event_time_expr>` (or a boolean escape hatch on
the model); the sub-plan fixes the spelling and names it in §Surface.

**Spec anchor.** `model_properties.md` §"Model-scoped declarations" → row **"Declared monotonicity
guarantee"** (name the concrete key in §Surface); §Constraints "Declared escape hatches may only widen";
§Design "Derive where decidable, declare where not" (event-time monotonicity is derivable in the common case
and declared only as an escape hatch). Consumed verdict: §Semantics "Event-time monotonicity trace"
(`Traceable | StaticSeed | NotTraceable`). Maturity: `not-yet`.

**Pre-conditions.** Group A landed (the frontmatter/config plumbing). F1 landed (the unified bound/reach
verdict whose pushdown the widened trace feeds). The event-time trace primitive (W1) is `done`.

**Depends on.** Group A (done); F1 (done).

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — the declaration parses from frontmatter into
  `ModelConfig`; a malformed/empty value is a configuration error (fail-loud), not a silent default.
- `crates/smelt-logical/src/…` unit (**widening**) — a model whose event-time expression is `NotTraceable`
  *because it is undecidable* (an opaque UDF / opaque body) is admitted for pushdown **only** when the
  declaration is present; without the declaration it stays rejected (the escape hatch widens exactly the
  undecidable verdict).
- `crates/smelt-logical/src/…` unit (**fail-closed reject**) — the declaration applied to a construct the
  trace *positively* disproves is **refused with a diagnostic**: a `StaticSeed` (constant/`NULL` event-time
  slot) declared monotonic is rejected (naming the constant seed); a row-nondeterministic value
  (`RANDOM()`/`UUID()`) reaching the event-time/skeleton position declared monotonic is rejected. The
  declaration cannot substitute for a positive disproof, and cannot be used to *narrow* eligibility.
- `crates/smelt-cli/tests/example_diagnostics` — an example model carrying the declaration builds with no
  diagnostics; a misuse (declaring monotonicity over a `StaticSeed`) surfaces the reject.

**Implementation shape.** Add the declaration field to `ModelConfig` (or a model-scoped `assertions:`/
`safety_overrides`-adjacent home) with `deny_unknown_fields` validation. At the trace-consumption site, when
`trace_event_time` returns `NotTraceable{reason}`, upgrade to admit **iff** `reason` is the *undecidable*
class (opaque UDF/body) **and** the declaration is present; leave `StaticSeed` and any nondeterminism-taint
reject untouched (the widen is verdict-scoped, not blanket). Emit the reject diagnostic on misuse.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `ModelConfig` (the new declaration field);
  `crates/smelt-core/src/metadata.rs` — `validate_timeseries`/model validation (the fail-loud parse check +
  the misuse reject; `MetadataError` variant if a new one is warranted, exhaustively matched in
  `smelt-db/src/lib.rs`).
- `crates/smelt-logical/src/analysis/monotonicity.rs` / the trace-consumption site in `rules/incremental.rs`
  — the verdict-scoped widen.

**Docs touched.**
- `model_properties.md` §Surface — name the concrete declared-monotonicity frontmatter key in the
  §"Model-scoped declarations" table row; §Known Divergences — narrow the "model-scoped declaration surfaces
  are tracked by …" note (declared-monotonicity now built).
- `diagnostics.md` — add the misuse diagnostic code if a new one lands.
- `docs-site/docs/guide/` (the batched/incremental page) — a short "escape hatch: declaring monotonicity when
  smelt can't prove it" note, framed timelessly.

**Review checklist.**
- [ ] The declaration widens **only** the undecidable (`NotTraceable`) verdict; a `StaticSeed` /
      nondeterminism-taint positive reject is untouched.
- [ ] A misuse (declared monotonic over a positive disproof) is **refused with a diagnostic** (fail-closed).
- [ ] Malformed/empty declaration is a configuration error (fail-loud), never a silent default.
- [ ] The declaration can only widen, never narrow eligibility.
- [ ] §Surface names the concrete key; the Known-Divergence note is narrowed; edits timeless.

**Commit.** `feat(logical): declared-monotonicity escape hatch — widens the undecidable trace verdict, fail-closed on positive disproof`

---

### Phase DC2: Functional dependency (`key → column`) declaration

**Goal.** Add the **functional dependency** declaration — the modeller's assertion that a column is a per-key
constant (`key → column`), admitting once-write `COALESCE` / 1:1-after-dedup enrichment. The declaration
**widens** what the enrichment path admits (a column the SQL cannot *prove* single-valued per key may be
treated as write-once). It may **not** substitute for a proof that the column *varies* within a key: if
fan-out / cardinality analysis (F6) proves the column is multi-valued per key, or the FD is used to skip a
dedup the proof requires, it is refused. Proposed frontmatter key: `functional_dependencies: [{ key:
[order_id], determines: [customer_tier] }]` (sub-plan fixes the spelling and names it in §Surface).

**Spec anchor.** `model_properties.md` §"Model-scoped declarations" → row **"Functional dependency (`key →
column`)"** (name the concrete key in §Surface); §Constraints "Declared escape hatches may only widen".
Consumed by (not wired here): the once-write enrichment transform (F14 targeted backfill /
`accumulating_snapshot`); the fail-closed guard reads F6's fan-out / join-contribution verdict. Maturity:
`not-yet`.

**Pre-conditions.** Group A landed. F6 landed (fan-out / cardinality proof, so the guard can refuse an FD the
SQL disproves).

**Depends on.** Group A (done); F6.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — the FD declaration parses; an FD naming a column
  or key absent from the model is a configuration error (fail-loud); a self-contradictory FD (empty key /
  empty determines) is rejected.
- `crates/smelt-logical/src/…` unit (**widening**) — a column the SQL cannot statically prove single-valued
  per key is admitted for once-write enrichment **only** when the FD is declared; without it, the enrichment
  path stays at the conservative (rebuild / re-derive) verdict.
- `crates/smelt-logical/src/…` unit (**fail-closed reject**) — an FD whose `determines` column F6 proves is
  *multi-valued per key* (a `OneToMany` fan-out into that column) is **refused with a diagnostic** (the
  declaration cannot substitute for the proof of variance); an FD used to *narrow* — to skip a dedup the
  proof requires — is refused.
- `crates/smelt-cli/tests/example_diagnostics` — an example model with a valid FD builds clean.

**Implementation shape.** Add the FD list to `ModelConfig` with structural validation (keys/columns resolve
against the model schema). At the enrichment-licence site, treat a declared `key → column` as write-once
**iff** F6 does not positively disprove it; on a proven fan-out into the determined column, emit the reject
diagnostic. No transform emitted here — this is the declaration + its widening/guard only.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `ModelConfig` FD field; `crates/smelt-core/src/metadata.rs` — parse +
  structural validation (+ `MetadataError` variant if warranted, exhaustively matched in `smelt-db`).
- `crates/smelt-logical/src/analysis/…` — the FD-vs-fan-out guard consuming F6.

**Docs touched.**
- `model_properties.md` §Surface — name the concrete FD frontmatter key in the declaration table row; §Known
  Divergences — narrow the declaration-surfaces note.
- `diagnostics.md` — the FD-misuse diagnostic code if new.
- `docs-site/` — an enrichment/backfill note if a user-facing surface exists; else verify guide prose.

**Review checklist.**
- [ ] The FD widens once-write enrichment only where the SQL is undecidable on per-key constancy.
- [ ] An FD F6 positively disproves (multi-valued per key) is **refused with a diagnostic** (fail-closed).
- [ ] An FD naming an absent key/column, or a self-contradictory FD, is a configuration error (fail-loud).
- [ ] The FD cannot narrow (skip a required dedup).
- [ ] §Surface names the concrete key; the note is narrowed; edits timeless.

**Commit.** `feat(logical): functional-dependency (key → column) declaration for once-write enrichment; fail-closed on proven fan-out`

---

### Phase DC3: Bounded-domain / space budget declaration

**Goal.** Add the **bounded-domain / space budget** declaration — the modeller's assertion that a column's
active domain is bounded, licensing an exact holistic aggregate (`MEDIAN`/`MODE`/exact-`COUNT(DISTINCT)`) via
an explicit per-key multiset. It **widens** what the holistic-aggregate path admits (an otherwise-refused
exact holistic aggregate becomes maintainable). Per the spec it is **fail-loud with a cap, never the
default**: the declaration must carry an explicit space budget (a cap); an absent/implicit cap is an error,
not a permissive default, and the declaration may not be applied to a non-holistic misuse. Proposed
frontmatter key: `bounded_domain: { column: category, max_cardinality: 10000 }` (sub-plan fixes spelling,
names it in §Surface).

**Spec anchor.** `model_properties.md` §"Model-scoped declarations" → row **"Bounded-domain / space budget"**
(name the concrete key in §Surface); §Constraints "may only widen". Consumed by (not wired here): the exact-
holistic multiset state (Group C C4 / the L2 multiset transform); the runtime cap → full-refresh fallback is
transform-side (L4). Reads F4's holistic discriminant to reject a non-holistic misuse. Maturity: `not-yet`.

**Pre-conditions.** Group A landed. F4 landed (algebraic discriminants — the holistic classification the
declaration attaches to).

**Depends on.** Group A (done); F4.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit (**fail-loud cap**) — a `bounded_domain` declaration
  **without** an explicit cap is a configuration **error** (never a silent default); a cap ≤ 0 / non-numeric
  is rejected; a well-formed declaration parses.
- `crates/smelt-logical/src/…` unit (**widening**) — an exact holistic aggregate over the declared bounded
  column is admitted for multiset maintenance **only** when the declaration (with a cap) is present; without
  it, the holistic aggregate stays refused (`refresh: full` / approximate suggestion).
- `crates/smelt-logical/src/…` unit (**fail-closed reject**) — the declaration applied to a **non-holistic**
  aggregate (F4 says `SUM`/monoid — no multiset needed) is refused/inert (it cannot license what needs no
  licence, and cannot narrow); the declaration cannot be used to substitute for the fail-closed refusal of an
  *unbounded* domain (no cap ⇒ error, above).
- `crates/smelt-cli/tests/example_diagnostics` — an example exact-`MEDIAN` model with a bounded-domain cap
  builds clean.

**Implementation shape.** Add the bounded-domain declaration (column + explicit cap) to `ModelConfig` with a
**required** cap (no `#[serde(default)]` fallback that would make it optional-to-zero). At the holistic-
aggregate licence site, admit multiset maintenance iff F4 classifies the aggregate holistic **and** the
declaration with a cap is present; otherwise keep the fail-loud refusal. The runtime cap-exceeded → full-
refresh path is L4, not built here.

**Critical files.**
- `crates/smelt-core/src/config.rs` — the bounded-domain field (cap required);
  `crates/smelt-core/src/metadata.rs` — the fail-loud cap validation (+ `MetadataError` variant, exhaustively
  matched in `smelt-db`).
- `crates/smelt-logical/src/analysis/…` / `rules/cumulative.rs` — the holistic licence gate consuming F4.

**Docs touched.**
- `model_properties.md` §Surface — name the concrete bounded-domain frontmatter key (with the "explicit cap,
  never default" rule) in the declaration table row; §Known Divergences — narrow the declaration-surfaces
  note.
- `cumulative_aggregate.md` §"The maintenance boundary" (rung 4) — reference the declaration by name as the
  opt-in that unlocks the exact holistic rung (no phase vocabulary).
- `diagnostics.md` — the missing-cap diagnostic code.
- `docs-site/` — a cumulative exact-holistic opt-in note if a user page exists; else verify prose.

**Review checklist.**
- [ ] An absent/implicit cap is a **fail-loud error**, never a permissive default.
- [ ] The declaration widens **only** an F4-classified holistic aggregate; a monoid misuse is refused/inert.
- [ ] The declaration cannot substitute for the refusal of an unbounded domain, and cannot narrow.
- [ ] §Surface names the concrete key + the cap rule; the note is narrowed; edits timeless.

**Commit.** `feat(logical): bounded-domain / space-budget declaration (explicit cap, fail-loud) licensing exact-holistic multiset`

---

### Phase DC4: Horizon-ceiling declaration (warning-only)

**Goal.** Add the **horizon-ceiling** declaration — the modeller's *warning ceiling* on the maintained
window. The horizon is **derived** from the model's reach (`model_properties.md`, fed by F1); a declared
ceiling **never relaxes the clamp**. smelt **warns** when the *derived* horizon would exceed the declared
ceiling and otherwise does nothing — the clamp always uses the derived value. This is the one *warning-only*
declaration; its fail-closed flavour is that it **cannot narrow** the clamp. Proposed frontmatter key:
`horizon_ceiling: '30 days'` (parsed by the shared `parse_interval`; sub-plan fixes spelling, names it in
§Surface).

**Spec anchor.** `model_maintenance.md` §"Windowed maintenance and the horizon" (the ceiling paragraph);
§Design "The horizon is derived, not declared" (a declaration is admitted only as a *ceiling* that warns);
§Constraints (the derived-horizon bullet); §Known Divergences "Windowed-by-default and the derived horizon
are contract, not yet fully built" (the ceiling declaration + its warning are not yet surfaced — narrow this
as the ceiling lands). Maturity: `not-yet` (and the derived-horizon proof it reads is itself `not-yet`).

**Pre-conditions.** Group A landed. F1 landed (the unified reach feeding the derived horizon). The
derived-horizon proof (`model_properties.md`, `not-yet`) must exist to compare against; if it has not landed,
DC4 builds the ceiling parse + warning against whatever horizon derivation exists, else records the
comparison as blocked with a link (block rule).

**Depends on.** Group A (done); F1 (done) + the derived maintained-window/horizon proof.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — the ceiling parses via the shared `parse_interval`;
  a symbolic (`'1 month'`) or malformed value is handled per the shared parser's rules (fail-loud on
  malformed, not a silent default).
- `crates/smelt-…/src/…` unit (**warning**) — when the *derived* horizon exceeds the declared ceiling, a
  **warning** diagnostic is emitted naming both values; when the derived horizon is within the ceiling, no
  diagnostic.
- `crates/smelt-…/src/…` unit (**fail-closed / cannot narrow**) — a ceiling **smaller** than the derived
  horizon does **not** shrink the emitted clamp window: assert the clamp/scan window is unchanged (still the
  derived value) and only a warning is produced. The declaration never relaxes or narrows the clamp.
- `crates/smelt-cli/tests/example_diagnostics` — an example model with a comfortable ceiling builds clean; a
  model whose derived horizon exceeds a tight ceiling surfaces the warning (not an error).

**Implementation shape.** Add the ceiling field to `ModelConfig` (interval, via `parse_interval`). At the
horizon-derivation consumption site, compare derived vs ceiling; emit a **warning** when derived > ceiling;
**never** substitute the ceiling for the derived clamp. Keep the clamp emit driven solely by the derived
value.

**Critical files.**
- `crates/smelt-core/src/config.rs` — the ceiling field (interval);
  `crates/smelt-core/src/metadata.rs` — parse validation.
- the horizon-derivation consumption site (`crates/smelt-logical/src/analysis/source_bounds.rs` /
  `smelt-runtime/src/compile.rs`) — the derived-vs-ceiling comparison + warning; the clamp stays derived.

**Docs touched.**
- `model_maintenance.md` §"Windowed maintenance and the horizon" — name the concrete ceiling frontmatter key
  as the warning-only ceiling (timeless); §Known Divergences — narrow the "ceiling declaration + its warning
  are not yet surfaced" clause.
- `model_properties.md` §"Model-scoped declarations" / §"Catalogued inputs" — cross-reference the ceiling as a
  model-scoped warning declaration (its normative home is `model_maintenance.md`).
- `diagnostics.md` — the horizon-ceiling **warning** code.
- `docs-site/docs/guide/incremental-models.md` — a short "declaring a horizon ceiling (warning only; the
  horizon is derived)" note.

**Review checklist.**
- [ ] The ceiling is **warning-only**; the clamp always uses the *derived* horizon.
- [ ] A ceiling smaller than the derived horizon does **not** shrink the clamp (fail-closed / cannot narrow).
- [ ] A malformed ceiling is fail-loud; a within-ceiling model warns not at all.
- [ ] §Surface names the concrete key; the Known-Divergence clause is narrowed; edits timeless.

**Commit.** `feat(maintenance): horizon-ceiling declaration (warning-only; never relaxes the derived clamp)`

---

### Phase DC5: Catalogue-by-reference — source mutation profile + source-lateness home

**Goal.** Reconcile the **catalogued inputs** that live on the source, not the model. Two world-facts:
(a) the **source mutation profile** (append-only / mutable / CDF) — today *inferred* from the presence of a
`timeseries:` clock (`models.md` §Known Divergences: no dedicated `sources:` declaration yet); and (b) the
**source-lateness margin** (the declared term of the reach split, `model_properties.md` §Semantics "Unified
bound / reach derivation"; default 0). This phase either **lands** the first-class source-side declaration
home for each (the honest `derive-else-declare` fallback), or — where the surface belongs to a different
master — **records it deferred** with a plan link. In both cases it **confirms** `model_properties.md`
§"Catalogued inputs" references the real homes so the proof inputs (F9 input-delta discovery) are traceable.

**Spec anchor.** `models.md` §"Input-consumption axis (derived, not declared)" (the mutation profile is the
one non-derivable world-fact, declared *on the source*); §Known Divergences "Source mutation profile is
inferred, not yet a first-class source declaration"; §"The declaration law" (source-lateness + mutation
profile listed among the *assertions*). `model_properties.md` §"Catalogued inputs (owned elsewhere)".
`sources.md` / `timeseries.md` — the declaration homes. Maturity: `not-yet` (mutation profile is inferred).

**Pre-conditions.** Group A landed. F9 landed (input-delta discovery — the proof that *reads* the mutation
profile; its fail-closed default is the conservative whole-relation re-scan for an unknown profile).

**Depends on.** Group A (done); F9.

**TDD tests to write first.**
- `crates/smelt-core/src/…` unit — a `sources:` declaration carrying an explicit mutation profile
  (`append_only` / `mutable` / `cdf`) parses into the source metadata; a source-lateness margin parses (via
  the shared interval parser; default 0).
- `crates/smelt-logical/src/…` unit (**widening**) — F9 reads the declared profile: a source **declared**
  `append_only` (no clock) is admitted for the window-forward / delta path instead of the conservative whole-
  relation re-scan the *inferred* default gives.
- `crates/smelt-…/src/…` unit (**fail-closed reject / conservative default**) — a source with **no** declared
  profile and no clock still defaults to the conservative **snapshot-diff / whole re-scan** (never an
  optimistic window-forward that would silently drop rows); a *contradictory* declaration (e.g.
  `append_only` on a source with a delete path, if detectable) is refused/warned. The declaration widens only
  where smelt cannot derive the fact; it never overrides a safe conservative default into an unsafe one.
- `crates/smelt-cli/tests/example_diagnostics` — an example source carrying the declaration builds clean.

**Implementation shape.** If landing the surface: add the mutation-profile + source-lateness fields to the
source YAML shape (`smelt-core` source config), validated fail-loud; route F9 to read the declared profile
with the conservative default preserved when absent. If deferring: record the residue in §"Deferred" with a
link to the owning master/plan, and land **only** the catalogue-by-reference confirmation in
`model_properties.md` §"Catalogued inputs" + `models.md` §Known Divergences (narrow the "inferred, not yet
first-class" note to name the home).

**Critical files.**
- `crates/smelt-core/src/` (source/`sources.yml` config shape) — the mutation-profile + source-lateness
  fields (if landed).
- `crates/smelt-logical/src/…` — F9's read of the declared profile with the conservative default.

**Docs touched.**
- `sources.md` / `timeseries.md` §Surface — add the source mutation-profile + source-lateness declaration (if
  landed), timeless; else a §Known-Divergence pointer to the owning plan.
- `models.md` §Known Divergences — narrow "Source mutation profile is inferred, not yet a first-class source
  declaration" to reference the new home (or restate the deferral with the link).
- `model_properties.md` §"Catalogued inputs (owned elsewhere)" — confirm the mutation-profile + source-
  lateness references point at the real homes.
- `docs-site/` — a sources declaration note if the surface lands; else none.

**Review checklist.**
- [ ] The mutation-profile / source-lateness home is **landed** (source-side, fail-loud) or **deferred with a
      plan link** — decided explicitly, not left implicit.
- [ ] F9 reads the declared profile; an **absent** profile keeps the conservative whole-relation default
      (fail-closed — no optimistic window-forward).
- [ ] `model_properties.md` §"Catalogued inputs" references the real homes; `models.md` note narrowed.
- [ ] Edits timeless.

**Commit.** `feat(sources): first-class source mutation-profile + source-lateness declaration; catalogue-by-reference reconcile`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this sub-plan.)

- **DC1 wires only the join driving-fact resolution site** (`rules::incremental::restrict_ctx_for_join`, via `resolve_join_driving_fact`), not the UNION-branch (`restrict_ctx_for_union`/`trace_union_branches`) or derived-table (`restrict_ctx_for_derived_tables`) pushdown-scoping sites — those keep calling the un-declared `trace_event_time` (equivalent to `declared_monotonic: false`), unchanged from their pre-DC1 behaviour. The pure classifier itself (`trace_event_time_declared`, `NotTraceableKind`) is fully general and exhaustively tested independent of any consumption site; wiring the remaining two sites is a small, mechanical follow-up (thread one extra `bool` parameter) left for whichever L4 phase next touches UNION/derived-table incremental eligibility, since neither of those sites currently hard-rejects on `NotTraceable` the way the join site does (UNION/derived-table treat it as a conservative no-op / stay-at-outer-clamp), so there is no live user-facing gap today.

- The **transforms/modes** each declaration licenses (once-write enrichment F14 / `accumulating_snapshot`;
  bounded-domain multiset state Group C C4; horizon-bounded MERGE / widened-scan clamp F13/F15) are L2/L4 and
  are not built here — only the declaration + its widening/guard.
- The **derived maintained-window / horizon proof** (`model_properties.md`, `not-yet`, fed by F1) is a
  fundamentals concern; DC4 layers the ceiling declaration on top and defers to that proof for the derived
  value.
- If DC5 decides the source mutation-profile surface belongs to a different master, the residue is recorded
  here with a plan link (per DC5's implementation shape).

## Verification

How to confirm L3 is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Every declaration is widen-only with a fail-closed guard.** Each of DC1–DC5 has both a *widening* test
  (the declaration admits a construct the proof alone rejected as undecidable) **and** a *fail-closed reject*
  test (the same declaration applied to a positive disproof, or used to narrow eligibility, is refused with a
  diagnostic / kept at the conservative default) — `model_properties.md` §Constraints "Declared escape
  hatches may only widen".
- **DC4 is warning-only.** The horizon ceiling never shrinks the clamp; a below-derived ceiling produces a
  warning and an unchanged (derived) clamp window.
- **Fail-loud, never default.** DC3's absent cap and DC1/DC2/DC5's malformed declarations are configuration
  errors, not silent defaults (the fail-loud discipline, `CLAUDE.md`; every new `MetadataError`
  variant is exhaustively matched in `smelt-db/src/lib.rs`).
- `cargo test -p smelt-cli --test example_diagnostics` and `-p smelt-lsp --test example_workspaces` — example
  workspaces (including any carrying the new declarations) build with zero diagnostics.
- `/smelt:validate model_properties`, `/smelt:validate models`, and `/smelt:validate model_maintenance`
  report zero drift for the declaration surfaces this layer touches; every declaration named in the specs'
  §"Model-scoped declarations" / horizon / catalogued-inputs sections has a concrete frontmatter/source key,
  and the "declaration surfaces are tracked by …" / "inferred, not yet first-class" notes are narrowed as
  each lands.
