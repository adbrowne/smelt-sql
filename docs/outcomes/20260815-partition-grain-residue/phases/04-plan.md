# Phase 4 plan — Per-`ModelDef` overrides for generator-emitted models

## Objective

Give generator-emitted models the per-emission partition-grain configuration the declared surface
needs: today every emission of a generator file inherits one file-wide `timeseries:` /
`incremental:` frontmatter block (`materialise_emitted_model_def`,
`crates/smelt-db/src/queries/project.rs:949`), so a generator cannot emit two partition-grain
models whose event-time columns differ. Advances success criterion 4 (and closes the
`incremental_shapes.md` §"The partition grain" Known Divergences bullet audited as residue #9).

## Design decision (recorded here, made in this plan)

`MODEL_DEF_FIELDS` grows from five to **seven** entries, adding two optional record-typed fields
spelt exactly like the frontmatter keys they override:

- `timeseries` — `Record { event_time_column: Text, partition_column: Text, granularity: Text,
  week_start: Text (opt), assert_monotonic: Bool (opt) }`, mirroring `TimeseriesConfig`.
- `safety_overrides` — `Record` of the six optional `PartitionGrainSafetyOverrides` booleans.

Semantics: **whole-block replacement**, not key-level merge — a present field replaces the
generator frontmatter's corresponding block entirely (half-inherited blocks are unreasonable to
explain and would let a partial override silently keep a stale `partition_column`). Both fields
are honoured only when `materialization == 'incremental'`; present on any other materialization is
a fail-loud diagnostic, never a silent ignore.

## Spec delta (first)

- `docs/specs/meta_language.md` §"`ModelDef` meta record type" — field table becomes seven rows;
  add field rules for `timeseries` / `safety_overrides` (sub-field schema, required sub-fields,
  whole-block replacement, incremental-only); the §Design paragraph "Why the closed five-field
  set" is re-titled/rewritten to record that the partition grain was the concrete pressure that
  opened the set, and that further keys (`owner:`, `backend_hints:`, `target:`) stay paced.
  Add the new code to §"Multi-model diagnostic codes".
- `docs/specs/diagnostics.md` — new row `ModelDefOverrideRequiresIncremental`.
- `docs/specs/incremental_shapes.md` §"The partition grain" — delete the "Per-`ModelDef` overrides
  … not part of the closed field set in v1" Known Divergences bullet; add one §Surface sentence
  saying a generator-emitted partition-grain model may carry its own `timeseries` /
  `safety_overrides` per emission.
- `docs-site/docs/meta-language/generators.md` + `reference.md` — field table + a worked
  per-cohort example with distinct event-time columns.

## Tests (red → green)

1. `smelt-types::signatures` `model_def_fields_are_the_closed_set` (existing, ~:8012) — updated to
   assert seven entries in canonical order and the two new field types.
2. `multi_model::model_def_timeseries_override_typechecks` — happy-path literal with a full
   `timeseries` block emits no sentinels, infers `SmeltType::ModelDef`.
3. `multi_model::model_def_timeseries_unknown_subfield_rejected` — `RecordFieldUnknown` anchored at
   the sub-field name, message naming `ModelDef.timeseries` and the sub-field set.
4. `multi_model::model_def_timeseries_missing_required_subfield_rejected` — omitted
   `partition_column` → `RecordFieldMissing` at the inner close brace.
5. `multi_model::model_def_override_on_non_incremental_emission_rejected` — `materialization:
   'view'` + `timeseries: {…}` → `ModelDefOverrideRequiresIncremental` at the field name.
6. `project::array_literal_emission_uses_per_modeldef_timeseries` — two `ModelDef` literals with
   different `event_time_column`s produce two `EmittedModelDef`s with different
   `timeseries_config`, regardless of the file-wide frontmatter block.
7. `project::lambda_emission_binds_timeseries_from_loader_record` — `event_time_column: c.time_col`
   in a `map(fn c => ModelDef {…})` chain resolves per loader row (the
   `extract_field_value_with_binding` path, ~:1258).
8. `project::emission_without_override_still_inherits_frontmatter_block` — regression guard on
   today's inheritance.
9. `project::safety_overrides_override_reaches_emitted_metadata` — the override lands on the
   emitted model's `PartitionGrainConfig::safety_overrides`, i.e. survives
   `discovery::model_file_from_emitted_def`.
10. `smelt-cli` e2e (`tests/e2e/`): `generator_emits_partition_grain_models_with_distinct_time_columns`
    — a temp workspace whose generator emits two `refresh: incremental` / `grain: partition`
    models over sources with differently-named time columns builds green with zero diagnostics.
11. `smelt-logical` `partition_residue_probes::probe_modeldef_per_model_override` — inverted to
    assert the seven-field set, doc comment updated to record phase 4 landing.
12. `smelt-lsp` completion/hover tests asserting the five-field set — updated to seven (offer order:
    required first, then the existing optionals, then the two override fields).

## Tasks

1. Spec + diagnostics-catalogue + docs-site edits above (spec-first).
2. Add the two entries to `MODEL_DEF_FIELDS` and extend its doc comment (`signatures.rs:3788`);
   update the five-entry invariant test.
3. Add `DiagnosticCode::ModelDefOverrideRequiresIncremental` + message in
   `smelt-db/src/diagnostics_types.rs`, wired through `meta_multi_model_diagnostic_message`.
4. Extend `infer_model_def_literal` (`type_inference/multi_model.rs`) with nested-record validation
   for the two fields (unknown sub-field, duplicate, missing required, incremental-only check),
   reusing `meta_record_diagnostic_message`.
5. Extract the overrides in both emission paths in `queries/project.rs`
   (`materialise_emitted_model_def` and the lambda-binding path), replacing the inherited block
   when present; keep the `materialization == "incremental"` gate as the only inheritance trigger.
6. Thread `safety_overrides` into the emitted `PartitionGrainConfig` so
   `discovery::model_file_from_emitted_def` carries it to `ModelMetadata`.
7. Add the CLI e2e fixture; invert the probe; sweep `examples/per_cohort_union/` for breakage.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-types --lib signatures`
- `cargo test -p smelt-db --lib multi_model` and `cargo test -p smelt-db --lib project`
- `cargo test -p smelt-logical --test partition_residue_probes`
- `cargo test -p smelt-lsp --test example_workspaces`
- `cargo test -p smelt-cli --test e2e` (new fixture) and `--test cohort_count_acceptance`

## Commit message

`feat(meta): per-ModelDef timeseries and safety_overrides for generator-emitted models`
