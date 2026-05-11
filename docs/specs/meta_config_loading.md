---
feature: meta_config_loading
status: experimental
last_reviewed: 2026-05-11
owners: [andrew]
---

# Meta Config Loading

> **What this is.** A normative spec for the file-loader family that supplies typed meta-world values from disk: `smelt.config.load_yaml(path, schema)`, `smelt.config.load_json(path, schema)`, `smelt.config.load_toml(path, schema)`, and the schema declarations they validate against. In scope: the loader API surface, schema authoring (inline and named), validation diagnostics with source-span retention through to the YAML/JSON/TOML row that violated the schema, determinism guarantees (workspace-relative paths, no network, no clock), and per-target file-overlay strategies. Out of scope: the meta-language constructs that consume loaded values — `List<T>`, HOFs, records, multi-model production (see `meta_language.md`); the `vars:` block in `smelt.yml` accessed via `smelt.config.var(name)` (see `meta_language.md`); the planner's use of generated models (see `planner_integration.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Implementation status.** The loader family is not yet implemented. The §Surface and §Semantics sections describe the intended surface; §Design and §Constraints are load-bearing now. See §Known Divergences for what is undecided and §References → Plans (history) for the tracking plan.

## Surface

The loader family exposes three calls, one per supported file format:

- `smelt.config.load_yaml(path: Text, schema: Schema) -> Schema`
- `smelt.config.load_json(path: Text, schema: Schema) -> Schema`
- `smelt.config.load_toml(path: Text, schema: Schema) -> Schema`

`Schema` declarations:

- **Inline:** `load_yaml('configs/sources.yaml', { name: Text, columns: List<Text> })`.
- **Named:** declared with `smelt.record SourceEntry = { … }` (see `meta_language.md`) and passed by name.

**Diagnostic anchoring.** Validation failures point at the YAML/JSON/TOML line and column that violated the schema, not at the loader call site. The loader call site is the secondary frame.

**Path resolution.** Workspace-relative; no `..` escape; no absolute paths in v1; no network or scheme prefixes.

**Per-target overlay.** Strategy is one of three candidates listed in §Known Divergences (path-interpolation, file overlay, post-load filter). Picked when the loaders ship.

## Semantics

### Load-bearing rules

1. **Workspace containment.** Loader paths must resolve to a file inside the workspace root. Paths escaping the root via `..` or absolute paths are a compile-time error.
2. **Salsa-tracked inputs.** Every loaded file must be registered as a Salsa input so type checking is reproducible and the LSP invalidates correctly when the file changes.
3. **Pure validation.** Schema validation is a pure function from (parsed file, schema) to (typed value | diagnostic). No clock, no random, no environment access.
4. **Deterministic output.** Same workspace state must produce the same loaded value for the same `(path, schema)` pair. Across re-runs, across LSP sessions, across builds.
5. **Diagnostic source-span retention.** YAML/JSON parsers retain line/column information. Validation failures attach diagnostics to the row/field that violated the schema. The loader call site is the secondary frame; the file row is the primary frame.

### Per-format and per-target rules

Per-format and per-target rules are not yet defined. See §Known Divergences for the open per-target overlay decision.

## Design

### Why a separate spec from `meta_language.md`

The loader family is large enough — multiple file formats, schema authoring discipline, validation diagnostic UX, per-target overlays, source-span retention — to warrant its own spec without bloating `meta_language.md` Surface. The meta-language consumes loaded values via `List<T>` / records / HOFs; the loaders supply them. Splitting the specs at this seam keeps each one under the ~300-line target.

### Why typed schemas, not untyped values

The research doc (§4.10.1, alternatives a–d) considered four typing approaches: untyped `Json`-like dynamic value; schema-typed (declared schema); schema-inferred-from-data; inline schema-at-call-site. Untyped values reproduce the dbt failure mode this whole effort exists to fix — errors land at the use site, far from the bad input. Schema-inferred-from-data makes the schema data-dependent, which silently breaks distant code under YAML refactoring. Schema-typed (with optional inline shorthand) was the chosen lean.

### Why YAML first

YAML dominates configuration in the data ecosystem. dbt schemas, smelt's own `sources.yml`, every modern data tool's config — all YAML. JSON and TOML come along cheaply because parsers exist; CSV is tempting (column lists in spreadsheets) but lacks a schema-friendly type system, and is deferred.

### Why workspace-only paths, no network

Determinism and reproducibility (§Constraints below). A spec change to allow network loads would need to define caching, retry, failure semantics, and security boundaries — entirely separate work. Keep the v1 surface narrow.

### Per-target overlay strategy options

Three options are on the table from research §4.10.1: path interpolation (`'configs/{target}/sources.yaml'`), file overlay (load `sources.yaml` then merge `sources.{target}.yaml`), post-load `filter`. The choice is open until example fixtures provide evidence — see §Known Divergences.

## Constraints & Invariants

- **No file access outside the workspace.** Enforced by path-canonicalisation check before file open.
- **No network access.** No HTTP / HTTPS / S3 / file:// schemes; only relative paths.
- **No environment-dependent loads in pure mode.** Environment variable substitution requires an explicit gating API in `smelt.config.env(name)` (separate from this loader family) which marks the consuming file as non-deterministic. The default-pure invariant holds for `load_yaml`/`load_json`/`load_toml`.
- **Salsa inputs cover every loaded file.** A change to any loaded YAML invalidates the type-check of every file that loaded it, transitively.
- **Validation produces structured diagnostics.** Errors are `Diagnostic` values with file/line/column anchors, not unstructured strings. The LSP renders them with frame stacks pointing at the loader call site as a secondary frame.
- **Schema is the source of truth for the loaded type.** The meta-language type checker treats `load_yaml(path, S)` as having return type `S` for type-checking purposes; the actual file content is checked against `S` at meta-evaluation time and either succeeds or produces a typed validation diagnostic.

### Out-of-scope by deliberate choice

- **Network loads.** No HTTP / S3 / etc. in v1; future spec.
- **Schema inference from sample data.** Future tooling can read a YAML and emit a schema declaration; this is a codegen-time helper, not a loader-time fallback.
- **Sum types in schemas.** Aligned with `meta_language.md` constraint — heterogeneous values require sum types, which are out of scope.
- **Recursive schemas.** A schema referring to itself (or mutually recursive schemas) is deferred — most config files are flat-or-nested-but-finite.

## Known Divergences / Open Questions

- **Loaders not implemented.** No code exists yet; the surface above is intended, not landed. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Per-target overlay strategy is undecided.** Three candidates listed in §Design; the choice will be made when example fixtures provide evidence. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Inline-schema sugar.** Whether `load_yaml(path, { name: Text, … })` (inline schema) is a first-class surface or just sugar for a named declaration is open. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Schema reuse across files.** Whether named schemas (`smelt.record`) live in their own files (`config_schemas/*.smelt.record`) or only inside meta-language files is a layout decision deferred until example fixtures show the pattern.
- **TOML support timing.** YAML and JSON ship first; TOML may follow based on user demand. The spec lists TOML in scope to keep the API surface stable, but implementation may lag.

## References

- **Code**: *(none yet — loaders unimplemented)*
- **Tests**: *(none yet — loaders unimplemented)*
- **User docs**: *(none yet — loaders unimplemented)*
  - `docs-site/docs/meta-language/generators.md` will reference this spec from the multi-model-production guide once the loaders ship.
- **Plans (history)**:
  - `docs/plans/20260509-meta-language-overall.md` — tracking plan for the loader family
- **Related specs**:
  - `docs/specs/meta_language.md` — the consuming meta-language constructs (`List<T>`, records, `Map<K,V>`, HOFs, multi-model production)
  - `docs/specs/sources.md` — `sources.yml` parser is the closest existing precedent for a workspace-input YAML loader
  - `docs/specs/smelt_yml.md` — `smelt.yml` `vars:` block, accessed via `smelt.config.var(name)` (a different loader path, not this spec)
- **Research**:
  - `docs/research/20260507-typed-meta-programming.md` §4.10.1 — design oracle, alternatives a–d for typing approach, per-target overlay strategy options
