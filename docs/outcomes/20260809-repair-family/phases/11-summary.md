# Phase 11 summary — Surface: `smelt explain` rendering, docs-site update

## Shipped

- `RepairDiscoveryPosture` + `discovery_posture(mutation: MutationProfile)` in
  `crates/smelt-logical/src/maintenance/repair.rs` — the single-owner predicate for "which
  affected-key discovery read this source posture needs" (`ClampedScan` / `SidecarDiff`).
- `crates/smelt-runtime/src/maintenance_driver.rs`'s `resolve_live_per_group_recompute_cell` now
  branches on `repair::discovery_posture(facts.mutation)` instead of its own inline
  `facts.mutation == MutationProfile::MutableSnapshot` comparison — dialect gate, digest-column
  derivation, and the `MaintenanceRepairDigestColumnsMissing` bail stay exactly where they were.
- `smelt_cli::explain::build_maintenance_plan_report` gains a `source_infos: &[SourceInfo]`
  parameter and, for a `Technique::PerGroupRecompute` cell only, a repair stanza: `repair key
  slice` (labelled a sound over-approximation), `repair read bound`, `affected-key discovery`
  (via `source_facts` + `discovery_posture` — never a second mutation-profile mapping), and, when
  a `write: diff_patch` pin resolves the cell to `ChosenTechnique::DiffPatch`, `write mechanism`
  and `diff_patch delete leg` — read from the real `choice::resolve_cell_choice`, not a
  display-only re-derivation. Every non-repair cell's rendering is unchanged.
- `crates/smelt-cli/src/commands/explain.rs` threads its already-in-scope `source_infos` through
  to the new parameter.
- `docs/specs/incremental_models.md` §Surface "CLI" — one added sentence on the `smelt explain
  <model>` bullet naming what a repair cell additionally prints.
- `docs-site/docs/guide/incremental-models.md` — new "Repairing only the affected groups" section
  (retraction over a `mutable_snapshot` dimension → repair cell, the explain stanza verbatim, the
  `write: diff_patch` pin). `docs-site/docs/reference/smelt-explain.md` and `.../reference/cli.md`
  — corrected the `--technique` accepted-name lists to the exact set `parse_technique_arg` accepts
  (both were missing `column_scoped_merge` and `per_group_recompute`).
- Tests: `discovery_posture_is_sidecar_only_for_mutable_snapshot` (unit, `smelt-logical`); four new
  `smelt-cli/tests/explain_maintenance.rs` tests — key-slice/read-bound, discovery posture,
  diff_patch write mechanism + complete delete leg (all via `RepairRecipe`/`render::stage_repair`),
  and a synthetic `KeyedFold` fixture proving the stanza is technique-scoped (no lines leak onto a
  non-repair cell).

## Decisions

- `find_source_info` (new, `smelt-cli/src/explain.rs`) resolves a repair cell's trigger source
  bare name to its `SourceInfo` using the same "strip a leading `sources` address segment"
  convention `smelt_runtime::execute::build_maint_source_facts` already uses — not a second
  lookup convention.
- The `write: diff_patch` resolution reuses `choice::resolve_cell_choice` directly (its own
  `DiffPatch { recompute: PerGroupRecompute, .. }` arm already grants `DeleteLeg::Complete` from
  the repair family's own admission premise) rather than calling
  `maintenance_driver::resolve_repair_write`, which needs a real `sql`/`JoinContext` walk this
  reporting path has no access to — matching phase 11's own plan decision.
- Test fixtures for the three positive-case tests stage a real `RepairRecipe` project via
  `smelt_maintenance_testkit::render::stage_repair` (never hand-authored SQL/YAML); the
  negative-case test builds a synthetic `PlanCell` inline (mirrors `explain_model.rs`'s own
  established pattern for printing-only tests) since no repair-family scope applies to it.

## For the next planner

- This closes the outcome's phase table (row 11 was the last row) and advances success criterion 5
  and closes criterion 6's docs half; criterion 6's non-docs half (standing gates) was already
  green going in and stays green.
- No new gaps surfaced by this phase. Phase 9's hardening items (bit_xor digest collision risk,
  unconfirmed snapshot-reconcile sidecar seed, untested stale group comparandum) and phase 7's
  unenforced region-`DeleteInsert` write-pin divergence remain exactly where prior phases left
  them — out of this phase's scope, not touched.
- `~/.local/lib/duckdb/libduckdb.so` (not `/usr/local/lib`, which lacked write permission in this
  environment) is what `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH` needed to point at here — worth checking
  first if a future session hits a `-lduckdb` link failure despite CLAUDE.md's documented
  `/usr/local/lib` default.

## Gates

- `bash .claude/scripts/verify-phase.sh` — all green (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_maintenance --test explain --test explain_show_sql` — 11
  + 4 + 6 passed.
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity` — 17 + 21 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 59 passed.
