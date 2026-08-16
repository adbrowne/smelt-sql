# Phase 8 summary — diagnostic rename lands in code

**Shipped:**
- `DiagnosticCode::MaintenanceSkeletonColumnAdded` renamed to `MaintenanceSkeletonChanged`
  (`crates/smelt-db/src/diagnostics_types.rs`), with its one mapping site
  (`crates/smelt-db/src/lib.rs::file_diagnostics`) updated and its message widened from
  "column '{c}' … never a column backfill" to cite `definition_deltas.md`
  §"Skeleton changes are a new relation" instead of the stale `incremental_models.md` pointer.
- `crates/smelt-lsp/src/backend.rs`'s exhaustive `DbCode` match updated
  (`maintenance-skeleton-column-added` → `maintenance-skeleton-changed`) — the LSP code string is
  user-visible and the plan's file list didn't name this site, but the compiler caught it via the
  exhaustive match once the enum variant renamed.
- `crates/smelt-logical/src/maintenance/ledger.rs::render_refusal`'s code string renamed
  (`smelt explain`'s refusal block), plus a new unit test
  `skeleton_refusal_names_the_renamed_code`.
- `crates/smelt-cli/src/commands/migrate.rs`'s `SkeletonChange` verdict now names the code: the
  human render appends `— MaintenanceSkeletonChanged`, and `--json` gained a `diagnostic_code`
  field on each group (`null` for non-skeleton verdicts) — the existing `"skeleton_change"` tag is
  unchanged, so the JSON contract only grew. New test
  `skeleton_change_plan_names_the_diagnostic_code` covers both renders.
- Internal pure variants (`smelt_logical::maintenance::Refusal::SkeletonColumnAdded`,
  `smelt_db::queries::maintenance::MaintenanceRefusal::SkeletonColumnAdded`) left unrenamed per
  the plan; their doc comments now name `MaintenanceSkeletonChanged` as the code they map to.
- Spec sweep: `docs/specs/diagnostics.md` (catalogue row + Known Divergences paragraph),
  `docs/specs/definition_deltas.md` (Design rationale reworded to stay timeless — no longer
  narrates the rename as an event; Known Divergences bullet narrowed to only what survives:
  not yet surfaced ahead of a run, tracked at phase 9), `model_transforms.md` (5 occurrences),
  `model_properties.md`, `incremental_models.md`, `schema_evolution.md` — mechanical rename only.
- New standing ratchet `crates/smelt-db/tests/integration/diagnostics_catalogue.rs::
  no_old_skeleton_code_name_in_specs_or_code` — scans `crates/`, `docs/specs/`, `docs-site/docs/`
  for the retired name (built from string parts so the test's own source doesn't self-trip).

**Decisions:**
- The `maintenance_diagnostics.rs::column_added_trigger_skeleton_position_refuses` test drives
  `derive_model_maintenance_plan` directly and cannot reach the `DiagnosticCode` mapping —
  `maintenance_plan_diagnostics` (the Salsa-query-facing wrapper) hardcodes an empty deployed-column
  set (the known divergence phase 9 closes), so the `Refusal → DiagnosticCode` path is genuinely
  unreachable through any Salsa-driven test today. Only the doc comment was updated (with a note
  explaining why); no new runtime assertion was invented for a path that doesn't exist yet.
- `definition_deltas.md`'s Design section previously narrated the rename as a completed event
  ("`X` is renamed to `Y`") — a timeless-oracle violation (CLAUDE.md). Reworded to state the
  current rule directly: one code, no split add/changed pair.

**For the next planner:**
- Phase 9 (surfacing) is unchanged in scope: `derive_model_maintenance_plan_plan`'s
  `deployed_column_names` still needs a real Salsa input for the LSP and `smelt explain` to fire
  `MaintenanceSkeletonChanged` ahead of a run.
- Nothing else surfaced. `refusal_catalogue_sync.rs` (the `render_refusal` ↔ `diagnostics.md`
  sync gate) passed cleanly on first run — the rename kept both sides in step.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`,
  `example_diagnostics`)
- `cargo test -p smelt-db --test maintenance_diagnostics --quiet` — 8 passed
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet` — 2 passed
- `cargo test -p smelt-logical --lib maintenance::ledger --quiet` — 6 passed
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet` — 15 passed
- `cargo test -p smelt-logical --test refusal_catalogue_sync --quiet` — 2 passed
