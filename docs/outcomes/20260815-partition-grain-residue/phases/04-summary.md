# Phase 4 summary — Per-`ModelDef` overrides for generator-emitted models

**Shipped:**
- `MODEL_DEF_FIELDS` grew from five to seven entries: `timeseries` (Record mirroring
  `TimeseriesConfig`) and `safety_overrides` (Record mirroring `PartitionGrainSafetyOverrides`),
  both optional, Record-typed (`crates/smelt-types/src/signatures.rs`).
- New nested-record validation in `crates/smelt-db/src/type_inference/multi_model.rs`:
  `validate_modeldef_override_literal` walks the two override fields' bespoke required/optional
  sub-field schemas (`MODELDEF_TIMESERIES_OVERRIDE_FIELDS` / `MODELDEF_SAFETY_OVERRIDES_OVERRIDE_FIELDS`),
  distinct from `check_record_literal` because that generic path treats every declared field as
  required.
- New diagnostic `ModelDefOverrideRequiresIncremental` (`diagnostics_types.rs`, wired through
  `smelt-lsp/src/backend.rs`'s exhaustive code-string match) — fires when `timeseries` /
  `safety_overrides` is present but `materialization != 'incremental'`.
- Extraction + whole-block-replacement wiring in `crates/smelt-db/src/queries/project.rs`
  (`materialise_emitted_model_def` for array-literal emissions,
  `materialise_modeldef_from_lambda_body` for the loader-driven `map(fn c => ModelDef{…})` path):
  the nested record literal is rendered to a YAML mapping and deserialized via
  `serde_yaml::from_str::<TimeseriesConfig>` / `::<PartitionGrainSafetyOverrides>`, reusing the
  same parser as frontmatter. The lambda path resolves `c.field`-style sub-field values against
  the loader row (mirrors `extract_field_value_with_binding`'s existing top-level resolution).
- `discovery::model_file_from_emitted_def` (both `smelt-cli` and its `smelt-runtime` duplicate)
  needed **no changes** — both already clone `emitted.incremental_config` /
  `emitted.timeseries_config` verbatim, so the override flows through automatically.
- LSP: `hover_text_for_model_def_optional_field_value` now serves `timeseries` /
  `safety_overrides`; `completion_for_model_def_field_key` was already field-generic (no code
  change needed, only a stale comment).
- Spec: `docs/specs/meta_language.md` field table/rules/diagnostics table/§Design updated;
  `docs/specs/incremental_shapes.md` Known Divergences bullet removed (landed), §Surface sentence
  added; `docs/specs/diagnostics.md` new row. Docs-site: `generators.md` (field table + worked
  per-cohort-override example) and `reference.md` (field table + diagnostic entry) updated.
- Tests: 4 type-inference tests (happy path, unknown sub-field, missing required sub-field,
  incremental-only refusal), 4 project-layer tests (array-literal per-emission override,
  lambda-binding per-row override, no-override regression guard, safety_overrides whole-block
  replacement reaching `incremental_config`), 1 inverted residue probe, 1 LSP completion test
  (five→seven fields), 1 CLI e2e fixture
  (`modeldef_per_emission_override_e2e.rs` — two partition-grain emissions from one generator
  with distinct `event_time_column`s, `smelt build` green with zero diagnostics).

**Decisions:**
- Reused `serde_yaml` deserialization into the existing `TimeseriesConfig` /
  `PartitionGrainSafetyOverrides` structs (both already `#[serde(deny_unknown_fields)]` with
  matching field names) rather than hand-rolling a second extractor — the sub-field type
  checking in `multi_model.rs` already rejects unknown/missing sub-fields before this runs, so a
  malformed override never reaches the YAML path in practice; a defensive `.ok()` drops to `None`
  rather than panicking if it ever does.
- Nested-record validation is bespoke (not `check_record_literal`) because that generic checker
  treats every declared field of a target `Record` type as required — `timeseries` has 3
  required + 2 optional sub-fields, `safety_overrides` has 6 optional. Duplicating the
  required/unknown/missing walk was simpler than adding an optional-field concept to
  `SmeltType::Record` for one caller.

**For the next planner:**
- Nothing deferred out of this phase's stated scope. The `unknown-census.toml` baseline needed a
  line-number resync (5 entries in `signatures.rs` shifted because of the new field-table/const
  additions above them) — routine baseline maintenance, not a design gap.
- Phase 8's close-out already has a known task (per the phase-1 audit decision log entry): fix
  `incremental_shapes.md`'s stale claim that per-source clamp observability is "specified ahead
  of a tracking plan" when it is in fact tracked by `20260704-model-updates-l4-batched.md` Phase
  BL8 — unrelated to this phase, not touched here.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-types --lib signatures` — pass (includes the updated 7-field invariant test).
- `cargo test -p smelt-db --lib multi_model` — pass (25/25, including 4 new tests).
- `cargo test -p smelt-db --lib project` — pass (32/32, including 4 new tests).
- `cargo test -p smelt-logical --test partition_residue_probes` — pass (probe inverted).
- `cargo test -p smelt-lsp --test example_workspaces` — pass (35/35).
- `cargo test -p smelt-cli --test e2e generator_emits_partition_grain_models_with_distinct_time_columns` — pass.
- `cargo test -p smelt-cli --test e2e cohort_count_acceptance` — pass (examples/per_cohort_union/ unaffected).
