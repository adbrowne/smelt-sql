# Plan: Meta-Language Phase D — Wide reflection: `smelt.models.*`, `smelt.sources.*`, `ModelRef`, `SourceRef`

**Date**: 2026-05-12
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Reflection: `smelt.models`, `smelt.sources`, `ModelRef`, `SourceRef`" (Surface, Per-phase semantic rules, Wide reflection — design rationale); cross-touched in [`docs/specs/expansion.md`](../specs/expansion.md) §"Frame-stack invariants" (`model_origin` / `source_origin` extensions to the anonymous frame), [`docs/specs/lsp.md`](../specs/lsp.md) §"Per-feature LSP obligations" (Phase D constructs), [`docs/specs/types.md`](../specs/types.md) §"Fragment sort subtyping" (`ModelRef <: TableExpr`, `SourceRef <: TableExpr`), [`docs/specs/data_catalog.md`](../specs/data_catalog.md) (workspace-reflection visibility note)
**Spec diff**: commits `2c16b12` (`spec(meta-language-D): author wide reflection surface, semantics, design`) and `3849446` (`spec(meta-language-D): correct ModelRef.tags merge order`) on branch `research/typed-meta-programming`
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` (Surface, Semantics, Design, Invariants, Known Divergences for wide reflection) plus the cross-spec touches in `expansion.md`, `lsp.md`, `types.md`, `data_catalog.md` — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec meta_language` to revise the spec rather than encode the divergence in code.
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

- Real-fixture tests under `examples/meta_workspace/` — Phase 5 onward exercises wide reflection there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no records / `Map<K, V>`, no multi-model production, no parameterised reducers, no user-writable record surface — those are Phases E1, E2, F.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic). The Salsa queries for `with_tag` and `all` live in `crates/smelt-db/src/lib.rs` and are called only from the orchestration layer; inference rules receive resolved workspace state as parameters.
- Timeless-oracle rule: spec and user-doc edits read as if the feature has always existed. Phase vocabulary lives in this plan only.

---

## Context

The meta-language Phase D spec increment landed in commits `2c16b12` and `3849446`. The spec authors two accessor namespaces (`smelt.models.{with_tag, all}` and `smelt.sources.{with_tag, all}`), two closed meta-record types (`ModelRef`, `SourceRef`) each with four fields (`path: Text`, `name: Text`, `tags: List<Text>`, `columns: List<ColumnRef>`), the `ModelRef <: TableExpr` and `SourceRef <: TableExpr` fragment-sort subtyping rules, six new diagnostic codes (`WithTagRequiresText`, `WithTagNamedArgument`, `WideReflectionUnknownAccessor`, `WideReflectionUnexpectedArgument`, `ModelRefFieldUnknown`, `SourceRefFieldUnknown`), and `model_origin` / `source_origin` extensions to the anonymous expansion frame from Phase B. This plan drives the implementation, examples, user docs, and skill update for that surface. It is the fourth of seven implementation phases (A–G); the wide-reflection accessors are load-bearing for Phase E2's per-cohort union killer demo and for any future workspace-introspection tooling.

## Scope

### In scope (spec coverage)

- `meta_language.md` Surface for wide reflection: `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, `smelt.sources.all`; the closed `ModelRef` and `SourceRef` record types; the six new diagnostic codes; LSP obligations for hover, goto-def, completion, and frame-stack diagnostics.
- `meta_language.md` Per-phase semantic rules for wide reflection — twelve normative rules covering Salsa-cached purity, path-sorted determinism, exact-string tag matching, two-tier body-check vs expansion-time evaluation, `ModelRef`/`SourceRef <: TableExpr` subtyping, `m.columns` ≡ `smelt.columns_of(m)`, identifier-lift narrowness (no new lift positions), `model_origin`/`source_origin` frame extension, determinism, closed accessor set, bounded termination.
- `meta_language.md` Wide reflection — design rationale — preserved as architectural invariants policed by the implementation.
- Six new Phase D diagnostic codes (`WithTagRequiresText`, `WithTagNamedArgument`, `WideReflectionUnknownAccessor`, `WideReflectionUnexpectedArgument`, `ModelRefFieldUnknown`, `SourceRefFieldUnknown`).
- `expansion.md` cross-spec touch — `model_origin` and `source_origin` extensions to the anonymous-frame form (per-element optional source-model/source-yaml provenance on the HOF frame, populated when the source list comes from `smelt.models.*` / `smelt.sources.*`).
- `lsp.md` cross-spec touch — Phase D's per-construct LSP obligations (hover, goto-def, completion, diagnostics-with-frame-stacks for `smelt.models.*`, `smelt.sources.*`, `ModelRef`, `SourceRef`).
- `types.md` cross-spec touch — `ModelRef <: TableExpr` and `SourceRef <: TableExpr` entries under "Fragment sort subtyping".
- `data_catalog.md` cross-spec touch — informational note that wide-reflection accessors observe the same model / source identities the catalog renders (path, name, tags). Catalog rendering is not normatively extended by Phase D; the deeper catalog change (multi-source-of-origin with cohort id) lands in Phase E2.
- LSP support for every wide-reflection surface element: hover on `smelt.models.<accessor>` / `smelt.sources.<accessor>` / `ModelRef`-typed bindings / field projections; goto-def from a `ModelRef` value at a `TableExpr` splice site to the model's source file (and the same for `SourceRef` to the source YAML); completion at `smelt.models.<cursor>` / `smelt.sources.<cursor>` and at field projection sites.
- Examples fixture `examples/meta_workspace/` covering happy paths (union-models-by-tag and audit-log completeness check) plus broken sub-fixtures for each new Phase D diagnostic, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/{reflection,reference}.md` (extend the narrow-reflection page from Phase C with the wide-reflection accessors and record types).
- `smelt-app-builder` skill: per-phase reference doc at `.claude/skills/smelt-app-builder/references/20260512-meta-workspace.md`.
- `/smelt-loop` `medium` tier: at least one Phase D-specific ask (e.g. "union every model tagged `cohort` into a single SELECT using `smelt.models.with_tag |> reduce(union_all)`").

### Explicitly deferred

- Records, `Map<K,V>`, schema-typed config loaders (`smelt.config.load_yaml`, `smelt.record Name = { … }`) — Phase E1. `ModelRef` and `SourceRef` are *internal* closed records in v1; the user-writable record surface lands later.
- Multi-model production (`generates: models`, `ModelDef`, generator file body shape, the per-cohort union killer demo) — Phase E2. Phase D ships the *reflection* over hand-authored models; Phase E2 closes the loop by letting generators emit them.
- Wider meta-`Text`-as-identifier lift positions (CTE names, table aliases, function names, model paths) — Phase E2 may add model-path component lift; Phase D does **not** extend the four-position lift table.
- Catalog rendering changes (per-model cohort id, multi-source-of-origin column) — Phase E2.
- Parameterised reducers, multi-arg lambdas, ternary — Phase F.
- LSP rename support for `ModelRef`/`SourceRef` field projections — Phase G.
- Additional `ModelRef` / `SourceRef` fields (`materialization`, `backends`, `description`, …) — speced as future extensions, shipped only if examples force them. The Phase D field set is exactly `{path, name, tags, columns}`.
- `with_tag` accepting glob patterns or case-folded matching — spec rejects; exact-string equality is the entire surface.
- Heterogeneous workspace-entity lists (`smelt.workspace.entities` returning a sum over `ModelRef`/`SourceRef`) — out of scope per "Out-of-scope by deliberate choice".

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | d75bc07 | 2026-05-12 |
| 2     | done     | f23615c | 2026-05-12 |
| 3     | done     | 7e7b90a | 2026-05-13 |
| 4     | done     | 95e430b | 2026-05-13 |
| 5     | done     | 47a0013 | 2026-05-13 |
| 6     | done     | f274453 | 2026-05-13 |
| 7     | done     | 8eea6f0 | 2026-05-13 |

---

### Phase 1: Type system — `ModelRef` / `SourceRef` witnesses + wide accessor entries + field projection (pure)

**Goal.** Add the `ModelRef` and `SourceRef` `SmeltType` witnesses with their closed four-field sets (`path: Text`, `name: Text`, `tags: List<Text>`, `columns: List<ColumnRef>`); the four wide-reflection accessor entries (`smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, `smelt.sources.all`) in the built-in registry / resolver; and pure type-inference for `ModelRef` / `SourceRef` field projection (closed-set lookup). Four Phase D diagnostics fire from this phase: `WithTagRequiresText`, `WithTagNamedArgument`, `WideReflectionUnknownAccessor`, `WideReflectionUnexpectedArgument`, `ModelRefFieldUnknown`, `SourceRefFieldUnknown` (`WithTagRequiresText` requires the compile-time-resolvable check to land in this phase; the remaining wiring into the orchestration layer lands here too — no Salsa calls inside `type_inference.rs` or `signatures.rs`).

**Pre-conditions.** Phase C done at commit `4a4c3e2`. Working tree clean. `cargo test`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**TDD tests to write first.**

- `crates/smelt-types/src/signatures.rs::tests::wide_reflection_accessor_signatures` — `smelt.models.with_tag` resolves to a signature `(Text) -> List<ModelRef>` with one positional parameter; `smelt.models.all` resolves to `() -> List<ModelRef>` with zero parameters; the analogous holds for `smelt.sources.with_tag` and `smelt.sources.all` returning `List<SourceRef>`.
- `crates/smelt-types/src/signatures.rs::tests::model_ref_field_set_is_closed` — the constant `MODEL_REF_FIELDS` (or equivalent registry) exposes exactly `{path: Text, name: Text, tags: List<Text>, columns: List<ColumnRef>}` and no other field; lookup of any other identifier returns `None`. Same for `SOURCE_REF_FIELDS`.
- `crates/smelt-types/src/signatures.rs::tests::model_ref_and_source_ref_field_sets_are_identical_shape` — `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` have the same field names and types in the same order (uniformity invariant from the design rationale).
- `crates/smelt-db/src/type_inference.rs::tests::with_tag_arg_must_be_compile_time_text` — `smelt.models.with_tag(42)` synthesises `List<ModelRef>` (recoverable) and emits exactly one `WithTagRequiresText` at the `42` argument span; `smelt.sources.with_tag(UPPER('x'))` (a runtime `Expr<Text>`) emits `WithTagRequiresText` at the call-arg span; `smelt.models.with_tag('core')` (a string literal) emits no Phase D diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::with_tag_rejects_named_argument` — `smelt.models.with_tag(tag => 'core')` emits exactly one `WithTagNamedArgument` at the named-argument span; `smelt.models.with_tag('core')` does not.
- `crates/smelt-db/src/type_inference.rs::tests::wide_reflection_unknown_accessor` — `smelt.models.bogus()` emits exactly one `WideReflectionUnknownAccessor` at the `bogus` token span; the same holds for `smelt.sources.bogus()` with the "sources" message substitution.
- `crates/smelt-db/src/type_inference.rs::tests::wide_reflection_all_takes_no_arguments` — `smelt.models.all(42)` emits exactly one `WideReflectionUnexpectedArgument` at the `42` argument span; `smelt.models.all()` does not. `smelt.sources.all(named => 'x')` emits `WideReflectionUnexpectedArgument` at the named-argument span.
- `crates/smelt-db/src/type_inference.rs::tests::model_ref_field_projection_synthesises_field_type` — given a binding `m: ModelRef`, `m.path` synthesises `Text`, `m.name` synthesises `Text`, `m.tags` synthesises `List<Text>`, `m.columns` synthesises `List<ColumnRef>`. The analogous test holds for `s: SourceRef`.
- `crates/smelt-db/src/type_inference.rs::tests::model_ref_field_projection_rejects_unknown_field` — given `m: ModelRef`, `m.foo` emits exactly one `ModelRefFieldUnknown` at the `foo` field span and synthesises `Unknown` (drop-on-error). The analogous holds for `s: SourceRef` emitting `SourceRefFieldUnknown`.

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - `SmeltType::ModelRef` and `SmeltType::SourceRef` variants (or alternative internal witnesses sharing infrastructure with `ColumnRef`'s closed-record pattern). Display impls show `ModelRef` / `SourceRef`.
  - `pub const MODEL_REF_FIELDS: &[(&str, FieldType)]` and `pub const SOURCE_REF_FIELDS: &[(&str, FieldType)]` (or equivalent struct array) with the four fields each, in the canonical order `{path, name, tags, columns}`. Lookup helpers `model_ref_field(name) -> Option<FieldType>` and `source_ref_field(name) -> Option<FieldType>`.
  - Built-in registry entries (or accessor-namespace resolver entries) for `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, `smelt.sources.all`. Each entry encodes the positional-only arity (`with_tag`: 1 positional, no named, no variadic; `all`: 0 positional, no named, no variadic) and the return type.
  - A closed-accessor-set check for `smelt.models.<name>` and `smelt.sources.<name>` — any name outside `{with_tag, all}` is the trigger for `WideReflectionUnknownAccessor` (the diagnostic is *emitted* by the inference layer, but the closed-set lookup lives here so the witness is the single source of truth).
- `crates/smelt-db/src/type_inference.rs`:
  - Pure inference for `smelt.models.<accessor>(...)` / `smelt.sources.<accessor>(...)`: dispatch through the existing `smelt.<path>` resolution recognising `smelt.models` / `smelt.sources` as accessor namespaces; check the accessor name against the closed set (emit `WideReflectionUnknownAccessor` on miss); check positional-only and the arity (emit `WithTagNamedArgument` on a named arg, `WideReflectionUnexpectedArgument` on any arg to `all`); check `with_tag`'s argument is compile-time-resolvable meta-`Text` (emit `WithTagRequiresText` if not — re-use Phase B's "compile-time-Text" predicate, the same one that gates `smelt.config.var`'s argument). Always synthesise the spec'd return type (recoverable) so downstream HOF type-check still works.
  - Pure inference for `ModelRef` / `SourceRef` field projection: when synthesising a field-access expression `e.f` where `e` synthesises to `ModelRef` (resp. `SourceRef`), look up `f` in `MODEL_REF_FIELDS` (resp. `SOURCE_REF_FIELDS`); on hit, synthesise the field's type; on miss, emit `ModelRefFieldUnknown` / `SourceRefFieldUnknown` at the field span and synthesise `Unknown`.
- `crates/smelt-db/src/lib.rs::DiagnosticCode`:
  - Six new variants: `WithTagRequiresText`, `WithTagNamedArgument`, `WideReflectionUnknownAccessor`, `WideReflectionUnexpectedArgument`, `ModelRefFieldUnknown`, `SourceRefFieldUnknown`. Message templates per spec §"Wide-reflection diagnostic codes" with the "models"/"sources" substitution baked into the `WithTagRequiresText` / `WithTagNamedArgument` / `WideReflectionUnknownAccessor` / `WideReflectionUnexpectedArgument` message templates.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs` — `SmeltType::ModelRef`/`SourceRef` (or equivalent), `MODEL_REF_FIELDS` / `SOURCE_REF_FIELDS`, wide-reflection accessor entries.
- `crates/smelt-types/src/lib.rs` — only if a re-export is needed.
- `crates/smelt-db/src/type_inference.rs` — pure inference for wide-reflection arg-checking and `ModelRef` / `SourceRef` field projection.
- `crates/smelt-db/src/lib.rs` — `DiagnosticCode` variants and `Display` impl entries; NOT the Salsa queries (Phase 3).
- `crates/smelt-types/src/signatures.rs::tests` and `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.

**Docs touched.**

- None new in this phase. The spec rules cited above are normative; this phase makes them executable.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `SmeltType` additions are non-breaking (no missed exhaustive matches across crates; check with `cargo clippy --all-targets`).
- [ ] `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` are the single source of truth for the v1 field sets; no field name appears as a string literal outside the registry definitions.
- [ ] `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` are structurally identical (same names, same types, same order) — uniformity invariant.
- [ ] Wide-reflection accessor entries have signatures exactly `(Text) -> List<ModelRef|SourceRef>` for `with_tag` and `() -> List<ModelRef|SourceRef>` for `all` — no variadic, no named-arg surface.
- [ ] `type_inference.rs` and `signatures.rs` remain pure (no `db.` Salsa calls).
- [ ] Six diagnostics anchor at the correct CST span (accessor name token, named-argument span, argument expression span, field token span as the spec dictates per code).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-D): ModelRef/SourceRef witnesses + wide reflection accessors + field projection (pure)`

---

### Phase 2: Subtyping — `ModelRef <: TableExpr`, `SourceRef <: TableExpr`

**Goal.** Implement the fragment-sort subtyping rule `ModelRef <: TableExpr` and `SourceRef <: TableExpr` so that a `ModelRef` (resp. `SourceRef`) value lifts to a `TableExpr` wherever a `TableExpr` is required (reducer-`union_all` arguments, `smelt.columns_of` arguments, `FROM`-clause splice positions). The List covariance rule from Phase A already lifts `List<ModelRef>` to `List<TableExpr>` via this one-way subtyping. No new diagnostic codes — type errors at splice points remain whatever the existing splice context produces (e.g. an `Unknown` schema produces `ColumnsOfUnresolvableSchema` if consumed by `smelt.columns_of`, but the subtyping rule itself does not invent diagnostics). The reverse direction (`TableExpr → ModelRef`) does not exist; the type system rejects an attempt to assign a non-`ModelRef` `TableExpr` value to a `ModelRef`-typed binding.

**Pre-conditions.** Phase 1 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/type_inference.rs::tests::model_ref_assignable_to_table_expr_in_columns_of_arg` — given a binding `m: ModelRef`, `smelt.columns_of(m)` synthesises `List<ColumnRef>` with no diagnostic (the subtyping lift fires). Verifies the `ModelRef -> TableExpr` direction at the `smelt.columns_of` call site.
- `crates/smelt-db/src/type_inference.rs::tests::source_ref_assignable_to_table_expr_in_columns_of_arg` — given a binding `s: SourceRef`, `smelt.columns_of(s)` synthesises `List<ColumnRef>` with no diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::list_of_model_ref_lifts_to_list_of_table_expr_in_reducer_arg` — given `xs: List<ModelRef>`, `reduce(xs, union_all)` synthesises the row-tail UNION result (per Phase B's reducer rules) with no diagnostic. The List covariance lifts the list element type.
- `crates/smelt-db/src/type_inference.rs::tests::model_ref_assignable_in_from_clause_splice` — given `m: ModelRef`, `SELECT * FROM m` (as a meta splice into the FROM clause) typechecks against the `m`'s underlying `TableExpr`; the existing column-resolution scope sees the model's columns.
- `crates/smelt-db/src/type_inference.rs::tests::table_expr_not_assignable_to_model_ref` — given a `TableExpr`-valued binding `t` (not a `ModelRef`), attempting to project `t.path` emits `ModelRefFieldUnknown`? No — the surface is type-system level: `t.path` synthesises against `TableExpr`'s field set (empty for the current spec), so the existing diagnostic for unknown-method-on-TableExpr fires. **Concretely**: this test asserts that the inverse direction does not silently succeed — assigning a `TableExpr` to a `ModelRef`-typed binding emits the existing type-mismatch diagnostic; the subtyping rule is one-way.
- `crates/smelt-db/src/type_inference.rs::tests::m_columns_equivalent_to_smelt_columns_of_m` — given `m: ModelRef`, the synthesised type and value of `m.columns` is identical to `smelt.columns_of(m)`. Both produce `List<ColumnRef>` in the source-column declaration order at expansion time. (Body-check time produces `List<ColumnRef>` parametrically.)

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs` (or wherever the fragment-sort assignability rules live):
  - Add `ModelRef <: TableExpr` and `SourceRef <: TableExpr` to the existing assignability rule set (probably in the function that decides whether a value's synthesised type is assignable to an expected parameter type). The rule is one-way; the reverse remains a type error.
  - The covariant `List` rule already in place from Phase A lifts `List<ModelRef>` to `List<TableExpr>` automatically once the element rule is registered.
- `crates/smelt-db/src/type_inference.rs`:
  - The check_against / synthesises-against path that decides whether a value can flow into a `TableExpr`-typed position consults the assignability rules; no per-call-site rewrite is needed in the inference rule body — the subtyping rule does the work.
  - `m.columns` field-projection inference (added in Phase 1) returns `List<ColumnRef>` parametrically at body-check time. The runtime equivalence with `smelt.columns_of(m)` is enforced at expansion time (Phase 3) by routing both through the same Salsa-cached schema-resolution path.
- `docs/specs/types.md` cross-spec touch — register the two new entries under "Fragment sort subtyping" (one paragraph each: rule, one-way direction, expected use sites).

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs` — assignability rule additions.
- `crates/smelt-db/src/type_inference.rs` — only if the assignability rule needs a hook on the call site (likely zero changes if signatures.rs is the right place).
- `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.
- `docs/specs/types.md` — Phase D cross-spec touch as described above.

**Docs touched.**

- `docs/specs/types.md` — Phase D cross-spec touch.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] The subtyping rule is one-way (verified by the inverse-direction test).
- [ ] List covariance from Phase A lifts `List<ModelRef>` and `List<SourceRef>` to `List<TableExpr>` automatically — no Phase D-specific list rule needed.
- [ ] `signatures.rs` and `type_inference.rs` remain pure (no `db.` Salsa calls in the subtyping rule).
- [ ] No regression in Phase A / B / C assignability rules.
- [ ] `types.md` Phase D touch describes the rule as it has always existed — no phase vocabulary in spec body (timeless-oracle rule).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-D): ModelRef and SourceRef subtype TableExpr`

---

### Phase 3: Expansion-time materialisation + Salsa queries + `model_origin` / `source_origin` frame

**Goal.** Wire each wide-reflection accessor to materialise the concrete list at expansion time via a Salsa query: `models_with_tag(workspace, tag)`, `models_all(workspace)`, `sources_with_tag(workspace, tag)`, `sources_all(workspace)` (or an equivalent shape that re-uses `all_models` / `project_sources` as inputs). Extend the anonymous expansion frame from Phase B with optional `model_origin` and `source_origin` fields (the wide-reflection siblings of Phase C's `column_origin`); the frame producer stamps the origin when the source list comes from `smelt.models.*` / `smelt.sources.*`. The `ModelRef.columns` / `SourceRef.columns` projections route through the existing `columns_of_for_table_expr` Salsa query from Phase C (re-use; no new query). Land the `expansion.md` cross-spec touch normatively.

**Pre-conditions.** Phases 1–2 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/lib.rs::tests::models_with_tag_returns_path_sorted_matches` — given a workspace with three models tagged `cohort` (paths `models/a.sql`, `models/b.sql`, `models/c.sql`) and one model not tagged `cohort` (`models/d.sql`), `models_with_tag(workspace, "cohort")` returns a `Vec<ModelRefValue>` of length 3 in path-sorted order `[a, b, c]` (byte-lexicographic on the workspace-relative path with `/` separators). Per-element fields are correctly populated (`path`, `name`, `tags`, `columns` projection deferred to the next test).
- `crates/smelt-db/src/lib.rs::tests::models_with_tag_uses_merged_tag_set` — given a model with `tags: [cohort]` in `smelt.yml` and `tags: [audit]` in SQL frontmatter, `models_with_tag(workspace, "cohort")` matches it AND `models_with_tag(workspace, "audit")` matches it. The merged tag set is the union per `Config::get_tags`.
- `crates/smelt-db/src/lib.rs::tests::models_with_tag_invalidates_on_tag_change` — modifying a model's frontmatter `tags:` invalidates the `models_with_tag` query result for the affected tag (Salsa-cache invariant); re-evaluation produces the updated set.
- `crates/smelt-db/src/lib.rs::tests::models_all_returns_all_models_path_sorted` — `models_all(workspace)` returns every model in the workspace in path-sorted order, byte-equal across two runs over the same workspace input (determinism).
- `crates/smelt-db/src/lib.rs::tests::sources_with_tag_and_sources_all_mirror_models_behaviour` — analogous to the above but over the project's sources (`project_sources` is the Salsa input). Tag merge is from the source YAML's declared `tags:` only (no two-source merge in this case — there is no per-source frontmatter equivalent).
- `crates/smelt-db/src/lib.rs::tests::model_ref_columns_routes_through_columns_of_query` — `ModelRefValue::columns_of(m, db)` returns the same `Vec<ColumnRefValue>` as `columns_of_for_table_expr(db, m's underlying TableExpr)`, byte-equal. (Re-use of Phase C's Salsa query; the field projection is just a deterministic re-dispatch.)
- `crates/smelt-db/src/function_body_check.rs::tests::wide_reflection_hof_lambda_carries_model_origin_frame` — given a HOF body `map(smelt.models.with_tag('cohort'), fn m => m.path)` that surfaces a diagnostic from inside the lambda (e.g. by stuffing the lambda with a deliberately ill-typed expression in a controlled test fixture), the resulting frame stack includes a Phase B anonymous frame with `function = "map"` and a Phase D extension `model_origin = Some(ModelOrigin { path: <source-path>, frontmatter_span: <span> })` — the optional source-model provenance on the per-element entry.
- `crates/smelt-db/src/function_body_check.rs::tests::wide_reflection_hof_lambda_carries_source_origin_frame` — analogous to the above for `smelt.sources.*`-sourced lists with `source_origin`.
- `crates/smelt-db/src/function_body_check.rs::tests::wide_reflection_expansion_preserves_path_order` — `smelt.models.with_tag(t)` materialises ModelRef values in workspace-relative path-sorted order; reordering is forbidden. A `reduce(..., union_all)` over the result preserves the order in the rendered SQL (mirroring the spec rule "row order in a `reduce(union_all)` over a wide-reflection result follows this order").

**Implementation shape.**

- `crates/smelt-db/src/lib.rs`:
  - New Salsa queries: `models_with_tag(workspace, tag) -> Arc<Vec<ModelRefValue>>`, `models_all(workspace) -> Arc<Vec<ModelRefValue>>`, `sources_with_tag(project, tag) -> Arc<Vec<SourceRefValue>>`, `sources_all(project) -> Arc<Vec<SourceRefValue>>`. Each reads the existing `all_models` / `project_sources` query, filters by tag (when applicable), projects to `ModelRefValue` / `SourceRefValue`, sorts ascending by `path` (byte-lexicographic on the workspace-relative path string with `/` separators), and returns. The query is keyed on the workspace / project input so Salsa invalidates only on workspace-state changes that affect the tag set or the file list.
  - `ModelRefValue` and `SourceRefValue` runtime types (or equivalent — the value-level shape distinct from the type-level `ModelRef` / `SourceRef` witnesses). Each carries `path: String`, `name: String`, `tags: Vec<String>`, plus the underlying `TableExpr`-id to route `m.columns` through `columns_of_for_table_expr`.
  - `ModelOrigin` and `SourceOrigin` structs (or one shared `ReflectionOrigin` enum) capturing the source-model / source-yaml path plus the frontmatter / YAML declaration span. The producer stamps this when materialising HOF frames over a wide-reflection list.
- `crates/smelt-db/src/function_body_check.rs`:
  - Extend the anonymous expansion frame from Phase B with optional `model_origin: Option<ModelOrigin>` and `source_origin: Option<SourceOrigin>` fields (or a single optional `reflection_origin: Option<ReflectionOrigin>` if that shape is cleaner). At HOF call-site materialisation, when the source list is the result of `smelt.models.*` / `smelt.sources.*`, per element, look up the model's / source's source span in `all_models` / `project_sources` and stamp the resulting frame.
- `docs/specs/expansion.md` cross-spec touch — register the `model_origin` and `source_origin` extensions to the anonymous-frame form (per-element optional source-model / source-yaml provenance). The Phase C `column_origin` extension stays; the wide-reflection siblings are added as parallel rows in the anonymous-frame schema description.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/lib.rs` — four new Salsa queries, `ModelRefValue` / `SourceRefValue` / `ModelOrigin` / `SourceOrigin` types.
- `crates/smelt-db/src/function_body_check.rs` — frame extensions, wide-reflection-sourced frame stamping.
- `crates/smelt-db/src/lib.rs::tests`, `crates/smelt-db/src/function_body_check.rs::tests` — the unit tests above.
- `docs/specs/expansion.md` — Phase D cross-spec touch as described above.

**Docs touched.**

- `docs/specs/expansion.md` — Phase D cross-spec touch.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] All four Salsa queries are correctly memoised, correctly invalidated on workspace-state changes (verified by the invalidation test).
- [ ] Path-sorted determinism preserved (byte-lexicographic; no case-folding, no glob, no topological order).
- [ ] `model_origin` / `source_origin` populated for wide-reflection-sourced lists; absent (`None`) for other anonymous frames (no regression in Phase B / C frame producers).
- [ ] `m.columns` / `s.columns` route through `columns_of_for_table_expr` (no second Salsa query for column projection of `ModelRef`/`SourceRef`).
- [ ] Tag merge for `models_with_tag` uses `Config::get_tags` exactly (smelt.yml first, then SQL frontmatter not already present).
- [ ] `expansion.md` Phase D touch describes the extensions as if they have always existed (timeless-oracle rule).
- [ ] `type_inference.rs` and `signatures.rs` purity preserved.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-D): expansion-time materialisation + model/source_origin frame extensions`

---

### Phase 4: LSP — hover, goto-def, completion for Phase D constructs

**Goal.** Add LSP support for every Phase D surface element per spec §"LSP support for wide reflection": hover for `smelt.models.<accessor>` / `smelt.sources.<accessor>` / `ModelRef`-typed bindings / field projections; goto-def from a `ModelRef` value at a splice site to the model's source file (and `SourceRef` to the source YAML); completion at `smelt.models.<cursor>` / `smelt.sources.<cursor>` and at `ModelRef`/`SourceRef` field projection sites. Land the `lsp.md` cross-spec touch and the `data_catalog.md` informational touch.

**Pre-conditions.** Phases 1–3 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-lsp/src/lib.rs::tests::hover_on_smelt_models_with_tag_call_shows_list_model_ref` — hovering on `smelt.models.with_tag('cohort')` returns `List<ModelRef>` plus, when the tag resolves to a literal at the cursor, the resolved match count and the first five matching model names. Analogous for `smelt.sources.with_tag`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_smelt_models_all_shows_workspace_count` — hovering on `smelt.models.all` shows the signature plus the workspace's total model count. Analogous for `smelt.sources.all`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_model_ref_lambda_parameter_shows_field_set` — hovering on `m` inside `map(smelt.models.with_tag('cohort'), fn m => …)` shows `ModelRef` plus the closed four-field list with each field's type. Analogous for `SourceRef`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_model_ref_field_projection_shows_field_type` — hovering on the `path` token of `m.path` shows `path: Text`; on `name` shows `name: Text`; on `tags` shows `tags: List<Text>`; on `columns` shows `columns: List<ColumnRef>`. Analogous for `SourceRef`.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_from_model_ref_at_splice_site_resolves_to_source_file` — given `m: ModelRef` (a lambda parameter from a HOF over `List<ModelRef>`) consumed at a `FROM`-clause splice (`SELECT * FROM m`), go-to-definition on the splice resolves to the model's source `.sql` file (the same file that `smelt.<path>` resolves to for that model). Analogous for `SourceRef` resolving to the source YAML file.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_from_model_ref_path_or_name_resolves_to_source_file` — go-to-definition on `m.path` or `m.name` returns the same model file. Same for `s.path`/`s.name` and the source YAML.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_smelt_models_namespace_offers_closed_set` — completion at `smelt.models.<cursor>` offers exactly `{with_tag, all}` and no other identifier. Same for `smelt.sources.<cursor>`.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_model_ref_field_offers_closed_set` — completion at `m.<cursor>` where `m: ModelRef` offers exactly `{path, name, tags, columns}`. Analogous for `SourceRef`.

**Implementation shape.**

- `crates/smelt-lsp/src/lib.rs`:
  - Hover handler: on `smelt.models.<accessor>` / `smelt.sources.<accessor>` call site, return the Phase D hover body (signature + workspace-state summary when statically resolvable). On a `ModelRef`/`SourceRef`-typed binding, return the closed field list. On a field projection, return the field's declared type.
  - Goto-def handler: extend the existing path-resolution to recognise `ModelRef` / `SourceRef` values at splice sites and route to the model's source file / source YAML.
  - Completion handler: at `smelt.models.<cursor>` / `smelt.sources.<cursor>`, offer the closed accessor set (`with_tag`, `all`). At `m.<cursor>` / `s.<cursor>` where the binding type is `ModelRef` / `SourceRef`, offer the closed field set.
- `docs/specs/lsp.md` Phase D cross-spec touch — register Phase D's per-construct LSP obligations (one short paragraph per LSP feature, mirroring the spec §"LSP support for wide reflection" entries).
- `docs/specs/data_catalog.md` cross-spec touch — informational note that wide-reflection accessors observe the same model / source identities the catalog renders. No normative change to catalog rendering; the per-model cohort id deferred to Phase E2.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-lsp/src/lib.rs` — hover, goto-def, completion handlers for Phase D constructs.
- `crates/smelt-lsp/src/lib.rs::tests` — the unit tests above.
- `docs/specs/lsp.md` — Phase D cross-spec touch.
- `docs/specs/data_catalog.md` — Phase D informational touch.

**Docs touched.**

- `docs/specs/lsp.md` — Phase D cross-spec touch.
- `docs/specs/data_catalog.md` — Phase D informational touch.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] All Phase D hover paths return the spec'd content.
- [ ] Goto-def from `ModelRef`/`SourceRef` values resolves to the underlying source file (model `.sql` or source YAML) or no-ops gracefully (never panics).
- [ ] Completion offers exactly the closed accessor / field sets at the respective sites; no leakage of arbitrary identifiers.
- [ ] No regression in Phase A / B / C LSP paths.
- [ ] `lsp.md` and `data_catalog.md` Phase D touches are timeless-oracle compliant.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-D): LSP hover + goto-def + completion for wide reflection`

---

### Phase 5: Examples fixture + `smelt-app-builder` skill update + `/smelt-loop` medium tier

**Goal.** Add `examples/meta_workspace/` exercising the Phase D surface end-to-end (union-models-by-tag worked example and an audit-log completeness check); add at least one broken-fixture sub-workspace per Phase D diagnostic code; extend the `smelt-app-builder` skill with a Phase D reference doc; extend the `/smelt-loop` `medium` tier with a Phase D-specific ask. The `example_diagnostics` test must pass for the clean fixture and report exactly the right diagnostic code on each broken sub-fixture.

**Pre-conditions.** Phases 1–4 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-cli/tests/example_diagnostics.rs` — extend to walk `examples/meta_workspace/` and its broken sub-fixtures. The clean workspace must report zero diagnostics. Each broken sub-fixture must report exactly one Phase D diagnostic code matching the fixture name (per Phase B/C precedent): `examples/meta_workspace_broken_with_tag_requires_text/`, `examples/meta_workspace_broken_with_tag_named_argument/`, `examples/meta_workspace_broken_wide_reflection_unknown_accessor/`, `examples/meta_workspace_broken_wide_reflection_unexpected_argument/`, `examples/meta_workspace_broken_model_ref_field_unknown/`, `examples/meta_workspace_broken_source_ref_field_unknown/`.

**Implementation shape.**

- `examples/meta_workspace/` (clean fixture):
  - `smelt.yml` — minimal workspace config.
  - `models/` — three to four models with overlapping tags (`cohort`, `audit`, `core`) to exercise `with_tag` matching plus union behaviour. Keep small but realistic; mix frontmatter tags and `smelt.yml` tags.
  - `models/all_cohorts.sql` — `smelt.models.with_tag('cohort') |> reduce(union_all)` worked example (the wide-reflection analog of Phase C's `coalesce_numeric`).
  - `models/audit_completeness.sql` — uses `smelt.models.all() |> filter(fn m => …) |> map(fn m => …)` to assert every model in the workspace has at least one row in a hand-authored audit log (a sentinel real-world acceptance test).
  - At least one model exercising `smelt.sources.with_tag` or `smelt.sources.all` over a small `sources.yml` fixture.
- `examples/meta_workspace_broken_*/` — one sub-fixture per Phase D diagnostic, narrowly designed so the fixture reports exactly one diagnostic and no cascading errors.
- `.claude/skills/smelt-app-builder/references/20260512-meta-workspace.md` — per-phase reference doc following the Phase A/B/C precedent (workflow gotchas only; point at user docs for syntax). Capture: closed accessor set (`with_tag`, `all`); closed `ModelRef`/`SourceRef` field set; `m.columns` ≡ `smelt.columns_of(m)`; ModelRef/SourceRef lifts to TableExpr at splice points (the user writes `reduce(smelt.models.with_tag('x'), union_all)`, not `map(fn m => m.table_expr) |> reduce(union_all)`); path-sorted determinism (row order in a unioned result is by model path); the meta-`Text`-as-identifier lift is **not** widened by Phase D — `m.path`/`m.name` only lift in the same four positions speced for Phase C.
- `.claude/commands/smelt-loop.md` (or the appropriate fixture file referenced by it) — extend the `medium` tier with a Phase D-specific ask. One concrete shape: "Use `smelt.models.with_tag` and `reduce(union_all)` to define a model that UNIONs every model tagged `cohort` in the workspace."

**Critical files (allowed to touch in this phase).**

- `examples/meta_workspace/` and `examples/meta_workspace_broken_*/` (new directories).
- `crates/smelt-cli/tests/example_diagnostics.rs` — extend the test to gate the new fixture.
- `.claude/skills/smelt-app-builder/references/20260512-meta-workspace.md` (new file).
- `.claude/commands/smelt-loop.md` and any associated tier-fixture file under `.claude/commands/` — Phase D ask addition.

**Docs touched.**

- None new in `docs-site/` this phase (Phase 6 is the user-docs phase). Skill ref doc and loop tier are workflow artifacts.

**Review checklist** (material findings only):

- [ ] Clean fixture reports zero diagnostics; every broken sub-fixture reports exactly one Phase D diagnostic with the correct code.
- [ ] Fixture is minimal-but-realistic — no contrived shapes; the `all_cohorts` example mirrors the meta-plan §8 killer-demo direction (without records / multi-model production, which are Phase E1 / E2).
- [ ] Skill ref doc captures workflow gotchas not derivable from user docs (lift narrowness, closed sets, subtyping to TableExpr).
- [ ] Loop tier ask is solvable with Phase A–D constructs only (no records, no multi-model production).
- [ ] `cargo test -p smelt-cli --test example_diagnostics` green.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all green.

**Commit.** `feat(meta-language-D): examples/meta_workspace + skill ref + smelt-loop medium tier ask`

---

### Phase 6: User docs — `reflection.md` extension + `reference.md` extension

**Goal.** Extend `docs-site/docs/meta-language/reflection.md` (authored in Phase C) with the wide-reflection accessors and record types. Extend `docs-site/docs/meta-language/reference.md` (alphabetical, per Phase A/B/C precedent) with `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, `smelt.sources.all`, `ModelRef`, `SourceRef`. The reflection page covers both narrow (Phase C) and wide (Phase D) reflection as one coherent concept area, with section ordering: narrow accessor → narrow record → wide accessors → wide records → identifier lift → diagnostics.

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- None — user docs are not gated by test runners. Verification is by docs-reviewer in Phase 7.

**Implementation shape.**

- `docs-site/docs/meta-language/reflection.md` (extend):
  - Add a "Wide reflection: workspace introspection" section after the narrow-reflection content.
  - `smelt.models.with_tag(tag: Text) -> List<ModelRef>` and `smelt.models.all() -> List<ModelRef>` — signatures plus one worked example each (union-by-tag and workspace-iteration respectively).
  - `smelt.sources.with_tag(tag: Text) -> List<SourceRef>` and `smelt.sources.all() -> List<SourceRef>` — analogous.
  - `ModelRef` and `SourceRef` — closed records, four fields with types and meanings; uniformity note.
  - Subtyping: `ModelRef <: TableExpr` and `SourceRef <: TableExpr` — one paragraph explaining "you can pass a `ModelRef` anywhere a `TableExpr` is expected; no explicit projection needed". Worked example: `reduce(smelt.models.with_tag('cohort'), union_all)` is the natural shape; you do not need to write `|> map(fn m => m.table_expr)`.
  - `m.columns` ≡ `smelt.columns_of(m)` equivalence — one sentence.
  - Determinism and ordering: path-sorted, deterministic across runs over the same workspace state, byte-equal Salsa cache.
  - Diagnostics: one paragraph per Phase D diagnostic code with a "what to fix" hint.
  - Cross-reference to `lists.md` (Phase A) and `hofs.md` / `pipes.md` (Phase B) for the building blocks.
- `docs-site/docs/meta-language/reference.md` (extend):
  - Add `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, `smelt.sources.all` entries (signature, one-line semantics, link to `reflection.md`).
  - Add `ModelRef` and `SourceRef` entries (closed field set with types, link to `reflection.md`).
  - Maintain alphabetical order across the page.

**Critical files (allowed to touch in this phase).**

- `docs-site/docs/meta-language/reflection.md` (extend).
- `docs-site/docs/meta-language/reference.md` (extend).
- `docs-site/docs/meta-language/index.md` — only if a brief Phase D entry needs adding to a contents block.

**Docs touched.**

- `docs-site/docs/meta-language/reflection.md` (extend).
- `docs-site/docs/meta-language/reference.md` (extend).

**Review checklist** (material findings only):

- [ ] Every Phase D Surface item from `meta_language.md` is documented in `reflection.md` or `reference.md`.
- [ ] No syntax appears in docs that is not speced.
- [ ] Every Phase D diagnostic code has a "what to fix" hint in `reflection.md`.
- [ ] Reference page remains alphabetical and complete (Phases A + B + C + D entries).
- [ ] Worked example shows the subtyping shape (no explicit `m.table_expr` projection).
- [ ] Path-sorted determinism is documented and a user-visible guarantee.
- [ ] Timeless-oracle compliance: no `### Phase D` headings, no `(Phase D)` labels, no `[deferred to Phase E1]` callouts in body. Open questions / known gaps belong in the spec's Known Divergences (already there), not in the user docs.

**Commit.** `docs(meta-language-D): wide reflection user docs + reference page extension`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase D applicable expert reviewer from meta-plan §5 over the Phase D diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below. This phase is the realisation of the user's original ask: "Use expert reviews by subagents with specific context to help guide the implementation."

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase D subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs` | `SmeltType::ModelRef` / `SmeltType::SourceRef` (or alternative witnesses) addition is non-breaking; `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` are the single source of truth for the v1 field sets and are structurally identical; the four wide-reflection accessor signatures are exactly `(Text) -> List<ModelRef|SourceRef>` and `() -> List<ModelRef|SourceRef>`; `with_tag`'s compile-time-Text check re-uses Phase B's predicate; the `ModelRef <: TableExpr` and `SourceRef <: TableExpr` subtyping rules are one-way; `type_inference.rs` purity preserved. |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/lib.rs` | Hover on `smelt.models.*` / `smelt.sources.*` / `ModelRef`-binding / `SourceRef`-binding / field projection returns the spec'd content; goto-def from `ModelRef` / `SourceRef` values at splice sites resolves to the underlying source file or no-ops gracefully; completion at `smelt.models.<cursor>` / `smelt.sources.<cursor>` offers exactly the closed accessor set; completion at field-projection sites offers exactly the closed field set; spans line up with CST; no panics on partial parses; no regressions in Phase A/B/C hover/goto/completion paths. |
| **examples-curator** | haiku | `examples/meta_workspace/` and `examples/meta_workspace_broken_*/` | Clean fixture is minimal-but-realistic; the union-by-tag and audit-completeness examples motivate the design without contrivance; every Phase D diagnostic code has a corresponding broken sub-fixture; broken fixtures report exactly one Phase D diagnostic with no cascading errors; passes `cargo test -p smelt-cli --test example_diagnostics`. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{reflection,reference,index}.md` | Every Phase D Surface item is documented; every Phase D diagnostic code has a "what to fix" hint; reference page is alphabetical and complete (Phases A+B+C+D); no syntax in docs that is not speced; subtyping shape is reflected in worked examples; path-sorted determinism is documented; timeless-oracle compliance. |
| **cross-feature-impact-reviewer** | sonnet | meta-plan §6 cross-feature implications table; `docs/specs/{expansion,lsp,types,data_catalog}.md` | The meta-plan §6 cross-feature implications row for Phase D is complete after this phase: `expansion.md` Phase D touch lands the `model_origin` / `source_origin` extensions; `lsp.md` Phase D touch lists per-construct LSP obligations; `types.md` registers the two subtyping entries; `data_catalog.md` records the informational note about wide-reflection observation of model / source identities (the deeper catalog change deferred to Phase E2). Confirms no spec touch was missed and no scope expanded beyond the table. |

**Loop discipline.**

1. **Round 1.** Dispatch all five experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-D.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md` wide-reflection sections, plus `expansion.md` / `lsp.md` / `types.md` / `data_catalog.md` cross-touches).
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase D — typically `git log --oneline 3849446..HEAD`, where `3849446` is the second spec-increment commit; the implementation commits land after).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-D): address {expert-name} feedback` (e.g. `review(meta-language-D): address type-expert feedback`).
   - Push after each commit (so the user sees progress on PR #117).

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed, not the whole panel. Provide the same prompt as round 1 plus a diff of what changed since round N−1. If the expert returns "no material findings", that expert is **clean** and exits the loop.

4. **Repeat** step 2 → step 3 until **every** expert is clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound). The third repeat means the fix is wrong or the spec is wrong; the user must arbitrate.
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7). That's a design problem, not an implementation problem.
   - An expert's findings would force a spec change. Run `/smelt:spec meta_language` first; if the spec edit is non-trivial or contentious, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase D. Pause; the autonomy loop should not silently absorb pre-existing breakage.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260509-meta-language-D.md` (to record round counts and final clean status).

**Docs touched.** None new — fixes may amend `docs-site/docs/meta-language/*` if the docs-reviewer flags a surface drift; or `docs/specs/expansion.md` / `docs/specs/lsp.md` / `docs/specs/types.md` / `docs/specs/data_catalog.md` if the cross-feature-impact-reviewer flags a missing or drifted touch.

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] All five experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below (see acceptance gate).
- [ ] No fix touched files outside the dispatching expert's scope (no scope creep).
- [ ] No expert ran more than 3 rounds; if any did, the autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 7 expert review: type-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}), cross-feature-impact-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-D): address {expert-name} feedback`. If round 1 came back clean for an expert, no commit for that expert. The acceptance-gate summary line lands in the next commit naturally (or in a tiny `chore(meta-language-D): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

Phase 7 expert review: type-expert clean (R1), lsp-expert clean (R1), examples-curator clean (R1, one material finding addressed — misleading comment in all_cohorts.sql), docs-reviewer clean (R1), cross-feature-impact-reviewer clean (R1). No stop-the-line fired.

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes.
- `cargo test -p smelt-cli --test example_diagnostics` passes — `examples/meta_workspace/` clean, broken sub-fixtures report the exact Phase D diagnostic codes.
- `/smelt:validate meta_language` reports zero drift.
- LSP smoke test in `examples/meta_workspace/`: hover, goto-def, completion all work for `smelt.models.*`, `smelt.sources.*`, `ModelRef` bindings, `SourceRef` bindings, field projections per spec.
- Phase 7 acceptance gate met: every applicable expert reviewer (type-expert, lsp-expert, examples-curator, docs-reviewer, cross-feature-impact-reviewer) reported "no material findings" on its final dispatch, recorded in "Deferred during implementation" with round counts per expert. No stop-the-line condition fired.
