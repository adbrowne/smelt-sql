# Phase 1 — Reconstruct the 2026-08-15 baseline bullet inventory

**Outcome:** `docs/outcomes/20260815-incremental-spec-closure-confirm`
**Advances:** success criterion 1 (the closure report's enumeration must be complete and
reconstructed from git history, not from memory), and supplies the input phase 2 classifies.

## Objective

Produce a committed, regenerable inventory of every `§Known Divergences` bullet and every
`(Open Question)` tag that existed in the four anchor specs at the program baseline commit
`03a431f3` ("outcome(20260815-definition-delta-migrate): scaffold" — the first commit of the
2026-08-15 program). No classification, no dispositions, no spec edits: this phase only fixes the
denominator so phase 2 cannot quietly shrink it. Baseline counts to expect (already sampled):
`definition_deltas` 7 bullets / 1 OQ, `incremental_models` 25 / 6, `incremental_shapes` 32 / 13,
`model_properties` 16 / 7 — 80 bullets, 27 Open-Question tags.

## Spec delta

None. This phase changes no user-visible behaviour and edits no file under `docs/specs/`.

## Tests

No cargo tests (docs-only phase). The red-green oracle is the extraction script itself:

1. `extract-baseline.sh` run before the inventory exists → prints the raw bullet list; the
   inventory-vs-extraction count check fails (red).
2. Same check after `baseline-inventory.md` is written → per-spec row counts equal the script's
   per-spec counts, and every bold lead-in string in the script output appears verbatim in exactly
   one inventory row (green). Encode this as `check-inventory.sh` in the outcome directory so
   phases 2–5 can re-run it.

## Tasks

1. Add `docs/outcomes/20260815-incremental-spec-closure-confirm/extract-baseline.sh`: for each of
   the four anchor specs, `git show 03a431f3:docs/specs/<f>.md`, slice from `## Known Divergences`
   to end of section, and emit one TSV line per `- **…**` bullet as
   `spec<TAB>subsection(### heading, or "-")<TAB>bold lead-in<TAB>has_open_question(yes/no)<TAB>baseline line no`.
   Pin the baseline commit in a single variable at the top of the script.
2. Add `check-inventory.sh`: re-runs the extractor and asserts per-spec row counts and lead-in
   coverage against `baseline-inventory.md`; non-zero exit on mismatch. Confirm it fails now.
3. Run the extractor; record its raw output verbatim as
   `baseline-inventory.tsv` (the machine artifact phase 2 joins against).
4. Write `baseline-inventory.md`: header stating the baseline commit and the regeneration command,
   then one table per spec with columns `ID | Subsection | Bullet (bold lead-in) | Open Question? |
   Disposition`. Assign stable IDs `DD-01…`, `IM-01…`, `IS-01…`, `MP-01…` in baseline file order.
   Leave every `Disposition` cell as `TBD (phase 2)` — filling them is explicitly phase 2's job.
5. Sanity-check the totals against the sampled numbers above; if the extractor disagrees, trust the
   extractor and note the discrepancy (e.g. a bullet with no bold lead-in, or a nested list item)
   in a `## Extraction notes` section rather than hand-editing rows in.
6. Record in `## Extraction notes` any bullet the regex cannot represent as one row (multi-paragraph
   bullets, sub-bullets) so phase 2 does not silently drop it.
7. Run `check-inventory.sh` — must pass.
8. Write `phases/01-summary.md`: totals per spec, extraction caveats, and the flag that
   `20260815-keyed-grain-residue` is `**Status:** blocked` (its phase 3, "Transactional ledger fold
   on every shipped backend"), so phase 2 must treat every bullet that outcome claims to close as
   residue under criterion 6 rather than assuming closure.

## Verification

- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh` (exit 0)
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`outcome(20260815-incremental-spec-closure-confirm): reconstruct the 2026-08-15 baseline bullet inventory`
