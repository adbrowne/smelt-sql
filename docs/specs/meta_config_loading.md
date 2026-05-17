---
feature: meta_config_loading
status: experimental
last_reviewed: 2026-05-14
owners: [andrew]
---

# Meta Config Loading

> **What this is.** A normative spec for the file-loader family that supplies typed meta-world values from disk: `smelt.config.load_yaml(path, schema)` and `smelt.config.load_json(path, schema)` (TOML is reserved but post-v1). Each loader takes a workspace-relative path and a meta-language schema (inline record type, named `smelt.record` declaration, `Map<K, V>` over such a schema, or `List<T>` of such a schema) and returns a meta-world value of the schema's declared type. In scope: the loader API surface, schema-driven validation diagnostics with source-span retention through to the YAML/JSON row that violated the schema, determinism guarantees (workspace-relative paths, no network, no clock, no environment access), and per-target file-overlay strategy. Out of scope: the meta-language constructs that consume loaded values — `List<T>`, HOFs, records, `Map<K, V>`, multi-model production (see `meta_language.md`); the `vars:` block in `smelt.yml` accessed via `smelt.config.var(name)` (see `meta_language.md`); the planner's use of generated models (see `planner_integration.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.

## Surface

### Loader calls

The loader family exposes two calls in v1, one per supported file format:

- `smelt.config.load_yaml(path: Text, schema: Schema) -> Schema`
- `smelt.config.load_json(path: Text, schema: Schema) -> Schema`

`smelt.config.load_toml(path, schema)` is **reserved**: parsing or naming the call site emits `ConfigLoaderTomlNotYetSupported` at the call expression. The name is reserved to prevent users from binding it in a `smelt.define` declaration and to make the future addition a non-breaking change.

`path` must be a string literal at the call site. A non-literal `path` (a variable, an expression) emits `ConfigLoaderPathNotLiteral` at the argument expression. The literal-only rule matches `smelt.config.var('x')`'s argument discipline (`ConfigVarNameNotLiteral`) and pins the Salsa input registration to the parse-time string.

### Schema authoring

`Schema` is any of:

- **Inline record type:** `{name: Text, columns: List<Text>}` — a brace-delimited typed field list at the schema argument position. The type checker reads this as an anonymous structural record per `meta_language.md` §"Inline record types".
- **Named record declaration:** `smelt.record SourceEntry = { … }` declared elsewhere in the workspace, passed by name as the schema argument.
- **`List<S>` of an inline or named record:** the loaded file's top level is parsed as a sequence and each element validated against `S`. Returns `List<S>`.
- **`Map<Text, S>` of an inline or named record:** the loaded file's top level is parsed as a mapping and each entry's value validated against `S`. Returns `Map<Text, S>`.

Schema arguments outside this admissible set emit `ConfigLoaderSchemaForbidden` at the offending argument expression. Bare scalar schemas (`load_yaml(path, Integer)`) are forbidden — every supported config file's root must be a record, a sequence of records, or a mapping of records, so the schema-typed return value has a stable shape consumers can navigate.

### Path resolution

- Paths are **workspace-relative**. A leading `/` (absolute path), a `..` segment that would escape the workspace root, or a scheme prefix (`http://`, `https://`, `s3://`, `file://`) emits `ConfigLoaderPathEscapesWorkspace` at the path literal.
- Paths use `/` as the path separator on every host OS; backslashes (`\`) in the literal emit `ConfigLoaderPathBackslash` at the offending character.
- The resolved file must exist; a missing file emits `ConfigLoaderFileNotFound` at the path literal.
- The resolved file is registered as a Salsa input. An edit to the file invalidates exactly the type-checks of files that load it (transitively through `smelt.define` calls).

### Per-target overlay

A loader call may resolve different files per build target:

```
configs/sources.yaml          # base
configs/sources.prod.yaml     # overlay for target=prod
configs/sources.dev.yaml      # overlay for target=dev
```

When `target` is set (via `smelt build --target prod` or `smelt.yml`'s `target:` field), the loader checks for `<basename>.<target>.<ext>` as a sibling of `<basename>.<ext>` and, if present, merges the overlay into the base by field. Overlay merging is **field-by-field deep merge** for record-shaped roots: an overlay's value for a field replaces the base's value for that field; an overlay field absent in the overlay is taken from the base. For `List<S>`-shaped roots the overlay **replaces** the base entirely (no list concatenation; users wanting concatenation author the merged list explicitly). For `Map<Text, S>`-shaped roots the overlay merges by key: an overlay key replaces the base's value for that key; a key absent in the overlay is taken from the base.

A target overlay file that does not validate against the schema emits the same diagnostic family as a base-file mismatch, anchored at the overlay file's offending row.

### Validation diagnostics

| Code | When | Message shape |
|---|---|---|
| `ConfigLoaderPathNotLiteral` | Loader `path` argument is not a string literal | `loader path must be a string literal; found {expr}` |
| `ConfigLoaderPathEscapesWorkspace` | Path is absolute, contains `..` escape, or has a scheme prefix | `loader path must be a workspace-relative path; found {path}` |
| `ConfigLoaderPathBackslash` | Path contains `\` | `loader paths use `/` as the path separator; found `\` in {path}` |
| `ConfigLoaderFileNotFound` | Resolved file does not exist in the workspace | `loader file `{path}` not found in workspace` |
| `ConfigLoaderSchemaForbidden` | Schema argument is not an admissible shape | `loader schema must be a record type, `List<record>`, or `Map<Text, record>`; found {actual}` |
| `ConfigLoaderTomlNotYetSupported` | `smelt.config.load_toml` is called | `smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1` |
| `ConfigLoaderParseError` | The file is not valid YAML / JSON | `failed to parse {format} file `{path}`: {parser_error}` (anchored at the YAML/JSON line, secondary frame at the loader call) |
| `ConfigLoaderRequiredFieldMissing` | A loaded value omits a field required by the schema | `field `{name}` required by schema is missing` (anchored at the YAML/JSON row, secondary frame at the loader call) |
| `ConfigLoaderUnknownField` | A loaded value contains a field not in the schema | `field `{name}` is not declared in the schema; expected one of: {fields}` (anchored at the YAML/JSON row, secondary frame at the loader call) |
| `ConfigLoaderTypeMismatch` | A loaded value's type does not match the schema's declared type | `field `{name}` expects {expected}; got {actual}` (anchored at the YAML/JSON value, secondary frame at the loader call) |
| `ConfigLoaderRootShapeMismatch` | The file's top-level shape (sequence, mapping, scalar) does not match the schema's expected root shape | `schema `{type}` expects {expected_shape}; file's top level is {actual_shape}` (anchored at the file's first line, secondary frame at the loader call) |
| `ConfigLoaderDuplicateMapKey` | A `Map<Text, S>`-shaped file contains the same key twice | `duplicate map key `{key}` at {row}; earlier appearance at {first_row}` |
| `ConfigLoaderNullCoercion` (warning) | A YAML `null` scalar coerces to an empty `Text` value at a schema field declared `Text` | `null value at {row} coerced to empty string; declare a default in the source file` |

### LSP support

- **Hover** on a loader call site shows the resolved file path, the file's resolved row count (for `List`/`Map` roots) or the schema's field set (for record roots), and the file's last-modified timestamp.
- **Hover** on the `schema` argument shows the resolved schema (inline structural display for inline schemas, declaration link for named schemas).
- **Goto-definition** on the loader name (`load_yaml`) resolves to the reference page (URL hint, graceful no-op when the client lacks support).
- **Goto-definition** on the `path` argument resolves to the loaded file (cursor on row 1).
- **Goto-definition** on a record-typed field of a loaded value, projected at the consumer site, resolves to the YAML/JSON row that produced the value (when statically traceable).
- **Completion** at the loader's first positional argument offers workspace-relative paths that exist as `.yaml` / `.yml` (for `load_yaml`) or `.json` (for `load_json`) files.
- **Completion** at the loader's second positional argument offers in-scope `smelt.record` names plus a stub `{<cursor>}` for inline schemas.
- **Diagnostics with frame stacks**: a `ConfigLoaderTypeMismatch` surfaced at a downstream consumer carries a primary frame at the YAML/JSON row and a secondary frame at the loader call site, both navigable from the LSP client.

## Semantics

### Load-bearing rules

1. **Workspace containment.** Loader paths must resolve to a file inside the workspace root. Paths escaping the root via `..` or absolute paths are a compile-time error per `ConfigLoaderPathEscapesWorkspace`.
2. **Salsa-tracked inputs.** Every loaded file (base and per-target overlays) must be registered as a Salsa input. Type checking of any file that uses a loaded value re-runs when an input changes; downstream invalidation propagates through the standard Salsa dependency graph.
3. **Pure validation.** Schema validation is a pure function from (parsed file bytes, declared schema) to (typed value, list of diagnostics). No clock, no random, no environment access. Re-runs over the same inputs produce byte-equal results.
4. **Deterministic output.** Same workspace state, same target, same loader call site → same loaded value. The two-tier (base + overlay) overlay strategy is deterministic by construction: at most one overlay file is read per loader call.
5. **Diagnostic source-span retention.** YAML and JSON parsers retain line/column information through validation. Validation failures attach diagnostics to the row/field that violated the schema (primary frame); the loader call site is the secondary frame.

### Per-format rules

#### YAML (`smelt.config.load_yaml`)

1. **Parser.** YAML 1.2 core schema (booleans `true` / `false`, integers, floats, strings, sequences, mappings, `null`). Tags (`!!str`, `!!int`, custom tags) are accepted only when they match the core-schema interpretation; non-core tags emit `ConfigLoaderParseError` at the tag position.
2. **Scalar coercion.** YAML scalars coerce to schema types per:
   - `Text` schema field accepts YAML strings and `null` (rendering `null` as empty string with a `ConfigLoaderNullCoercion` warning matching `meta_language.md`'s `smelt.config.var` `ConfigVarNullCoercion`).
   - `Integer` schema field accepts YAML integers; floats with a non-zero fractional part emit `ConfigLoaderTypeMismatch`.
   - `Float` schema field accepts YAML floats and integers (the integer widens).
   - `Boolean` schema field accepts YAML booleans only; YAML strings `"true"` / `"false"` are not coerced (the user must use a YAML boolean).
   - `Date` / `Timestamp` schema fields accept YAML strings in the canonical ISO-8601 format; other formats emit `ConfigLoaderTypeMismatch`.
   - `Decimal(p, s)` schema field accepts YAML numbers (integer or float) that fit within the precision/scale; mismatches emit `ConfigLoaderTypeMismatch`.
3. **Sequence-of-records.** A `List<S>` schema expects the YAML file's top level (or the schema field's value) to be a YAML sequence; each element is a YAML mapping validated against `S`. A non-sequence top level for a `List<S>` schema emits `ConfigLoaderRootShapeMismatch`.
4. **Mapping-of-records.** A `Map<Text, S>` schema expects the YAML file's top level to be a YAML mapping; each entry's key must be a YAML string (otherwise `ConfigLoaderTypeMismatch` at the key) and value a YAML mapping validated against `S`. Duplicate keys emit `ConfigLoaderDuplicateMapKey`.
5. **Anchors and aliases.** YAML anchors (`&anchor`) and aliases (`*anchor`) are resolved by the parser before validation; the validated value sees the aliased shape. Recursive anchors (an alias referenced inside its own definition) emit `ConfigLoaderParseError` (the YAML parser rejects them as unsafe; loader does not attempt a fixed-point).
6. **Comments.** YAML comments (`# …`) are preserved by the parser only for hover (showing the source line); they do not affect validation.

#### JSON (`smelt.config.load_json`)

1. **Parser.** RFC 8259 strict-mode JSON. No trailing commas, no comments, no unquoted keys. Violations emit `ConfigLoaderParseError` at the offending byte.
2. **Scalar coercion.** Same rules as YAML's core schema with these differences:
   - `null` coerces only to `Text` (rendering as `''` with `ConfigLoaderNullCoercion`); other schema types treating `null` as a value emit `ConfigLoaderTypeMismatch`. JSON's stricter null semantics are preserved.
   - `Decimal` accepts JSON numbers; precision-overflow emits `ConfigLoaderTypeMismatch`.
3. **Sequence and mapping rules.** Same as YAML.
4. **Determinism.** JSON files have a canonical serialisation; same logical content always parses to the same value.

### Loader value materialisation

1. **Body-check time.** At a loader call site, the call's synthesised type is the declared schema's type, computed once the path's existence and schema's admissibility are validated. The loaded file's contents are read once during type-check and the materialised value is cached as a Salsa-stored result.
2. **Expansion time.** A loaded value's projections are materialised exactly as a hand-authored value would be. A `List<S>` value's `.length` (when the surface adds it) or `m.entries()` walks the value once; the per-element data is the parsed file's row, with the row's source span carried for `map_origin` / `list_origin` frame extensions.
3. **Determinism preservation.** A loader call's value is a pure function of (file bytes, schema). The Salsa query layer guarantees re-evaluation produces identical results until the file or schema changes; the LSP invalidates downstream type-checks on file edit.

### Schema selection

When multiple schema sites declare structurally identical inline records (e.g. two loader calls each pass `{name: Text}`), they share a type identity per `meta_language.md` §"Inline record types are structurally typed". When a named `smelt.record` and an inline record with identical field sets coexist, both are admissible at the loader's schema position; the loaded value carries the schema reference used at the call site for hover and goto-def purposes (the structural identity does not affect type checking).

## Design

### Why a separate spec from `meta_language.md`

The loader family is large enough — two file formats, schema-driven validation, per-target overlay, diagnostic source-span retention, Salsa input registration — to warrant its own spec without bloating `meta_language.md`. The meta-language consumes loaded values via `List<T>` / records / `Map<K, V>` / HOFs; the loaders supply them. Splitting the specs at this seam keeps each one under its target line budget.

### Why typed schemas, not untyped values

Research §4.10.1 considered four typing approaches: (a) untyped `Json`-like dynamic value; (b) schema-typed (declared schema); (c) schema-inferred-from-data; (d) inline schema-at-call-site. Untyped values reproduce the dbt failure mode this whole effort exists to fix — errors land at the use site, far from the bad input. Schema-inferred-from-data makes the schema data-dependent, which silently breaks distant code under YAML refactoring. Schema-typed (with inline as a sugar over named) was the chosen lean. The spec admits both inline schemas (for one-shot configs) and named schemas (for shapes that recur across files); this matches the research-doc framing.

### Why YAML and JSON first, TOML reserved

YAML dominates configuration in the data ecosystem — dbt schemas, smelt's own `sources.yml`, every modern data tool's config. JSON rides along cheaply (`serde_json` exists in the build, parser is small). TOML is reserved (not removed from the spec) so users cannot bind `load_toml` in their own code; shipping it later is non-breaking. CSV is tempting (column lists in spreadsheets) but lacks a schema-friendly type system — sequences only, no fields — and is deferred indefinitely.

### Why workspace-only paths, no network

Determinism and reproducibility (§Constraints). A spec change to allow network loads would need to define caching, retry, failure semantics, and security boundaries — entirely separate work. The v1 surface is narrow on purpose.

### Why the literal-only `path` argument

A non-literal `path` (a variable, an expression) would require resolving an arbitrary `Text` value at type-check time, which interacts with reflection laziness and Salsa input registration. Constraining `path` to a string literal keeps the Salsa input contract simple: every loader call has exactly one input file path known at parse time. Future work could relax this with explicit input-registration semantics, but no concrete example demands it.

### Per-target overlay: file-overlay over path-interpolation

Three candidates from research §4.10.1: (i) **path interpolation** in the literal (`load_yaml('configs/{target}/sources.yaml', S)`), (ii) **file overlay** (load `sources.yaml`, merge `sources.<target>.yaml` if present), (iii) **post-load filter** (load all targets' rows, apply `filter` by `target` field). Option (iii) requires authors to bake the target into every row, doubling YAML size for two-target cases. Option (i) requires string interpolation in path literals, which is a parallel surface (and conflicts with the literal-only `path` rule). Option (ii) is the chosen lean: the base file is the source of truth, the overlay is an opt-in adjustment, and the merge rule is field-by-field (predictable, no concatenation surprises).

The merge rule is **replace, not concatenate** for `List<S>` overlays. List concatenation surprises authors who expect the overlay to replace the base; users wanting concatenation author a merged list explicitly. The field-level deep merge for record-shaped roots matches the common case (override one or two fields per target).

### Why deep-merge for record fields but replace for lists

Record-field deep merge: a user wants to override `connection_string` per target without restating every other field. Replacing the entire record forces overlay authors to copy the whole base record, which is exactly the duplication overlays exist to avoid. Deep merge is the convenient default.

List replacement: lists are typically sequence-of-records (`List<SourceEntry>`); deep-merging element-by-element would require an identity rule (merge by `name`? by index? by primary key?) and produce surprising behaviour when source files reorder. Replacement is the unambiguous default; users wanting merge author the merged list.

### Why diagnostic anchoring at the YAML/JSON row

A validation error like "field `min_revenue` is missing" must anchor at the YAML row that omits the field, not at the smelt loader call site. The YAML parser retains line/column information through to validation; the diagnostic carries both spans (primary at the YAML row, secondary at the loader call). The dbt failure mode this spec exists to fix is "error in YAML surfaces at distant SQL" — anchoring at the YAML row reverses that.

### Why no `Optional<V>` for missing fields

A field declared on the schema is **required**; absence emits `ConfigLoaderRequiredFieldMissing`. The alternative — optional fields per `Optional<V>` annotation — was considered but deferred:

- The user-facing optionality discipline interacts with the (out-of-scope) `Optional<T>` sum-type surface.
- Real config files rarely have genuinely-optional fields; the common case is a field present in every row, with a per-target overlay providing defaults.
- Once `Optional<T>` ships as a separate spec, this loader spec gets a one-line extension.

### Why named schemas are workspace-scoped

A `smelt.record SourceEntry = {…}` declaration is visible from every loader call site in the workspace. Per-file or per-directory scoping was considered and rejected because it would require a discovery mechanism (import statements) that the rest of the meta-language deliberately avoids. The workspace-global model matches `smelt.define`, `smelt.<path>`, and `smelt.config.var('x')`.

## Constraints & Invariants

- **No file access outside the workspace.** Enforced by path-canonicalisation check before file open; `ConfigLoaderPathEscapesWorkspace` is the user-facing surface.
- **No network access.** No HTTP / HTTPS / S3 / file:// schemes; only relative paths.
- **No environment-dependent loads in pure mode.** Environment variable substitution is not supported in v1; consuming-callers wanting per-environment variation use the per-target overlay file. A future `smelt.config.env(name)` API would be a separate spec.
- **Salsa inputs cover every loaded file.** A change to any loaded YAML / JSON file invalidates the type-check of every file that loaded it, transitively. Per-target overlay files are also Salsa inputs.
- **Validation produces structured diagnostics.** Errors are `Diagnostic` values with file/line/column anchors, not unstructured strings. The LSP renders them with frame stacks pointing at the loader call site as a secondary frame.
- **Schema is the source of truth for the loaded type.** The meta-language type checker treats `load_yaml(path, S)` as having return type `S` for type-checking purposes; the actual file content is checked against `S` at meta-evaluation time and either succeeds (producing a typed value) or produces a typed validation diagnostic.
- **Deterministic re-evaluation.** Same `(file bytes, schema, target)` triple always produces the same loaded value. No clock, no random, no environment.
- **YAML 1.2 core schema is the canonical interpretation.** Non-core tags are rejected; this prevents unsafe tags (e.g. `!!python/object`) from gaining a foothold in the loader path.

### Out-of-scope by deliberate choice

- **Network loads.** No HTTP / S3 / etc. in v1; future spec.
- **Schema inference from sample data.** Future tooling can read a YAML and emit a schema declaration; this is a codegen-time helper, not a loader-time fallback.
- **Sum types in schemas.** Aligned with `meta_language.md` constraint — heterogeneous values require sum types, which are out of scope.
- **Recursive schemas.** A schema referring to itself (or mutually recursive schemas) is deferred — most config files are flat-or-nested-but-finite.
- **`Optional<V>` fields.** Required-field-only in v1; optional fields wait on a sum-type / option-type surface.
- **Environment-variable interpolation in paths or values.** Out-of-scope per the pure-determinism invariant.
- **CSV / Parquet loaders.** CSV lacks a schema-friendly type system; Parquet is data-world territory (loaded via `FROM` clauses, not config). Both deferred.
- **TOML.** Reserved name, post-v1.
- **Network paths and authentication.** Workspace-relative only.

## Known Divergences / Open Questions

- **Loaders are implemented per this spec.** Residual divergences (recursive schemas, per-key deep-merge for `Map<Text, S>` overlays, `Optional<V>` schema fields) are listed below.
- **Recursive schemas.** Whether mutually-recursive `smelt.record` declarations should be admissible at loader-call schema positions is deferred. The acyclic-DAG record-declaration rule applies; loader-schemas inherit the same restriction.
- **Per-target overlay merge of `Map<K, V>` is per-key replace, not per-key deep-merge.** A future spec edit may add per-key deep-merge if real configs demand it; today's rule is "overlay key replaces base value at that key", matching the `List<S>` replacement rule's spirit (the overlay is a value substitution, not a structural blend).
- **Loader-call type synthesis is callable but not yet consumed by upstream inference.** `infer_loader_call_smelt_type` synthesises `SmeltType::Record` / `SmeltType::List(Record)` / `SmeltType::Map(Text, Record)` correctly for a `smelt.config.load_yaml`/`_json` call site. However, the production inference dispatch (`infer_smelt_path_call_type` in `crates/smelt-db/src/type_inference/`) returns `Option<TypedColumn>` (a `DataType`-based wrapper) and cannot structurally carry meta-language types. The first production consumer — LSP hover on a loader call — wires `infer_loader_call_smelt_type` directly; HOF dispatch over loader-call source lists is wired in a later iteration. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **LSP goto-def on the loader name and hover on the schema argument are not yet wired.** Spec §LSP support asserts a graceful-no-op goto-def from the bare loader-name token (`load_yaml`, `load_json`) and a hover popup on the `schema` argument that renders the resolved schema. The corresponding pure helpers have not been written; `goto_def_for_loader_path` and `hover_text_for_loader_call` handle adjacent surfaces (path argument and full call site respectively). Tracked in `docs/plans/20260509-meta-language-E1.md` "Deferred during implementation".
- **`Date` / `Timestamp` / `Decimal` schema-field strict-format validation is not yet implemented.** §Per-format YAML rule 2 asserts that `Date` / `Timestamp` fields require canonical ISO-8601 strings and that `Decimal(p, s)` fields validate against the declared precision/scale, emitting `ConfigLoaderTypeMismatch` on violations. The current YAML/JSON loaders accept any string at `Date` / `Timestamp` fields and any number at `Decimal` fields without format/precision validation. The diagnostic infrastructure (`ConfigLoaderTypeMismatch` code, span tracking) is in place; only the format-check predicates and Decimal range/scale arithmetic are missing. Tracked in `docs/plans/20260509-meta-language-overall.md`.

## References

- **Code**:
  - `crates/smelt-db/src/lib.rs` and `crates/smelt-db/src/queries/loader.rs` — Salsa-tracked `LoaderFileInput` (raw text + exists bit, registered via `set_loader_file`); `loader_resolved_value_with_overlay(call_site)` for the validated meta-world value with per-target overlay resolution.
  - `crates/smelt-db/src/type_inference/loader_and_reflection.rs` — `smelt.config.load_yaml` / `smelt.config.load_json` dispatch (schema admissibility check, return-type synthesis via `infer_loader_call_smelt_type`, validation invocation); literal-only `path` argument validation.
  - `crates/smelt-db/src/loader.rs` — per-format parsers (YAML via `marked_yaml`, JSON via `serde_json`), schema validation with source-span retention, per-target overlay resolution and merge.
  - `crates/smelt-db/src/diagnostics_types.rs::DiagnosticCode` — every diagnostic code listed under §Validation diagnostics.
  - `crates/smelt-lsp/src/{lib,backend,hover}.rs` — hover for loader call sites (`hover_text_for_loader_call`), goto-def for `path` arguments (`goto_def_for_loader_path`) and loaded-record field projections (`goto_def_for_loaded_record_field_projection`), completion for `path` (filesystem-aware by extension) and `schema` (in-scope `smelt.record` names) positions. Loader-name goto-def and schema-argument hover are pure-helpers-pending — see Known Divergences.
- **Tests**:
  - `crates/smelt-db/src/loader.rs::tests` and `crates/smelt-db/src/tests.rs` — per-format parsing (YAML core schema, JSON strict mode); inline-schema validation; named-schema validation; `List<S>` and `Map<Text, S>` root shapes; per-target overlay resolution and merge (record deep-merge, list replacement, map per-key replacement); duplicate-key detection (`yaml_parse_map_root_emits_duplicate_key`); Salsa invalidation (`loader_file_text_is_salsa_input`, `loader_resolved_value_invalidated_on_file_change`, `overlay_file_change_invalidates_loader_value`).
  - `crates/smelt-db/src/type_inference/tests.rs` — loader call type-checking (schema admissibility, return-type synthesis); literal-only `path` enforcement; reserved `load_toml` diagnostic.
  - `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/meta_config/` acceptance gate (loaders + record consumers + per-target overlay).
- **User docs**:
  - `docs-site/docs/meta-language/config-loaders.md` — `smelt.config.load_yaml`, `smelt.config.load_json`, schema authoring, per-target overlay, validation diagnostics.
  - `docs-site/docs/meta-language/reference.md` — alphabetical reference includes loader entries.
- **Plans (history)**:
  - `docs/plans/20260509-meta-language-overall.md` — tracking plan for the loader family
  - `docs/plans/20260509-meta-language-E1.md`
- **Related specs**:
  - `docs/specs/meta_language.md` — the consuming meta-language constructs (`List<T>`, records, `Map<K,V>`, HOFs, multi-model production)
  - `docs/specs/sources.md` — `sources.yml` parser is the closest existing precedent for a workspace-input YAML loader
  - `docs/specs/smelt_yml.md` — `smelt.yml` `vars:` block, accessed via `smelt.config.var(name)` (a different loader path, not this spec)
- **Research**:
  - `docs/research/20260507-typed-meta-programming.md` §4.10.1 — design oracle, alternatives a–d for typing approach, per-target overlay strategy options
