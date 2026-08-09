# Phase 1 — Spec: decomposed-state semantics

**Outcome:** `docs/outcomes/20260809-rung2-state-shapes/outcome.md`
**Kind:** spec-only (no production code changes)

## Objective

Fix the normative semantics of rung-2 decomposed state for the key-addressed profile:
where state columns physically live, how the presentation projection hides them, and
which admissions widen as a result. This is the spec-first gate for success criteria
1–4 — every later phase implements against the text this phase writes.

## Spec delta

All edits in `docs/specs/incremental_models.md` unless named otherwise.

1. **New section §"Decomposed state (rung 2) in keyed models"**, placed immediately after
   §"The algebraic maintenance ladder". It must fix, with rationale:
   - **Physical layout decision.** State columns live in the *same* stored table as the
     presented columns, suffix-named `<output>__<part>` (`__sum`, `__count`, `__ord`,
     `__written`, `__n`/`__sx`/`__sxx`). The presented column is materialised alongside
     them at merge time from the new state. Rejected alternative: a separate
     `<model>__state` table plus a presentation *view* — it would make `ref()` resolve
     to a view, add a second relation to every backend's DDL/atomic-swap path, and buy
     nothing, since `π` is a per-row pure function of the same row's state.
   - **Presentation projection.** State columns are excluded from the model's public
     schema: `smelt.ref()` expansion, `SELECT *`, declared-schema checks and downstream
     type inference see only presented columns. Naming collision with a user column of
     the same name is a fail-loud refusal, not a silent rename.
   - **State-shape catalogue** — the concrete shapes this rung licenses:
     `AVG(x)` → `(sum, count)`, `π = sum/count` (NULL when `count = 0`);
     `STDDEV_*`/`VAR_*` → `(n, Σx, Σx²)` with the presented `π` per family;
     `MAX_BY(v, o)`/`MIN_BY(v, o)` → `(v, o)` where `o` is the hidden ordering state;
     once-write → `(value, written_flag)`.
   - **Fold rules per shape** — the `(state, delta)` combiner for each, and the statement
     that each is a commutative monoid, so the equivalence invariant and
     reorder-independence hold unchanged.
2. **§"The column-family catalogue"** — replace the order-monotone overwrite row's "Extra
   licence" (hand-written companion `MAX(<ordering>)` projection) with hidden ordering
   state; keep `MAX_BY(x, x)` as its own degenerate case. Add a **decomposed fold** family
   row (`AVG`, `STDDEV_*`, `VAR_*`) admitted window-forward, idempotent = no,
   order-independent = yes, invertible = per underlying combiner, licence = ledger-graded
   as additive.
3. **§ once-write spellings** — widen to admit `COALESCE(MAX(col), <fallback>)` and
   `COALESCE(MAX(a), MAX(b))`, each backed by hidden state: the fallback form stores the
   raw nullable reduction and applies the fallback in `π`; the multi-candidate form stores
   one state slot per candidate and applies the preference order in `π`. State the
   NULL-preservation obligation is now discharged by the state, not by the spelling ban,
   and say what still refuses (a non-key-derived candidate without its functional
   dependency).
4. **Diagnostics** — `KeyedUnknownCombiner` / `KeyedOnceWriteUnproven` prose in
   §Diagnostics loses the companion-projection and fallback fixes; add a
   `KeyedStateColumnCollision` entry for the naming collision above. Mirror the new code
   into `docs/specs/diagnostics.md`.
5. **§Known Divergences** — the three rung-2 entries (once-write narrow spellings, the
   ordering value with no decomposed-state storage, "ladder rungs 2–4 specified ahead of
   use") are **rewritten**, not deleted: each keeps only the residual gap and points at
   this outcome as the tracking artifact. Deletion happens in phase 7 once the code lands.
6. **`docs/specs/model_properties.md`** — the decomposable discriminant section gains a
   pointer that concrete state shapes are catalogued in `incremental_models.md`; the
   once-write provenance section drops the NULL-preservation-by-spelling framing.
7. **`docs/specs/model_transforms.md`** §F12 — the "built as a mechanism / only `AVG`"
   line updates to name the widened shape catalogue as the keyed profile's consumer.

No `Phase N` vocabulary in any spec body (timeless-oracle rule).

## Tests

Spec-only phase: no new Rust tests. The checks are mechanical and run in Verification.

- `spec_phase_vocabulary` (manual `rg` check, below) — no `Phase [A-Z0-9]` in edited spec bodies.
- Existing suites must stay green unchanged; any test that fails here means a code change
  leaked into a spec phase.

## Tasks

1. Read §"The algebraic maintenance ladder", §"The column-family catalogue", the once-write
   spelling rules, §Diagnostics, and the three rung-2 §Known Divergences entries.
2. Write the new §"Decomposed state (rung 2) in keyed models" section (delta item 1).
3. Apply the catalogue-table edits (item 2) and the once-write widening (item 3).
4. Apply the diagnostics edits in both spec files (item 4).
5. Rewrite the three Known Divergences entries (item 5).
6. Apply the `model_properties.md` and `model_transforms.md` cross-reference edits (items 6–7).
7. Re-read the whole diff for timeless-oracle compliance and for internal contradiction with
   §"The equivalence invariant" and §"Per-cell admission".
8. Write `phases/01-summary.md`: the decisions fixed (physical layout, naming, per-shape
   folds), anything the spec pass discovered that reshapes phases 2–7, and the exact spec
   anchors phase 2 implements against.

## Verification

- `bash .claude/scripts/verify-phase.sh` (spec-only, but must stay green).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/model_properties.md docs/specs/model_transforms.md docs/specs/diagnostics.md`
  — hits only permitted in Known Divergences / References lines paired with a plan link.
- `rg -n "companion" docs/specs/incremental_models.md` — surviving mentions must be in
  Known Divergences, describing the residual gap only.
- Every diagnostic code named in the spec edits resolves in `docs/specs/diagnostics.md`.

## Commit message

`spec(incremental): rung-2 decomposed state — state shapes, presentation projection, widened keyed admissions`
