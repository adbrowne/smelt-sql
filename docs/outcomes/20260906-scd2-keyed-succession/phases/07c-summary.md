# Phase 7c summary — Re-run tolerance under deletes

## Shipped

- `emit_succession_clock_tie_probe`'s tie signature
  (`crates/smelt-logical/src/maintenance/emit/succession.rs`) now compares a
  delete row's flag alone (`CASE WHEN __smelt_is_delete THEN 'D' ELSE 'I|' ||
  (<payload sig>) END`) instead of concatenating flag + payload for every row.
  Doc comment quotes the spec sentence anchoring the rule.
- 4 new DuckDB-proven unit tests in the same file: tombstoned-delete replay is
  silent (the red test), delete-vs-insert still fires, two non-identical
  inserts still fire, two identical deletes at one `(k, t)` stay silent.
- Restored phase 7b's weakened leg 6 as
  `repeated_window_application_with_deletes_is_idempotent`
  (`crates/smelt-cli/tests/maintenance_conformance/succession.rs`) — a
  delete-flagged recipe refolded twice, presented + tombstone tables
  byte-identical.
- New `refold_after_a_full_refresh_ledger_rebuild_is_clean` answering 7b's
  uninvestigated question: stage a delete, drive two windows, `--full-refresh`
  rebuild, re-drive the last window — no `SuccessionClockTie`, oracle still
  matches. That path was already sound; this only adds coverage.

## Decisions

- Scoped the fix to the signature expression only, per the plan's defect
  note — `build_domain_cte`'s NULL projection and `emit_succession_patch`'s
  payload asymmetry are correct as-is and untouched.
- Left `sig_expr`/`content_sig` as local `format!` variables rather than
  extracting a shared helper — one call site, no reuse pressure.

## For the next planner

- No new follow-ups surfaced. The rebuild-then-refold leg (test 6) came back
  green on the first try, confirming the `--full-refresh`/`repair` ledger path
  never had the sibling bug 7b worried about — nothing further to schedule
  there.
- `verify-phase.sh` and `large-file-check.sh` both fully green this phase —
  no shrink-step debt carried forward, unlike phases 2b/3/3a/5a/5b.

## Gates

- `cargo test -p smelt-logical --quiet` — pass (68 succession tests, full
  crate suite including `walk_coverage`)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — pass
  (95 tests, full seeded sample)
- `cargo test -p smelt-runtime --test statement_parity --quiet` — pass (40
  tests)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `bash .claude/scripts/large-file-check.sh` — OK, no regression
