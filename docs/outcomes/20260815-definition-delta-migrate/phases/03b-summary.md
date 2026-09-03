# Phase 3b summary

**Shipped:**
- `crates/smelt-runtime/src/definition_delta.rs`: single-owner derivation (`derive_plan` + `detect_definition_delta`/`DefinitionDeltaStatus`) — `smelt run`, `smelt explain`, and `smelt migrate` all read this one module now; `commands/migrate.rs` refactored onto it (steps 5–7 deleted, behavior unchanged, `migrate_plan`/`migrate_apply` green unchanged).
- Run gate in `smelt-runtime/src/execute.rs` (before the schema-evolution gate): a maintained model with an existing table refuses a fold over a `Pending` definition delta via `DefinitionDeltaPendingError`, exit `3` (`commands/run.rs::exit_code_for`, wired in `main.rs`). `--full-refresh` (new `RunArgs`/`ExecuteRequest` flag, previously doc-commented but never wired to any CLI surface) and `--dry-run` are exempt.
- `smelt explain <model>`/`--json` reports a pending delta (`ExplainDefinitionDeltaJson`/text line), never derives/executes beyond the plan.
- `DiagnosticCode::DefinitionDeltaPending` cataloged in `diagnostics.md` and `definition_deltas.md`; LSP code-string match extended.
- `docs/specs/definition_deltas.md` §Detection sharpened per spec-first rule; `cli.md` §"Exit codes" gained a `smelt run` specifics line; docs-site `cli.md` gained `--full-refresh` + the refusal paragraph.
- Tests: `smelt-runtime` unit (6, incl. the phase's 5 plus a `pure_column_addition` discriminator), `smelt-cli` `definition_delta_gate.rs` (5), `explain_definition_delta.rs` (3), `migrate_apply.rs::apply_resumes_rerun_safe_in_progress_plan`.

**Decisions:**
- **Pure column addition is exempt from the run-gate refusal.** The generative `maintenance_conformance` suite's `pure_backfill_column_add_executes_in_place_update` red-lit against the new gate: it exercises the maintenance driver's pre-existing, documented `Trigger::ColumnAdded` → `Technique::InPlaceUpdate` live mechanism with no `smelt migrate` step. Added `DefinitionDiff::is_pure_column_addition` (smelt-logical) and a `pure_column_addition` field on `Pending`; the run gate skips refusal only for that shape. `smelt explain`/`smelt migrate` still report and offer it. Recorded in the spec and the outcome decision log.
- `derive_plan` (lower layer, full `MigrationPlan`+hash) vs `detect_definition_delta` (status classification) split so `smelt migrate` gets the full plan/statements it needs while the run gate/explain get just a status.
- Approval semantics reused verbatim from phase 3 (`plan_hash` match ⇒ Approved/InProgress by the `in_progress` flag).

**For the next planner:**
- `--full-refresh` on `smelt run` was previously undocumented-and-unwired dead code on `ExecuteRequest`; now live. Worth a sweep for other CLI/UI doc-comment promises that were never wired (not investigated further here).
- Phase 4 (rename `backbuild`→`rebuild`) is next per the table; nothing in this phase touched CLI/docs naming beyond what's listed above.
- `pure_column_addition`'s carve-out is narrow by design (added-only, no drop/change/where/skeleton/set-op) — a future column *rename* or *drop* is NOT exempt and will refuse, consistent with the maintenance driver's own narrower coverage.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test definition_delta_gate --test explain_definition_delta --test migrate_plan --test migrate_apply` — 21/21 pass.
- `cargo test -p smelt-runtime --lib definition_delta` — 6/6 pass.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — pass.
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity` — 27/27 pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 71/71 pass (initially 70/71 red on the pure-column-addition conflict above; green after the fix).
