# Phase 8 summary — Validate + close out

**Shipped:**
- `docs/specs/incremental_shapes.md` §References → "The key grain" — added the five code
  entries phases 1/2/4/7 introduced that were absent (`maintenance/derive.rs`'s
  `KeyedRetractableContribution` classifier seam, `Backend::execute_write_with_bookkeeping` +
  its DuckDB override, `rules::cumulative::execution_postures`,
  `RunReporter::state_structure_unavailable`) and the three test entries
  (`keyed_frontier_bookkeeping.rs`, `execution_postures.rs`, `arb_once_write_null_schedule`).
  Every added path verified to exist and contain the named symbol before writing it in.
- `last_reviewed` bumped `2026-09-03` → `2026-09-04`.

**Decisions:**
- No code changes this phase — validation surfaced no drift attributable to phases 1/2/4/5/7.
  Each of those phases' own summaries record that they already applied their spec delta as they
  landed (Known Divergences bullets deleted/updated in the same commit), so by the time this
  phase ran, §Known Divergences / Key grain already reflected the landed state correctly (verified
  by reading the current spec text directly, not trusting the phase summaries' own say-so).

**Per-criterion verdict:**
1. `KeyedRetractableContribution` classifier/diagnostic/fixture/test — met (phase 1).
2. Re-run-tolerant window-forward frontier record — met (phase 2).
3. Transactional ledger fold on every shipped backend — **unmet by design**. Phase 3 is blocked
   on a recorded human decision (outcome.md §Blocked, 2026-09-03 entry): a prior decision record
   (`docs/research/20260816-open-questions-triage.md` item 12) already answered "transactional
   Spark ledger" as a deferred Future Extension, which conflicts with criterion 3's framing that
   this is a pure conformance gap. Three candidate options are recorded there; none chosen yet.
4. Derived execution postures computed and printed by `smelt explain` — met (phase 4).
5. Generative conformance pool nullable payload, once-write NULL direction covered — met (phase 5).
6. `/smelt:validate incremental_shapes` reports no drift for the bullets this outcome closed;
   standing gates green — met by this phase (see Gates below and drift summary).

**Drift report summary (category b/c only — category a was empty, nothing to fix):**
- Surface/Semantics/Constraints/Invariants: spot-checked every diagnostic code this outcome's
  phases touch (`KeyedRetractableContribution` — wired through `smelt-db`, `smelt-lsp`), all
  §References code/test paths (partition grain + key grain) resolve to real files/symbols,
  §Known Divergences already reflects each phase's landed state (no stale "not yet
  implemented" wording survives for closed items).
- Timeless-oracle check: `grep -nE 'Phase [A-Z0-9]' docs/specs/incremental_shapes.md
  docs-site/docs/reference/state.md` — empty, no leakage.
- Category (c) — criterion 3 / phase 3 residue: the DuckDB-only ledger transactionality gap
  stays recorded in §Known Divergences exactly as phase 2/7 left it; not touched here, per the
  Blocked entry.
- No category (b) drift found owned by other outcomes.

**For the next planner:**
- Criterion 3 needs a human decision before it can be scheduled as a follow-up phase (in this
  outcome or a new one) — see outcome.md §Blocked's three candidate options.
- Nothing else outstanding; this closes the outcome's remaining `planned` row.

**Gates (all run this phase, foreground, output read directly):**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test
  keyed_frontier_bookkeeping --test projection_dialect_invariance --test dialect_seam` — 55
  passed (11+4+3+4+33).
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
- `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` — clean.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
