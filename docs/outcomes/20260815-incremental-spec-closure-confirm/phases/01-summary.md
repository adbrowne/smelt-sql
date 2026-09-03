# Phase 1 summary — baseline bullet inventory

## Shipped

- `extract-baseline.sh` — extracts every `§Known Divergences / Open Questions` bullet from the
  four anchor specs at baseline commit `03a431f3`, emitting `spec, subsection, bold lead-in,
  has_open_question, baseline line no` as TSV.
- `check-inventory.sh` — re-runs the extractor and asserts `baseline-inventory.md` still matches
  it (per-spec row counts + lead-in coverage); exits non-zero on drift. Verified red before the
  inventory existed, green after.
- `baseline-inventory.tsv` — the raw extractor output, committed as the machine artifact phase 2
  joins against.
- `baseline-inventory.md` — one table per spec (`DD-`/`IM-`/`IS-`/`MP-` prefixed IDs, in baseline
  file order), every `Disposition` cell `TBD (phase 2)`, plus a `## Extraction notes` section.

## Totals

80 bullets total: `definition_deltas` 7, `incremental_models` 25, `incremental_shapes` 32,
`model_properties` 16 — all four match the plan's sampled per-spec bullet counts exactly.

Open-Question counts diverge from the plan's sample by a few, and the divergence is itself the
interesting finding: `definition_deltas` 1/7, `incremental_models` **7**/25 (sample said 6),
`incremental_shapes` **16**/32 (sample said 13), `model_properties` 6/16 (sample said 7, one
*over*-count — the sampled total of 27 happened to net out close to the true 30 by one under- and
one over-count cancelling elsewhere). Root cause: 5 bullets across `incremental_models` and
`incremental_shapes` wrap `(Open` / `Question)` across a markdown line break (`IM-18`, `IM-22`,
`IS-14`, `IS-20`, `IS-27`); a naive single-line grep — which is almost certainly how the plan's
sample was produced — misses these. `extract-baseline.sh` collapses whitespace across the whole
accumulated bullet body before testing, so it catches them. Per the plan's own instruction ("if
the extractor disagrees, trust the extractor"), the corrected true counts are documented in
`baseline-inventory.md` §Extraction notes rather than silently reconciled.

## Extraction caveats

- No bullet needed dropping to a single row: every `- **…**` top-level markdown bullet across all
  four specs is representable as exactly one table row. Compound-prose bullets with internal
  semicolon-separated lists (`IS-21` "Locality machinery gaps", `MP-01` "Several declared
  proofs…") are still single bullets, not nested sub-bullets.
- `definition_deltas` `DD-06` phrases its open-question-ness in prose ("Open question — plan-hash
  scope.") rather than a trailing `(Open Question)` tag; the free-text scan still flags it.
- `incremental_shapes` is the only anchor spec with `### ` subsections inside its Known
  Divergences section ("The partition grain", "The key grain"); the other three specs have none,
  recorded as subsection `-`.

## For the next planner

- `20260815-keyed-grain-residue` is `**Status:** blocked` on its phase 3 ("Transactional ledger
  fold on every shipped backend"; all its other rows are `done`). `incremental_shapes` `IS-24`
  ("The reconciliation ledger's fold is transactional on DuckDB only (Open Question)", L1137) is
  the bullet that outcome was closing. Phase 2 must **not** mark `IS-24` closed on that outcome's
  say-so — treat it as residue under success criterion 6 (report it explicitly; don't paper over
  it) unless the repo state independently shows the fold is now transactional on every backend.
- The corrected Open-Question counts (7 / 16, not 6 / 13) are a small, self-contained finding
  worth a one-line mention in the final closure report even though this phase's job was only to
  fix the denominator, not classify it.

## Gates

- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh` — OK (80
  bullets).
- `bash .claude/scripts/verify-phase.sh` — run below; docs-only change, no Rust source touched.
