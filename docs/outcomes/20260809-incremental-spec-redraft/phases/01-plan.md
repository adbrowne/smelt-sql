# Phase 1 — Terminology + outline: frontier/ledger unification, ratified deletion list

## Objective

Fix the redraft's vocabulary and its blueprint before any large section is rewritten.
Land the single **frontier** definition in `incremental_models.md` and demote the two
existing ledgers to its named realizations (success criterion 1), and commit a
grep-anchored outline + deletion list that phases 2–7 execute against (criteria 2–4).
No section is rewritten in this phase beyond the ledger/frontier text itself.

## Spec delta

`docs/specs/incremental_models.md` — §Semantics:

1. **New `### The frontier`**, placed immediately before the current
   `### The reconciliation ledger` (line ~1197). Defines a frontier once: *the record of
   which typed deltas a cell has absorbed*, addressed by the delta type (watermark for
   append-only-window, key set / feed offset for keyed upsert, whole for general) and
   **graded by combiner algebra** (additive → delta identities, idempotent → watermark
   only). States the two operations (fold with a never-fold-twice precondition;
   recompute-reset) once, at this level.
2. **Retitle and thin** `### The reconciliation ledger` → `#### The frontier record
   (reconciliation ledger)` and `#### The transactional merge ledger` → `#### The
   transactional frontier write (merge ledger)`, each nested under the frontier concept
   in its existing location, each keeping only what is *specific to that realization*
   (region×group entry shape and schema-evolution op; per-model backend table,
   transaction co-writing, posture-driven refusal). The grading rules and fold/reset
   semantics move up to §"The frontier" and are cross-referenced, not repeated.
3. **Delete the cross-filing divergence entry** at §Known Divergences line ~2494 — the
   clause "…needs a per-cell maintained frontier the interval ledger does not track" is
   rewritten in frontier vocabulary (per-cell frontier addressing is unbuilt; the
   record is per-region) with its tracking link kept. No divergence entry may name
   one ledger as a foreign concept to the other.
4. Section-reference strings elsewhere (`§"The reconciliation ledger"`,
   `§"The transactional merge ledger"`) are updated to the new titles — mechanical.

Nothing in `model_properties.md` changes in this phase.

## Tests

Docs-phase; the oracle is grep + the standing extraction gates.

1. `frontier_defined_once` (shell check, recorded in the summary):
   `rg -c '^### The frontier' docs/specs/incremental_models.md` == 1, and every other
   `frontier` occurrence is either inside that section or a `§"The frontier"` reference
   or a realization-qualified use.
2. `no_ledger_cross_filing`: `rg -n 'ledger' docs/specs/incremental_models.md` shows no
   Known-Divergence entry describing one ledger in terms of the other's absence.
3. `spec_examples_extract`: existing spec-example extraction gate still green (the
   ledger sections carry no code fences, so this is a regression guard).
4. `timeless_grep`: `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` empty.

## Tasks

1. Read §"The reconciliation ledger" (1197–1209), §"The transactional merge ledger"
   (1731–1749) and every `ledger`/`frontier` reference site listed by
   `rg -n 'ledger|frontier' docs/specs/incremental_models.md`.
2. Write the new `### The frontier` section (target ≤ 25 lines) and thin both
   realization subsections to what is realization-specific.
3. Update all `§"The reconciliation ledger"` / `§"The transactional merge ledger"`
   reference strings to the new titles.
4. Rewrite the cross-filing Known-Divergence clause in frontier vocabulary; keep the
   tracking link.
5. Write `docs/outcomes/20260809-incremental-spec-redraft/phases/01-outline.md` (the
   blueprint phases 2–7 execute against), containing exactly three things:
   - **Target outline** — the redrafted section list for `incremental_models.md` with a
     one-line intent and a line budget per section; total budget ≤ 1,800 lines (from
     2,997). Records which existing sections merge, which are demoted to subsections,
     and which are deleted outright.
   - **Terminology table** — old term → redraft term → where defined once
     (frontier/ledger, typed delta, plan cell, verb, contract point, shape profile);
     one row per term, with the "defined once" anchor being a section title.
   - **Ratified deletion list** — every accretion from
     `docs/research/20260809-incremental-rethink.md` §5 turned into a row of
     (item, `file:line` anchors verified by `rg` in this phase, owning phase number,
     disposition: delete / rewrite / implement). Must cover at minimum:
     anti-exclusivity polemics, the dead `IncrementalStrategy` variants (`Append`,
     `InsertOverwrite`), `batched.*` config fossils, `nondeterministic_columns`,
     `grain: key_per_partition`, the "seven proofs" phrasing, and the settled-decision
     essays in both specs' §Known Divergences. An anchor that `rg` cannot confirm is
     recorded as "not present — no work", not silently dropped.
6. Re-verify each deletion-list anchor with `rg` before committing (the list is only
   useful if it is grep-true on the day it lands).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `rg -c '^### The frontier' docs/specs/incremental_models.md` → 1
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty
- `rg -n '§"The reconciliation ledger"|§"The transactional merge ledger"' docs/specs/` → empty
- Every `file:line` anchor in `01-outline.md` re-confirmed by `rg` (paste the confirming
  command's output count into `phases/01-summary.md`).

## Commit message

`docs(incremental-spec): define the frontier once and ratify the redraft outline`
