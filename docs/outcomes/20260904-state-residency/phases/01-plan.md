# Phase 1 — Spec delta: single normative statement of ledger residency and availability resolution

**Outcome:** `docs/outcomes/20260904-state-residency/outcome.md`
**Status when planned:** pending → planned (2026-09-04)

## Objective

Land the spec-first half of this outcome so every later phase implements against one written
answer. `state.md` becomes the sole normative statement of correctness-state residency and
availability resolution; `run_state.md`, `incremental_models.md` and `incremental_shapes.md`
stop restating it (and stop describing `.smelt/reconciliation.json` as normative layout in
their bodies); `diagnostics.md` gains catalogue rows for both codes. Advances criteria 4
(codes catalogued) and 8 (divergences rewritten to residual gaps), and is the precondition for
phases 2–7. Docs-only phase: no crate changes.

## Spec delta

This phase *is* the spec delta. Five files, each edit stated as end-state-first (timeless-oracle
rule) with today's gap moved to §Known Divergences.

1. **`docs/specs/state.md`**
   - Frontmatter `last_reviewed: 2026-09-04`.
   - §Overview: one sentence establishing cross-reference discipline — other specs *cite*
     §"The residency rule", §"The degradation contract" and §Diagnostics; they never restate
     the residency class of a structure or the downgrade rule.
   - §Known Divergences: each of the five bullets that today says "no tracking plan yet" names
     `docs/outcomes/20260904-state-residency/outcome.md` as the tracking artifact.

2. **`docs/specs/run_state.md`**
   - §"Relationship to the reconciliation ledger" (line 133): the reconciliation-ledger clause
     states the end-state only — a per-model backend table, both gradings, transactional with
     the fold, cited to `state.md` §"The residency rule" — and drops the inline
     "today stored under `.smelt/reconciliation.json` … a divergence" prose.
   - §Layout tree (line 29), the locking sentence (line 48), the atomic-write sentence
     (line 52) and the **Fixed layout** invariant (line 166): drop `reconciliation.json` — it
     is not a `.smelt/` artifact.
   - The legacy-layout migration bullet (line 43) keeps the name, with a clause: a legacy
     root-level `reconciliation.json` is not migrated; the ledger is engine-resident.
   - §Known Divergences: new bullet — the shipped store still writes
     `.smelt/targets/<t>/reconciliation.json` (`crates/smelt-state/src/reconciliation.rs`);
     tracking `docs/outcomes/20260904-state-residency/outcome.md`.
   - §References: mark `src/reconciliation.rs` as the file the move deletes.

3. **`docs/specs/incremental_models.md`**
   - §Known Divergences: exactly one bullet on ledger residency, pointing at `state.md`
     §"The residency rule" and this outcome; the existing "ledger's warehouse substrate is
     DuckDB-only" bullet keeps its end-state sentence and adds the outcome link.
   - No body edit at lines 1389-1393 — it already defers correctly.

4. **`docs/specs/incremental_shapes.md`**
   - §"The transactional frontier write (merge ledger)": the end-state for a backend with no
     ledger realisation is availability resolution's recorded downgrade
     (`MaintenanceStateDowngraded`), not a skip — one sentence, cited to `state.md`.
   - §Known Divergences: today the idempotent-grade ledger record is skipped on a non-DuckDB
     backend and reported via `RunReporter::state_structure_unavailable`; tracking this
     outcome's criterion 3.
   - §References (line 1376): re-describe that reporter hook as the current stand-in for the
     recorded downgrade, not as the specified behaviour.

5. **`docs/specs/diagnostics.md`**
   - New `### State residency` catalogue section (after `### Contract lattice`), owned by
     `docs/specs/state.md`, with `MaintenanceStateDowngraded` (Warning) and
     `DeclaredContractRequiresState` (Error), triggers copied in substance from `state.md`
     §Diagnostics.
   - §Known divergences: new bullet — both rows are catalogue-ahead-of-variant (no
     `DiagnosticCode` variant yet; the coverage gate only asserts enum → catalogue), landing
     `docs/outcomes/20260904-state-residency/outcome.md` phases 4-5.

## Tests

No new Rust tests — this phase adds no behaviour. The existing gates below are the oracle;
the catalogue edit is the one that can actually fail a gate.

- `crates/smelt-db/tests/integration/diagnostics_catalogue.rs::every_diagnostic_code_is_catalogued`
  — must stay green with the two new catalogue rows (asserts enum → catalogue only, so
  catalogue-ahead-of-variant rows are admissible; confirm, don't assume).

## Tasks

1. Edit `state.md`: `last_reviewed`, the cross-reference-discipline sentence, outcome links on
   the five divergence bullets.
2. Edit `run_state.md`: body de-restatement at line 133, four layout mentions dropped, legacy
   bullet clause, new divergence bullet, §References note.
3. Edit `incremental_models.md`: consolidate/repoint the ledger-residency divergence bullets.
4. Edit `incremental_shapes.md`: end-state sentence, new divergence bullet, §References
   re-description.
5. Edit `diagnostics.md`: new `### State residency` section + divergence bullet.
6. `rg` sweep: no spec body outside `state.md` §Known Divergences asserts that the
   reconciliation ledger lives under `.smelt/`
   (`rg -n 'reconciliation\.json' docs/specs/` returns only run_state's legacy-migration
   clause and the two divergence bullets).
7. `rg -n 'Phase [A-Z0-9]' docs/specs/{state,run_state,incremental_models,incremental_shapes,diagnostics}.md`
   — timeless-oracle check on the edited files (pre-existing hits in §Known Divergences paired
   with a plan link are tolerated; add none).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test integration diagnostics_catalogue 2>&1 | tail -20`
- The two `rg` sweeps in tasks 6-7, output pasted into the summary.

## Commit message

`docs(state-residency): make state.md the sole normative statement of ledger residency and availability resolution`
