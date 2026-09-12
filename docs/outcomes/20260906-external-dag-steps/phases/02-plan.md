# Phase 2 plan — Parse and validate the external-step declaration in `smelt-core`

## Objective

Make an `external_step:` YAML file a first-class, *parsed* project entity: discovered and
classified as a step (never as a source), validated shape-first, and refused fail-loud with a
named `DiagnosticCode` for every malformed form. Advances success criterion 2 (declaration and
validation, `diagnostics_catalogue` green) and lays the discovery API phase 3 builds the graph
node from. No DAG membership, no invocation, no reporting — those are phases 3-5.

## Spec delta

None. Phase 1 landed the full normative surface in `docs/specs/sources.md`
§"Externally-produced sources (black-box steps)" (declaration shape, key table, discriminator
rule) and the four diagnostic rows. This phase implements it. Two mechanical doc edits are
still required and are *catalogue registration*, not new behaviour:

- `docs/specs/diagnostics.md` §Surface catalogue table — add two rows next to the existing
  `MalformedSource` / `SourceTypeError` rows: `` `MalformedExternalStep` `` (Error — an
  `external_step:` block violates the shape rules in `sources.md`) and
  `` `SourceProducerConflict` `` (Error — two steps name the same source in `produces:`).
  Required by the `every_diagnostic_code_is_catalogued` gate. The two run-path codes
  (`ExternalStepNotInvocable`, `ExternalStepFailed`) are **not** added here — they arrive with
  their enum variants in phase 4, so no variant sits unused.
- `docs/specs/sources.md` §Known Divergences — narrow the "specified but not yet parsed,
  ordered, or invoked" entry to "not yet ordered or invoked" and drop the two now-live codes
  from its list.

## Design decisions this phase settles

- **Where the cross-entity checks live** (question left open by phase 1's summary): per-file
  *shape* checks are in `parse_external_step_yaml` and yield `MalformedExternalStep`;
  the two checks that need the whole project — a `produces:` address that resolves to no
  declared source, and two steps naming one source — live in a pure workspace-level function
  `validate_external_steps(&[ExternalStepInfo], &[SourceInfo]) -> Vec<(PathBuf, ExternalStepError)>`
  in `smelt-core`, called from the Salsa query as a second pass. This mirrors exactly how
  `project_source_diagnostics` already runs the per-target `name:`-key check as a second pass
  after `discover_source_errors`.
- **New module, not `sources.rs`.** `crates/smelt-core/src/external_step.rs` — `sources.rs` is
  1297 lines and under the large-file ratchet.
- **Cadence reuses the interval grammar, not the type.** Parse with `DataLatency::parse` (the
  existing `'1 day'` grammar) but store in a distinct `StepCadence { seconds, display }` newtype,
  so the spec's cadence-vs-`mutation_profile.lateness` distinction cannot collapse by aliasing.

## Tests (red-green)

`crates/smelt-core/tests/external_step_yaml.rs` (new):
1. `yml_with_external_step_classifies_as_step` — `classify` returns `EntityKind::ExternalStep`, not `Source`.
2. `external_step_beats_csv_sibling` — discriminator is checked before the seed-sidecar tiebreaker.
3. `source_discovery_skips_step_files` — `discover_source_infos`/`discover_source_errors` ignore a step file entirely (no spurious `MalformedSource`).
4. `parses_description_produces_command_cadence` — happy path: two `produces:` entries, argv `command:`, `cadence: '1 day'`.
5. `absent_produces_is_malformed` / `empty_produces_is_malformed`.
6. `absent_command_is_malformed` / `empty_command_is_malformed` / `non_list_command_is_malformed`.
7. `columns_alongside_external_step_is_malformed`.
8. `unparseable_cadence_is_malformed`.
9. `unknown_key_is_malformed` — `deny_unknown_fields`, no silent default.
10. `produces_address_naming_no_declared_source_is_malformed` — `validate_external_steps`.
11. `two_steps_producing_one_source_conflict` — `SourceProducerConflict`, anchored deterministically at the later-sorted path.

`crates/smelt-db/tests/integration/external_step_diagnostics.rs` (new, registered in `main.rs`):
12. `malformed_step_yields_malformed_external_step` — the code reaches `project_source_diagnostics`.
13. `duplicate_producer_yields_source_producer_conflict` — likewise, both files named in the message.

`crates/smelt-cli/tests/example_diagnostics/…`:
14. `broken_workspace_external_step_fixtures` — each new `examples/broken/models/sources/step_*.yml` fixture produces exactly its expected code, and no other `examples/broken` file regresses.

Existing gate, no new test: `cargo test -p smelt-db --test integration diagnostics_catalogue`.

## Tasks

1. Add `EntityKind::ExternalStep` and the top-level `external_step:` discriminator to `crates/smelt-core/src/resolver.rs::classify`, checked **before** the `.csv`-sibling and `looks_like_source_yaml` branches; keep step files out of `looks_like_source_yaml`.
2. New `crates/smelt-core/src/external_step.rs`: `ExternalStepInfo`, `StepCadence`, `ExternalStepError` (thiserror, one variant per malformed form + `ProducesUnknownSource` + `ProducerConflict`), `parse_external_step_yaml`, with `#[serde(deny_unknown_fields)]` on the raw struct.
3. `discover_external_steps(project_dir, paths) -> Vec<ExternalStepInfo>` and `discover_external_step_errors(...) -> Vec<(PathBuf, ExternalStepError)>`, addressed via `ModelDiscovery::compute_address_segments` and sorted by path, mirroring the source pair.
4. `validate_external_steps(&[ExternalStepInfo], &[SourceInfo])` — resolve each `produces:` entry (`smelt.` + `address_segments.join(".")`) and detect the duplicate-producer conflict.
5. Re-export from `crates/smelt-core/src/lib.rs`.
6. Add `MalformedExternalStep` and `SourceProducerConflict` to `DiagnosticCode` (`smelt-db/src/diagnostics_types/mod.rs`) and the LSP slug match in `smelt-lsp/src/backend/mod.rs`.
7. Extend `project_source_diagnostics` (`smelt-db/src/queries/project.rs`) with a third pass emitting the step's parse errors and the workspace-level validation errors, keeping the existing path sort — this gives LSP and the build gate parity for free.
8. Add the `examples/broken/models/sources/step_*.yml` fixtures: missing `produces`, bad `command`, `columns:` present, bad `cadence`, unknown `produces:` address, and a `step_dup_producer_{a,b}.yml` pair both naming `smelt.sources.maintenance_orders`.
9. Land the two doc edits from §Spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test external_step_yaml`
- `cargo test -p smelt-db --test integration external_step_diagnostics diagnostics_catalogue`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces` (no step files in the clean examples yet — asserts none of the discovery changes regress source classification)
- `bash .claude/scripts/large-file-check.sh`; hardening ratchets unmoved (`cargo test -p smelt-core --test hardening_budget`)

## Commit message

`feat(sources): parse and validate external-step declarations fail-loud`
