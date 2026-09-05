## Drift Report: incremental_models §Surface "CLI" + §References User docs

**Spec**: docs/specs/incremental_models.md (last_reviewed: 2026-09-05)
**Date**: 2026-09-05
**Scope**: §Surface "CLI" (lines 417–541) and §References → **User docs** only, per
`docs/outcomes/20260904-delta-signature-front-door/phases/05-plan.md` criterion 5. This is
**not** a full-spec validate — the semantics/invariants legs over the rest of a 2,240-line
spec are out of scope; `docs/TODO.md`'s `/smelt:validate` baseline bullet still tracks that
larger sweep.

### Automated checks (this pass)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy over both CI feature sets,
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test docs_front_door` — 6/6 (5 existing + new
  `spec_user_docs_block_lists_existing_pages`).
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage` — all green.
- `cargo test -p smelt-logical --test backbuild_docs` — 7/7 green.

### Surface drift found and fixed
- ❌→✅ **`smelt rebuild` flag names were wrong in the spec.** Lines 485 and 519 stated
  `smelt rebuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]`. The
  actual CLI (`crates/smelt-cli/src/main.rs` `RebuildArgs`) takes a single positional
  `<selector>` plus `--start`/`--end` — matching `docs-site/docs/reference/cli.md` §"smelt
  rebuild", which was already correct. The prior `2026-09-04-definition_deltas-closure.md`
  drift report had asserted the wrong flag names as ✅ without checking against `main.rs`;
  this pass caught it by reading the actual `RebuildArgs` struct. Fixed both spec lines to
  `smelt rebuild <selector> --start <ISO-8601> --end <ISO-8601>`.
- ⚠️→✅ **Stale code comment, not a spec bug.** `BakeoffArgs::pin`'s doc comment in `main.rs`
  said "Not yet implemented (`docs/plans/20260719-prod-w7-bakeoff.md` Phase 5)", but
  `crates/smelt-cli/src/bakeoff.rs::build_pin_suggestion` shows `--pin` is fully implemented
  and matches the spec's description (lines 503–506) and `docs-site/docs/reference/cli.md`
  line 1430. Removed the stale comment; no spec change needed.
- ✅ `smelt explain` headline (Known Divergences bullet already narrowed in phase 1),
  `--json`'s `delta_signature` object (`crates/smelt-cli/src/explain.rs` — `delta_signature_headline`,
  `DeltaSignatureHeadline`), per-cell/per-edge/decomposed-state/repair-cell/`--show-sql`
  sections all match `crates/smelt-cli/src/explain.rs` and `docs-site/docs/reference/cli.md`.
- ✅ `smelt run --since-upstream --source/--landed`, `--auto`, `smelt build --period
  --include-upstreams`, `smelt migrate <model>`, `smelt bakeoff` (all flags) match
  `crates/smelt-cli/src/main.rs` and are documented consistently in
  `docs-site/docs/reference/cli.md`.
- ✅ Run-flags-by-shape table (lines 517–521, now corrected) matches the three derived-shape
  refusal rules in lines 523–541 and the code's flag requirements
  (`event_time_start`/`event_time_end` mutually `requires` each other on `RunArgs`).

### §References → User docs drift
- ✅ All five listed paths resolve: `index.md`,
  `guide/{incremental-models,sql-models,materializations}.md`, `concepts/how-it-works.md`,
  `reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`. Confirmed with a new
  standing gate (below) rather than a one-off `ls`.
- ✅ `reference/cli.md` does document `--since-upstream`, `--include-upstreams`, and
  `smelt explain`'s cell/clamp/ledger report with `--show-sql`, as claimed.
- ✅ `reference/smelt-yml.md` documents the `maintenance:` block, as claimed.
- N/A `guide/migrations.md` is **not** listed here, correctly — `smelt migrate` is owned by
  `definition_deltas.md` §Surface (line 490–491), which already lists
  `docs-site/docs/guide/migrations.md` in its own §References → User docs (line 534). Adding
  it here would violate the "one home per statement" rule (`docs/specs/CLAUDE.md`).

### New standing gate
- `crates/smelt-cli/tests/docs_front_door.rs::spec_user_docs_block_lists_existing_pages` —
  extracts every `docs-site/docs/...md` path (including `{a,b,c}` brace-expansion) from the
  spec's §References → User docs bullet and asserts each resolves to a real file. Confirmed
  red by temporarily pointing one path at the retired `guide/backbuild-synthesis.md`, then
  green after reverting.

### Not validated in this pass
- The rest of `incremental_models.md` (Overview, Semantics, the graph layer, decomposed
  state, contract lattice, diagnostics table beyond the CLI-adjacent codes, Known Divergences
  entries not touched by this outcome) — tracked by `docs/TODO.md`'s standing
  `/smelt:validate` baseline bullet.
- `incremental_shapes.md` and `definition_deltas.md` surfaces, beyond the one cross-reference
  check above.
- Live-backend re-verification of `smelt bakeoff`/`smelt rebuild` behavior — this pass is a
  static spec-vs-code-vs-docs cross-check, not a fresh functional test run beyond the standing
  gates listed above.
