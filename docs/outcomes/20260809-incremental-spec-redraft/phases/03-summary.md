# Phase 3 summary — write addressing + maintenance mechanics redraft

## Shipped

- `docs/specs/incremental_models.md` §Semantics lines 752–1115 (364 lines, 7 headings) redrafted
  into 280 lines, 8 headings (one new umbrella, `### Maintenance mechanics`), all seven pre-existing
  heading strings preserved verbatim (`§"…"` citations across crates/specs/root `CLAUDE.md` still
  resolve — checked by `03-check.sh`'s `no_orphan_refs`).
- `phases/03-claims.md` — 94-claim inventory of lines 752–1115 (pre-edit), with the diagnostic-code
  list at the top.
- `phases/03-check.sh` — seven executable checks (structure, orphan-refs, claim fixture presence,
  diagnostic codes, ≤280-line budget, timeless grep, no-split-code-span), confirmed red on the
  unedited spec (structure/budget) before editing, all green after.
- Restatements of §"Per-cell admission"'s obligation list (repair-family admission) collapsed to a
  citation naming the three obligations by number instead of re-deriving them.

## Decisions

- Repair-family admission obligations 4/6/7 are now stated as a citation into §"Per-cell admission"
  rather than the old three-bullet restatement — that list already lived there (added by phase 2's
  antecedent content), so this is citation-consolidation, not new content.
- No rejected-alternative essay existed in the 752–1115 range (unlike phase 2's range), so no
  material moved to §Design this phase.
- Hitting the ≤280-line budget required two passes: an initial condensation pass (364→298 lines)
  followed by an adversarial-verify-driven second pass. The first condensation pass legitimately
  dropped 10 of 94 claims' load-bearing rationale/detail (never a diagnostic code, obligation
  number, or strength word — always a parenthetical explaining *why*, per the verifier). All 10
  were restored verbatim or near-verbatim and the budget re-closed by tightening line-wrap slack
  elsewhere (merging short trailing paragraph lines upward) rather than cutting content again.

## For the next planner

- Phase 4 (shape profiles + composed key+time table) and phase 5 (Overview/Design/Constraints/
  Limitations/References) are unaffected by this phase's line-range; no new gaps surfaced in their
  territory.
- `phases/01-outline.md`'s deletion-list "Owning phase" column still needs the phase-3-insertion
  remap noted in the outcome's 2026-08-11 decision-log entry (4/5/6 → current 5/6/7) — unchanged by
  this phase, flagging again since it's easy to miss when phase 5 starts.
- The `no_split_code_spans` check (phase-2-inherited hazard) caught three real issues during this
  phase's reflow (a formula backtick-span and two inline-code spans breaking across the 99–102 char
  wrap). Worth keeping this check in every subsequent redraft phase's `NN-check.sh`.
- Line-budget enforcement at this density (≈94 claims / 280 lines) left very little slack — future
  phases redrafting comparably dense ranges should budget claim-inventory time for at least one
  restore-after-adversarial-verify cycle, not treat the check-script pass as a single condensation
  pass.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/03-check.sh` → all seven PASS
  (structure, no_orphan_refs, claim_inventory, diagnostic_codes, budget 280/280, timeless,
  no_split_code_spans).
- Adversarial claim verification (independent subagent, all 94 claims): first pass 84/94 preserved,
  10/94 weakened (all rationale/detail drops, zero lost, zero strengthened, zero diagnostic-code or
  obligation-number changes); all 10 fixed and spot-checked present in the final text — 94/94.
- `bash .claude/scripts/verify-phase.sh` (with `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH` set
  to `~/.local/lib/duckdb`) → PASS (fmt, clippy zero-warnings, cargo test workspace,
  example_diagnostics).
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty (whole-file, not just range).
