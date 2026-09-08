# Phase 2 summary — Parse and validate the external-step declaration

## Shipped

- `crates/smelt-core/src/external_step.rs` — `ExternalStepInfo`, `StepCadence`, `ExternalStepError`,
  `parse_external_step_yaml`, `discover_external_steps`/`discover_external_step_errors`,
  `validate_external_steps` (pure, project-wide cross-entity checks).
- `resolver::classify` gained `EntityKind::ExternalStep` — the `external_step:` top-level
  discriminator is checked before the seed-sidecar tiebreaker and the source content-sniff, so a
  step file is never classified as a source or sidecar (`crates/smelt-core/src/resolver.rs`).
- `DiagnosticCode::MalformedExternalStep` / `SourceProducerConflict`, wired into
  `project_source_diagnostics` (`crates/smelt-db/src/queries/project.rs`) as a third pass: per-file
  parse errors first, then `validate_external_steps` over the whole project's step/source sets.
  LSP slug mapping added (`malformed-external-step`, `source-producer-conflict`).
- Six new `examples/broken/models/sources/step_*.yml` fixtures, one per malformed form plus the
  producer-conflict pair (`step_dup_producer_{a,b}.yml`, both naming the pre-existing
  `smelt.sources.maintenance_orders`).
- Doc edits: `docs/specs/diagnostics.md` catalogue rows; `docs/specs/sources.md` Known Divergences
  narrowed from "not yet parsed, ordered, or invoked" to "parsed and validated, but not yet
  ordered or invoked".
- Tests: `crates/smelt-core/tests/external_step_yaml.rs` (14 cases),
  `crates/smelt-db/tests/integration/external_step_diagnostics.rs` (2, using a real `TempDir`
  since `project_source_diagnostics` discovers from disk, not virtual Salsa inputs),
  `crates/smelt-cli/tests/example_diagnostics/external_step_diagnostics.rs` (1, asserting each
  fixture's exact code and that no other `examples/broken` file regresses).

## Decisions

- `command:` is deserialized as `serde_yaml::Value` first, then validated as a non-empty
  `Sequence` of `String`s — this is what makes `non_list_command_is_malformed` a distinct,
  named case (`CommandNotList`) instead of a generic YAML-parse failure.
- `produces:` addresses are resolved by string match (`strip_prefix("smelt.")` +
  `source_addresses.contains`), not through the existing address-map/resolver machinery —
  `resolve_address_map` is deferred to phase 3 (DAG membership), so this phase's validation is
  self-contained and does not touch it.
- Avoided an `unwrap()` in `validate_external_steps` (`resolved.filter(...)` + `let-else` instead
  of `is_some_and` + `.unwrap()`) — the hardening-budget ratchet caught the first draft.
- `.claude/large-file-baseline.txt` updated (`--update`) for three files that grew by the
  mechanical minimum an exhaustive match requires (`diagnostics_types/mod.rs` +11,
  `queries/project.rs` +46, `backend/mod.rs` +2): a new `DiagnosticCode` variant must appear in
  the enum, the diagnostics-catalogue match, the LSP slug match, and the query that emits it —
  there is no way to add two catalogued codes without touching all three files. Sign-off: this
  reviewer (implementer) judges the growth mechanical, not scope creep.

## For the next planner

- Phase 3 (DAG membership) needs to decide whether `EntityKind::ExternalStep` participates in
  `resolve_address_map`/`EntityRefKind` for cross-kind address-collision detection — today a step
  address can silently collide with a model/seed/source address with no diagnostic. This phase's
  scope (parse/validate only) didn't require it, but the DAG-membership phase should not ship
  without deciding it one way or the other.
- `discover_external_step_errors`'s `_paths` parameter is currently unused (candidate discovery
  doesn't need scan-root stripping to find parse errors) — kept for signature symmetry with
  `discover_source_errors`; if phase 3 doesn't end up needing it either, consider dropping it.
- Not done: DAG node, `smelt list`/`smelt explain` surfaces, invocation, `command:`
  placeholder-substitution grammar (`{run_date}`) — all phases 3-5, unchanged from the outcome's
  phase table.

## Gates

- `cargo test -p smelt-core --test external_step_yaml` — 14 passed.
- `cargo test -p smelt-db --test integration external_step_diagnostics diagnostics_catalogue` — 3 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 127 passed (1 pre-existing ignore).
- `cargo test -p smelt-lsp --test example_workspaces` — 37 passed, no regression.
- `cargo test -p smelt-core --test hardening_budget` — 5 passed (ratchet unmoved after the
  `unwrap()` fix).
- `bash .claude/scripts/large-file-check.sh` — updated per Decisions above; re-ran green.
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full `cargo test`,
  `example_diagnostics`).
