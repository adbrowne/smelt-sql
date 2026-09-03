# Phase 3 summary — resolve drifted and out-of-scope bullets

## Shipped

- `baseline-inventory.md` — added a `Verdict` column (all four tables); the 16 `drifted` rows
  carry `accurate` (11) or `relocated` (5), all other rows `-`.
- `check-classification.sh` — extended to require every `drifted` row to carry a Verdict starting
  `accurate`/`relocated`/`stale-fixed`, and every non-`drifted` row to carry exactly `-`. Confirmed
  red on all 16 rows before filling the column, green after.
- `baseline-inventory.md` §"Out-of-scope spot-check" — new section, one row per item named in
  `docs/outcomes/20260815-definition-delta-migrate` §"Out of scope": spec/section it maps to,
  presence, tagging accuracy, missing-behaviour confirmation, verdict.
- `current-inventory.tsv` regenerated at `HEAD` (byte-identical to phase 2's — no drift).

## Decisions

- All 16 `drifted` rows verified `accurate`/`relocated`, none `stale-fixed`: no spec text needed
  editing. This phase therefore produced **no spec diff** — the spec-fix branch of the plan never
  triggered.
- The `IS-24`/`IS-18` mislabeling `rg` sweep (task 7) came back clean: neither
  `20260815-keyed-grain-residue` nor `20260815-partition-grain-residue` cites either ID.
- Found one staleness, but in the **outcome's own historical prose**, not spec text:
  `20260815-definition-delta-migrate` §"Out of scope" says `docs/plans/20260704-model-updates.md`'s
  D1–D3 rows' fate is "unclear ... individually" — true for D1/D2, but D3
  (`refresh: materialized_view`) has since shipped as fully-specified surface
  (`docs/specs/materialized_view.md`). No spec bullet claims D3 is undecided, so nothing to fix
  under this phase's spec-only-if-trivial rule, and no product decision is needed (D3 is already
  decided and shipped) — so per the standing rule this is neither a new phase row nor a `##
  Blocked` entry, just a closure-report footnote. Recorded in the spot-check table, not edited (the
  owning outcome is `done`; editing another outcome's historical record is out of convention, same
  as the plans-are-historical rule).

## For the next planner

- Phase 4 (`/smelt:validate` × 4) has a real spec diff of zero to validate against from this
  phase — expect clean runs unless drift predates this phase.
- Phase 5's closure report should carry: the 35/29/16/0 classification totals (phase 2), the
  11-accurate/5-relocated drifted verdict split (this phase), the `IS-18` vs `IS-24` mislabel
  correction (phase 2, re-confirmed clean here), and the D3-shipped footnote above as a known
  staleness in `20260815-definition-delta-migrate`'s own out-of-scope text (not a spec drift, not
  actionable here, but worth naming so the closure report doesn't imply zero staleness anywhere).
- No new phase row and no `## Blocked` entry were needed — every drifted/out-of-scope item
  resolved to "accurate as documented."

## Gates

- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh` — OK (80
  bullets, unchanged).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh` — red
  (all 16 drifted rows) before the Verdict column was filled, OK (80/80 valid) after.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full test
  suite, example_diagnostics). Docs-only change; no Rust source touched, no spec text touched.
