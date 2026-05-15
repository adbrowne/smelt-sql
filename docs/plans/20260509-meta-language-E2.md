# Plan: Meta-Language Phase E2 — Multi-model production (`generates: models`, `ModelDef`, per-cohort union killer demo)

**Date**: 2026-05-15
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Multi-model production" (Surface lines 493–591; Semantics rule §"Multi-model production"; Design §"Multi-model production — design rationale"; Invariants §"Multi-model production invariants"); plus cross-feature touches in `expansion.md`, `architecture.md`, `cli.md`, `data_catalog.md`, `datagen.md`, `incremental_models.md`, `model_selection.md`, `python_models.md`, `schema_evolution.md`.
**Spec diff**: commits `d9ae889` (`spec(meta-language-E2): author multi-model production spec + cross-feature touches`) and `8a3dbbf` (`spec(meta-language-E2): soften reflection-forbid rationale; add dynamic-frontmatter open question`) on branch `research/typed-meta-programming`.
**Tracking PR / branch**: PR #117 (retitled `feat: typed meta-programming`) — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Multi-model production" (Surface, Semantics, Design, Invariants, Known Divergences) and skim the nine cross-feature spec touches enumerated under §"Cross-feature spec touches" below — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec meta_language` (or the cross-feature spec) to revise the spec rather than encode the divergence in code.
2. Confirm you are on branch `research/typed-meta-programming`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 7 is the expert-reviewer dispatch loop** — after Phases 1–6 commit, dispatch the meta-plan §5 expert reviewers applicable to E2, address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 7. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 7's acceptance gate is met.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first to update).
- `cargo test` or `cargo clippy --all-targets` surfaces a pre-existing failure unrelated to the plan.
- Phase 7: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.
- A `<generator>` frame change forces an `expansion.md` invariant to relax — meta-plan §7's "cross-feature impact wider than predicted" stop-the-line applies.

**Conventions every phase:**

- Real-fixture tests under `examples/per_cohort_union/` and `examples/staging_from_sources/` — Phase 6 exercises the full surface there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no parameterised reducers, no multi-arg lambdas, no ternary, no LSP rename (Phases F and G). No `Array<U>(…)` runtime-array constructor (deferred). No `generated_path_prefix` frontmatter override. No path-component identifier lift. No per-`ModelDef` frontmatter beyond the closed five-field set.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference/` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic). All Salsa queries for the W1–W4 workspace-shape resolution pipeline live in `crates/smelt-db/src/queries/project.rs` (next to `models_with_tag` / `models_all`); inference rules consume resolved workspace state as parameters.
- Timeless-oracle rule: spec and user-doc edits read as if the feature has always existed. Phase vocabulary lives in this plan only — never inside `docs/specs/` or `docs-site/docs/` body sections.

---

## Context

The meta-language Phase E2 spec increment landed in commits `d9ae889` and `8a3dbbf`. The spec authors one load-bearing surface — **multi-model production** — and nine cross-feature touches.

- **Multi-model production.** A `.sql` file's YAML frontmatter may declare `generates: models`, marking the file as a generator file whose body is a meta-language expression of type `List<ModelDef>`. `ModelDef` is a closed five-field built-in record type `{name: Text, body: TableExpr, materialization: Text, tags: List<Text>, description: Text}`, user-constructible only inside generator bodies. The compiler resolves workspace shape in a single bounded pass (Stages W1–W4): discover generator files; evaluate each generator's body in isolation; emit and collision-check; full type-check against the union of hand-authored and generator-emitted models. Ten new diagnostic codes (`GeneratesUnknownValue`, `GeneratesMixedWithBareModel`, `GenerateFileBareSelectForbidden`, `GenerateFileBodyTypeError`, `ModelDefOutsideGeneratorFile`, `ModelDefInvalidName`, `ModelDefInvalidMaterialization`, `ModelDefDuplicateName`, `ModelDefHandAuthoredCollision`, `GeneratorBodyForbidsModelReflection`) anchor at the offending CST span. A new `<generator>` anonymous expansion frame plugs into the existing `expansion.md` frame-stack contract so diagnostics from inside emitted-`ModelDef` HOF chains carry a navigable provenance root pointing at the generator file's body range.

- **Cross-feature spec touches.** `expansion.md` registers the `<generator>` frame variant; `architecture.md` extends bare-model naming (Project layout + Bare-model naming) without retiring the stable invariants; `model_selection.md` adds a `generator_file:<path>` selector method; `cli.md` adds an `origin` field to `smelt explain --json`; `data_catalog.md` adds a parallel `origin` field plus a "Source" line in the markdown output; `incremental_models.md` documents emitted-incremental-models coexistence; `python_models.md` records the Python-`@model`-cannot-emit-via-`generates:` known divergence; `datagen.md` records the generators-of-generators / datagen forbid; `schema_evolution.md` notes generator emissions participate in schema diffs.

This plan drives the implementation, examples, user docs, skill update, and `/smelt-loop` extension for that surface. Phase E2 is the sixth of seven implementation phases (A–G); the multi-model production primitive composes with Phase E1's records, `Map<K, V>`, and schema-typed loaders to deliver the **per-cohort union killer demo** at `examples/per_cohort_union/` (meta-plan §8) and the **staging-from-sources demo** at `examples/staging_from_sources/`.

## Scope

### In scope (spec coverage)

- `meta_language.md` Surface for multi-model production: `generates: models` frontmatter directive, `ModelDef` closed five-field record type, generator file body shape (`List<ModelDef>`), emitted-model `smelt.<path>` rule (`<dir>.<file_stem>.<modeldef.name>`), generator interaction with reflection (forbid `smelt.models.*` inside generator bodies; admit `smelt.sources.*`, loaders, `smelt.config.var`, literal `smelt.<path>` to hand-authored models), name uniqueness and collision rules, the ten new diagnostic codes, and the LSP obligations (hover on `generates:` frontmatter, hover on `ModelDef { … }` opening brace, hover on `ModelDef.name` and `ModelDef.body`, goto-def from a generator-emitted model reference to the emitting `ModelDef.name` field, completion at `generates: <cursor>`, completion at `ModelDef { <cursor> … }` field-key positions, diagnostics-with-frame-stacks carrying the `<generator>` outer frame).
- `meta_language.md` Semantics §"Multi-model production" rules 1–10 — the normative evaluation rules for generator-file detection, body type checking, record-literal evaluation, the W1–W4 workspace-shape resolution pipeline, determinism, body evaluation context, emitted-model `smelt.<path>` resolution, generator-body reflection forbid, termination, cross-file evaluation order.
- `meta_language.md` Design §"Multi-model production — design rationale" — preserved as architectural invariants policed by the implementation (frontmatter-directive shape, `ModelDef` user-constructibility, closed field set, path including file stem, generator-body reflection forbid, generators-of-generators forbid, path-component lift deferral, frontmatter-inheritance globally, hand-authored-wins-on-collision).
- `meta_language.md` Constraints §"Multi-model production invariants" — generator marking via frontmatter; body is `List<ModelDef>`-typed; `ModelDef` is the only user-constructible closed meta record; closed field set; `ModelDef.body` is the only `TableExpr` carve-out in a record field; emitted-path rule; generator-body reflection forbid; W1–W4 single-pass shape resolution; determinism; hand-authored wins on collision; four-position lift unchanged.
- `meta_language.md` Known Divergences entry — replace the "Multi-model production is normative; implementation is forthcoming" bullet with a recap of what shipped (the surface, the W1–W4 pipeline, the ten diagnostics, the `<generator>` frame, the LSP support); leave the "dynamic frontmatter as a meta-evaluable value" open-question entry as a forward-looking architectural note.
- `expansion.md` Surface §"`FrameInfo` shape" — register the `<generator>` frame variant per the committed spec text (a frame stamped with `function = "<generator>"`, `fn_id = None`, `decl_path = Some(<generator file path>)`, `decl_range = None`, `call_site_range = <range of the file's body expression>`). Replace the "implementation is forthcoming" Known-Divergences bullet with the implementation-landed reality once Phase 5 ships.
- `architecture.md` §"Project layout" + §"Bare-model naming" — extend (do not retire) the bare-model invariants for generator files: `models/` may contain bare-SELECT models *or* generator files; `generates: models` is mutually exclusive with `name:` field / Layer-1 delimiters; emitted-model paths include the file stem.
- `model_selection.md` §"Surface" + §"Semantics" — `generator_file:<workspace-relative path>` selector method; matches every emitted model from the named generator file against the post-W3 workspace shape; the `+` directional modifier composes normally; selectors against a non-generator path match nothing (no error).
- `cli.md` §"`smelt explain --json` output schema" — `origin` field on model entries: omitted for hand-authored; `{type: "generated", generator_file: <path>, generator_name: <name>}` for emitted models. Promote the "implementation forthcoming" Known-Divergences bullet to the landed reality.
- `data_catalog.md` §"`smelt docs --json` output schema" + §"Generator-emitted model provenance" — `origin` field on model entries; "Source" line in the markdown output for generator-emitted models pointing at the generator file; tag-index covers emitted models.
- `incremental_models.md` Known Divergences — generator-emitted incremental models inherit the file's `incremental:` block; per-`ModelDef` overrides await a future spec edit. Promote to "Implementation is now landed" once Phase 5 wires `ModelDef.materialization: 'incremental'`.
- `python_models.md` Known Divergences — Python `@model` functions cannot emit via `generates: models`; the SQL meta-language generator surface is non-overlapping with Python generators.
- `datagen.md` Known Divergences — generators-of-generators forbidden; the meta-language generator surface does not produce `smelt-datagen` configs.
- `schema_evolution.md` Known Divergences — generator-emitted models participate in schema diffs on equal terms with hand-authored models.
- LSP support for every Phase E2 surface element: hover on `generates: models` frontmatter (inferred body type + emitted-model count when statically resolvable); hover on `ModelDef { … }` opening brace (inferred emitted-model smelt path when `name` is statically known); hover on `ModelDef.name` field value (emitted smelt path); hover on `ModelDef.body` (synthesised `TableExpr` type + inferred column list when resolvable); goto-def from a generator-emitted model reference (`smelt.<path>` at a consumer site) to the emitting `ModelDef.name` field value-token; completion at `generates: <cursor>` (offering `models`); completion at `ModelDef { <cursor> … }` field-key (the closed five-field set, required fields first); diagnostics-with-frame-stacks carrying the `<generator>` outer frame for emissions surfacing from inside an emitted-`ModelDef` HOF chain.
- Workspace-shape resolution pipeline (W1–W4) as Salsa queries: `generator_files()`, `evaluate_generator(file)`, `emitted_models()`, `models_all_with_generators()`. Integration into the existing `models_with_tag` and `models_all` reflection queries so wide reflection sees the unioned set.
- Examples fixtures `examples/per_cohort_union/` (the killer demo per meta-plan §8) and `examples/staging_from_sources/`, plus broken sub-fixtures for each new Phase E2 diagnostic, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/generators.md` (new) and reference-page additions covering `generates: models`, `ModelDef`, the closed field set, the worked killer-demo example, the `generator_file:` selector, the `origin` field in CLI/catalog output, the `<generator>` frame extension. Update existing `index.md`, `reflection.md` (covering the generator-body reflection forbid), and `reference.md`.
- `smelt-app-builder` skill: per-phase reference doc at `.claude/skills/smelt-app-builder/references/20260515-meta-multi-model-production.md` documenting the `generates: models` workflow, frontmatter-exclusivity gotchas, and the per-cohort-union pattern.
- `/smelt-loop` `large` tier — new tier requiring an agent to author both a generator file (`generates: models` frontmatter + body returning `List<ModelDef>` driven by a YAML config) and a downstream union consumer (`smelt.models.with_tag('<tag>') |> reduce(union_all)`). Phase G runs the tier end-to-end; Phase E2 ships the fixture and the loop-command extension.

### Explicitly deferred

- Parameterised reducers (`concat_with(sep)`), multi-arg lambdas (`fn (a, b) => body`), the meta-world ternary (`if cond then a else b`), `zip_with` — Phase F.
- LSP rename for the new `ModelDef` and `generates:` constructs — Phase G.
- Full LSP completeness sweep (universal hover/goto-def/completion across every shipped construct) — Phase G.
- `/smelt-loop` `large` tier execution + resulting skill diffs — Phase G runs the tier to completion; Phase E2 ships the fixture and the command-line shape.
- `Array<U>(…)` runtime-array constructor — deferred per meta-language Known Divergences; does not interact with E2.
- `generated_path_prefix` frontmatter override for flatter emitted paths — out-of-scope (revisit when path-depth pain emerges).
- Path-component identifier lift (admitting meta-`Text` in `smelt.<…>` positions) — out-of-scope (the four-position lift remains exactly the column-reference / AS-alias / ORDER BY / GROUP BY positions).
- CTE-name and table-alias identifier lift — out-of-scope.
- Per-`ModelDef` frontmatter beyond the closed five-field set — out-of-scope; concrete demand for `incremental:` blocks, `owner:`, `backend_hints:` per emission must wait for a future spec edit.
- Transparent generator file naming — out-of-scope; emitted paths always include the generator file's stem.
- Python `@model` generators — `python_models.md` known divergence; Python's generation surface remains non-overlapping.
- Generators-of-generators — structurally forbidden; the spec rejects both the literal-path and `smelt.models.*` paths to chaining. No implementation work required (the forbid falls out of `GeneratorBodyForbidsModelReflection` plus the rule "generator-body literal `smelt.<path>` resolves only against hand-authored models").
- Catalog Markdown-output overhaul beyond the new "Source" line for generator-emitted models — broader rendering changes are post-plan.
- Backwards-compatibility shims for pre-E2 workspaces — none required; `generates:` is a new key with no prior meaning.
- Performance optimisation beyond the Salsa caching of the W1–W4 pipeline — concrete profiling is a Phase G concern.
- Dynamic frontmatter (`ModelConfig` as a meta-evaluable value, per-emission frontmatter overrides) — open question, no E2 implementation work. The static-shape model ships in v1; the open question is documented in `meta_language.md` Known Divergences.

## Cross-feature spec touches

These cross-feature spec edits already landed in commits `d9ae889` and `8a3dbbf`. This plan implements the code that makes them executable. **Do not re-edit these specs unless an implementation gap surfaces a spec error**; that route is `/smelt:spec` first.

- `docs/specs/expansion.md` — `<generator>` frame in §"`FrameInfo` shape"; Known Divergences entry to flip to "landed" once Phase 5 wires the frame stamping.
- `docs/specs/architecture.md` — §"Project layout" + §"Bare-model naming" extended for generator files (existing invariants preserved on the `stable` axis).
- `docs/specs/model_selection.md` — `generator_file:<path>` selector method in Surface + Semantics.
- `docs/specs/cli.md` — `origin` field in `smelt explain --json` output schema + Known Divergences entry to flip to "landed".
- `docs/specs/data_catalog.md` — `origin` field on model entries; new §"Generator-emitted model provenance" subsection covers Markdown rendering.
- `docs/specs/incremental_models.md` — Known Divergences entry on emitted-incremental coexistence; flip to "landed" once Phase 5 wires.
- `docs/specs/python_models.md` — Known Divergences entry for Python-`@model`-cannot-emit-via-`generates:`.
- `docs/specs/datagen.md` — Known Divergences entry for generators-of-generators / datagen non-overlap.
- `docs/specs/schema_evolution.md` — Known Divergences entry for generator emissions participating in schema diffs.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 23bf398 | 2026-05-15 |
| 2     | done     | 43f17f6 | 2026-05-15 |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Type system foundation — `SmeltType::ModelDef` + `MODEL_DEF_FIELDS` + `FileMetadata::Generator` + diagnostic codes (pure)

**Goal.** Add the `SmeltType::ModelDef` variant (mirroring `ColumnRef` / `ModelRef` / `SourceRef`'s closed-field shape) and the `MODEL_DEF_FIELDS` static. Add `generates: Option<String>` to `ModelMetadata`. Add `FileMetadata::Generator { metadata, body_offset }` variant alongside `Empty` / `Single` / `Multi`. Extend `extract_file_metadata` to route files with `generates: models` frontmatter to the new variant — including the mutual-exclusivity guards (`GeneratesMixedWithBareModel` returned as a `MetadataError::GeneratesMixedWithBareModel` variant for parser dispatch in Phase 2). Register every Phase E2 diagnostic code (the ten new codes in `crates/smelt-db/src/diagnostics_types.rs::DiagnosticCode`). All work is pure; no Salsa imports added to inference, no parser changes yet (Phase 2).

**Pre-conditions.** Phase E1 done at commit `b9df522`. Working tree clean. `cargo test`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**TDD tests to write first.**

- `crates/smelt-types/src/signatures.rs::tests::model_def_fields_registry_is_closed_and_exact` — `MODEL_DEF_FIELDS` exposes exactly the five names `{name, body, materialization, tags, description}`; lookup of any other identifier (e.g. `incremental`, `owner`) returns `None`; each entry's type matches the spec table (`Text`, `TableExpr`, `Text`, `List<Text>`, `Text`).
- `crates/smelt-types/src/signatures.rs::tests::model_def_smelt_type_round_trips_field_access` — `SmeltType::ModelDef` constructed via the public constructor exposes a `field_type(name)` accessor returning the spec-declared type for each of the five names; returns `None` for unknown.
- `crates/smelt-types/src/signatures.rs::tests::model_def_is_assignment_isolated_from_record` — `SmeltType::ModelDef` does not unify with a structurally-equal `SmeltType::Record` (the spec rule "ModelDef is the only user-constructible closed meta record type" requires it to be distinguishable from user-declared `smelt.record` types of identical shape); `is_subtype_of(ModelDef, Record{…})` is `false` in both directions.
- `crates/smelt-types/src/signatures.rs::tests::model_def_admits_table_expr_in_body_field` — the `body` field's declared type in `MODEL_DEF_FIELDS` is `TableExpr` (the only carve-out admitting `TableExpr` in a record field; `find_forbidden_type_name` from the record path is bypassed only for this field name on `ModelDef`).
- `crates/smelt-types/src/signatures.rs::tests::model_def_is_meta_only_does_not_reach_data_world` — `is_data_world_type(SmeltType::ModelDef) == false`; the existing `is_meta_only_type` predicate returns `true`.
- `crates/smelt-core/src/metadata.rs::tests::parse_generates_models_frontmatter_routes_to_generator_variant` — a `.sql` source with `---\ngenerates: models\n---\n[ModelDef {…}]` parses to `FileMetadata::Generator { metadata, body_offset }` where `metadata.generates == Some("models".into())` and `body_offset` points at the byte just after the closing `---\n` newline.
- `crates/smelt-core/src/metadata.rs::tests::parse_generates_unknown_value_emits_metadata_error` — `---\ngenerates: views\n---\nSELECT 1` returns `MetadataError::GeneratesUnknownValue { value: "views".into(), value_span: <line/column of the value token> }`. The error variant carries the value span so Phase 2 can anchor the diagnostic at the YAML value token.
- `crates/smelt-core/src/metadata.rs::tests::parse_generates_with_name_field_emits_mixed_error` — `---\ngenerates: models\nname: foo\n---\n[]` returns `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::NameField, span: <line/col of the name: key> }`. The variant carries which mutual-exclusivity rule fired (name field vs Layer-1 delimiter).
- `crates/smelt-core/src/metadata.rs::tests::parse_generates_with_section_delimiter_emits_mixed_error` — `---\ngenerates: models\n---\n--- name: foo ---\nSELECT 1\n--- name: bar ---\nSELECT 2` returns `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::SectionDelimiter, span: <line/col of first --- name: --- delimiter> }`.
- `crates/smelt-core/src/metadata.rs::tests::parse_no_generates_keeps_single_or_multi_variants` — files without `generates:` frontmatter continue to parse to the existing `Single`, `Multi`, or `Empty` variants (regression).
- `crates/smelt-core/src/metadata.rs::tests::parse_generates_models_with_other_frontmatter_keys_admits_them` — `---\ngenerates: models\ntags: [cohort]\nowner: data-team\n---\n[]` parses to `Generator { metadata, body_offset }` where `metadata.tags == ["cohort"]` and `metadata.owner == Some("data-team".into())` (frontmatter inheritance per spec rule "file-wide concerns live in the generator's frontmatter").
- `crates/smelt-db/src/diagnostics_types.rs::tests::diagnostic_codes_multi_model_set_complete` — every multi-model diagnostic code (`GeneratesUnknownValue`, `GeneratesMixedWithBareModel`, `GenerateFileBareSelectForbidden`, `GenerateFileBodyTypeError`, `ModelDefOutsideGeneratorFile`, `ModelDefInvalidName`, `ModelDefInvalidMaterialization`, `ModelDefDuplicateName`, `ModelDefHandAuthoredCollision`, `GeneratorBodyForbidsModelReflection`) exists in the `DiagnosticCode` enum and renders the spec-`meta_language.md` §"Multi-model production diagnostic codes" message format verbatim.

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - `SmeltType::ModelDef` variant (no payload — the field set is encoded as the static `MODEL_DEF_FIELDS`, the same shape as `ColumnRef` / `ModelRef` / `SourceRef`).
  - `pub static MODEL_DEF_FIELDS: LazyLock<Vec<(&'static str, SmeltType)>>` with the five entries in the spec-declared order `[(name, Text), (body, TableExpr), (materialization, Text), (tags, List<Text>), (description, Text)]`. Order is user-facing (drives completion ordering); the closed-set property is the load-bearing invariant.
  - `fn model_def_field_type(name: &str) -> Option<&'static SmeltType>` helper — closed-set lookup. Used by Phase 3's record-literal validator without re-implementing the registry walk.
  - Extend `is_meta_only_type(&SmeltType) -> bool` so `ModelDef` returns `true` (mirrors the existing `ColumnRef` / `ModelRef` / `SourceRef` clauses).
  - Extend `is_subtype_of` so `ModelDef` is distinguishable from any `Record{…}` even with identical fields (return `false` in both directions). This preserves the spec invariant "`ModelDef` is the only user-constructible closed meta record type"; users cannot widen a `Record{…}` into a `ModelDef` through structural typing.
  - The `body` field's `TableExpr` carve-out is encoded by the registry entry alone; the existing `find_forbidden_type_name` in `record.rs` walks user-declared `smelt.record` field types and ignores `MODEL_DEF_FIELDS`, so no change is required there.
- `crates/smelt-core/src/metadata.rs`:
  - `ModelMetadata` gains `pub generates: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`. The struct keeps `#[serde(deny_unknown_fields)]` — no other field is added.
  - `FileMetadata::Generator { metadata: Box<ModelMetadata>, body_offset: usize }` variant. The `metadata` carries the parsed frontmatter (everything but `generates:`); `body_offset` points at the first byte after the closing `---\n` delimiter (same shape as `Single { sql_offset }`).
  - New `MetadataError` variants: `GeneratesUnknownValue { value: String, value_span: (usize, usize) }`, `GeneratesMixedWithBareModel { offending: MixedKind, span: (usize, usize) }` where `MixedKind` is `NameField | SectionDelimiter`. The span carries 1-based line + column (the existing convention in `metadata.rs::has_malformed_delimiter`).
  - Extend `extract_file_metadata` and `extract_single_model` to detect `generates:` early in the YAML parse and dispatch to a new `extract_generator(...)`. The implementation parses the frontmatter as `ModelMetadata`, validates `generates == "models"` (otherwise `GeneratesUnknownValue`), checks the rest of the frontmatter for a `name:` field (`GeneratesMixedWithBareModel { NameField }`), and scans the post-frontmatter body for `--- name: ` Layer-1 delimiters (`GeneratesMixedWithBareModel { SectionDelimiter }`). Returns `FileMetadata::Generator { metadata, body_offset }`.
- `crates/smelt-db/src/diagnostics_types.rs`:
  - Ten new `DiagnosticCode` variants with `Display` impl entries rendering the spec-table message format. Verify by running a test that round-trips every variant's `to_string()` and compares against a hard-coded expected message shape.
- `crates/smelt-core/src/lib.rs`:
  - Only if a re-export is needed for `FileMetadata::Generator` / `MixedKind`.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs` — `SmeltType::ModelDef`, `MODEL_DEF_FIELDS`, `model_def_field_type`, `is_meta_only_type` and `is_subtype_of` extensions, accompanying tests.
- `crates/smelt-types/src/lib.rs` — only if a re-export is needed.
- `crates/smelt-core/src/metadata.rs` — `ModelMetadata.generates`, `FileMetadata::Generator`, `MetadataError::GeneratesUnknownValue` / `GeneratesMixedWithBareModel`, `extract_generator`, accompanying tests.
- `crates/smelt-core/src/lib.rs` — re-exports only.
- `crates/smelt-db/src/diagnostics_types.rs` — ten `DiagnosticCode` variants and `Display` impl entries; accompanying test.
- `crates/smelt-types/src/signatures.rs::tests`, `crates/smelt-core/src/metadata.rs::tests`, `crates/smelt-db/src/diagnostics_types.rs::tests` — the unit tests listed above.

**Docs touched.**

- None new in this phase (code-only; the spec is already authored).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `SmeltType::ModelDef` addition is non-breaking — `cargo clippy --all-targets` finds no exhaustive-match panics in any crate.
- [ ] `MODEL_DEF_FIELDS` is the single source of truth for the closed five-field set; no field name appears as a string literal outside the registry definition, the message templates, and the validator's `model_def_field_type` walk.
- [ ] `ModelDef` is distinguishable from a structurally-equal `Record{…}` in `is_subtype_of` (the dedicated test passes; widening either direction returns `false`).
- [ ] `FileMetadata::Generator` variant addition is non-breaking — `cargo clippy --all-targets` finds no exhaustive-match panics across `smelt-core`, `smelt-cli`, `smelt-planner`, `smelt-db`, `smelt-lsp`.
- [ ] `ModelMetadata.generates` is `Option<String>` (not a bool) so that `GeneratesUnknownValue { value: ... }` carries the offending string for the diagnostic message.
- [ ] `MetadataError::GeneratesMixedWithBareModel` carries which mutual-exclusivity rule fired (name field vs Layer-1 delimiter) so Phase 2's parser dispatch can produce the spec's distinct diagnostic message shapes.
- [ ] `extract_generator` does not run the bare-SELECT parser on the body — that's Phase 2 territory (the body is a meta-language expression, not SQL).
- [ ] All ten diagnostic codes register and render per spec message format.
- [ ] No Salsa imports added to `smelt-types` or `smelt-core::metadata`; no analysis logic added to `smelt-parser`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E2): SmeltType::ModelDef + FileMetadata::Generator + diagnostic codes (pure)`

---

### Phase 2: Parser dispatch — generator file body shape + frontmatter validation + bare-SELECT forbid

**Goal.** Wire the parser to dispatch generator files (those with `FileMetadata::Generator`) to the meta-language expression parser instead of the SQL `SELECT` parser. Emit `GenerateFileBareSelectForbidden` when a generator file's body starts with a SQL `SELECT` / `WITH` / `VALUES` keyword at the top level. Emit `GeneratesUnknownValue` and `GeneratesMixedWithBareModel` from the Phase 1 `MetadataError` variants by routing the metadata-extraction errors into the Salsa file-parsing query so they surface as standard diagnostics at the offending frontmatter token. No type-inference yet (Phase 3). No Salsa orchestration of generator emission yet (Phase 4); just the per-file parse routing and the metadata-error → diagnostic bridge.

**Pre-conditions.** Phase 1 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/queries/parse.rs::tests::parse_generator_file_routes_to_meta_language_body` — given a fixture file with `---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT * FROM orders }]`, the file parses to a CST whose body root is a `LIST_LITERAL` containing one `RECORD_LITERAL` (the `ModelDef`), not a SQL `SELECT_STATEMENT`. Zero parser-emitted errors.
- `crates/smelt-db/src/queries/parse.rs::tests::parse_generator_file_with_hof_chain_in_body` — a generator file whose body is `smelt.config.load_yaml('c.yaml', Cohort) |> map(fn c => ModelDef { name: c.name, body: SELECT * FROM orders })` parses to a `PIPE_EXPR` containing the loader call and a `map` HOF; the lambda body is a `RECORD_LITERAL`.
- `crates/smelt-db/src/queries/parse.rs::tests::parse_generator_file_with_top_level_select_emits_bare_select_forbidden` — given `---\ngenerates: models\n---\nSELECT * FROM orders`, `file_diagnostics(...)` returns one `GenerateFileBareSelectForbidden` anchored at the `SELECT` keyword's CST span. The body's CST is best-effort recovered (e.g. parses as the bare SELECT and stamps the diagnostic at the keyword); downstream type-checking does not panic.
- `crates/smelt-db/src/queries/parse.rs::tests::parse_generator_file_with_top_level_with_emits_bare_select_forbidden` — same as above but for `WITH cte AS (SELECT 1) SELECT * FROM cte`. The `WITH` keyword anchors the diagnostic.
- `crates/smelt-db/src/queries/parse.rs::tests::parse_generator_file_with_top_level_values_emits_bare_select_forbidden` — same for `VALUES (1), (2)`.
- `crates/smelt-db/src/queries/parse.rs::tests::generates_unknown_value_surfaces_as_diagnostic` — a file with `---\ngenerates: views\n---\n[]` produces a `GeneratesUnknownValue` diagnostic anchored at the `views` token (line/column from the `MetadataError`).
- `crates/smelt-db/src/queries/parse.rs::tests::generates_mixed_with_name_field_surfaces_as_diagnostic` — a file with `---\ngenerates: models\nname: foo\n---\n[]` produces a `GeneratesMixedWithBareModel` diagnostic anchored at the `name:` key.
- `crates/smelt-db/src/queries/parse.rs::tests::generates_mixed_with_section_delimiter_surfaces_as_diagnostic` — a file with `---\ngenerates: models\n---\n--- name: foo ---\nSELECT 1` produces a `GeneratesMixedWithBareModel` diagnostic anchored at the offending Layer-1 delimiter line.
- `crates/smelt-db/src/queries/parse.rs::tests::non_generator_file_parses_unchanged` — regression: a `Single` / `Multi` / `Empty` file's parse and diagnostic output is byte-identical to pre-E2 behaviour.
- `crates/smelt-db/src/queries/parse.rs::tests::generator_file_body_offset_is_consumed_correctly` — given a generator file with multi-line frontmatter, the parser consumes from `body_offset` onward; line/column information in diagnostics correctly resolves to the post-frontmatter span. Verifies the `body_offset` field from Phase 1's `FileMetadata::Generator` is plumbed through.

**Implementation shape.**

- `crates/smelt-db/src/queries/parse.rs`:
  - Extend the parse dispatch (the function that decides whether to call `smelt-parser::parse_sql` or the meta-language parser) to inspect `FileMetadata::Generator` and route to the meta-language expression parser. The meta-language expression parser already exists from Phase B (HOF chains, list literals, lambdas, pipe) — Phase 2 reuses it for generator bodies.
  - Bare-SELECT detection: a meta-language parser invocation that finds a top-level `SELECT` / `WITH` / `VALUES` keyword at the body's first token emits `GenerateFileBareSelectForbidden`. Implementation routes via the existing parser's first-token peek; the keyword set lives in a shared constant `BARE_SQL_KEYWORDS`.
  - Metadata-error bridging: `extract_file_metadata` returns `MetadataError::GeneratesUnknownValue` / `GeneratesMixedWithBareModel`; the parse query catches these and translates them into `DiagnosticSentinel` values with the appropriate `DiagnosticCode` variant and the spans carried on the error. The diagnostic flows through the existing `file_diagnostics` aggregation in `crates/smelt-db/src/queries/check_types.rs`.
- `crates/smelt-parser/src/parser.rs`:
  - If the meta-language expression parser needs a "start at this byte offset" entry point not previously exposed (because earlier phases parse from the file start), expose it as `parse_meta_expression_from_offset(text: &str, offset: usize) -> SyntaxNode`. The offset comes from `FileMetadata::Generator.body_offset`.
  - No new tokens introduced.
- `crates/smelt-db/src/queries/check_types.rs`:
  - `file_diagnostics` aggregator includes the generator-file diagnostics from the parse query. The aggregation already collects parser-emitted diagnostics; this phase ensures the new codes are surfaced.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/queries/parse.rs` — file-parse dispatch, bare-SELECT detection, metadata-error → diagnostic bridge.
- `crates/smelt-parser/src/parser.rs` — only if a new entry point is required for offset-based meta-expression parsing (likely already present from Phase B; add only if missing).
- `crates/smelt-db/src/queries/check_types.rs` — only if the diagnostic aggregator needs adjusting for the new codes (likely no change beyond the existing pattern).
- `crates/smelt-db/src/queries/parse.rs::tests` — the unit tests above.

**Docs touched.**

- None (parser surface is internal; user-visible surface lands in Phase 6).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] Generator-file parse dispatch is gated on `FileMetadata::Generator`; non-generator files take the SQL `SELECT` / `WITH` parser path unchanged (regression test passes).
- [ ] `GenerateFileBareSelectForbidden` anchors at the offending `SELECT` / `WITH` / `VALUES` keyword's CST span, not at the file's first byte.
- [ ] `GeneratesUnknownValue` and `GeneratesMixedWithBareModel` diagnostics' spans match the frontmatter value / key positions from the `MetadataError` variants (the line/column carried on the error).
- [ ] No new lexer tokens introduced.
- [ ] No regression in `Single` / `Multi` / `Empty` parsing (regression test passes; example-diagnostics gate is green for `examples/timeseries/`, `examples/retail_analytics/`, every other pre-E2 example).
- [ ] Parser-level error recovery: a generator file with a parse error mid-body produces a partial CST and a parser-emitted error diagnostic, not a panic; the surrounding workspace continues to parse.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E2): parser dispatch for generator files + bare-SELECT forbid + frontmatter diagnostics`

---

### Phase 3: Type inference — `ModelDef` record-literal validation + generator-body type-check + reflection forbid (pure)

**Goal.** Wire pure type-inference for `ModelDef` record literals, generator file body type-checking against `List<ModelDef>`, and the generator-body `smelt.models.*` forbid. All work lives in a new `crates/smelt-db/src/type_inference/multi_model.rs` module plus dispatch additions in `crates/smelt-db/src/type_inference/{dispatch,record}.rs`. No Salsa orchestration of workspace shape yet (Phase 4); just the per-file inference layer that consumes resolved workspace state as parameters on `TypeContext`. Real workspace integration (the `is_inside_generator_file` flag and the hand-authored-models-set used by literal `smelt.<path>` resolution inside generator bodies) is plumbed in through `TypeContext` extensions; the Salsa queries that supply them are Phase 4.

**Pre-conditions.** Phases 1–2 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_against_target_emits_no_diagnostic_on_happy_path` — a `ModelDef { name: 'us_west', body: SELECT * FROM orders, materialization: 'view', tags: ['cohort'], description: 'US west cohort' }` literal inside a generator file's body emits zero diagnostics; the synthesised type is `SmeltType::ModelDef`; every field's value is validated against its `MODEL_DEF_FIELDS` declared type.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_with_only_required_fields_applies_defaults` — `ModelDef { name: 'us_west', body: SELECT 1 }` (omitting the three optional fields) emits zero diagnostics; the constructed value carries `materialization == 'view'`, `tags == []`, `description == ''`.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_missing_required_name_emits_record_field_missing` — `ModelDef { body: SELECT 1 }` emits one `RecordFieldMissing` anchored at the literal's closing brace, naming `name`.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_missing_required_body_emits_record_field_missing` — `ModelDef { name: 'us_west' }` emits one `RecordFieldMissing` for `body`.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_unknown_field_emits_record_field_unknown` — `ModelDef { name: 'x', body: SELECT 1, owner: 'team' }` emits one `RecordFieldUnknown` anchored at the `owner` token (`owner` is not in the closed five-field set; per spec rule "additional fields require a future spec edit"). The unknown field is dropped from the constructed record.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_invalid_name_chars_emits_diagnostic` — `ModelDef { name: 'us.west', body: SELECT 1 }` emits one `ModelDefInvalidName` anchored at the `'us.west'` value expression (path-safe characters are ASCII alphanumerics and underscore; dots/slashes/whitespace are rejected). Same for `''`, `'foo bar'`, `'foo/bar'`.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_invalid_materialization_emits_diagnostic` — `ModelDef { name: 'us_west', body: SELECT 1, materialization: 'ephemeral' }` emits one `ModelDefInvalidMaterialization` anchored at the `'ephemeral'` value expression (the closed set is `{'view', 'table', 'incremental'}`).
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_model_def_literal_outside_generator_emits_diagnostic` — `[ModelDef { name: 'x', body: SELECT 1 }]` evaluated inside a non-generator file's `smelt.define` body emits one `ModelDefOutsideGeneratorFile` anchored at the `ModelDef`'s opening brace; the literal evaluates as `SmeltType::ModelDef` (recoverable for follow-on checks but cannot escape into a non-generator-emission context).
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_generator_file_body_with_list_of_model_def_literals_synthesises_list_model_def` — the parsed body `[ModelDef {…}, ModelDef {…}]` synthesises `List<ModelDef>`; zero diagnostics on happy-path literals.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_generator_file_body_with_hof_chain_synthesises_list_model_def` — the body `smelt.config.load_yaml('c.yaml', Cohort) |> map(fn c => ModelDef { name: c.name, body: SELECT * FROM smelt.orders WHERE region = c.region })` synthesises `List<ModelDef>` when the loader returns `List<Cohort>` and the lambda body's `ModelDef` literal validates.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_generator_file_body_type_mismatch_emits_diagnostic` — a body whose synthesised type is `List<Cohort>` (not `List<ModelDef>`) emits one `GenerateFileBodyTypeError` anchored at the body's top-level CST node.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::infer_generator_file_body_with_empty_list_admits_zero_emissions` — the body `[]` (with a target type inferable from `List<ModelDef>`) synthesises `List<ModelDef>`; zero diagnostics; the file emits zero models.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::generator_body_smelt_models_with_tag_emits_reflection_forbid` — the body `smelt.models.with_tag('cohort') |> reduce(union_all) |> map(fn _ => ModelDef { name: 'x', body: SELECT 1 })` emits one `GeneratorBodyForbidsModelReflection` anchored at the `smelt.models.with_tag` call site; the body continues to type-check (recoverable).
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::generator_body_smelt_models_all_emits_reflection_forbid` — same with `smelt.models.all`; same anchor pattern.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::generator_body_smelt_sources_admits` — `smelt.sources.with_tag('staging') |> map(fn s => ModelDef {…})` does NOT emit `GeneratorBodyForbidsModelReflection` (sources are loader-time per spec); the body type-checks normally.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::generator_body_loader_call_admits` — a body using `smelt.config.load_yaml('cohorts.yaml', Cohort) |> map(fn c => ModelDef {…})` admits the loader call.
- `crates/smelt-db/src/type_inference/multi_model.rs::tests::generator_body_literal_smelt_path_to_hand_authored_admits` — a body referencing `smelt.staging.orders` (a hand-authored model) admits the reference; the workspace shape passed to the type context exposes `staging.orders` as a hand-authored model so resolution succeeds.

**Implementation shape.**

- `crates/smelt-db/src/type_inference/multi_model.rs` (new module):
  - `pub fn infer_model_def_literal(literal: &RecordLiteral, ctx: &TypeContext, expected: Option<&SmeltType>) -> ModelDefLiteralInferResult` — bidirectional. Walks the literal's five fields, validates each against `MODEL_DEF_FIELDS` (required/missing/unknown/duplicate/type-mismatch via the same per-field diagnostic codes the record path emits, except substituting `RecordFieldUnknown` for unknown fields and adding `ModelDefInvalidName` / `ModelDefInvalidMaterialization` for value-validity checks). Applies defaults for omitted optional fields per spec semantics rule 3. Emits `ModelDefOutsideGeneratorFile` when `ctx.is_inside_generator_file == false` and the literal is the construction site of a `ModelDef`.
  - `pub fn infer_generator_file_body(body: &Expr, ctx: &TypeContext) -> GeneratorBodyInferResult` — synthesises the body's type, validates against `List<ModelDef>`, emits `GenerateFileBodyTypeError` on mismatch. The body's evaluation context sets `ctx.is_inside_generator_file = true` and `ctx.workspace_shape_includes_generators = false` (the W2 stage rule).
  - `pub fn check_generator_body_reflection_forbid(body_ast: &SyntaxNode, ctx: &TypeContext) -> Vec<DiagnosticSentinel>` — walks the AST looking for `smelt.models.with_tag` / `smelt.models.all` call expressions; emits one `GeneratorBodyForbidsModelReflection` per occurrence at the call site. Implementation uses the AST visitor pattern already established in `function_body_check.rs`.
  - `pub fn validate_model_def_name(value: &str, span: Span) -> Option<DiagnosticSentinel>` — pure value-validity check per spec rule "name must be non-empty Text of `[A-Za-z0-9_]+`". Returns `Some(ModelDefInvalidName)` on miss.
  - `pub fn validate_model_def_materialization(value: &str, span: Span) -> Option<DiagnosticSentinel>` — pure value-validity check against the closed set `{'view', 'table', 'incremental'}`. Returns `Some(ModelDefInvalidMaterialization)` on miss.
- `crates/smelt-db/src/type_inference/type_context.rs`:
  - Extend `TypeContext` with `pub is_inside_generator_file: bool` (defaults to `false`) and `pub workspace_shape_includes_generators: bool` (defaults to `true` for hand-authored bodies, set to `false` by the Phase 4 W2 evaluator when invoking inference on a generator's body). The two flags gate `ModelDefOutsideGeneratorFile` and the literal-`smelt.<path>`-resolution-against-hand-authored-only rule respectively.
- `crates/smelt-db/src/type_inference/dispatch.rs`:
  - Route `RECORD_LITERAL` nodes whose target type is `SmeltType::ModelDef` to `infer_model_def_literal` instead of the generic `infer_record_literal`. Discrimination by target type at the dispatch entry point.
- `crates/smelt-db/src/type_inference/record.rs`:
  - No structural change; the generic record-literal path continues to handle user-declared `smelt.record` types. The `ModelDef`-specific path is a sibling dispatch in `multi_model.rs`.
- `crates/smelt-db/src/queries/check_types.rs`:
  - Surface the new diagnostic codes in `file_diagnostics` aggregation. The aggregator walks parsed AST nodes; the visitor must recognise `RECORD_LITERAL` whose target type resolves to `ModelDef` and dispatch to the multi_model checker.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/type_inference/multi_model.rs` (new) — pure inference for `ModelDef` literals, generator-body type-check, reflection forbid, value-validity checks.
- `crates/smelt-db/src/type_inference/dispatch.rs` — route `RECORD_LITERAL` to `infer_model_def_literal` when target type is `ModelDef`.
- `crates/smelt-db/src/type_inference/type_context.rs` — `is_inside_generator_file`, `workspace_shape_includes_generators` flags.
- `crates/smelt-db/src/type_inference/mod.rs` — module-export only.
- `crates/smelt-db/src/queries/check_types.rs` — surface the new diagnostics in `file_diagnostics` (the existing aggregator is extended to walk generator bodies).
- `crates/smelt-db/src/type_inference/multi_model.rs::tests` — the unit tests listed above.

**Docs touched.**

- None new in this phase (code-only).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `multi_model.rs` is pure (no `db.` Salsa calls; consumes `TypeContext` parameters); the file passes the `CLAUDE.md` Pure Function Rule.
- [ ] `MODEL_DEF_FIELDS` is consulted as the single source of truth for field validation; no closed-set duplication.
- [ ] Default application for optional fields happens at `ModelDef` literal-construction time per spec semantics rule 3.
- [ ] `ModelDefInvalidName` regex matches exactly `[A-Za-z0-9_]+` non-empty (verified by tests covering empty / dot / slash / whitespace cases).
- [ ] `ModelDefInvalidMaterialization` enforces exactly `{'view', 'table', 'incremental'}` (verified by a negative-case test).
- [ ] `ModelDefOutsideGeneratorFile` fires only when `is_inside_generator_file == false`; setting the flag inside a generator body suppresses it.
- [ ] `GenerateFileBodyTypeError` fires when the body's synthesised type does not unify with `List<ModelDef>`; uses the existing bidirectional-check unification machinery from Phase A.
- [ ] `GeneratorBodyForbidsModelReflection` fires structurally (every `smelt.models.with_tag` / `smelt.models.all` inside a generator body, regardless of whether the call would resolve cleanly against the hand-authored-only set) per spec semantics rule 8.
- [ ] `smelt.sources.*` calls and loader calls inside generator bodies do NOT trigger the forbid (verified by a positive-case test).
- [ ] Drop-on-error for `ModelDef` field diagnostics: a single missing / unknown / duplicate / type-mismatch / invalid-value field does not avalanche follow-on diagnostics within the same literal.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E2): ModelDef literal inference + generator body type-check + reflection forbid (pure)`

---

### Phase 4: Salsa W1–W4 workspace-shape resolution + emission + collision detection + wide-reflection integration

**Goal.** Wire the W1–W4 workspace-shape resolution pipeline as Salsa queries in `crates/smelt-db/src/queries/project.rs` (next to the existing `models_with_tag` / `models_all` queries). W1 discovers generator files via `generator_files()`; W2 evaluates each generator's body via `evaluate_generator(file_path)` (calling the pure inference from Phase 3 with `workspace_shape_includes_generators = false`); W3 materialises the per-file `List<ModelDef>` and detects per-file `ModelDefDuplicateName` and cross-file `ModelDefHandAuthoredCollision`; W4's downstream type-checks see the unioned model set via the extended `models_with_tag` / `models_all` reflection accessors. Each Salsa query is memoised and incrementally invalidated on workspace input changes. Emitted models become first-class participants in `ModelRef` materialisation, with `path` carrying the generator file's workspace-relative path and `name` carrying the `ModelDef.name` value.

**Pre-conditions.** Phases 1–3 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/queries/project.rs::tests::generator_files_returns_only_generator_files` — a workspace with three `.sql` files (one bare-SELECT, one multi-section, one generator-file with `generates: models`) returns the generator file from `generator_files()`; the bare-SELECT and multi-section files are excluded. Salsa cache invalidates on edit to any of the three files.
- `crates/smelt-db/src/queries/project.rs::tests::evaluate_generator_returns_emitted_model_defs_on_happy_path` — a generator file with body `[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'eu', body: SELECT 2 }]` returns `EvaluatedGenerator { emissions: [ModelDef('us_west'), ModelDef('eu')], diagnostics: [] }` from `evaluate_generator(file_path)`.
- `crates/smelt-db/src/queries/project.rs::tests::evaluate_generator_propagates_inference_diagnostics` — a generator file whose body emits `RecordFieldMissing` / `ModelDefInvalidName` / `GeneratorBodyForbidsModelReflection` returns those diagnostics on the `EvaluatedGenerator` value; emissions for invalid `ModelDef` literals are dropped from the emissions list.
- `crates/smelt-db/src/queries/project.rs::tests::evaluate_generator_with_loader_and_hof_chain` — a generator file whose body is `smelt.config.load_yaml('cohorts.yaml', Cohort) |> map(fn c => ModelDef { name: c.name, body: SELECT * FROM orders WHERE region = c.region })` returns one `ModelDef` per cohort-yaml entry; emissions order follows the loader's sorted-by-key order.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_aggregates_across_generator_files` — a workspace with two generator files each emitting two models returns four emitted models from `emitted_models()` in sorted order by `(generator_file_path, modeldef_name)`.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_computes_smelt_path_correctly` — a generator at `models/cohorts.gen.sql` emitting `ModelDef { name: 'us_west', … }` produces an emitted model with smelt path `cohorts.us_west` (the file stem `cohorts` is included; the directory chain joined by dots; the leaf name from `ModelDef.name`). Same for `models/staging/sources.gen.sql` emitting `ModelDef { name: 'orders', … }` → `staging.sources.orders`.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_detects_duplicate_name_within_file` — a generator emitting two `ModelDef` values with `name: 'us_west'` produces a `ModelDefDuplicateName` diagnostic on the second occurrence; only the first occurrence is retained in the emissions list.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_detects_hand_authored_collision` — a generator at `models/cohorts.gen.sql` emits `ModelDef { name: 'us_west' }` (smelt path `cohorts.us_west`); a hand-authored model at `models/cohorts/us_west.sql` exists with the same smelt path; the emission is dropped, and one `ModelDefHandAuthoredCollision` is emitted at the generator's `name` field value expression.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_detects_cross_generator_collision` — two generator files at `models/a.gen.sql` and `models/b.gen.sql` both emit `ModelDef { name: 'cohort' }`. The byte-lexicographically-earlier file (`a.gen.sql`) wins; the later file's emission is dropped, and one `ModelDefHandAuthoredCollision` is emitted at the later file's `name` field value.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_models_empty_list_admits_zero_emissions_no_diagnostic` — a generator file with body `[]` emits zero models with no diagnostic; the file contributes no workspace entries.
- `crates/smelt-db/src/queries/project.rs::tests::models_all_with_generators_includes_generator_emissions` — `models_all_with_generators(workspace)` returns hand-authored models AND generator-emitted models in path-sorted order. Verified by a workspace with two hand-authored and two generator-emitted models.
- `crates/smelt-db/src/queries/project.rs::tests::models_with_tag_includes_generator_emissions` — `models_with_tag('cohort')` returns hand-authored AND generator-emitted models tagged `cohort` (the `ModelDef.tags` field merges with the generator file's frontmatter `tags:` and with `smelt.yml` `models.<emitted_name>.tags` overlay per the existing `Config::get_tags` rule).
- `crates/smelt-db/src/queries/project.rs::tests::generator_body_literal_smelt_path_resolves_against_hand_authored_only` — a generator body containing `smelt.staging.orders` (a hand-authored model) resolves the reference; a generator body containing `smelt.cohorts.us_west` (an emitted-only path from the same generator) fails to resolve (no diagnostic emitted by this layer; the resolution path returns `None` and the surrounding context handles it).
- `crates/smelt-db/src/queries/project.rs::tests::salsa_cache_invalidates_on_generator_file_edit` — editing a generator file's body invalidates `evaluate_generator(file_path)` for that file and `emitted_models()` (which depends on every generator's emissions); does NOT invalidate `evaluate_generator(file_path)` for *other* generator files (each evaluates independently).
- `crates/smelt-db/src/queries/project.rs::tests::salsa_cache_invalidates_on_yaml_dependency_edit` — editing a YAML file consumed by a generator's `smelt.config.load_yaml` call invalidates that generator's `evaluate_generator` result (via the existing `loader_resolved_value` query's downstream chain).
- `crates/smelt-db/src/queries/project.rs::tests::evaluated_generator_deterministic_byte_equal_across_runs` — the same workspace input produces byte-equal `EvaluatedGenerator` values across two cold Salsa runs (no clock, random, or unstable iteration order leaks in).

**Implementation shape.**

- `crates/smelt-db/src/queries/project.rs`:
  - New Salsa query `pub fn generator_files(db: &dyn Database, workspace: Workspace) -> Arc<Vec<FileId>>` — walks `workspace.files()`, filters by `extract_file_metadata(...) == FileMetadata::Generator { .. }`. Memoised at workspace-input granularity.
  - New `EvaluatedGenerator` struct: `pub struct EvaluatedGenerator { pub emissions: Vec<EmittedModelDef>, pub diagnostics: Vec<DiagnosticSentinel> }`.
  - New `EmittedModelDef` struct: `pub struct EmittedModelDef { pub generator_file: PathBuf, pub name: String, pub name_span: TextRange, pub body_text: String, pub body_span: TextRange, pub materialization: String, pub tags: Vec<String>, pub description: String }`. The `name_span` and `body_span` are workspace-relative so goto-def from emitted-model references can resolve back into the generator file's CST.
  - New Salsa query `pub fn evaluate_generator(db: &dyn Database, file: FileId) -> Arc<EvaluatedGenerator>` — parses the file's body, calls Phase 3's `infer_generator_file_body` and `check_generator_body_reflection_forbid`, evaluates the resulting `List<ModelDef>` value (the evaluation walks the body's AST, materialising each `ModelDef` literal into an `EmittedModelDef` value via record-literal field evaluation; the materialisation is pure given the workspace's loaders and sources are already resolved). `TypeContext` is built with `is_inside_generator_file = true` and `workspace_shape_includes_generators = false`. The query does NOT cross-check collisions — that's `emitted_models()`'s job.
  - New Salsa query `pub fn emitted_models(db: &dyn Database, workspace: Workspace) -> Arc<EmittedModelsResult>` where `EmittedModelsResult` carries the survivor emissions list (sorted by smelt path), the discarded emissions list, and the collision diagnostics. Iteration order over `generator_files()` is workspace-relative-path lexicographic per spec semantics rule 10. Per-file `ModelDefDuplicateName` detection happens first (against each generator's own emissions); cross-file `ModelDefHandAuthoredCollision` detection happens next (against the hand-authored models from `models_all_pre_e2(workspace)` and against earlier-iterated generators' survivors).
  - Extend the existing `models_with_tag` / `models_all` queries: introduce a new `models_all_with_generators(workspace)` that returns the union (hand-authored + emitted) sorted ascending by `path` then `name`. Replace `models_with_tag(workspace, tag)`'s and `models_all(workspace)`'s internal computation to call `models_all_with_generators` and filter / project as needed. Existing callers see the unioned result transparently.
  - `models_with_tag` is unchanged at its signature level; emitted-model tags are read from `EmittedModelDef.tags` (which is `ModelDef.tags` from the literal) merged with `smelt.yml` `models.<emitted_name>.tags` via the existing `Config::get_tags` rule. The merge order matches the spec rule "Tags merge with any workspace-level smelt.yml models.<emitted_name>.tags overlays per the existing Config::get_tags rule".
  - The `ModelRefValue` materialiser (`make_model_ref_value`) is extended to accept either a hand-authored `FileId + ModelSection` or an `EmittedModelDef`; the resulting `ModelRefValue` carries `path = <generator file path>` and `name = <ModelDef.name>` for emitted models (per spec semantics rule 7).
- `crates/smelt-db/src/queries/check_types.rs`:
  - `file_diagnostics(workspace, file)` aggregates the emitted_models() collision diagnostics for that file's generator entries — when `file` is a generator file, the aggregator merges Phase 3's per-file diagnostics with the cross-file collision diagnostics computed at `emitted_models()` time.
  - Workspace-wide diagnostics from collision detection are anchored at the offending `name` field's span in the source generator file.
- `crates/smelt-db/src/lib.rs`:
  - Re-exports for the new query function names.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/queries/project.rs` — new Salsa queries (`generator_files`, `evaluate_generator`, `emitted_models`, `models_all_with_generators`); `EvaluatedGenerator`, `EmittedModelDef`, `EmittedModelsResult` types; extension of `make_model_ref_value` to handle emitted models.
- `crates/smelt-db/src/queries/check_types.rs` — collision-diagnostic aggregation into `file_diagnostics`.
- `crates/smelt-db/src/lib.rs` — re-exports only.
- `crates/smelt-db/src/queries/project.rs::tests` — the unit tests listed above.
- `crates/smelt-db/tests/` — integration tests verifying Salsa invalidation behaviour across edits.

**Docs touched.**

- None new in this phase (code-only).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] The W1–W4 pipeline is implemented as four Salsa queries (`generator_files`, `evaluate_generator`, `emitted_models`, `models_all_with_generators`), each with its own memoisation; invalidation tests cover the edit-propagation chain.
- [ ] `evaluate_generator` is per-file; cross-file dependencies are introduced only at `emitted_models()` aggregation time. Editing one generator does NOT invalidate another's `evaluate_generator` result.
- [ ] Iteration order over generators is workspace-relative-path lexicographic (verified by the cross-generator collision test).
- [ ] `models_with_tag` / `models_all` return the unioned set (hand-authored + emitted) in path-sorted order; existing callers see the unioned result transparently.
- [ ] `ModelRefValue` for an emitted model carries `path = <generator file path>` and `name = <ModelDef.name>` per spec semantics rule 7.
- [ ] Per-file `ModelDefDuplicateName` (first occurrence wins) and cross-file `ModelDefHandAuthoredCollision` (hand-authored wins; among generators, lexicographically-earlier wins) are detected at the `name` field's source span.
- [ ] `evaluate_generator` builds the `TypeContext` with `is_inside_generator_file = true` and `workspace_shape_includes_generators = false`; the spec invariants from Phase 3 are honoured at the integration point.
- [ ] Literal `smelt.<path>` references inside generator bodies resolve only against hand-authored models (the resolver excludes generator emissions when `workspace_shape_includes_generators == false`).
- [ ] Determinism: the same workspace input produces byte-equal emission lists across runs (verified by the dedicated test).
- [ ] No `O(workspace)` re-scans on a single-file edit; Salsa caches the per-file query results and only invalidates affected queries.
- [ ] No bypass-of-purity in `type_inference/`: only `queries/project.rs` and `lib.rs` host the Salsa calls; `multi_model.rs` consumes resolved state via `TypeContext`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E2): Salsa W1–W4 generator pipeline + emission + collision + wide-reflection integration`

---

### Phase 5: `<generator>` expansion frame + cross-feature wiring (CLI `origin`, catalog `origin`, model selector, incremental coexistence)

**Goal.** Wire the `<generator>` anonymous expansion frame per `expansion.md` (stamped at the generator file's body range when a generator's body is evaluated); flow it into the frame-stack contract so diagnostics from inside an emitted-`ModelDef`'s HOF chain carry the generator file as the outermost provenance root. Wire the cross-feature touches into the CLI and catalog: the `origin` field on `smelt explain --json` and `smelt docs --json` output schemas; the `generator_file:` model selector method. Wire `ModelDef.materialization: 'incremental'` coexistence with `incremental_models.md`'s rules (the emitted model's `incremental:` block is inherited from the generator file's frontmatter; no per-emission incremental overrides in v1). No LSP work yet (Phase 6); no example fixtures yet (Phase 6).

**Pre-conditions.** Phases 1–4 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/function_body_check.rs::tests::generator_frame_stamps_at_generator_body_range` — a diagnostic surfacing from inside a generator's body HOF chain (e.g. a `RecordFieldMissing` on a `ModelDef` literal nested under a `map(fn c => ModelDef {…})` call) carries a frame stack whose outermost entry has `function = "<generator>"`, `fn_id = None`, `decl_path = Some(<generator file path>)`, `decl_range = None`, `call_site_range = <range of the body's top-level CST node>`, `param = ""`.
- `crates/smelt-db/src/function_body_check.rs::tests::generator_frame_outermost_with_hof_inner_frames` — a diagnostic from inside `smelt.config.load_yaml('c.yaml', Cohort) |> map(fn c => ModelDef { tags: c.bogus, body: SELECT 1, name: c.name })` carries a frame stack with the HOF anonymous frame at `frames[0]` (innermost), the `<generator>` frame at `frames.last()` (outermost). The `RecordFieldUnknown` on `c.bogus` anchors at the offending span; the frame stack propagates the outer-to-inner chain.
- `crates/smelt-cli/tests/explain_json_output.rs::tests::emitted_model_carries_origin_field` — `smelt explain --json --select generator_file:models/cohorts.gen.sql` returns a JSON document where the emitted models' entries include `"origin": {"type": "generated", "generator_file": "models/cohorts.gen.sql", "generator_name": "us_west"}`; hand-authored models omit the `origin` field entirely (skip-serializing-if-none rule).
- `crates/smelt-cli/tests/explain_json_output.rs::tests::hand_authored_model_omits_origin_field` — regression: `smelt explain --json --select staging.orders` (hand-authored) returns no `origin` key.
- `crates/smelt-core/src/selector.rs::tests::generator_file_selector_parses_workspace_relative_path` — `Selector::parse("generator_file:models/cohorts.gen.sql")` returns a `Selector::GeneratorFile { path: PathBuf::from("models/cohorts.gen.sql"), upstream: false, downstream: false }`; `Selector::parse("+generator_file:models/foo.gen.sql+")` parses with both modifiers true.
- `crates/smelt-core/src/selector.rs::tests::generator_file_selector_with_empty_path_emits_parse_error` — `Selector::parse("generator_file:")` returns `SelectorParseError::EmptyPath`.
- `crates/smelt-core/src/selector.rs::tests::generator_file_selector_matches_all_emissions` — a workspace with `models/cohorts.gen.sql` emitting two models (`us_west`, `eu`); the selector matches both emitted models.
- `crates/smelt-core/src/selector.rs::tests::generator_file_selector_against_non_generator_path_matches_nothing` — a selector pointing at a hand-authored `.sql` file or a missing file returns an empty match set (no error).
- `crates/smelt-core/src/selector.rs::tests::generator_file_selector_excludes_collision_losers` — a generator file emits a model whose smelt path collides with a hand-authored model (`ModelDefHandAuthoredCollision`); the `generator_file:` selector for that generator does NOT include the dropped emission in its match set.
- `crates/smelt-cli/tests/docs_json_output.rs::tests::emitted_model_carries_origin_in_docs_json` — `smelt docs --json` output includes the `origin` field on every emitted-model entry, mirroring `smelt explain --json`'s shape.
- `crates/smelt-cli/tests/docs_markdown_output.rs::tests::emitted_model_has_source_line_in_markdown` — the markdown rendering of an emitted model's metadata block includes a `**Source:** models/cohorts.gen.sql (cohort: us_west)` line (or equivalent — the spec leaves the exact wording to the implementation as long as the generator file and the `ModelDef.name` are surfaced).
- `crates/smelt-db/src/queries/project.rs::tests::emitted_incremental_model_inherits_frontmatter_incremental_block` — a generator file whose frontmatter declares `incremental: { partition_column: dt, granularity: day }` and whose body emits `ModelDef { name: 'us_west', body: SELECT * FROM orders, materialization: 'incremental' }` produces a `ModelRefValue` whose downstream incremental-config matches the generator's frontmatter (the file-wide `incremental:` block applies). The existing `incremental_models.md` rules — batch-safety classification, partition-column filter injection, etc. — apply on equal terms.
- `crates/smelt-db/src/queries/project.rs::tests::emitted_non_incremental_model_does_not_inherit_incremental_block` — if the generator's frontmatter declares `incremental: {…}` but the `ModelDef.materialization == 'view'` (the default), the emitted model is a view, not incremental; the `incremental:` block is silently ignored for that emission.

**Implementation shape.**

- `crates/smelt-db/src/function_body_check.rs`:
  - `pub fn make_generator_frame(generator_file_path: &Path, body_range: TextRange) -> Frame` — the producer-side constructor for the `<generator>` frame variant per `expansion.md` Surface. Fields: `function = "<generator>"`, `fn_id = None`, `decl_path = Some(path)`, `decl_range = None`, `call_site_range = body_range`, `param = ""`.
  - Wire `evaluate_generator` (Phase 4) to push the `<generator>` frame onto the frame stack before evaluating the body; pop on completion. The frame stamping uses the existing frame-stack infrastructure; the only addition is the new frame variant and its constructor.
- `crates/smelt-types/src/origin.rs` (or wherever the `Origin` enum currently lives — check the existing `data_catalog.md` schema in code):
  - `pub enum ModelOriginKind { HandAuthored, Generated { generator_file: PathBuf, generator_name: String } }`. Default for hand-authored skips serialization.
- `crates/smelt-cli/src/commands/explain.rs`:
  - Extend the `explain --json` model-entry serializer to include the `origin` field when the model is emitted by a generator. The discriminator comes from the `ModelRefValue` query result (which Phase 4 extended to carry the generator-file path and the emitted name).
- `crates/smelt-cli/src/commands/docs.rs`:
  - Same extension for `smelt docs --json` output. The catalog renderer's Markdown path adds a "Source" line for emitted models pointing at the generator file.
- `crates/smelt-core/src/selector.rs`:
  - Add `Selector::GeneratorFile { path: PathBuf, upstream: bool, downstream: bool }` variant; extend the parser to recognise `generator_file:<path>` after the optional `+` prefix. Empty path returns `SelectorParseError::EmptyPath`.
  - Add a resolver function `fn resolve_generator_file_selector(selector: &Selector, workspace: &Workspace) -> Vec<ModelId>` that looks up `emitted_models()`, filters by `EmittedModelDef.generator_file == selector.path`, and returns the survivor model IDs (collision losers excluded).
  - Wire the new variant into the existing selector evaluation pipeline.
- `crates/smelt-db/src/queries/project.rs`:
  - The `ModelRefValue` for an emitted model already carries the generator-file path from Phase 4. This phase wires the model's `incremental:` block from the generator's file-wide frontmatter (via `EvaluatedGenerator.metadata.incremental`) into the downstream incremental-models machinery. The integration point is in the existing `ModelMetadata`-consuming code path; emitted models present their generator's frontmatter `incremental:` block when `ModelDef.materialization == 'incremental'`.
- `crates/smelt-planner/src/...` or wherever the planner consumes `ModelRefValue`:
  - If the planner's `IncrementalModel` shape needs adjusting to accept emitted models, do it here. Likely a `From<EmittedModelDef + GeneratorFrontmatter>` conversion or an extension to the existing `model_to_incremental_config` helper.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/function_body_check.rs` — `<generator>` frame constructor and frame-stack stamping at generator body evaluation.
- `crates/smelt-types/` — `ModelOriginKind` enum (or extension to existing origin types).
- `crates/smelt-cli/src/commands/explain.rs`, `crates/smelt-cli/src/commands/docs.rs` — `origin` field serialization for emitted models; Markdown "Source" line.
- `crates/smelt-core/src/selector.rs` — `Selector::GeneratorFile` variant + parser + resolver.
- `crates/smelt-db/src/queries/project.rs` — emitted-incremental-model frontmatter inheritance wiring.
- `crates/smelt-planner/src/` — only if the planner's incremental-model shape needs an adjustment for emitted models.
- All accompanying unit and integration tests under the touched crates.

**Docs touched.** *Write spec edits as if the feature has always existed — no `Phase 5` headings, no `(Phase E2)` labels.*

- `docs/specs/expansion.md` Known Divergences — flip the `<generator>` frame "implementation forthcoming" bullet to "landed"; surface is normative in the existing `<generator>` frame entry in §"`FrameInfo` shape".
- `docs/specs/cli.md` Known Divergences — flip the "generator-emitted model `origin` field is normative; implementation forthcoming" bullet to "landed".
- `docs/specs/incremental_models.md` Known Divergences — flip the "Generator-emitted incremental models" bullet to reflect the implemented file-wide-`incremental:`-inheritance reality.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `<generator>` frame is stamped exactly at the body's CST top-level node range (verified by a frame-introspection test).
- [ ] The frame stack contract from `expansion.md` is honoured: outermost is `<generator>`, inner frames are the HOF anonymous frames; the renderer reads `decl_path` and `call_site_range` for the `<generator>` frame.
- [ ] `origin` field in `smelt explain --json` and `smelt docs --json` is omitted for hand-authored models (regression test); populated for emitted models with the spec-declared shape.
- [ ] Markdown rendering surfaces a "Source" line pointing at the generator file for emitted models.
- [ ] `generator_file:` selector parses, resolves against `emitted_models()`, excludes collision losers, returns empty for non-generator paths (no error).
- [ ] Emitted incremental models inherit the generator's frontmatter `incremental:` block; the existing batch-safety classification, filter injection, and DELETE+INSERT execution apply.
- [ ] Emitted non-incremental models do not pick up the generator's `incremental:` block.
- [ ] Spec touches honour the Timeless-oracle rule: no phase vocabulary in `docs/specs/` bodies.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E2): <generator> frame + CLI/catalog origin + generator_file: selector + incremental coexistence`

---

### Phase 6: LSP support + examples (`per_cohort_union`, `staging_from_sources`) + user docs + skill + `/smelt-loop` `large` tier

**Goal.** Wire LSP support for every E2 surface element: hover on `generates:` frontmatter, `ModelDef` literals, `ModelDef` field-value expressions; goto-def from a generator-emitted-model reference to the emitting `ModelDef.name` field's value-token; completion at `generates: <cursor>` and `ModelDef { <cursor> … }` positions; diagnostics-with-frame-stacks for the `<generator>` frame at LSP-render time. Ship the **per-cohort union killer demo** at `examples/per_cohort_union/` (meta-plan §8) and the secondary `examples/staging_from_sources/` example. Add user-doc pages at `docs-site/docs/meta-language/generators.md` + reference-page additions. Ship the `smelt-app-builder` skill reference doc. Extend `/smelt-loop` with a new `large` tier (the loop's execution + skill-diff landing happen in Phase G; Phase E2 ships the fixture and command-line shape).

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-lsp/tests/integration.rs::tests::hover_on_generates_frontmatter_shows_body_type_and_emission_count` — opening `examples/per_cohort_union/models/cohorts.gen.sql` and hovering over `generates: models` shows `List<ModelDef> · 3 emitted models` (the count is statically resolved from the loader-supplied cohorts yaml; the format string is implementer's choice as long as the body type and the emission count are surfaced).
- `crates/smelt-lsp/tests/integration.rs::tests::hover_on_model_def_literal_opening_brace_shows_emitted_path` — hovering on the `{` of a `ModelDef { name: 'us_west', … }` literal in `cohorts.gen.sql` shows the inferred emitted smelt path `cohorts.us_west` (when `name` is statically known); shows just `ModelDef` when `name` is non-static.
- `crates/smelt-lsp/tests/integration.rs::tests::hover_on_model_def_name_field_value_shows_smelt_path` — hovering on `'us_west'` in `name: 'us_west'` shows `cohorts.us_west`.
- `crates/smelt-lsp/tests/integration.rs::tests::hover_on_model_def_body_field_value_shows_table_expr_and_columns` — hovering on the `SELECT * FROM orders` body shows `TableExpr` and the inferred column list when resolvable.
- `crates/smelt-lsp/tests/integration.rs::tests::goto_def_on_emitted_model_reference_resolves_to_model_def_name_field` — a downstream model `models/all_cohorts.sql` containing `smelt.models.with_tag('cohort') |> reduce(union_all)` exposes goto-def from the `with_tag` resolved reference; for an emitted model in the result list, goto-def resolves to the `ModelDef.name` field's value-token in the generator file.
- `crates/smelt-lsp/tests/integration.rs::tests::completion_on_generates_offers_models_only` — typing `generates: <cursor>` in a `.sql` file's frontmatter offers exactly `models` as a single completion entry; no other values.
- `crates/smelt-lsp/tests/integration.rs::tests::completion_on_model_def_field_key_offers_closed_five_field_set` — typing `ModelDef { <cursor>` offers exactly `{name, body, materialization, tags, description}` with required fields (`name`, `body`) surfaced first per the spec's ordering rule.
- `crates/smelt-lsp/tests/integration.rs::tests::diagnostics_with_frame_stack_carry_generator_outer_frame` — a deliberately-broken `ModelDef` literal in a generator body produces a diagnostic whose frame stack ends with a `<generator>` frame pointing at the generator file's body range.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::per_cohort_union_example_has_zero_diagnostics` — the `examples/per_cohort_union/` workspace, with `cohorts.yaml`, `models/cohorts.gen.sql`, `models/all_cohorts.sql`, and `tests/cohort_count.test.sql`, builds cleanly with zero LSP diagnostics across all files.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::staging_from_sources_example_has_zero_diagnostics` — the `examples/staging_from_sources/` workspace builds cleanly with zero LSP diagnostics.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::per_cohort_union_broken_subfixtures_emit_expected_diagnostics` — sub-fixtures under `examples/per_cohort_union/broken/` (one per new diagnostic code, mirroring the pattern from `examples/meta_config/broken/`) each emit exactly the expected diagnostic at the expected span.
- `crates/smelt-cli/tests/cohort_count_acceptance.rs::tests::union_row_count_matches_per_cohort_sum` — the per-cohort-union demo's acceptance test (`tests/cohort_count.test.sql`) asserts the row count of `all_cohorts_unioned` equals the sum of per-cohort row counts. This is the integration test for the whole language end-to-end (Phase B reducers + Phase C reflection + Phase E1 records + Phase E2 multi-model production composed).

**Implementation shape.**

- `crates/smelt-lsp/src/hover.rs`:
  - `pub fn hover_text_for_generates_frontmatter(file: FileId, db: &dyn Database) -> Option<HoverText>` — pure helper consulting `evaluate_generator(file)` for the body type and the emissions count.
  - `pub fn hover_text_for_model_def_literal_open_brace(...) -> Option<HoverText>` — pure helper consulting the literal's evaluated `ModelDef` value (the `name` field's static-resolution path).
  - `pub fn hover_text_for_model_def_name_field_value(...) -> Option<HoverText>` — pure helper that combines the literal's `name` with the generator-file path-component logic.
  - `pub fn hover_text_for_model_def_body_field_value(...) -> Option<HoverText>` — pure helper consulting the body's inferred `TableExpr` schema.
  - Wire into `Backend::hover`'s dispatch by detecting the cursor's parent CST node kind (`FRONTMATTER_KEY`, `RECORD_LITERAL` open brace, `RECORD_FIELD_VALUE` under a `ModelDef` literal).
- `crates/smelt-lsp/src/goto_def.rs`:
  - `pub fn goto_def_for_emitted_model_reference(...) -> Option<Location>` — pure helper that resolves a generator-emitted model reference (encountered as a `ModelRef` value in a reflection HOF chain, or as a literal `smelt.<path>` at a consumer site referring to an emitted path) to the `ModelDef.name` field's value-token in the generator file.
  - Wire into `Backend::goto_definition`'s dispatch.
- `crates/smelt-lsp/src/completion.rs`:
  - `pub fn completion_for_generates_value(...) -> Vec<CompletionItem>` — single entry `models` with label and documentation.
  - `pub fn completion_for_model_def_field_key(existing_fields: &[String]) -> Vec<CompletionItem>` — returns the closed five-field set minus already-present fields; required fields surfaced first per the spec rule.
  - Wire into `Backend::completion`'s dispatch.
- `crates/smelt-lsp/src/diagnostics.rs` (or wherever frame-stack rendering lives):
  - Extend the renderer to read `decl_path` and `call_site_range` from `<generator>` frames; renders the outermost-frame trailer as `(generator: <relative path>)` in the diagnostic message.
- `examples/per_cohort_union/` (new workspace):
  - `smelt.yml` minimal config.
  - `cohorts.yaml`: `[{name: us_west, region: 'us-west-2', min_revenue: 100}, {name: us_east, region: 'us-east-1', min_revenue: 100}, {name: eu, region: 'eu-west-1', min_revenue: 50}]`.
  - `models/cohorts.gen.sql`:
    ```
    ---
    generates: models
    tags: [cohort]
    ---
    smelt.record Cohort = { name: Text, region: Text, min_revenue: Integer }

    smelt.config.load_yaml('cohorts.yaml', Cohort)
      |> map(fn c => ModelDef {
           name: c.name,
           body: SELECT * FROM smelt.orders
                 WHERE region = c.region AND revenue >= c.min_revenue
         })
    ```
  - `models/orders.sql` — hand-authored model providing the upstream `orders` table; a synthetic test fixture with three regions / six rows.
  - `models/all_cohorts_unioned.sql`:
    ```sql
    smelt.models.with_tag('cohort') |> reduce(union_all)
    ```
  - `tests/cohort_count.test.sql`:
    ```sql
    ---
    materialization: test
    ---
    SELECT
      (SELECT COUNT(*) FROM smelt.all_cohorts_unioned)
      = (SELECT SUM(c.count) FROM (
          SELECT COUNT(*) AS count FROM smelt.orders WHERE region = 'us-west-2' AND revenue >= 100
          UNION ALL SELECT COUNT(*) FROM smelt.orders WHERE region = 'us-east-1' AND revenue >= 100
          UNION ALL SELECT COUNT(*) FROM smelt.orders WHERE region = 'eu-west-1' AND revenue >= 50
        ) c) AS passes
    ```
  - `broken/` sub-fixtures, one per new diagnostic code: `broken/generates_unknown_value/`, `broken/generates_mixed_with_name_field/`, `broken/generates_mixed_with_section_delimiter/`, `broken/generate_file_bare_select_forbidden/`, `broken/generate_file_body_type_error/`, `broken/model_def_outside_generator_file/`, `broken/model_def_invalid_name/`, `broken/model_def_invalid_materialization/`, `broken/model_def_duplicate_name/`, `broken/model_def_hand_authored_collision/`, `broken/generator_body_forbids_model_reflection/`. Each sub-fixture is a minimal workspace exhibiting exactly the diagnostic; the `example_diagnostics` test asserts the diagnostic emits at the expected span.
- `examples/staging_from_sources/` (new workspace):
  - `sources/raw_db.yml` declaring three source tables.
  - `models/staging.gen.sql` — a generator file whose body iterates the workspace's sources via `smelt.sources.with_tag('raw')` and emits one staging model per source.
  - Minimal smoke fixture; one acceptance test asserting the count of emitted staging models.
- `docs-site/docs/meta-language/generators.md` (new):
  - Concept overview of the `generates: models` directive.
  - The closed `ModelDef` field set (table form).
  - The per-cohort-union worked example (citing the demo's file paths).
  - The W1–W4 workspace-shape resolution rules (high-level prose).
  - Frontmatter inheritance rules.
  - Generator-body reflection forbid + the alternative (`smelt.sources.*`).
  - All ten new diagnostic codes (table form).
- `docs-site/docs/meta-language/index.md`:
  - Add multi-model production to the concept overview as a peer of records/maps/loaders.
- `docs-site/docs/meta-language/reflection.md`:
  - Add a section noting that `smelt.models.*` is forbidden inside generator bodies; document the `smelt.sources.*` alternative.
- `docs-site/docs/meta-language/reference.md`:
  - Add alphabetised entries for `generates: models`, `ModelDef`, the five `ModelDef` fields, the `<generator>` frame, the `generator_file:` selector, the `origin` field in CLI / catalog output. Include type signatures and minimal examples.
- `.claude/skills/smelt-app-builder/references/20260515-meta-multi-model-production.md` (new):
  - Short reference (workflow gotchas only): when to choose a generator file over multi-section files; the `generates:` + `name:` mutual exclusivity gotcha; the file-stem-in-path gotcha; the smelt.models.* forbid + sources.* alternative; the killer-demo pattern.
  - Body stays short (point at the user docs for syntax).
- `.claude/commands/smelt-loop.md`:
  - Add a new `large` tier with an instruction shape requiring the agent to:
    1. Author a generator file emitting one model per row in a small YAML config.
    2. Author a downstream model that unions the emissions via `smelt.models.with_tag(…) |> reduce(union_all)`.
    3. Author a test asserting an acceptance property of the union.
  - Tier metadata: success criteria, fixture path, expected diagnostic count (zero for the happy path).
  - The loop's *execution* runs in Phase G; Phase E2 only ships the tier fixture and the command's tier metadata.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-lsp/src/hover.rs`, `crates/smelt-lsp/src/goto_def.rs`, `crates/smelt-lsp/src/completion.rs`, `crates/smelt-lsp/src/diagnostics.rs`, `crates/smelt-lsp/src/backend.rs` — LSP pure helpers + backend dispatch + frame-stack renderer extension.
- `examples/per_cohort_union/` (new) and `examples/staging_from_sources/` (new) — full workspaces plus broken sub-fixtures.
- `docs-site/docs/meta-language/{generators.md,index.md,reflection.md,reference.md}` — new and edited user-doc pages.
- `.claude/skills/smelt-app-builder/references/20260515-meta-multi-model-production.md` — new skill reference.
- `.claude/commands/smelt-loop.md` — new `large` tier fixture metadata.
- `crates/smelt-lsp/tests/integration.rs`, `crates/smelt-cli/tests/example_diagnostics.rs`, `crates/smelt-cli/tests/cohort_count_acceptance.rs` — accompanying integration tests.

**Docs touched.** *Write as if the feature has always existed — no `Phase 6` headings, no `(Phase E2)` labels, no `[deferred to Phase E2]` callouts.*

- `docs-site/docs/meta-language/generators.md` (new) — full generators reference; killer-demo worked example; W1–W4 prose-style overview.
- `docs-site/docs/meta-language/index.md` — concept overview adds multi-model production as a peer of records/maps/loaders.
- `docs-site/docs/meta-language/reflection.md` — generator-body reflection forbid + `smelt.sources.*` alternative.
- `docs-site/docs/meta-language/reference.md` — alphabetised additions for every new construct.
- `docs/specs/meta_language.md` Known Divergences — flip the "Multi-model production is normative; implementation is forthcoming" bullet to "Multi-model production is implemented and shipping; the W1–W4 pipeline, the ten diagnostics, the `<generator>` frame, and the LSP support are all live"; keep the "Frontmatter as a meta-evaluable value" open question.
- `docs/specs/data_catalog.md` — promote the "Generator-emitted model provenance" implementation-forthcoming references (if any) to "landed"; the surface itself is already in place.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] LSP hover, goto-def, completion, and diagnostics dispatchers consult the pure helper functions per the established Phase C/D pattern; helpers are unit-tested separately.
- [ ] LSP integration tests cover every Phase E2 surface element (hover on every position, goto-def, completion at every position, frame-stack diagnostics).
- [ ] `examples/per_cohort_union/` is a real-fixture workspace that builds and passes acceptance tests; the demo exercises Phase B reducers + Phase C reflection + Phase E1 records + Phase E2 multi-model production end-to-end (per meta-plan §8).
- [ ] `examples/staging_from_sources/` exercises the `smelt.sources.*`-driven generator pattern.
- [ ] Every new Phase E2 diagnostic code has a corresponding `broken/` sub-fixture under `examples/per_cohort_union/broken/` that emits exactly that diagnostic at the expected span.
- [ ] User-doc edits read as timeless feature descriptions (no `Phase E2` headings, no `(Phase 6)` inline labels); reference page is alphabetised and complete.
- [ ] `smelt-app-builder` skill reference is workflow-focused (gotchas, not syntax restated).
- [ ] `/smelt-loop` `large` tier metadata is in place; the tier's *execution* (and resulting skill-diff landing) is explicitly deferred to Phase G.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test integration` all green.

**Commit.** `feat(meta-language-E2): LSP + examples/per_cohort_union + examples/staging_from_sources + user docs + skill + /smelt-loop large tier`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase E2 applicable expert reviewer from meta-plan §5 over the Phase E2 diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below.

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase E2 subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **expansion-expert** | sonnet | `crates/smelt-db/src/function_body_check.rs` + `docs/specs/expansion.md` | `<generator>` frame stamping discipline — frame populates `decl_path` + `call_site_range` per the contract; outermost-frame ordering is honoured; multi-model production preserves `Caller`/`Callee` provenance through HOF chains anchored under the `<generator>` frame; `model_origin` extension producer-side wiring is correct. |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/{hover,goto_def,completion,diagnostics,backend}.rs` + the integration tests | Hover types correct for every Phase E2 position; goto-def into the `ModelDef.name` field's value-token resolves to the right source span in the generator file; completion in `ModelDef { … }` and `generates: …` positions; rename-not-supported (Phase G) gracefully reported; frame-stack diagnostics render the `<generator>` trailer. |
| **salsa-expert** | sonnet | `crates/smelt-db/src/queries/project.rs` (W1–W4 queries); `crates/smelt-db/src/queries/check_types.rs` (collision aggregation) | Correct memoisation of `generator_files()`, `evaluate_generator(file)`, `emitted_models()`, `models_all_with_generators()`; per-file `evaluate_generator` is not invalidated by edits to other generator files; cross-file collision detection runs in O(N) over the workspace, not O(N²); no accidental O(workspace) scans on a single-file edit; the chain `loader_file_text → loader_resolved_value → evaluate_generator → emitted_models` invalidates correctly when an upstream YAML changes. |
| **examples-curator** | haiku | `examples/per_cohort_union/`, `examples/staging_from_sources/` | Minimal-but-realistic; not contrived; both build cleanly with zero LSP diagnostics; the per-cohort-union acceptance test exercises Phase B reducers + Phase C reflection + Phase E1 records + Phase E2 multi-model production as a single end-to-end demo (meta-plan §8); every new Phase E2 diagnostic code has a `broken/` sub-fixture. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{generators.md,index.md,reflection.md,reference.md}` plus the spec edits in `docs/specs/{meta_language,expansion,cli,incremental_models}.md` | User docs match Surface section of `meta_language.md` exactly; no syntax in docs that isn't speced; reference page alphabetical and complete; Timeless-oracle rule honoured (no phase vocabulary in spec or user-doc bodies); flips on the "implementation forthcoming" bullets were made appropriately. |
| **cross-feature-impact-reviewer** | sonnet | meta-plan §6 cross-feature implication table | After this phase, confirm the cross-feature impact list is complete: `expansion.md`, `architecture.md`, `model_selection.md`, `incremental_models.md`, `python_models.md`, `data_catalog.md`, `schema_evolution.md`, `cli.md`, `datagen.md` touches all landed correctly; the planner-integration row (no change required) is verified; no other specs need touches that haven't been made. |

**Loop discipline.**

1. **Round 1.** Dispatch all six experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-E2.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md` §"Multi-model production"; plus the cross-feature spec sections in scope for the expert).
   - The exact file scope from the table above (the allowlist).
   - The diff range to review (`git log --oneline <phase-base>..HEAD`, where `<phase-base>` is the commit prior to Phase 1 — `b9df522`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: numbered list of findings with `file:line` refs, or "no material findings".
2. **Address findings.** For each expert that returns material findings:
   - Mechanical fix (≤~30 lines, single concern) → edit directly.
   - Non-trivial fix → dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-E2): address {expert-name} feedback`. Push immediately.
3. **Re-dispatch** only the expert whose findings were addressed, providing the round-1 prompt plus a diff of what changed. "No material findings" → that expert is **clean** and exits the loop.
4. **Repeat** step 2 → step 3 until every expert is clean.
5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7).
   - An expert's findings would force a non-trivial spec change — pause for the user.
   - A fix surfaces a pre-existing failure unrelated to this phase.
   - A cross-feature-impact reviewer finding identifies a spec that needs a normative edit not yet made; pause and run `/smelt:spec` first.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus this plan file (to record round counts).

**Review checklist** (material findings only — applied to the expert-dispatch *process*):

- [ ] All six experts (expansion, lsp, salsa, examples-curator, docs-reviewer, cross-feature-impact) were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded under "Deferred during implementation".
- [ ] No fix touched files outside the dispatching expert's scope.
- [ ] No expert ran more than 3 rounds; if any did, autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] All cargo checks green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation":

> Phase 7 expert review: expansion-expert clean (R{n}), lsp-expert clean (R{n}), salsa-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}), cross-feature-impact-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-E2): address {expert-name} feedback`. If round 1 came back clean, no commit for that expert. The acceptance-gate summary lands in the next commit (or in `chore(meta-language-E2): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo test -p smelt-cli --test example_diagnostics` — `examples/per_cohort_union/` and `examples/staging_from_sources/` (happy + broken sub-fixtures) all gate green.
- `cargo test -p smelt-cli --test cohort_count_acceptance` — the per-cohort-union demo's row-count acceptance test passes.
- `cargo test -p smelt-lsp --test integration` — every LSP-integration test for Phase E2 surface elements passes.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` workspace-wide all green.
- `/smelt:validate meta_language` reports zero drift against `docs/specs/meta_language.md` for the multi-model-production surface, semantics, design, invariants, and Known Divergences entries.
- `/smelt:validate` against each cross-feature spec touched (`expansion.md`, `architecture.md`, `cli.md`, `data_catalog.md`, `model_selection.md`, `incremental_models.md`) reports zero drift on the E2-touched sections.
- Manual smoke: open `examples/per_cohort_union/models/cohorts.gen.sql` in an LSP-enabled editor; verify hover on `generates: models`, on a `ModelDef` literal's `{`, on `ModelDef.name` / `body` shows the expected text; verify goto-def from a `with_tag('cohort')` reflection chain resolves into the generator file's `ModelDef.name`; verify completion at `generates: <cursor>` offers `models`.
- The phase status row in `docs/plans/20260509-meta-language-overall.md` is updated from `pending` to `done` with the date and final commit hash. Push.
- Emit `<<PHASE_COMPLETE>>` per meta-plan sentinel emission contract.
