# Phase 7 summary — diagnostic rename lands in code + sibling-spec sweep

**Shipped:**
- `Refusal::SkeletonColumnAdded` → `Refusal::SkeletonChanged` (`smelt-logical/src/maintenance/mod.rs`, `derive.rs`, 3 push sites).
- `MaintenanceRefusal::SkeletonColumnAdded` → `SkeletonChanged` (`smelt-db/src/queries/maintenance.rs`).
- `DiagnosticCode::MaintenanceSkeletonColumnAdded` → `MaintenanceSkeletonChanged` (`smelt-db/src/diagnostics_types.rs`, mapping arm in `lib.rs`).
- LSP wire code string `"maintenance-skeleton-column-added"` → `"maintenance-skeleton-changed"` (`smelt-lsp/src/backend.rs`).
- Extracted the giant `DbCode → &str` match into a standalone `pub(crate) fn diagnostic_code_str` (was inline in `to_lsp_diagnostic`, using `self` nowhere) so the rename is directly unit-testable without constructing a `Backend`/`Client`. New test `skeleton_changed_maps_to_stable_code_string` in `smelt-lsp/src/tests.rs`.
- Updated the 4 existing test files (`maintenance_diagnostics.rs`, `maintenance_tracer.rs`, `maintenance_tracer_evolution.rs`, `maintenance_conformance/gate.rs`) to the new variant name.
- New grep gate `no_stale_skeleton_column_added_spelling` in `crates/smelt-db/tests/maintenance_diagnostics.rs`, scanning `crates/` and `docs/specs/` (excluding `docs/plans`, `docs/handoffs`, `docs/research`, `docs/outcomes`, `target`) for the stale spelling.
- Swept `docs/specs/{diagnostics,incremental_models,model_properties,model_transforms,schema_evolution}.md` to the new name; removed `definition_deltas.md`'s "diagnostic code is not yet renamed" Known Divergences bullet; retargeted the two "not yet surfaced ahead of a run" bullets (`model_properties.md`, `incremental_models.md`) to phase 7b.

**Decisions:**
- The guard test's needle is built via `["Skeleton", "Column", "Added"].concat()` rather than a literal string, so the test's own source doesn't trip the check it performs — the plan's verification step (`rg -n 'SkeletonColumnAdded' crates/ docs/specs/` → no matches) is a literal zero-match requirement, not "zero outside excluded dirs plus this file's own reference."
- Reworded `definition_deltas.md`'s "one code, not a split pair" design paragraph to describe the decision without naming the retired identifier — both to satisfy the grep gate and because `docs/specs/CLAUDE.md`'s timeless-oracle rule already forbids naming historical/pre-rename identifiers in spec prose.
- `to_lsp_diagnostic`'s ~220-arm code-string match used no `self`; extracting it to a free function was a pure refactor (no behavior change) required to make the rename's LSP leg unit-testable per the plan's test list — this is the only code shape change in the phase beyond the mechanical rename.

**For the next planner:** nothing new surfaced outside the plan's own scope. Phase 7b (surfacing `MaintenanceSkeletonChanged` ahead of a run via a deployed-schema Salsa world-fact input) is next and already fully scoped in the outcome's phase-7 decision-log entry.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — PASS.
- `cargo test -p smelt-db --test maintenance_diagnostics` — PASS (8/8, including the new grep gate).
- `cargo test -p smelt-logical --test maintenance_tracer --test maintenance_tracer_evolution` — PASS (19 + 9).
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (74/74).
- `cargo test -p smelt-lsp --lib skeleton_changed_maps_to_stable_code_string` — PASS.
- `rg -n 'SkeletonColumnAdded' crates/ docs/specs/` — no matches.
