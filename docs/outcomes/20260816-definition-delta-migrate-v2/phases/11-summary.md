# Phase 11 summary — validate + close out

**Shipped:**
- `crates/smelt-logical/tests/backbuild_docs.rs::spec_references_are_live_paths` — a standing gate
  that every backtick, slash-containing path in `definition_deltas.md` §References → Code / Tests
  exists on disk (brace-expansion aware, e.g. `{mod,diff,classify,emit,requalify,plan}.rs`), so the
  list can't rot silently again.
- `docs/specs/definition_deltas.md` §References refreshed: Code gained `migrate.rs`,
  `schema_evolution.rs` (`resolve_definition_change_route`), `migration_approvals.rs`,
  `commands/migrate.rs`/`commands/rebuild.rs`, `workspace_ingest.rs` (`read_deployed_columns`);
  Tests gained `backbuild_docs.rs`, the migrate/exit-code/rebuild-rename/maintenance-conformance
  CLI suites; User docs now names the rewritten guide and the new `## smelt migrate` CLI
  reference (was "none yet"); Plans (history) gained this outcome's own `outcome.md`.
- §Known Divergences re-verified against the code, all four bullets kept (all still live) but
  reworded: bullet 1 rewritten to state its actual mechanism-level shape (only column additions
  get a dedicated live trigger; a redefinition/removal has no narrower handling — same underlying
  gap as the pending-delta-run-refusal bullet, not a distinct one) and repointed at "Out of scope"
  instead of a bare "Tracked: outcome.md"; bullet 3 (destructive legs refused) reworded to state
  plainly it's a deliberate phase-3 narrowing with no current tracker for the probe emitters, not
  an implied future commitment of this outcome. Bullets 2 and 4 were already accurate and already
  cited "Out of scope" correctly — untouched apart from surrounding renumbering.
- `last_reviewed` bumped to 2026-08-17.

**Decisions:**
- Investigated whether bullet 1's old wording ("falls to a full recompute") was still true: it
  wasn't — `diff_schemas` (name/type/nullability only) never even notices a same-type
  redefinition, so nothing recomputes anything; the change is silently folded under the new SQL.
  That's exactly the pending-delta-run-refusal gap, so the two bullets were reconciled to point at
  each other rather than describing the same hole twice with different (and one inaccurate) claims.
- Checked whether the schema-evolution `AtomicGroup` route re-recording `definition_sql` on an
  *ordinary* run (not `smelt migrate --apply`) contradicts §Detection's "never overwrites the
  recorded definition" claim. It doesn't: §"Boundary with `schema_evolution.md`" already
  establishes that route *is* the definition-delta migration path for additive changes, so a
  successful ALTER-and-backfill genuinely resolves the delta — recording the new definition then
  is correct, not a violation. No spec edit needed there.
- `spec_references_are_live_paths` only checks backtick spans containing `/`, so a bare
  parenthetical like `(resolve_definition_change_route)` outside backticks is correctly ignored —
  designed this way so §References prose can name a function without the gate mistaking it for a
  file path.

**For the next planner:**
- Destructive-leg verification probes (row-count/fingerprint checks) still have no tracker beyond
  this outcome's own decision log — worth a follow-up plan if `--apply` on a column-drop/table-swap
  leg becomes a real need.
- Per-cell frontier addressing for migration resume (bullet 2) and the pending-delta run refusal
  (bullet 4) remain genuinely open; both already have accurate homes in the programme's other
  outcomes / "Out of scope" per the existing text — nothing new to schedule from this phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 83 passed
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 23 passed
- `cargo test -p smelt-runtime --test execute_parity --quiet` — 4 passed
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 4 passed
- `cargo test -p smelt-logical --test backbuild_docs --quiet` — 5 passed (includes the new gate)
- `cargo test -p smelt-cli --test rebuild_dry_run --quiet` — 5 passed
- `cd docs-site && uv run mkdocs build --strict` — exit 0; only pre-existing unrelated anchor
  warnings (none touching `backbuild-synthesis.md` or `smelt migrate`)
- `/smelt:validate definition_deltas`-equivalent manual sweep: Surface/Semantics/timeless-oracle/
  freshness all checked by hand (this phase's tooling is doc-only, no product behaviour changed) —
  no drift found beyond what this phase closed
