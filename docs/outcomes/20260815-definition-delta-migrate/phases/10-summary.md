# Phase 10 — summary

**Shipped:**
- `/smelt:validate definition_deltas` drift report (below), all items closed.
- `docs/specs/definition_deltas.md`: fixed the stale "phase 11" pointer to "phase 12" (per-cell
  frontier addressing, renumbered 2026-08-30) and bumped `last_reviewed` to 2026-09-02.
- `docs/specs/diagnostics.md` Known Divergences: corrected the `MaintenanceSkeletonChanged`
  paragraph — it no longer says "today, none does" plumb the deployed-schema snapshot in; phase 9
  wired `DeployedSchemaInput` into exactly that caller (`smelt-db`'s `maintenance_plan`), reaching
  `file_diagnostics()`/`smelt explain` ahead of a run.
- `docs/specs/architecture.md` Known Divergences: corrected the backbuild executed-statement-parity
  bullet — it no longer says no CLI/runtime consumer drives a backbuild script through a real
  backend; `smelt migrate --apply` does, proven by a real-DuckDB CLI test. Narrowed the remaining
  gap to specifically extending `statement_parity`'s byte-identical structural leg to the backbuild
  emitter family.
- New standing gate: `crates/smelt-cli/tests/rebuild_dry_run.rs::no_stale_backbuild_verb_or_diagnostic_name_in_docs`
  — greps `docs/specs/` and `docs-site/docs/` for `smelt backbuild` and the old skeleton-column-added
  diagnostic spelling (built via `concat!` to avoid tripping `smelt-db`'s own
  `no_stale_skeleton_column_added_spelling` gate, which scans all of `crates/`), red-green verified.

**Decisions:**
- Drift found in sibling specs (`diagnostics.md`, `architecture.md`) counted as in-scope: the plan
  says corrections land in `definition_deltas.md` "or a sibling spec" when validate finds drift,
  and both were factual claims about this outcome's own shipped surface gone stale.
- The `architecture.md` backbuild-parity gap is left without a phase pointer — no row in 11–21
  owns "extend `statement_parity`'s structural leg to backbuild specifically"; it's an honest
  untracked residue, not force-linked to an unrelated row.
- `deployed_column_names` (phase 9's own residue, already folded into phase 16 during phase 10's
  planning step) needed no further action here — confirmed it's not a `definition_deltas.md`
  concern.

**For the next planner:**
- No behavioral gaps found. Every Surface item, Semantics rule, and Constraint was verifiably
  upheld against real code (CLI wiring, diagnostics enum, References file paths all checked).
- The `architecture.md` backbuild statement_parity structural-leg gap (above) has no owning phase;
  worth a line item if a future phase touches `statement_parity.rs`.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 74 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4 + 23 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
- `cargo test -p smelt-db --test maintenance_diagnostics` — 15 passed.
- `cargo test -p smelt-cli --test rebuild_dry_run --features duckdb` — 4 passed (includes the new
  guard).
- `cargo test -p smelt-cli --test e2e --features duckdb` — 175 passed.

## Drift report (validate summary)

- Automated checks: fmt/clippy/test/example_diagnostics — PASS (see Gates above).
- Surface drift: all `definition_deltas.md` Surface items (Detection, `smelt migrate`,
  `smelt rebuild`, Diagnostics table) verified present and matching in code
  (`crates/smelt-cli/src/commands/migrate.rs`, `diagnostics_types.rs`) and docs-site
  (`guide/backbuild-synthesis.md`, `reference/cli.md`).
- Semantics/invariant drift: none found — the frontier, oracle, atomicity, and plan-and-approve
  rules all have live code and tests per the spec's own References.
- Sibling-sweep (criterion 8): `cli.md`, `model_selection.md`, `architecture.md` (verb-table
  prose), `models.md`, `seeds.md`, `docs-site/docs/guide/backbuild-synthesis.md`,
  `docs-site/docs/reference/cli.md` all already correct (landed phases 4/7/8) — no residue.
- Timeless-oracle: no `Phase [A-Z0-9]` leakage in spec body or referenced user docs.
- Freshness: code touched 2026-09-02 (phase 9), spec now `last_reviewed: 2026-09-02` — fresh.
- Two wording-only drift items found and fixed (`diagnostics.md`, `architecture.md`, both above).
  No `/smelt:plan` follow-up needed.
