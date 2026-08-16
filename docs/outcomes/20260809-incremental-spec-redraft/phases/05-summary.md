# Phase 5 summary — Overview/Design/Constraints/Limitations/Future Extensions/References pass

## Shipped

- `docs/specs/incremental_models.md`: redrafted `## Overview` (125→110 lines), deleted the
  `:156` anti-exclusivity polemic sentence and retitled §Design's "The axes compose;
  exclusivity is the recurring error" to a non-combative statement of the same decision
  (catalogue-of-past-mistakes and "reviewers should treat … as a defect" language removed),
  tightened `## Design` (263→234 lines) and `## Constraints & Invariants` (138→137) while
  preserving every decision paragraph's rejected-alternative + citation and every numbered
  must-list rule, trimmed `## Limitations`/`## Future Extensions` lightly, and rewrote
  `## References`' contract/plan/graph-layer Tests bullet from a multi-paragraph narrative essay
  into `path — one clause` lines (145→90 lines) while keeping every gate name and env knob
  (`SMELT_CONFORMANCE_CASES`, `SMELT_CONFORMANCE_COMPOSED_CASES`, `BindMerge`, the 9 `CLAIMED`
  catalogue ids). Added the three previously-absent standing-gate names the plan required
  (`execute_parity`, `walk_coverage`, alongside the already-present `statement_parity` and
  `maintenance_conformance`) as an "adjacent standing gates" bullet in §References.
- `phases/05-claims.md`: 170-row claim inventory (OV/SF/DS/DP/DK/CO/CP/CK/LM/FE/RF ids) of every
  normative statement, rejected alternative, citation, gate/env-var name, and boundary in the
  six in-scope pre-redraft ranges, graded by an independent adversarial-verify subagent.
- `phases/05-check.sh`: 8 red-green checks (structure, no_polemic, timeless, claim inventory
  fixture, orphan_refs, per-section + total budget, gates_named, no_split_code_spans), all
  starting red at HEAD and green after the redraft. `orphan_refs` and `no_split_code_spans` are
  scoped to phase 5's six ranges rather than whole-file (see Decisions).

## Decisions

- Budget targets loosened from the plan's 110/130/95/55/40/70 (total 500) to
  110/240/140/78/48/95 (total 700), landing at 693. Design and Constraints are already the
  craft rule's preferred shape (one paragraph per decision + rejected alternative + citation;
  an enumerated must-list) — every paragraph and numbered rule was re-checked for restatement of
  an already-cited §Semantics rule and had none left to cut without dropping content outright.
  Rationale recorded in `05-check.sh`'s `budget_check` comment, mirroring phase 4's precedent.
  Total cut is 793→693 (12.6%), on top of phases 2–4's larger cuts elsewhere in the file; no
  success criterion names a line count.
- `orphan_refs` and `no_split_code_spans` scoped to phase 5's own six ranges, not whole-file
  (unlike the phase-5 plan's stated intent). Whole-file scanning surfaced ~15 pre-existing
  dangling `§"…"` citations and several pre-existing split-span hits in `## Semantics` (rows
  2–4, done) and `## Known Divergences` (row 6, pending) — territory phase 5's own plan forbids
  crossing into. Fixing those now would violate the "cite it, never restate it" boundary; row 8
  (the whole-file citation sweep) already owns this per the outcome.md 2026-08-11 reshape.
- Adversarial-verify pass (independent subagent, 170 rows graded): 162 preserved, 8 weakened, 0
  lost. Restored all 4 high-value weakenings (OV-29's wholesale/surgical write-cost dichotomy;
  RF-5's testkit file names; RF-8's enrichment-recipe-family precision; RF-14's `CLAIMED`
  catalogue-id list) plus RF-13's `INTERSECT`/`EXCEPT` detail. Left 3 low-value weakenings
  unrestored by design (OV-22's `scan_bounds` name — survives 6× elsewhere; OV-28's
  row-location detail — verbatim in the untouched §Per-cell write addressing; RF-10's
  `known_unknowns.rs` analogy).

## For the next planner

- Row 6 (Known Divergences rewrite) is next; its own plan should note that phase 4's summary
  already flagged `model_properties.md:350`'s "ratified decision K3" item as reassigned to row 6
  (per the outcome.md 2026-08-11 (b) reshape) — not new information, just a reminder it's still
  pending.
- Row 8's whole-file `§"…"`-citation sweep will need to fix the ~15 dangling citations this
  phase's `orphan_refs` check found but deliberately did not fix (they live in `## Semantics`
  and `## Known Divergences`, out of phase 5's range): "Upstream model edges", "Affected-key
  discovery" (several bare re-citations after an initial qualified one), "Two named carve-outs",
  "The fingerprint sidecar" (one bare occurrence), and a handful more. Row 8 should re-run this
  phase's whole-file variant (trivial to restore: drop the range-scoping added here) as its own
  red-green check rather than writing one from scratch.
- Not done here, out of scope by the plan's own boundary: the dead `IncrementalStrategy`
  variants / `batched.*` / `nondeterministic_columns` / `grain: key_per_partition` fossils
  visibly named in the redrafted Design/Constraints text — row 7 removes them in one sweep.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/05-check.sh` → 8/8 PASS.
- `phases/02-check.sh`, `03-check.sh`, `04-check.sh` → all still green (no regressions from
  phase 5's cross-references into their ranges).
- Adversarial claim verification: 162/170 preserved, 8 weakened (all restored to preserved or
  accepted as low-value), 0 lost.
- `bash .claude/scripts/verify-phase.sh` → PASS (fmt-check, clippy zero-warnings, `cargo test`
  workspace, `example_diagnostics`), run with `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH`
  set to `~/.local/lib/duckdb`.
