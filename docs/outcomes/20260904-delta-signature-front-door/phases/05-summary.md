# Phase 5 summary — validate + close out

## Shipped

- `docs/TODO.md` §"docs-site sync" bullet deleted (lines 35–39); the adjacent
  `/smelt:validate` baseline and `smelt migrate`/`rebuild` wiring bullets stay, as agreed
  out-of-scope work.
- **Real spec bug found and fixed**: `docs/specs/incremental_models.md` lines 485 and 519
  claimed `smelt rebuild --event-time-start/--event-time-end [selectors]`; the actual CLI
  (`RebuildArgs` in `crates/smelt-cli/src/main.rs`) takes `<selector> --start --end`, matching
  `docs-site/docs/reference/cli.md` which was already correct. Fixed both spec lines.
- Removed a stale doc comment on `BakeoffArgs::pin` in `main.rs` claiming "Not yet
  implemented" — `crates/smelt-cli/src/bakeoff.rs::build_pin_suggestion` shows it's fully
  shipped and matches the spec/docs-site.
- New standing gate `crates/smelt-cli/tests/docs_front_door.rs::spec_user_docs_block_lists_existing_pages`
  — parses every `docs-site/docs/...md` path (with `{a,b,c}` brace expansion) out of the
  spec's §References → User docs bullet and asserts each resolves. Confirmed red by pointing
  a path at the retired `guide/backbuild-synthesis.md`, then green.
- `last_reviewed:` bumped 2026-09-03 → 2026-09-05.
- `docs/validations/2026-09-05-incremental_models-cli-surface.md` — the committed evidence
  artifact, scope statement, per-item table, and "not validated" list.

## Decisions

- Validation scoped to §Surface "CLI" (lines 417–541) + §References User docs only, per the
  plan — not a full-spec sweep. Recorded in outcome.md decision log (phase 5, plan step).
- `guide/migrations.md` correctly stays **absent** from `incremental_models.md`'s References
  block — it's already owned by `definition_deltas.md` §References (line 534), consistent
  with the spec-craft "one home per statement" rule.

## For the next planner

- **The prior validation pass missed a real bug.** `docs/validations/2026-09-04-definition_deltas-closure.md`
  asserted the wrong `smelt rebuild` flags as ✅ without checking `main.rs` directly — it
  trusted the spec's own text. Worth a note that CLI-surface validation must always check the
  `clap` struct, not just cross-reference spec ↔ docs-site (which can agree with each other
  while both being silently right and the spec still being tested against nothing but itself
  — in this case docs-site was actually correct and the spec was wrong).
- Not validated (unchanged from the plan's stated scope): the rest of `incremental_models.md`
  (Overview/Semantics/graph layer/decomposed state/contract lattice/full diagnostics table),
  `incremental_shapes.md` and `definition_deltas.md` surfaces beyond one cross-check, and any
  live-backend functional re-verification of `smelt bakeoff`/`smelt rebuild`. All tracked by
  `docs/TODO.md`'s standing `/smelt:validate` baseline bullet, which stays.
- No further follow-up work surfaced that serves this outcome's success criteria — all five
  are met (see validation report). Recommend closing this outcome as done and advancing the
  backlog.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, cargo
  test, example_diagnostics).
- `cargo test -p smelt-cli --test docs_front_door` — 6/6 (new test included).
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage` — 3/3, 3/3, 1/1.
- `cargo test -p smelt-logical --test backbuild_docs` — 7/7.
- `rg -n 'docs-site sync' docs/TODO.md` — empty.
- `rg -in 'four.corners' docs-site/docs/` — empty.
