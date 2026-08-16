# Phase 1 summary — Terminology + outline

**Shipped:**
- `docs/specs/incremental_models.md` §Semantics: new `### The frontier` section (landed
  immediately before the reconciliation-ledger material) defines the frontier once — addressing
  by output-delta type, grading by combiner algebra (additive → delta identities, idempotent →
  watermark), and the fold / recompute-reset operations — and names its two realizations.
- `### The reconciliation ledger` retitled and thinned to `#### The frontier record
  (reconciliation ledger)`, nested under the frontier concept, keeping only the region×group
  entry shape and the schema-evolution fold.
- `#### The transactional merge ledger` retitled and thinned to `#### The transactional frontier
  write (merge ledger)`, keeping only the per-model backend table, transactional co-write, and
  posture-driven refusal.
- Every `§"The reconciliation ledger"` / `§"The transactional merge ledger"` cross-reference in
  `incremental_models.md`, `run_state.md`, `sources.md`, and `model_transforms.md` updated to the
  new titles (including two references that wrapped across a line break and survived the first
  sed pass, fixed by hand).
- The cross-filing Known-Divergences clause (per-cell `deferral` scheduling entry) rewritten in
  frontier vocabulary: "per-cell frontier addressing... the frontier record does not yet track
  (its addressing today is per-region, not per-cell)" replaces the old "a per-cell maintained
  frontier the interval ledger does not track" phrasing that named the frontier record as a
  foreign concept.
- `docs/outcomes/20260809-incremental-spec-redraft/phases/01-outline.md`: target section outline
  (budgeted ≤ 1,800 lines, down from 3,017), a 6-row terminology table, and a ratified deletion
  list (11 rows, each anchor re-confirmed by `rg` this phase) that phases 2–7 execute against.
- One incidental fix: a stray `Phase 6` plan-vocabulary citation at (then) line 2556 — unrelated
  to frontier work, but it broke this phase's own `timeless_grep` gate — trimmed to cite the plan
  file without the phase number.

**Decisions:**
- Kept the frontier's two operations (fold, recompute-reset) defined once at the frontier level
  and referenced, not repeated, by each realization subsection — this is what makes "no
  divergence entry may name one ledger as foreign to the other" (task 3) checkable by grep rather
  than by re-reading prose.
- The outline's per-section line budgets are planning targets for phases 2–7, not literal caps
  enforced by this phase's own tests; only the total (≤ 1,800) and the top-level table structure
  are load-bearing for later phases.
- `grain: key_per_partition`'s deletion-list disposition is "delete from the declared surface,"
  not "implement" — implementing its execution path is new behaviour, which this outcome's §Out
  of scope reserves for a separate queued outcome; only removal is sanctioned here.

**For the next planner:**
- Phase 2 can start directly from the terminology table and the Semantics subsection budget —
  both are already anchored to current section names.
- The deletion list's `batched.*` `smelt.yml`-override row (owning phase 6) still needs a named
  top-level replacement decided for the MERGE-dedup-only `unique_key` it carries before the
  removal itself can land — flagged in the outline, not resolved here (out of phase-1 scope).
- Two spec-CLAUDE.md craft violations survive outside this phase's touched text and are already
  captured in the deletion list for phases 4–5: the `ratified decision K3` internal label
  (`model_properties.md:350`) and the two anti-exclusivity polemic sentences
  (`incremental_models.md:156`, `:2053`).
- Did not touch `model_properties.md` (explicitly out of scope this phase) or any docs-site page
  (phase 7's job) — the terminology table's "typed delta" row is the one place phase 2 needs to
  actually introduce new phrasing rather than just re-anchor existing terms.

**Gates:**
- `rg -c '^### The frontier' docs/specs/incremental_models.md` → `1`
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty
- `rg -n '§"The reconciliation ledger"|§"The transactional merge ledger"' docs/specs/` → empty
- All 11 deletion-list anchors re-confirmed by `rg` this phase (counts recorded inline in
  `01-outline.md`'s closing "Not present" note; every row had at least one match).
- `bash .claude/scripts/verify-phase.sh` (full, not `--fast`) → `VERIFY: ALL GREEN` (fmt-check,
  clippy zero-warnings, full `cargo test` workspace, `example_diagnostics`).
- `cargo test --quiet -p smelt-cli --test example_diagnostics` → 119 passed, 1 ignored.
