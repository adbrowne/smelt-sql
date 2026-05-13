# Plan: Meta-Language Phase E1 — Records, `Map<K, V>`, YAML/JSON schema-typed loaders

**Date**: 2026-05-13
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Records" and §"Maps" (Surface, Semantics, Design, Constraints); [`docs/specs/meta_config_loading.md`](../specs/meta_config_loading.md) (entire spec)
**Spec diff**: commit `cc27e5d` (`spec(meta-language-E1): records, Map<K,V>, schema-typed loaders`) on branch `research/typed-meta-programming`
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Records" and §"Maps" (Surface, Semantics, Design, Invariants, Known Divergences) and `docs/specs/meta_config_loading.md` end-to-end — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec meta_language` (or `meta_config_loading`) to revise the spec rather than encode the divergence in code.
2. Confirm you are on branch `research/typed-meta-programming`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 7 is the expert-reviewer dispatch loop** — after Phases 1–6 commit, dispatch the meta-plan §5 expert reviewers applicable to this phase, address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 7. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 7's acceptance gate is met.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first to update).
- `cargo test` or `cargo clippy --all-targets` surfaces a pre-existing failure unrelated to the plan.
- Phase 7: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.

**Conventions every phase:**

- Real-fixture tests under `examples/meta_config/` — Phase 6 exercises the full surface there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no multi-model production (`generates: models`, `ModelDef`), no parameterised reducers, no multi-arg lambdas, no ternary — those are Phases E2 and F.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic). All Salsa queries for loaders (and the workspace `smelt.record` registry) live in `crates/smelt-db/src/lib.rs`; inference rules consume resolved workspace state as parameters.
- Timeless-oracle rule: spec and user-doc edits read as if the feature has always existed. Phase vocabulary lives in this plan only.

---

## Context

The meta-language Phase E1 spec increment landed in commit `cc27e5d`. The spec authors three load-bearing surfaces:

- **Records.** `smelt.record TypeName = { fields }` top-level declarations, inline record types `{f: T, …}` at type-annotation positions, record literals `{f: v, …}` at value positions, dotted field projection, width subtyping, and eleven record diagnostic codes (`SmeltRecordRedefinition`, `RecordFieldUnknown`, `RecordFieldMissing`, `RecordFieldDuplicate`, `RecordFieldTypeMismatch`, `RecordLiteralUnknownTarget`, `RecordFieldNotProjectable`, `RecordFieldTypeForbidden`, `RecordCyclicDeclaration`, `RecordInDataWorld`, and reflection-witness exclusion via `RecordFieldTypeForbidden`).
- **`Map<K, V>`.** Meta-only key-value collection (`K = Text` in v1) with the closed five-method API (`entries`, `keys`, `values`, `get`, `has`), invariance in both axes, sorted-by-key iteration, and seven map diagnostic codes (`MapKeyTypeNotText`, `MapApiUnknown`, `MapApiArityMismatch`, `MapApiNamedArgument`, `MapApiUnexpectedArgument`, `MapGetMissingKey`, `MapApiArgTypeMismatch`).
- **Schema-typed config loaders.** `smelt.config.load_yaml(path, schema)` and `smelt.config.load_json(path, schema)` with `smelt.config.load_toml` reserved; per-target file-overlay (`<basename>.<target>.<ext>` sibling, record deep-merge / list replace / map per-key replace); validation diagnostics anchored at the YAML/JSON row that violated the schema (primary frame) and the loader call site (secondary frame); thirteen loader diagnostic codes per `meta_config_loading.md` §"Validation diagnostics" (including the `ConfigLoaderNullCoercion` warning).

This plan drives the implementation, examples, user docs, and skill update for that surface. Phase E1 is the fifth of seven implementation phases (A–G); the record and `Map<K, V>` types plus the schema-typed loaders are the prerequisites for Phase E2's per-cohort-union killer demo (which adds `ModelDef` multi-model production on top of these primitives).

## Scope

### In scope (spec coverage)

- `meta_language.md` Surface for records: `smelt.record Name = { fields }` declarations, inline record types, record literals, field projection, width subtyping, and the eight record diagnostic codes; LSP obligations for hover, goto-def, completion, and frame-stack diagnostics.
- `meta_language.md` Surface for maps: `Map<K, V>` type formation, the closed `{entries, keys, values, get, has}` API, `m.get` missing-key behaviour, and the seven map diagnostic codes; LSP obligations.
- `meta_language.md` Semantics §"Records" rules 1–11 and §"Maps" rules 1–8 — the normative evaluation rules for declaration registration, cyclic-DAG enforcement, structural inline equality, bidirectional literal checking, drop-on-error per field, sorted-key iteration, closed Map API, invariance, and termination bounds.
- `meta_language.md` Design §"Records — design rationale" and §"Maps — design rationale" — preserved as architectural invariants policed by the implementation.
- `meta_language.md` Constraints §"Record invariants" and §"Map invariants" — workspace-globally unique record names, immutability, DAG declarations, no record-shape inference, reflection-witness exclusion; `Map<K, V>` `K = Text`, loader-only origin, invariance, closed API, meta-only consumption.
- `meta_language.md` Known Divergences entry for E1 (replace the "records / maps / loaders not yet implemented" entry with the actual landed surface; the residual divergences — recursive schemas, per-key deep-merge for `Map<Text, S>` overlays, `Optional<V>` — remain documented in `meta_config_loading.md`).
- `meta_config_loading.md` Surface for loader calls, schema authoring, path resolution, per-target overlay (resolution and merge rule), validation diagnostics, LSP support.
- `meta_config_loading.md` Semantics §"Load-bearing rules", §"Per-format rules" (YAML core schema, JSON strict mode), §"Loader value materialisation", §"Schema selection".
- `meta_config_loading.md` Constraints — workspace containment, no network, deterministic re-evaluation, YAML 1.2 core schema, schema-is-source-of-truth.
- The thirteen `meta_config_loading.md` validation diagnostic codes (`ConfigLoaderPathNotLiteral`, `ConfigLoaderPathEscapesWorkspace`, `ConfigLoaderPathBackslash`, `ConfigLoaderFileNotFound`, `ConfigLoaderSchemaForbidden`, `ConfigLoaderTomlNotYetSupported`, `ConfigLoaderParseError`, `ConfigLoaderRequiredFieldMissing`, `ConfigLoaderUnknownField`, `ConfigLoaderTypeMismatch`, `ConfigLoaderRootShapeMismatch`, `ConfigLoaderDuplicateMapKey`, `ConfigLoaderNullCoercion` (warning)).
- LSP support for every Phase E1 surface element: hover on `smelt.record` names, record-typed bindings, field projections, record literal opening braces, `Map<K, V>`-typed bindings, Map-API method invocations, loader call sites, and loader schema arguments; goto-def from a `smelt.record` reference to the declaration site, from a record literal field name to the declared field's source span, from a loader's `path` argument to the loaded file (cursor on row 1), and from a record-typed field of a loaded value (projected at a consumer site) to the YAML/JSON row that produced the value; completion at record-literal field-key positions, field-projection sites, `m.<cursor>` Map API positions, `m.get(<cursor>)` / `m.has(<cursor>)` statically-known-key positions, the loader's `path` argument (filesystem entries by extension), and the loader's `schema` argument (in-scope `smelt.record` names plus an inline-schema stub).
- Per-target file overlay implementation (resolution and merge); record deep-merge, list replacement, map per-key replacement.
- YAML and JSON parsers with source-span retention through validation.
- Examples fixture `examples/meta_config/` covering happy paths (declared record + YAML loader + per-target overlay; `Map<Text, Record>` driving an `entries`-projection HOF chain) plus broken sub-fixtures for each new Phase E1 diagnostic (~18 codes), gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/{records,maps,config-loaders,reference}.md` extending the existing reference page.
- `smelt-app-builder` skill: per-phase reference doc at `.claude/skills/smelt-app-builder/references/20260513-meta-records-maps-loaders.md`.
- `/smelt-loop` `medium` tier: at least one Phase E1-specific ask (e.g. "Use `smelt.config.load_yaml` to author a tenant-threshold table and consume it via `m.entries() |> map(fn e => …)`").

### Explicitly deferred

- Multi-model production (`generates: models`, `ModelDef` value type, generator file body shape, the per-cohort union killer demo) — Phase E2. Phase E1 ships the record / map / loader primitives the demo composes; Phase E2 closes the loop.
- Recursive (`mutually recursive`) `smelt.record` declarations — `meta_config_loading.md` Known Divergences. v1 records are a DAG; cyclic declarations emit `RecordCyclicDeclaration`.
- `Optional<V>` schema fields — `meta_config_loading.md` §"Why no `Optional<V>`". Required-field-only in v1.
- `smelt.config.load_toml` — reserved per spec. Phase E1 emits `ConfigLoaderTomlNotYetSupported` at the call site; the name cannot be bound in `smelt.define`.
- Per-key **deep** merge for `Map<Text, S>` overlays — `meta_config_loading.md` Known Divergences. Phase E1 ships per-key replace; deep-merge is a future spec edit.
- Network paths, environment-variable interpolation, schema inference from sample data, CSV/Parquet loaders, sum-typed schemas — `meta_config_loading.md` §"Out-of-scope by deliberate choice".
- Parameterised reducers, multi-arg lambdas, ternary — Phase F.
- LSP rename support for record / loader constructs — Phase G.
- Catalog rendering changes — Phase E2.
- The `Array<U>(…)` runtime-array constructor — Phase E2 (does not interact with Phase E1's brace surface, but flagged for completeness).
- Additional Map methods (`map_values`, `merge`, `filter_keys`) — spec-rejected; require a future spec edit.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 9ca6416 | 2026-05-13 |
| 2     | done     | 2981610 | 2026-05-13 |
| 3     | done     | —      | 2026-05-13 |
| 4     | pending  | —      | —    |
| 5     | pending  | —      | —    |
| 6     | pending  | —      | —    |
| 7     | pending  | —      | —    |

---

### Phase 1: Type system — `SmeltType::Record` + `SmeltType::Map` variants + workspace `smelt.record` registry + closed Map API table + diagnostic codes (pure)

**Goal.** Add the two `SmeltType` variants — `Record` (carrying a sorted field-name → type map plus optional `name: Option<String>` metadata so a named declaration's hover string remains attributable while structural equality ignores the name) and `Map` (carrying boxed `K` and `V` invariant in both). Add the closed `MAP_API_METHODS` registry naming the five method names with arity and return-type spec. Add a workspace-level `SmeltRecordDeclaration` registry type and a pure helper that builds the registry from a list of parsed declarations, detecting `SmeltRecordRedefinition` (duplicate names, first-declaration-wins) and `RecordCyclicDeclaration` (DAG cycle detection via DFS over field-type references to declared names). Register every Phase E1 diagnostic code (eight record codes plus three closed-set / cyclic / data-world codes — eleven total record codes — plus seven map codes plus the thirteen loader codes; some loader-side codes land textually here but the wiring lands in Phase 5). No parsing yet (Phase 2). No bidirectional inference yet beyond constructor / lookup helpers (Phase 3). All work in `crates/smelt-types/src/signatures.rs` and `crates/smelt-db/src/lib.rs`'s `DiagnosticCode` enum is pure; no Salsa imports added to inference.

**Pre-conditions.** Phase D done at commit `e6d9437`. Working tree clean. `cargo test`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**TDD tests to write first.**

- `crates/smelt-types/src/signatures.rs::tests::record_type_round_trips_field_order_canonicalised` — `SmeltType::Record { fields: BTreeMap<String, SmeltType>, name: Some("SourceEntry".into()) }` constructed twice with field-insertion in different orders compares equal under `==`; the `Display` impl renders fields in declared (user-facing) order when `name` is `Some`, in field-name lex order when `name` is `None` (inline). Asserts the sorted-canonical structural-equality invariant from spec rule 4.
- `crates/smelt-types/src/signatures.rs::tests::record_inline_and_named_with_same_field_set_are_structurally_equal` — two `SmeltType::Record` values with identical field maps and differing `name` (`Some("X")` vs `None`) are equal under structural-equality predicate `is_same_record_type` (or whatever helper the code uses); they remain distinguishable for hover (the `name` field is exposed by an accessor).
- `crates/smelt-types/src/signatures.rs::tests::map_type_invariant_both_axes` — `is_subtype_of(Map<Text, Integer>, Map<Text, Number>) == false`; `is_subtype_of(Map<Text, Number>, Map<Text, Integer>) == false`; `is_subtype_of(Map<Text, Integer>, Map<Text, Integer>) == true`. Verifies invariance in `V` (and trivially in `K` since `K = Text` is the only admissible v1 key).
- `crates/smelt-types/src/signatures.rs::tests::map_api_methods_registry_is_closed_and_exact` — `MAP_API_METHODS` exposes exactly the five names `{entries, keys, values, get, has}`; lookup of any other identifier returns `None`; each entry's arity is `Arity::Exact(0)` for `entries/keys/values` and `Arity::Exact(1)` for `get/has`; the return-type formula for `entries` resolves to `List<Record{{key: K, value: V}}>` given a binding for `K` and `V`.
- `crates/smelt-types/src/signatures.rs::tests::record_width_subtyping_rule` — `is_subtype_of(Record{a: Text, b: Integer}, Record{a: Text}) == true`; the reverse is `false`; `is_subtype_of(Record{a: Text}, Record{a: Integer}) == false` (field-name match but field-type mismatch). Verifies width subtyping per spec rule 8.
- `crates/smelt-types/src/signatures.rs::tests::record_subtyping_through_nested_field` — `is_subtype_of(Record{a: Record{x: Text, y: Integer}}, Record{a: Record{x: Text}}) == true`. Width subtyping composes through nested record fields.
- `crates/smelt-types/src/signatures.rs::tests::smelt_record_registry_builder_detects_redefinition` — a synthetic list of two `SmeltRecordDeclaration` values with the same `name` produces a registry retaining the first declaration as authoritative and emitting one `SmeltRecordRedefinition` sentinel anchored at the second declaration's name span.
- `crates/smelt-types/src/signatures.rs::tests::smelt_record_registry_builder_detects_cycle_self` — a single declaration `Node = {child: Node}` produces one `RecordCyclicDeclaration` sentinel anchored at the cycle's introducing field-type expression.
- `crates/smelt-types/src/signatures.rs::tests::smelt_record_registry_builder_detects_cycle_mutual` — two declarations `A = {b: B}` and `B = {a: A}` produce exactly one `RecordCyclicDeclaration` sentinel (anchored at the introducing edge in DFS order — implementer's choice as long as it's deterministic).
- `crates/smelt-types/src/signatures.rs::tests::smelt_record_registry_builder_rejects_reflection_witness_field_types` — a declaration `Cohort = {model: ModelRef}` produces one `RecordFieldTypeForbidden` sentinel anchored at the `ModelRef` token; the same for `ColumnRef` and `SourceRef`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_codes_record_set_complete` — every record diagnostic code (`SmeltRecordRedefinition`, `RecordFieldUnknown`, `RecordFieldMissing`, `RecordFieldDuplicate`, `RecordFieldTypeMismatch`, `RecordLiteralUnknownTarget`, `RecordFieldNotProjectable`, `RecordFieldTypeForbidden`, `RecordCyclicDeclaration`, `RecordInDataWorld`) exists in the `DiagnosticCode` enum and renders the spec-`meta_language.md` §"Record diagnostic codes" message format.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_codes_map_set_complete` — every map diagnostic code (`MapKeyTypeNotText`, `MapApiUnknown`, `MapApiArityMismatch`, `MapApiNamedArgument`, `MapApiUnexpectedArgument`, `MapGetMissingKey`, `MapApiArgTypeMismatch`) exists and renders per spec.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_codes_loader_set_complete` — every loader diagnostic code (`ConfigLoaderPathNotLiteral`, `ConfigLoaderPathEscapesWorkspace`, `ConfigLoaderPathBackslash`, `ConfigLoaderFileNotFound`, `ConfigLoaderSchemaForbidden`, `ConfigLoaderTomlNotYetSupported`, `ConfigLoaderParseError`, `ConfigLoaderRequiredFieldMissing`, `ConfigLoaderUnknownField`, `ConfigLoaderTypeMismatch`, `ConfigLoaderRootShapeMismatch`, `ConfigLoaderDuplicateMapKey`, `ConfigLoaderNullCoercion`) exists and renders per `meta_config_loading.md` §"Validation diagnostics".

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - `SmeltType::Record { fields: BTreeMap<String, SmeltType>, name: Option<String> }` variant. Structural equality ignores `name` (use a `PartialEq` impl that compares fields only, or a `same_record_type` helper that the assignability rule consults). Display formats the closed field list as `Record<{f1: T1, f2: T2}>` (or `<TypeName>` when `name` is `Some`, with fallback to the structural form).
  - `SmeltType::Map { key: Box<SmeltType>, value: Box<SmeltType> }` variant. Invariance is enforced in the assignability function — `is_subtype_of(Map{K1,V1}, Map{K2,V2}) == (K1 == K2 && V1 == V2)`.
  - `pub const MAP_API_METHODS: &[MapApiMethod]` (or equivalent struct array) with the five entries: `{name: "entries", arity: Arity::Exact(0), return_type_formula: |k, v| List(Record({"key": k, "value": v}))}`, `keys` → `List<K>`, `values` → `List<V>`, `get` → `V`, `has` → `Boolean`. Each entry encodes whether named arguments are permitted (all `false`).
  - `pub struct SmeltRecordDeclaration { pub name: String, pub fields: Vec<(String, SmeltType, Span)>, pub name_span: Span, pub source_path: Arc<str> }`. The declaration carries the source-path so `SmeltRecordRedefinition`'s message can name the first declaration's path.
  - `pub fn build_record_registry(decls: &[SmeltRecordDeclaration]) -> (RecordRegistry, Vec<DiagnosticSentinel>)` — pure function that walks the declarations, builds a `HashMap<String, &SmeltRecordDeclaration>`, detects redefinition (emit `SmeltRecordRedefinition`), validates each field type (emit `RecordFieldTypeForbidden` on `ColumnRef`/`ModelRef`/`SourceRef`/`Lambda`), and performs DFS cycle detection over declared-name field-type edges (emit `RecordCyclicDeclaration`). The registry exposes `lookup(name) -> Option<&SmeltRecordDeclaration>` for the inference layer.
  - Width-subtyping assignability rule registered in `is_subtype_of` / `is_assignable_to` for `SmeltType::Record`. Per spec rule 8, the wider record (more fields) is the subtype.
  - `Map<K, V>` invariance rule registered in the same assignability function (equality, no covariance).
- `crates/smelt-db/src/lib.rs`:
  - `DiagnosticCode` variants added: eleven record codes, seven map codes, thirteen loader codes (the loader codes are *registered* here; the emission paths land in Phase 5). Render messages per the spec tables verbatim.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs` — `SmeltType::Record`, `SmeltType::Map`, `MAP_API_METHODS`, `SmeltRecordDeclaration`, `build_record_registry`, assignability rule additions.
- `crates/smelt-types/src/lib.rs` — only if a re-export is needed for the new types.
- `crates/smelt-db/src/lib.rs` — `DiagnosticCode` variant additions and `Display` impl entries; NOT Salsa queries (Phase 5).
- `crates/smelt-types/src/signatures.rs::tests` and `crates/smelt-db/src/lib.rs::tests` — the unit tests above.

**Docs touched.**

- None new in this phase (code-only). The spec rules cited above are normative; this phase makes them executable.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `SmeltType::Record` and `SmeltType::Map` additions are non-breaking (no missed exhaustive matches across crates; check with `cargo clippy --all-targets`).
- [ ] Structural equality on `SmeltType::Record` ignores `name`; the `name` accessor preserves the named-declaration hover label per spec rule 4.
- [ ] `MAP_API_METHODS` is the single source of truth for the v1 Map API; no method name appears as a string literal outside the registry definition or message templates.
- [ ] Width subtyping for records is one-directional (wider <: narrower) and composes through nested record fields.
- [ ] `Map<K, V>` invariance holds in both axes (verified by the dedicated test).
- [ ] `build_record_registry` is pure (no `db.` Salsa calls); cycle detection is deterministic; redefinition retains the first declaration.
- [ ] Reflection-witness exclusion is exhaustive (`ColumnRef`, `ModelRef`, `SourceRef`, `Lambda`).
- [ ] All Phase E1 diagnostic codes register and render per spec message format.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): SmeltType::Record + Map + workspace record registry + diagnostic codes (pure)`

---

### Phase 2: Parser — `smelt.record` top-level declarations + record literals + inline record types + Map method-call dispatch

**Goal.** Parse `smelt.record TypeName = { field1: Type1, field2: Type2, … }` as a top-level statement (sibling of `smelt.define`); parse `{f1: v1, f2: v2, …}` as a record-literal expression at value positions; parse `{f1: T1, f2: T2, …}` as an inline-record type expression at type-annotation positions; route post-dot identifier-followed-by-`(` on a Map-typed expression through the new `MAP_METHOD_CALL` CST production so Phase 4 inference can dispatch the closed Map API. Lexer reuses existing `LBRACE` / `RBRACE` / `COLON` tokens (already in the SQL grammar); no new tokens. Parser-level error recovery at sync points so a malformed record body does not avalanche.

**Pre-conditions.** Phase 1 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-parser/src/parser.rs::tests::parse_smelt_record_decl_top_level` — `smelt.record SourceEntry = { name: Text, columns: List<Text> }` parses to one `SMELT_RECORD_DECL` CST node with a name token (`SourceEntry`), two field-binding children (each a `(IDENT, COLON, TYPE_EXPR)` triple), and zero errors. Trailing comma admitted.
- `crates/smelt-parser/src/parser.rs::tests::parse_smelt_record_decl_field_with_record_type` — `smelt.record Cohort = { source: SourceEntry, settings: { threshold: Integer } }` parses with one field whose type is a `RECORD_TYPE_INLINE` CST node and one field whose type is a bare type-name reference (`SourceEntry`).
- `crates/smelt-parser/src/parser.rs::tests::parse_smelt_record_decl_recovers_on_malformed_field` — `smelt.record Bad = { x: Text, y: , z: Integer }` recovers: the parser produces a `SMELT_RECORD_DECL` node with three field-binding children (the middle one carries an error token for the missing type) and continues parsing the rest of the file without avalanche.
- `crates/smelt-parser/src/parser.rs::tests::parse_record_literal_in_select_item` — `SELECT smelt.foo({a: 1, b: 'x'}) FROM t` parses with a function-call argument that is a `RECORD_LITERAL` CST node containing two `(IDENT, COLON, EXPR)` children.
- `crates/smelt-parser/src/parser.rs::tests::parse_record_literal_in_define_default_value` — `smelt.define foo(cfg: Cohort = {threshold: 10}) = …` parses the default-value position as a `RECORD_LITERAL`. (Tests bidirectional placement.)
- `crates/smelt-parser/src/parser.rs::tests::parse_inline_record_type_at_define_parameter` — `smelt.define foo(cfg: { name: Text, count: Integer }) = …` parses the parameter's type-annotation as a `RECORD_TYPE_INLINE` CST node.
- `crates/smelt-parser/src/parser.rs::tests::parse_inline_record_type_nested_in_list` — `smelt.define foo(cs: List<{ name: Text }>) = …` parses to `List<RECORD_TYPE_INLINE>`.
- `crates/smelt-parser/src/parser.rs::tests::parse_map_method_call_entries` — `smelt.define foo(m: Map<Text, Integer>) = m.entries()` parses the body's `m.entries()` as a `MAP_METHOD_CALL` CST node (distinct from generic field-projection on records and from the Phase C/D record-witness field projection on `ColumnRef`/`ModelRef`/`SourceRef`).
- `crates/smelt-parser/src/parser.rs::tests::parse_map_method_call_get_with_arg` — `m.get('k')` parses as `MAP_METHOD_CALL` with one positional argument and zero named arguments.
- `crates/smelt-parser/src/parser.rs::tests::record_literal_vs_inline_record_type_disambiguation` — at a value position (e.g. inside a `SELECT` argument), `{a: 1}` parses as `RECORD_LITERAL`; at a type-annotation position (e.g. inside `smelt.record Foo = {…}` or `smelt.define foo(x: {…}) = …`), `{a: Integer}` parses as `RECORD_TYPE_INLINE`. The decision is parser-level (production-context-driven); no shared CST kind.
- `crates/smelt-parser/src/parser.rs::tests::record_literal_recovers_on_missing_value` — `{a: , b: 2}` produces a `RECORD_LITERAL` node with two field children, the first carrying an error token for the missing value, parser continues.
- `crates/smelt-parser/src/ast.rs::tests::ast_wrappers_for_record_constructs_round_trip` — typed AST wrappers `SmeltRecordDecl`, `RecordLiteral`, `RecordTypeInline`, `MapMethodCall` each round-trip from CST → AST → CST (text equality on the rendered span).

**Implementation shape.**

- `crates/smelt-parser/src/syntax_kind.rs`:
  - New syntax kinds: `SMELT_RECORD_DECL`, `RECORD_LITERAL`, `RECORD_TYPE_INLINE`, `MAP_METHOD_CALL`, plus auxiliary `RECORD_FIELD` (the `IDENT COLON …` triple) shared between literal, inline-type, and decl bodies.
- `crates/smelt-parser/src/parser.rs`:
  - Extend the existing top-level statement dispatcher to recognise `smelt.record` (token sequence `smelt . record`, mirroring `smelt.define`) and call a new `parse_smelt_record_decl` production.
  - Extend the type-annotation parser (the function consuming `: TYPE_EXPR` in `smelt.define` parameter lists and elsewhere) to recognise `{` and call a new `parse_record_type_inline` production.
  - Extend the expression / value parser to recognise `{` at value positions and call a new `parse_record_literal` production. The decision between record-literal and existing `{…}` SQL grammar uses (e.g. `STRUCT_PACK`) is production-context-driven; record literals only appear in meta-value positions, not in raw SQL grammar.
  - Extend the post-dot parser (the existing field-projection / method-call path) to recognise a method call (identifier followed by `(`) and emit `MAP_METHOD_CALL` when the LHS is a meta-typed expression. The LHS-type discrimination is parser-level minimal — emit `MAP_METHOD_CALL` whenever the dotted method has an argument list; the type-inference layer in Phase 4 produces `MapApiUnknown` if the LHS isn't a `Map<K, V>`. (Alternative: emit a generic `METHOD_CALL` CST kind and let Phase 4 route. Implementer's choice; the test asserts the chosen kind.)
  - Error recovery: after a malformed field, advance to the next `COMMA` or `RBRACE` sync point; the failing field carries an error token but the surrounding declaration / literal continues parsing.
- `crates/smelt-parser/src/ast.rs`:
  - Typed AST wrappers `SmeltRecordDecl`, `RecordLiteral`, `RecordTypeInline`, `RecordField`, `MapMethodCall` with accessor methods (`name()`, `fields()`, `value()`, `target_type()`, etc.).

**Critical files (allowed to touch in this phase).**

- `crates/smelt-parser/src/syntax_kind.rs`, `crates/smelt-parser/src/parser.rs`, `crates/smelt-parser/src/ast.rs` — productions and AST wrappers.
- `crates/smelt-parser/src/parser.rs::tests`, `crates/smelt-parser/src/ast.rs::tests` — the unit tests above.

**Docs touched.**

- None (parser surface is internal; user-visible surface lands in Phase 6).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] No new lexer tokens introduced; `LBRACE`/`RBRACE`/`COLON` reuse is confirmed via lexer-test grep.
- [ ] `smelt.record` keyword precedence does not regress `smelt.define`, `smelt.config.*`, or `smelt.models.*` / `smelt.sources.*` top-level recognition.
- [ ] Record-literal vs inline-record-type disambiguation is production-context-driven and reproducible across the three parser entry points (value position, type-annotation position, `smelt.record` body).
- [ ] `MAP_METHOD_CALL` disambiguation from `ColumnRef` / `ModelRef` / `SourceRef` field projections is correct — the latter remain field projections (no parentheses), the former is always a method call (parentheses required by spec). Verified by tests under `crates/smelt-parser/src/parser.rs::tests`.
- [ ] Parser-level error recovery handles malformed record bodies, inline types, and literals without avalanche; subsequent file content parses cleanly.
- [ ] No analysis logic or Salsa imports added to `smelt-parser`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): parser — smelt.record decls + record literals + inline types + Map method-call`

---

### Phase 3: Type-inference — records (declarations, literals, field projection, width subtyping)

**Goal.** Wire pure type-inference for the record surface. Build the workspace `RecordRegistry` from the file's `smelt.record` declarations at type-context construction. Bidirectional record-literal checking against a target type, with required-field / unknown-field / duplicate-field / type-mismatch diagnostics and drop-on-error per field. Field projection on a record-typed value with closed-set lookup, emitting `RecordFieldUnknown` (and `RecordFieldNotProjectable` for mid-chain projection of non-record fields). Width subtyping registered in the assignability rule from Phase 1, exercised at every assignment site. `RecordLiteralUnknownTarget` for unanchored literals; `RecordInDataWorld` for record-typed bindings referenced in a Data-World position outside a splice context. All inference remains pure (no Salsa calls in `type_inference.rs`); the registry is built by an orchestration helper in `lib.rs` (Phase 5 territory) and passed into the inference as a parameter on `TypeContext` — Phase 3 establishes the shape, Phase 5 wires it end-to-end.

**Pre-conditions.** Phases 1–2 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_against_named_target_emits_no_diagnostic_on_happy_path` — given a workspace with `smelt.record Cohort = { name: Text, threshold: Integer }`, the literal `{name: 'us_west', threshold: 100}` at a position expecting `Cohort` synthesises `Record{Cohort}` with no diagnostic; the constructed record's field types match the declaration.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_emits_record_field_missing` — `{name: 'us_west'}` against `Cohort` (above) emits exactly one `RecordFieldMissing` anchored at the literal's closing brace, naming `threshold`. The synthesised type is `Record{Cohort}` (recoverable; downstream consumers see the partial record).
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_emits_record_field_unknown` — `{name: 'us_west', threshold: 100, bogus: true}` against `Cohort` emits one `RecordFieldUnknown` anchored at the `bogus` token; `bogus` is dropped from the constructed record; no follow-on diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_emits_record_field_duplicate` — `{name: 'us_west', name: 'eu', threshold: 100}` against `Cohort` emits one `RecordFieldDuplicate` anchored at the second `name` occurrence.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_emits_record_field_type_mismatch` — `{name: 'us_west', threshold: 'lots'}` against `Cohort` emits one `RecordFieldTypeMismatch` anchored at the `'lots'` value expression; the constructed record carries `Unknown` at `threshold` (drop-on-error).
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_literal_emits_record_literal_unknown_target_when_unanchored` — a bare `{name: 'us_west', threshold: 100}` in a position with no inferable target type (e.g. directly inside an `Expr<Text>` argument) emits one `RecordLiteralUnknownTarget` anchored at the literal's opening brace; the literal evaluates as `Record<Unknown>`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_field_projection_synthesises_field_type` — given `c: Cohort`, `c.name` synthesises `Text`; `c.threshold` synthesises `Integer`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_field_projection_emits_record_field_unknown_on_miss` — `c.bogus` emits one `RecordFieldUnknown` anchored at the `bogus` token; the projection synthesises `Unknown`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_record_field_projection_emits_record_field_not_projectable_mid_chain` — `c.name.foo` (where `c.name` is `Text`) emits one `RecordFieldNotProjectable` anchored at `.foo`; the surrounding expression's type is `Unknown`.
- `crates/smelt-db/src/type_inference.rs::tests::record_width_subtyping_assigns_wider_to_narrower` — given `c: Cohort` (`{name: Text, threshold: Integer}`) at a position expecting `Record{name: Text}`, the assignment is admitted with no diagnostic. The reverse direction emits a type-mismatch diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::record_width_subtyping_projection_diagnostic_unchanged_under_widening` — given `c: Cohort` consumed at a position expecting `Record{name: Text}`, projecting `c.threshold` at that consumer position emits `RecordFieldUnknown` against the narrower static type (the closed declared set wins per spec rule "Width subtyping does not weaken field-projection diagnostics").
- `crates/smelt-db/src/type_inference.rs::tests::smelt_record_declaration_redefinition_emits_diagnostic` — given two `smelt.record Cohort = {…}` declarations in different files, the workspace's type-context construction emits exactly one `SmeltRecordRedefinition` anchored at the second declaration's name token; first declaration is authoritative for downstream references.
- `crates/smelt-db/src/type_inference.rs::tests::smelt_record_cyclic_declaration_emits_diagnostic` — given `smelt.record A = {b: B}` and `smelt.record B = {a: A}`, the type-context construction emits one `RecordCyclicDeclaration`; downstream uses of `A` and `B` continue to type-check (one declaration retained as the cycle-broken authoritative entry).
- `crates/smelt-db/src/type_inference.rs::tests::record_field_type_forbidden_for_reflection_witnesses` — `smelt.record Bad = {m: ModelRef}` emits one `RecordFieldTypeForbidden` anchored at the `ModelRef` token at declaration registration time. Same for `ColumnRef`, `SourceRef`, `Lambda<…>`.
- `crates/smelt-db/src/type_inference.rs::tests::record_in_data_world_emits_diagnostic_when_consumed_at_sql_position` — a binding `c: Cohort` referenced bare inside a SQL `WHERE` clause (a non-splice Data-World position) emits one `RecordInDataWorld` at the binding's reference token. Projecting `c.name` at the same position does not (the projection enters the splice via the `Text` field type).
- `crates/smelt-db/src/type_inference.rs::tests::inline_record_and_named_record_with_same_field_set_are_assignable` — given `smelt.record Cohort = {name: Text, threshold: Integer}`, a value typed `Record{Cohort}` is assignable to a position expecting `{name: Text, threshold: Integer}` and vice versa with no diagnostic.

**Implementation shape.**

- `crates/smelt-db/src/type_inference.rs`:
  - Extend `TypeContext` to carry an `Arc<RecordRegistry>` (the workspace-globally-built record-declaration table from Phase 1). The Salsa orchestration in `lib.rs` (Phase 5) builds this; Phase 3 only consumes it inside pure inference.
  - `pub fn infer_record_literal(literal: &RecordLiteral, ctx: &TypeContext, expected: Option<&SmeltType>) -> RecordLiteralInferResult` — bidirectional. Walks fields left-to-right, looks each up against the target's declared field set (`expected` resolved against the registry for named targets; structural fields for inline targets), emits per-field diagnostics, returns the constructed `SmeltType::Record` plus the diagnostic list. Drop-on-error per field per spec rule 6.
  - `pub fn infer_record_field_projection(receiver_type: &SmeltType, field_name: &str, field_span: Span, ctx: &TypeContext) -> FieldProjectionResult` — closed-set lookup; emit `RecordFieldUnknown` or `RecordFieldNotProjectable` per spec rule 7.
  - Extend the existing dispatch in `infer_expression_type` (or equivalent) to route `RECORD_LITERAL` to `infer_record_literal` (with `expected` derived from the surrounding context) and route dotted-identifier on a record-typed receiver to `infer_record_field_projection`.
  - `RecordInDataWorld` emission at every Data-World-position binding-reference site that resolves to a `SmeltType::Record` and is not consumed at a splice point. Re-use the existing splice-context machinery from Phase B.
  - `pub fn validate_record_declaration_field_types(decl: &SmeltRecordDeclaration) -> Vec<DiagnosticSentinel>` — pure helper that re-uses Phase 1's `build_record_registry` logic at a per-declaration granularity for declaration-time diagnostics (`RecordFieldTypeForbidden`, embedded as Phase 1; this phase wires the per-file diagnostic emission at the declaration site).
- `crates/smelt-db/src/lib.rs`:
  - A new pure-function call site (`record_registry_for_workspace(...)`) that consumes parsed `smelt.record` declarations from every file and builds the `RecordRegistry`. The Salsa query that supplies declarations is `smelt_record_declarations(workspace)`; the wiring of that query is Phase 5 territory. Phase 3 lands the *pure builder* and tests it with a synthetic workspace fixture.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/type_inference.rs` — pure inference for records (literals, projection, declaration validation).
- `crates/smelt-types/src/signatures.rs` — only if a small helper accessor needs adding (e.g. a `RecordRegistry::lookup` method). The registry type from Phase 1 is the single source of truth.
- `crates/smelt-db/src/lib.rs` — the pure builder `record_registry_for_workspace` only; the Salsa query and orchestration land in Phase 5.
- `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.

**Docs touched.**

- None new in this phase (code-only).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `type_inference.rs` and `signatures.rs` remain pure (no `db.` Salsa calls in any function body).
- [ ] Bidirectional record-literal checking terminates (a literal walks fields exactly once per spec rule 11).
- [ ] Width subtyping does not weaken field-projection diagnostics (verified by the dedicated test).
- [ ] Inline and named records with identical field sets are mutually assignable per spec rule 4.
- [ ] Drop-on-error per field: a single missing / duplicate / type-mismatch / unknown field does not avalanche follow-on diagnostics within the same literal.
- [ ] `RecordInDataWorld` fires only at Data-World non-splice positions; record-typed values consumed inside `smelt.define` bodies, HOFs, and other meta-positions do not trigger it.
- [ ] Cycle detection is deterministic and emits exactly one `RecordCyclicDeclaration` per cycle (no per-edge avalanche).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): record literal + field projection + width subtyping inference (pure)`

---

### Phase 4: Type-inference — `Map<K, V>` API dispatch + invariance + statically-known-key resolution

**Goal.** Wire pure type-inference for the closed `Map<K, V>` API. Dispatch the five methods (`entries`, `keys`, `values`, `get`, `has`) on `Map`-typed receivers via the `MAP_API_METHODS` registry from Phase 1, emitting `MapApiUnknown` on miss, `MapApiArityMismatch` / `MapApiNamedArgument` / `MapApiUnexpectedArgument` on shape mismatches, and `MapApiArgTypeMismatch` on key-type mismatch at `m.get(k)` / `m.has(k)`. Statically-known-key resolution at `m.get(k)` / `m.has(k)` when the Map's contents are bound at call time and `k` is a string literal: present → typed value; absent → `MapGetMissingKey` + `Unknown`; non-static `k` → typed `V`, evaluation deferred to expansion time. `MapKeyTypeNotText` at any `Map<K, V>` type expression with `K != Text`. `Map<K, V>` invariance enforced by the assignability rule from Phase 1, exercised at every assignment site (verified end-to-end here). All inference pure.

**Pre-conditions.** Phases 1–3 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/type_inference.rs::tests::map_api_entries_synthesises_list_of_record` — given a binding `m: Map<Text, Integer>`, `m.entries()` synthesises `List<Record{key: Text, value: Integer}>` with no diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_keys_and_values_synthesise_lists` — `m.keys()` synthesises `List<Text>`; `m.values()` synthesises `List<Integer>`. Both with no diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_get_synthesises_value_type_on_non_static_key` — given `m: Map<Text, Integer>` and a non-literal `k: Text`, `m.get(k)` synthesises `Integer` (evaluation deferred); no diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_get_statically_known_present_key_synthesises_value_and_resolves` — given a Map whose contents are bound at type-check time (e.g. a loader-supplied Map with `{'a': 1, 'b': 2}`), `m.get('a')` synthesises `Integer` and the resolved value is `1`. No diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_get_statically_known_missing_key_emits_diagnostic` — `m.get('c')` (with `m` as above) emits one `MapGetMissingKey` anchored at the call expression; synthesised type is `Unknown` for drop-on-error.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_has_statically_known_returns_boolean_literal` — `m.has('a')` (present) synthesises `Boolean` and resolves to `TRUE`; `m.has('c')` (absent) synthesises `Boolean` and resolves to `FALSE` (no diagnostic; `has` is the presence-check sibling of `get`).
- `crates/smelt-db/src/type_inference.rs::tests::map_api_unknown_method_emits_diagnostic` — `m.bogus()` emits one `MapApiUnknown` anchored at the `bogus` token; synthesised type is `Unknown`.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_arity_mismatch_on_get_emits_diagnostic` — `m.get()` and `m.get('a', 'b')` each emit one `MapApiArityMismatch` anchored at the argument-list span.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_named_argument_emits_diagnostic` — `m.get(key => 'a')` emits one `MapApiNamedArgument` anchored at the named-argument span.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_unexpected_argument_on_entries_emits_diagnostic` — `m.entries('x')` emits one `MapApiUnexpectedArgument` anchored at the `'x'` argument; `m.keys()` and `m.values()` are checked analogously.
- `crates/smelt-db/src/type_inference.rs::tests::map_api_arg_type_mismatch_emits_diagnostic` — given `m: Map<Text, Integer>`, `m.get(42)` emits one `MapApiArgTypeMismatch` anchored at the `42` argument.
- `crates/smelt-db/src/type_inference.rs::tests::map_key_type_not_text_emits_diagnostic` — a type expression `Map<Integer, Text>` in any annotation position emits one `MapKeyTypeNotText` anchored at the `Integer` key-type expression; the surrounding declaration treats the map as `Map<Text, Text>` to avoid avalanche (per spec rule 1).
- `crates/smelt-db/src/type_inference.rs::tests::map_invariance_in_value_axis_rejects_assignment` — given `m: Map<Text, Integer>` at a position expecting `Map<Text, Number>`, the assignment emits a type-mismatch diagnostic (Map is invariant in `V` even when `Integer <: Number`).
- `crates/smelt-db/src/type_inference.rs::tests::map_invariance_does_not_block_record_value_projection` — given `m: Map<Text, Cohort>` (where `Cohort` is a wider record), projecting `m.values()[0].name` type-checks; the width-subtyping recovery happens at the projection of `entries()[i].value`, not at the Map type level (per spec rule "Width subtyping over record-typed `V` is handled at the projection of `m.entries()[i].value`, not at the `Map` type level").

**Implementation shape.**

- `crates/smelt-db/src/type_inference.rs`:
  - `pub fn infer_map_method_call(receiver_type: &SmeltType, method_name: &str, args: &[ArgExpr], ctx: &TypeContext) -> MapMethodCallResult` — pure. Closed-set lookup against `MAP_API_METHODS`; arity / named-arg / arg-type checks; static-key resolution branch when the Map's contents are bound (the contents come from a Salsa-cached loader value in Phase 5; Phase 4's tests use synthetic in-memory Maps).
  - `pub fn validate_map_type_expression(map_type: &MapTypeExpr) -> Vec<DiagnosticSentinel>` — pure. Checks `K = Text`; emits `MapKeyTypeNotText` otherwise. Called at every type-annotation position that produces a `SmeltType::Map`.
  - Extend the existing dispatch in `infer_expression_type` (or equivalent) to route `MAP_METHOD_CALL` to `infer_map_method_call`. The LHS-type discrimination (Map vs Record method-call vs unknown) lives here.
  - Statically-known-key resolution path: an `m: SmeltType::Map` whose accompanying value carries a `BTreeMap<String, Value>` of resolved entries (the loader-supplied shape from Phase 5) is the discriminator for static resolution. Phase 4 lands the shape; Phase 5 wires real loader values.
  - `Map<K, V>` invariance is already in `signatures.rs::is_subtype_of` from Phase 1; Phase 4 exercises it end-to-end at every assignment site that produces or consumes a `Map`.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/type_inference.rs` — pure inference for Map method dispatch, type-expression validation, statically-known-key resolution shape.
- `crates/smelt-types/src/signatures.rs` — only if a small accessor on `MAP_API_METHODS` needs adding.
- `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.

**Docs touched.**

- None new in this phase (code-only).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `type_inference.rs` purity preserved.
- [ ] `MAP_API_METHODS` is the only source of truth for the closed Map API; no method name appears as a string literal in the dispatch code outside the registry walk.
- [ ] Statically-known-key resolution path is correctly gated: triggered when the Map's contents are bound at call time AND the argument is a string literal; deferred otherwise.
- [ ] `MapKeyTypeNotText` fires at the offending key-type expression with avalanche protection per spec rule 1.
- [ ] `Map<K, V>` invariance correctly registered (verified by the dedicated test) and does not block the documented record-value-projection escape hatch.
- [ ] Drop-on-error semantics: `MapApiUnknown`, `MapGetMissingKey`, `MapApiArgTypeMismatch` synthesise `Unknown` so surrounding expression continues type-checking.
- [ ] No new diagnostic codes added — the seven from Phase 1 cover the surface.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): Map<K,V> API dispatch + invariance + statically-known-key resolution`

---

### Phase 5: Loader Salsa inputs + dispatch + YAML/JSON parsers + validation with source spans

**Goal.** Wire the schema-typed config loader family end-to-end: Salsa inputs for per-loader-call file paths and per-target overlay files; Salsa query `loader_resolved_value(call_site_id)` that resolves the loader's path + schema → typed meta-world value with source spans on every validation diagnostic; a new module `crates/smelt-db/src/loader.rs` housing the per-format parsers (YAML via `marked-yaml` for span retention; JSON via `serde_json` with byte-offset tracking) and the schema validator. Implement type-inference dispatch for `smelt.config.load_yaml`, `smelt.config.load_json`, and the reserved `smelt.config.load_toml`. Emit the thirteen loader diagnostic codes with primary frame at the YAML/JSON row (or call site for argument-shape codes) and secondary frame at the loader call site for content-validation codes. Build the workspace `smelt_record_declarations(workspace)` Salsa query that supplies the `RecordRegistry` to the `TypeContext` from Phase 3. **Per-target overlay is deferred to Phase 6** to keep this phase focused on the base-file load path; the Salsa input shape leaves room for the overlay query in Phase 6.

**Pre-conditions.** Phases 1–4 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/loader.rs::tests::yaml_parse_record_root_happy_path` — given a YAML file `{name: us_west, threshold: 100}` parsed against schema `Cohort = {name: Text, threshold: Integer}`, the validator returns a `Value::Record` with both fields bound; no diagnostics.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_record_root_missing_field` — given `{name: us_west}` against `Cohort`, the validator returns a partial record and emits one `ConfigLoaderRequiredFieldMissing` anchored at the YAML file's first line (the row-level anchor per spec); the diagnostic's secondary frame is the loader call site.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_record_root_unknown_field` — given `{name: us_west, threshold: 100, bogus: true}` against `Cohort`, emits one `ConfigLoaderUnknownField` anchored at the YAML row of `bogus`.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_record_root_type_mismatch` — given `{name: us_west, threshold: 'lots'}` against `Cohort`, emits one `ConfigLoaderTypeMismatch` anchored at the YAML row of the value `'lots'`.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_list_root_emits_root_shape_mismatch_when_mapping` — given a YAML mapping `{a: 1}` against schema `List<Cohort>`, emits one `ConfigLoaderRootShapeMismatch` anchored at the file's first line.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_list_root_validates_each_element` — given a YAML sequence of three Cohort entries with the second omitting `threshold`, emits exactly one `ConfigLoaderRequiredFieldMissing` anchored at the second element's row.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_map_root_validates_each_entry` — given a YAML mapping of three `name -> Cohort` entries with the third's value omitting `name`, emits one `ConfigLoaderRequiredFieldMissing` anchored at the third entry's row.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_map_root_emits_duplicate_key` — given a YAML mapping with two entries for key `'us_west'`, emits one `ConfigLoaderDuplicateMapKey` anchored at the second `'us_west'` row; the diagnostic carries the first row as a secondary frame per the spec message shape.
- `crates/smelt-db/src/loader.rs::tests::yaml_parse_invalid_yaml_emits_parse_error` — given malformed YAML, emits one `ConfigLoaderParseError` anchored at the parser's reported line/column.
- `crates/smelt-db/src/loader.rs::tests::yaml_null_coercion_to_text_emits_warning` — given a `Text` field bound to YAML `null`, the validator returns the field as empty string and emits one `ConfigLoaderNullCoercion` (severity: warning) at the YAML row.
- `crates/smelt-db/src/loader.rs::tests::json_parse_record_root_happy_path` — given JSON `{"name": "us_west", "threshold": 100}` against `Cohort`, returns a `Value::Record`; no diagnostics.
- `crates/smelt-db/src/loader.rs::tests::json_parse_trailing_comma_emits_parse_error` — given JSON with a trailing comma, emits one `ConfigLoaderParseError` at the byte offset.
- `crates/smelt-db/src/loader.rs::tests::json_null_coerces_only_to_text` — given JSON `null` at an `Integer` field, emits `ConfigLoaderTypeMismatch`; at a `Text` field, emits `ConfigLoaderNullCoercion` warning.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_path_must_be_literal_emits_diagnostic` — `smelt.config.load_yaml(some_var, {f: Text})` emits one `ConfigLoaderPathNotLiteral` at the argument expression.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_path_escapes_workspace_emits_diagnostic` — paths `'/etc/passwd'`, `'../escape.yaml'`, `'http://x.com/c.yaml'`, `'s3://bucket/c.yaml'` each emit one `ConfigLoaderPathEscapesWorkspace` at the path literal.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_path_backslash_emits_diagnostic` — `'configs\\cohorts.yaml'` emits one `ConfigLoaderPathBackslash` at the offending character span.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_file_not_found_emits_diagnostic` — `smelt.config.load_yaml('nope.yaml', {f: Text})` against a workspace where the file doesn't exist emits one `ConfigLoaderFileNotFound` at the path literal.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_schema_forbidden_emits_diagnostic` — `smelt.config.load_yaml('c.yaml', Integer)` emits one `ConfigLoaderSchemaForbidden` at the schema argument; bare scalars / `Lambda<…>` / `ColumnRef` are all forbidden.
- `crates/smelt-db/src/type_inference.rs::tests::load_toml_emits_reserved_diagnostic` — `smelt.config.load_toml('c.toml', {f: Text})` emits one `ConfigLoaderTomlNotYetSupported` at the call expression; the call's synthesised type is `Unknown`.
- `crates/smelt-db/src/type_inference.rs::tests::load_yaml_synthesises_schema_type_on_happy_path` — `smelt.config.load_yaml('cohorts.yaml', Cohort)` synthesises `Record{Cohort}` when the file is valid; the resolved value carries the parsed record for static-key Map resolution downstream.
- `crates/smelt-db/src/lib.rs::tests::loader_file_text_is_salsa_input` — adding/modifying a loader-target file invalidates the `loader_resolved_value` query for any call site referencing that file (Salsa-cache invariant).
- `crates/smelt-db/src/lib.rs::tests::smelt_record_declarations_query_collects_workspace_decls` — a workspace with two files each declaring one `smelt.record` produces a registry with both names; modifying one file's declarations invalidates downstream type-checks.

**Implementation shape.**

- `crates/smelt-db/src/loader.rs` (new module):
  - `pub struct ParsedConfigFile { format: ConfigFormat, value: ParsedValue, span_index: SpanIndex }` — the format-agnostic intermediate carrying per-row spans.
  - YAML parser: `pub fn parse_yaml(text: &str) -> Result<ParsedConfigFile, ConfigLoaderParseError>`. Implementation uses `marked-yaml` (preferred for span retention) or `serde_yaml` if dependency-tree constraints force it; the choice is implementer's, with `marked-yaml` as the strong default given the spec's diagnostic anchoring requirement.
  - JSON parser: `pub fn parse_json(text: &str) -> Result<ParsedConfigFile, ConfigLoaderParseError>`. Implementation uses `serde_json::from_str_seed` (or a minimal hand-rolled tokenizer over `serde_json::Deserializer`) to track byte offsets for each value node.
  - `pub fn validate_against_schema(parsed: &ParsedConfigFile, schema: &SmeltType, call_site_span: Span) -> ValidationResult` — pure schema-validation function. Returns a typed `Value` (the meta-world representation of the parsed-and-validated content) plus a list of diagnostics, each with primary span at the YAML/JSON row and secondary span at the loader call site.
  - Per-format scalar-coercion rules per `meta_config_loading.md` §"Per-format rules" — `Text`/`Integer`/`Float`/`Boolean`/`Date`/`Timestamp`/`Decimal` coercions and `null` handling.
- `crates/smelt-db/src/lib.rs`:
  - New Salsa inputs: `loader_file_text(workspace_relative_path: Arc<str>) -> Arc<str>` and `loader_file_exists(workspace_relative_path: Arc<str>) -> bool`. Registered at workspace-init time for every loader call site (the call site walk happens in `type_inference.rs` via a parsed-file scan; the inputs are seeded by the Salsa orchestration layer).
  - New Salsa query: `loader_file_parsed(workspace_relative_path: Arc<str>) -> Arc<Result<ParsedConfigFile, ConfigLoaderParseError>>`.
  - New Salsa query: `loader_resolved_value(call_site_id: LoaderCallSiteId) -> Arc<LoaderResolvedValue>` — composes the path → text → parsed → validated chain.
  - New Salsa query: `smelt_record_declarations(workspace) -> Arc<Vec<SmeltRecordDeclaration>>` — walks every workspace file's parsed AST and collects top-level `smelt.record` declarations; feeds Phase 3's `RecordRegistry`.
  - `LoaderCallSiteId` is the parse-time identifier of a `smelt.config.load_yaml` / `_json` call expression (file path + AST node id); the Salsa key shape is whatever lets memoisation work across edits.
- `crates/smelt-db/src/type_inference.rs`:
  - Dispatch for `smelt.config.load_yaml`, `smelt.config.load_json`, `smelt.config.load_toml` per spec §"Loader calls" and §"Path resolution":
    - `load_toml` → `ConfigLoaderTomlNotYetSupported` at the call expression; synthesised type is `Unknown` (recoverable).
    - Literal-only `path` argument check: emit `ConfigLoaderPathNotLiteral` if not a string literal.
    - Path validation: workspace-relative check (no leading `/`, no `..` escape, no scheme prefix) → `ConfigLoaderPathEscapesWorkspace`; backslash check → `ConfigLoaderPathBackslash`; existence check → `ConfigLoaderFileNotFound`.
    - Schema admissibility check: bidirectionally accept inline record types, named record references, `List<S>` of either, `Map<Text, S>` of either; reject everything else → `ConfigLoaderSchemaForbidden`.
    - On success: call the Salsa `loader_resolved_value` query, attach the validation diagnostics, synthesise the schema's declared type.
  - Inline schema and named schema both feed `validate_against_schema`; the named-schema path resolves the record reference through the `RecordRegistry`.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/loader.rs` (new module) — YAML/JSON parsers, schema validation, source-span retention.
- `crates/smelt-db/src/lib.rs` — Salsa inputs and queries for loader paths and parsed/resolved values; `smelt_record_declarations(workspace)` query; orchestration of `TypeContext` to carry the `RecordRegistry`.
- `crates/smelt-db/src/type_inference.rs` — loader-call dispatch (path / schema admissibility / synthesised return type); routes into the Salsa `loader_resolved_value` query.
- `Cargo.toml` for `smelt-db` — add `marked-yaml` (or `serde_yaml`) and confirm `serde_json` is already in the tree.
- `crates/smelt-db/src/loader.rs::tests`, `crates/smelt-db/src/type_inference.rs::tests`, `crates/smelt-db/src/lib.rs::tests` — the unit tests above.

**Docs touched.**

- None. The spec is the oracle; if the validator implementation surfaces a gap, run `/smelt:spec meta_config_loading` first.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] YAML parser preserves per-row spans through validation; primary frame at the YAML row, secondary at the loader call site for content-validation diagnostics; primary at the loader call site (or path literal / schema arg) for argument-shape diagnostics.
- [ ] JSON parser preserves byte-offset spans through validation; analogous anchoring.
- [ ] Every loader diagnostic from `meta_config_loading.md` is reachable and the message format matches the spec table.
- [ ] Salsa input registration covers every loader-target file; `loader_resolved_value` invalidates correctly on file edits (verified test).
- [ ] `smelt_record_declarations(workspace)` is the single Salsa-cached source of `RecordRegistry`; `TypeContext` consumes it via the Phase 3 orchestration shape.
- [ ] `type_inference.rs` and `signatures.rs` remain pure (no `db.` Salsa calls inside analysis logic; loader dispatch *constructs* a query call site via the orchestration layer in `lib.rs`).
- [ ] `load_toml` emits the reserved diagnostic and cannot be bound in `smelt.define` (spec rule on reservation).
- [ ] Per-target overlay is NOT implemented in this phase; the Salsa input shape leaves room for the Phase 6 overlay query without restructuring.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): YAML/JSON loader parsers + Salsa inputs + schema validation`

---

### Phase 6: Per-target overlay + LSP + examples/meta_config + user docs + skill update + `/smelt-loop` medium tier extension

**Goal.** Land the per-target file-overlay resolution and merge (`<basename>.<target>.<ext>` sibling, record deep-merge, list replace, map per-key replace) on top of the Phase 5 base-load path. Wire LSP support for every Phase E1 construct — hover, goto-def, completion paths per spec — across records, maps, and loaders. Add the `examples/meta_config/` fixture exercising the killer-demo's E1 prerequisites (declared record + YAML loader + `Map<Text, Record>` + `entries`-projection HOF chain + per-target overlay) plus one broken sub-fixture for each new Phase E1 diagnostic. Author the user docs at `docs-site/docs/meta-language/{records,maps,config-loaders,reference}.md`. Add the `smelt-app-builder` skill reference doc and extend `/smelt-loop`'s `medium` tier with at least one Phase E1-specific ask. Update the `meta_language.md` Known Divergences entry to reflect that records / maps / loaders are landed (residual divergences — recursive schemas, per-key deep-merge for Map overlays, `Optional<V>` — remain).

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/loader.rs::tests::overlay_record_root_deep_merges_overridden_field` — given a base file `{name: 'us_west', region: 'us-west-2', threshold: 100}` and an overlay file `{threshold: 50}` (target=`prod`), the resolved value's `threshold` is `50`; `name` and `region` come from the base.
- `crates/smelt-db/src/loader.rs::tests::overlay_list_root_replaces_base` — given a base file `[{name: a}, {name: b}]` and an overlay `[{name: c}]`, the resolved value is `[{name: c}]` (replace, not concatenate).
- `crates/smelt-db/src/loader.rs::tests::overlay_map_root_replaces_per_key` — given a base `{a: {threshold: 100}, b: {threshold: 200}}` and an overlay `{a: {threshold: 50}}`, the resolved value is `{a: {threshold: 50}, b: {threshold: 200}}` (per-key replace; b unchanged).
- `crates/smelt-db/src/loader.rs::tests::overlay_validation_failure_anchors_at_overlay_row` — an overlay file that fails the schema check emits one diagnostic anchored at the overlay file's offending row (not at the base file or the loader call site).
- `crates/smelt-db/src/loader.rs::tests::overlay_absent_falls_through_to_base` — when no `<basename>.<target>.<ext>` sibling exists, the resolved value equals the base file's value byte-for-byte; no overlay-load query fired.
- `crates/smelt-db/src/lib.rs::tests::overlay_file_change_invalidates_loader_value` — modifying the overlay file invalidates `loader_resolved_value` for the affected target; switching targets invalidates appropriately.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_smelt_record_decl_name_shows_field_list` — hover on the name token of `smelt.record Cohort = {…}` returns text containing the closed field list with types and the declaration's file path.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_record_typed_binding_shows_record_type` — hover on `c` inside `smelt.define foo(c: Cohort) = …` returns `Cohort` plus the closed field list with types.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_record_field_projection_shows_field_type` — hover on `.name` of `c.name` returns `Text`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_record_literal_opening_brace_shows_inferred_target` — hover on `{` of `{name: 'us_west', threshold: 100}` (in a position expecting `Cohort`) returns `Cohort`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_map_typed_binding_shows_resolved_summary` — hover on `m` (resolved to `{'a': 1, 'b': 2}`) returns `Map<Text, Integer>` plus "2 entries" plus the first five keys.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_map_method_call_shows_signature_and_resolution` — hover on `m.entries()` returns the signature `Map<Text, V> -> List<{key: Text, value: V}>` plus the resolved length when statically known.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_loader_call_shows_resolved_path_and_summary` — hover on `smelt.config.load_yaml('cohorts.yaml', Cohort)` returns the resolved path, the file's row count or field set, and the file's last-modified timestamp.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_on_smelt_record_name_resolves_to_declaration` — go-to-definition on a reference to `Cohort` (in a type-annotation position or loader schema argument) resolves to the declaration's name-span in the workspace.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_on_record_literal_field_resolves_to_declared_field_span` — go-to-definition on `name` in `{name: 'us_west', …}` (against target `Cohort`) resolves to the `name` declaration in the `Cohort` body.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_on_loader_path_resolves_to_file` — go-to-definition on the path literal of `smelt.config.load_yaml('cohorts.yaml', Cohort)` resolves to `cohorts.yaml` at row 1.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_on_loaded_record_field_projection_resolves_to_yaml_row` — given a loader-resolved value `c = smelt.config.load_yaml('cohorts.yaml', Cohort)` and a projection `c.name`, go-to-definition on `.name` resolves to the YAML file's `name` row when statically traceable.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_record_literal_field_key_offers_unfilled_fields` — completion at `{name: 'us_west', <cursor>}` (target=`Cohort`) offers `threshold` (the unfilled remaining field) with type `Integer` as the completion-item detail.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_record_field_projection_offers_closed_set` — completion at `c.<cursor>` (where `c: Cohort`) offers `{name, threshold}` with their declared types.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_map_method_position_offers_closed_set` — completion at `m.<cursor>` offers `{entries, keys, values, get, has}` with arities and signatures as completion-item details.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_map_get_arg_offers_statically_known_keys` — completion at `m.get(<cursor>)` (where `m`'s keys are statically known) offers the first ~50 keys with their bound values' types.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_loader_path_offers_filesystem_entries` — completion at the path argument of `smelt.config.load_yaml('<cursor>')` offers workspace-relative `.yaml`/`.yml` files; for `load_json`, `.json` files.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_loader_schema_offers_in_scope_record_names` — completion at the schema argument offers in-scope `smelt.record` names plus a `{<cursor>}` stub for inline schemas.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::examples_meta_config_clean_passes` — the `examples/meta_config/` clean workspace produces zero diagnostics.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::examples_meta_config_broken_subfixtures_each_emit_one_diagnostic` — each broken sub-fixture (one per Phase E1 diagnostic, named per the existing precedent `examples/meta_config_broken_<diagnostic_code_snake>/`) reports exactly one Phase E1 diagnostic with the correct code.

**Implementation shape.**

- `crates/smelt-db/src/loader.rs`:
  - `pub fn resolve_with_overlay(base_path: &Path, target: Option<&str>, schema: &SmeltType, db: &dyn LoaderDb) -> LoaderResolvedValue` — orchestrates the base-file load + overlay-file load (if `target` set and `<basename>.<target>.<ext>` exists) + merge per the rule.
  - `pub fn merge_values(base: Value, overlay: Value, schema: &SmeltType) -> Value` — pure merge function:
    - Record root: per-field overlay-replaces-base; recursive into nested records.
    - List root: overlay replaces base entirely.
    - Map root: per-key overlay-replaces-base; keys absent in the overlay come from the base.
- `crates/smelt-db/src/lib.rs`:
  - Extend `loader_resolved_value(call_site_id)` to consult `target` (from the project's `smelt.yml` `target:` field or the build CLI's `--target` flag) and call `resolve_with_overlay`. The Salsa input `loader_file_text` covers overlay files too — registered eagerly for every `<basename>.<target>.<ext>` sibling, with `loader_file_exists` controlling whether the merge runs.
- `crates/smelt-lsp/src/lib.rs`:
  - Hover handler: route `RECORD_LITERAL` / `RECORD_TYPE_INLINE` / record-typed bindings / record-field projections / `MAP_METHOD_CALL` / Map-typed bindings / loader-call sites through Phase E1-specific rendering paths per spec §"LSP support for records", §"LSP support for maps", `meta_config_loading.md` §"LSP support".
  - Goto-def handler: record-decl names resolve to declarations; loader path arguments resolve to file row 1; loader-resolved record-field projections resolve to YAML/JSON rows when statically traceable.
  - Completion handler: record-literal field-key positions (offer unfilled target fields); record field-projection sites (offer closed declared set); `m.<cursor>` (offer closed Map API); `m.get(<cursor>)` / `m.has(<cursor>)` (offer statically-known keys when resolvable); loader path arguments (filesystem entries by extension); loader schema arguments (in-scope `smelt.record` names + inline stub).
- `examples/meta_config/` (clean fixture):
  - `smelt.yml` — minimal workspace config, declares a `target:` field for overlay demonstration.
  - `cohorts.yaml` — list-of-records: `[{name: 'us_west', region: 'us-west-2', threshold: 100}, {name: 'us_east', region: 'us-east-1', threshold: 100}, {name: 'eu', region: 'eu-west-1', threshold: 50}]`.
  - `cohorts.prod.yaml` — overlay raising thresholds (demonstrates list-replace; the overlay re-lists every cohort with the prod thresholds).
  - `tenants.yaml` — map-of-records: `{tenant_a: {plan: 'pro', threshold: 100}, tenant_b: {plan: 'free', threshold: 10}}`.
  - `models/cohorts.sql` — declared record + YAML loader + `entries`-projection HOF chain producing a SELECT (e.g. a small aggregation per cohort name).
  - `models/tenants.sql` — declares `smelt.record Tenant = {plan: Text, threshold: Integer}` and consumes `tenants.yaml` as `Map<Text, Tenant>` via `m.entries() |> map(fn e => …)`.
- `examples/meta_config_broken_*/` — one sub-fixture per Phase E1 diagnostic (~18 codes), narrowly designed so each fixture reports exactly one diagnostic and no cascading errors. Codes covered: every record code, every map code, every loader code.
- `docs-site/docs/meta-language/records.md` — `smelt.record` declarations, inline record types, record literals, field projection, width subtyping, diagnostics. Worked examples drawn from `examples/meta_config/`.
- `docs-site/docs/meta-language/maps.md` — `Map<K, V>` type, the closed API, missing-key behaviour, iteration order, invariance, diagnostics. Worked examples.
- `docs-site/docs/meta-language/config-loaders.md` — `smelt.config.load_yaml`, `smelt.config.load_json`, reserved `load_toml`; schema authoring (inline and named, list/map roots); path resolution and workspace-relative discipline; per-target overlay; validation diagnostics with frame stacks; LSP support.
- `docs-site/docs/meta-language/reference.md` — extend with `smelt.record`, record-literal `{…}`, inline record type `{…}`, field projection `.field`, `Map<K, V>`, `m.entries`, `m.keys`, `m.values`, `m.get`, `m.has`, `smelt.config.load_yaml`, `smelt.config.load_json`, `smelt.config.load_toml`. Maintain alphabetical order across the page.
- `.claude/skills/smelt-app-builder/references/20260513-meta-records-maps-loaders.md` — workflow gotchas: loader paths must be string literals; schemas are bidirectionally inferred only at loader call sites; named schemas live workspace-globally and have goto-def, inline schemas are anonymous; `Map<K,V>` iteration is byte-lex-sorted by key, not insertion order; `m.get(k)` on a statically-missing key is a diagnostic, not silent `Unknown`; per-target overlay files use the `<basename>.<target>.<ext>` sibling convention with deep-merge for records / replace for lists / per-key replace for maps; record field types may not be `ColumnRef`/`ModelRef`/`SourceRef`/`Lambda<…>`.
- `.claude/commands/smelt-loop.md` (or its tier-fixture file) — extend the `medium` tier with a Phase E1-specific ask, e.g. "Author a per-tenant threshold YAML and consume it via `smelt.config.load_yaml(…, Map<Text, Tenant>) |> entries() |> map(fn e => …)` to produce a SELECT joining each tenant's settings against an orders table."
- `docs/specs/meta_language.md` — update the Known Divergences entry that previously read "records / maps / loaders not yet implemented" to "Records, `Map<K, V>`, and schema-typed loaders are implemented in v1; recursive schemas, per-key deep-merge for `Map<Text, S>` overlays, and `Optional<V>` schema fields remain deferred per `meta_config_loading.md`." (Single-line edit; keep the prose timeless-oracle compliant.)

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/loader.rs` — per-target overlay resolution and merge.
- `crates/smelt-db/src/lib.rs` — Salsa input registration for overlay files; extend `loader_resolved_value` to consult the target.
- `crates/smelt-lsp/src/lib.rs` — every LSP path enumerated above.
- `examples/meta_config/` and `examples/meta_config_broken_*/` (new directories).
- `crates/smelt-cli/tests/example_diagnostics.rs` — extend to gate the new fixtures.
- `docs-site/docs/meta-language/{records,maps,config-loaders,reference,index}.md` — user docs; touch `index.md` only if adding a contents-block entry.
- `.claude/skills/smelt-app-builder/references/20260513-meta-records-maps-loaders.md` (new file).
- `.claude/commands/smelt-loop.md` and any associated tier-fixture file — Phase E1 ask addition.
- `docs/specs/meta_language.md` — Known Divergences single-line update only; no surface or semantics changes.

**Docs touched.**

- `docs-site/docs/meta-language/records.md` (new).
- `docs-site/docs/meta-language/maps.md` (new).
- `docs-site/docs/meta-language/config-loaders.md` (new).
- `docs-site/docs/meta-language/reference.md` (extend).
- `docs-site/docs/meta-language/index.md` (optional minor extend).
- `docs/specs/meta_language.md` (Known Divergences single-line update).

**Review checklist** (material findings only):

- [ ] Per-target overlay merge rule matches spec exactly (record deep-merge, list replace, map per-key replace); verified by the dedicated tests.
- [ ] Overlay file is Salsa-tracked; edits invalidate downstream type-checks.
- [ ] LSP hover/goto-def/completion paths exist for every Phase E1 surface element enumerated in the spec.
- [ ] Loader path completion offers filesystem entries by extension; loader schema completion offers in-scope `smelt.record` names.
- [ ] Goto-def from a loaded record's field projection to the YAML/JSON row works for statically traceable values.
- [ ] Clean fixture reports zero diagnostics; every broken sub-fixture reports exactly one Phase E1 diagnostic with the correct code.
- [ ] User docs match the spec's Surface section exactly; no syntax appears in docs that is not speced.
- [ ] Reference page remains alphabetical and complete (Phases A+B+C+D+E1 entries).
- [ ] Worked examples are runnable — they correspond to `examples/meta_config/`.
- [ ] Skill ref doc captures workflow gotchas not derivable from user docs.
- [ ] `/smelt-loop` medium tier ask is solvable with Phases A–E1 constructs only (no multi-model production).
- [ ] Timeless-oracle compliance in docs and the spec Known Divergences edit (no phase vocabulary in spec body).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-E1): per-target overlay + LSP + examples/meta_config + user docs + skill + loop`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase E1 applicable expert reviewer from meta-plan §5 over the Phase E1 diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below. This phase is the realisation of the user's original ask: "Use expert reviews by subagents with specific context to help guide the implementation."

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase E1 subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **parser-expert** | sonnet | `crates/smelt-parser/src/{lexer,parser,ast,syntax_kind}.rs` | New productions (`SMELT_RECORD_DECL`, `RECORD_LITERAL`, `RECORD_TYPE_INLINE`, `MAP_METHOD_CALL`) do not conflict with existing tokens; `smelt.record` keyword precedence does not regress `smelt.define` / `smelt.config.*` / `smelt.models.*` / `smelt.sources.*`; record-literal vs inline-record-type disambiguation is production-context-driven and reproducible; `MAP_METHOD_CALL` disambiguates correctly from `ColumnRef`/`ModelRef`/`SourceRef` field projections (which have no argument list); parser-level error recovery for malformed record bodies / inline types / literals does not avalanche; no analysis logic or Salsa imports leaked into `smelt-parser`; recursive-descent depth/recovery invariants intact. |
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs` | `SmeltType::Record` and `SmeltType::Map` additions are non-breaking (no missed exhaustive matches across crates); structural equality on `Record` ignores `name` while preserving the named-declaration hover label; `MAP_API_METHODS` is the single source of truth for the closed Map API; bidirectional record-literal checking terminates and respects drop-on-error per field; width subtyping is one-directional, composes through nested record fields, and does not weaken field-projection diagnostics; `Map<K, V>` invariance correctly registered; static-key resolution at `m.get(k)` / `m.has(k)` is gated correctly; `MapKeyTypeNotText` fires at the offending type expression with avalanche protection; `type_inference.rs` and `signatures.rs` purity preserved (no `db.` Salsa calls in analysis logic); LUB / widening rules consistent with existing rules; record-declaration cycle detection is deterministic and emits exactly one diagnostic per cycle. |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/lib.rs` | Hover for records, maps, loaders matches spec content (closed field lists, signatures, resolution summaries); goto-def from record-decl names, record-literal field names, loader path arguments, and loaded-record field projections resolves to the spec-listed targets or no-ops gracefully on unsupported clients; completion at record-literal field-key positions offers unfilled fields; completion at record / map / loader sites offers exactly the closed sets; loader path completion is filesystem-aware by extension; spans line up with CST; no panics on partial parses (mid-edit record bodies, malformed YAML); no regressions in Phase A/B/C/D LSP paths. |
| **examples-curator** | haiku | `examples/meta_config/` and `examples/meta_config_broken_*/` | Clean fixture is minimal-but-realistic; the declared-record + YAML-loader + map-via-entries-HOF-chain composition motivates the Phase E1 design without contrivance; per-target overlay is demonstrated; every Phase E1 diagnostic code has a corresponding broken sub-fixture; broken sub-fixtures report exactly one Phase E1 diagnostic with no cascading errors; passes `cargo test -p smelt-cli --test example_diagnostics`. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{records,maps,config-loaders,reference,index}.md` | User docs match the Surface section of the two specs (`meta_language.md` records/maps; `meta_config_loading.md`) exactly; no syntax appears in docs that is not speced; every Phase E1 diagnostic code has a "what to fix" hint; reference page remains alphabetical and complete (Phases A+B+C+D+E1); timeless-oracle compliance (no `### Phase E1` headings, no `(Phase E1)` labels, no `[deferred to Phase E2]` callouts in body — open questions / known gaps belong in the spec's Known Divergences only). |

**Loop discipline.**

1. **Round 1.** Dispatch all five experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-E1.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md` §"Records", §"Maps", `docs/specs/meta_config_loading.md` end-to-end).
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase E1 — typically `git log --oneline cc27e5d..HEAD`, where `cc27e5d` is the spec-increment commit; the implementation commits land after).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-E1): address {expert-name} feedback` (e.g. `review(meta-language-E1): address type-expert feedback`).
   - Push after each commit (so the user sees progress on PR #117).

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed, not the whole panel. Provide the same prompt as round 1 plus a diff of what changed since round N−1. If the expert returns "no material findings", that expert is **clean** and exits the loop.

4. **Repeat** step 2 → step 3 until **every** expert is clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound). The third repeat means the fix is wrong or the spec is wrong; the user must arbitrate.
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7). That's a design problem, not an implementation problem.
   - An expert's findings would force a spec change. Run `/smelt:spec meta_language` (or `meta_config_loading`) first; if the spec edit is non-trivial or contentious, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase E1. Pause; the autonomy loop should not silently absorb pre-existing breakage.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260509-meta-language-E1.md` (to record round counts and final clean status).

**Docs touched.** None new — fixes may amend `docs-site/docs/meta-language/*` if the docs-reviewer flags a surface drift; the cross-feature implications for Phase E1 are limited to the records/maps/loaders surface and the spec touches landed in Phase 6 (the Known Divergences single-line update).

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] All five experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below (see acceptance gate).
- [ ] No fix touched files outside the dispatching expert's scope (no scope creep).
- [ ] No expert ran more than 3 rounds; if any did, the autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 7 expert review: parser-expert clean (R{n}), type-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-E1): address {expert-name} feedback`. If round 1 came back clean for an expert, no commit for that expert. The acceptance-gate summary line lands in the next commit naturally (or in a tiny `chore(meta-language-E1): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes.
- `cargo test -p smelt-cli --test example_diagnostics` passes — `examples/meta_config/` clean, broken sub-fixtures report the exact Phase E1 diagnostic codes (record codes, map codes, loader codes).
- `/smelt:validate meta_language` reports zero drift.
- `/smelt:validate meta_config_loading` reports zero drift.
- LSP smoke test in `examples/meta_config/`: hover, goto-def, completion all work for `smelt.record` declarations, record literals, record field projections, `Map<K, V>` bindings and method calls, loader call sites, loader path arguments, and loader schema arguments per spec.
- Phase 7 acceptance gate met: every applicable expert reviewer (parser-expert, type-expert, lsp-expert, examples-curator, docs-reviewer) reported "no material findings" on its final dispatch, recorded in "Deferred during implementation" with round counts per expert. No stop-the-line condition fired.
