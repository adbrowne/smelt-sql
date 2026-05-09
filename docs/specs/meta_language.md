---
feature: meta_language
status: experimental
last_reviewed: 2026-05-09
owners: [andrew]
---

# Meta-Language

> **What this is.** A normative spec for smelt's typed compile-time meta-language: the user-visible mechanism for constructing, transforming, and reducing lists of fragments at compile time. In scope: `List<T>`, list literals, spread operator, higher-order functions (`map` / `filter` / `reduce`), lambdas, the pipe operator `|>`, contextual reducers, reflection, records, `Map<K, V>`, and multi-model production from compile-time configuration. Out of scope: `smelt.define` function-level fragment composition (see `functions.md`); the data-world `DataType` vocabulary that meta values may eventually splice into (see `types.md`); codegen-time expansion of named functions (see `expansion.md`); resolution of names within meta-evaluated bodies (see `scoping.md`); the YAML/JSON file loader family that supplies meta-world data from disk (see `meta_config_loading.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Skeleton notice.** This spec is being filled phase by phase per `docs/plans/20260509-meta-language-overall.md`. Sections marked `[deferred to Phase X]` are placeholders — the surface they describe does not yet exist in code. The framing in §Design and §Constraints is load-bearing now (it constrains every later phase) and is filled in immediately.

## Surface

The meta-language adds compile-time-only constructs to smelt SQL files and `smelt.define` bodies. None of this surface reaches the database engine; meta evaluation happens during type checking and produces fragments that splice into Data-World SQL.

### Phase A — `List<T>`, list literals, spread *(deferred to Phase A)*

Will define:

- `List<T>` type (parameterised meta-only collection type).
- List literal syntax `[a, b, c]` (with bidirectional disambiguation against runtime `Array<T>` literals).
- Spread operator `...xs` in comma-separated positions (SELECT lists, function arguments, ORDER BY).

### Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var` *(deferred to Phase B)*

Will define:

- Lambda syntax `fn x => body` (single-arg in v1).
- Higher-order functions: `map`, `filter`, `reduce`.
- Pipe operator `|>` (first-arg pipe).
- Contextual reducers: `comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`.
- `smelt.config.var(name)` — compile-time variable lookup from `smelt.yml` `vars:` block.

### Phase C — Narrow reflection: `smelt.columns_of`, `ColumnRef` *(deferred to Phase C)*

Will define `ColumnRef` meta record type (`name: Text`, `type: DataType`, `is_numeric: Boolean`, …) and the `smelt.columns_of(t: TableExpr) -> List<ColumnRef>` accessor.

### Phase D — Wide reflection: workspace introspection *(deferred to Phase D)*

Will define `ModelRef` and `SourceRef` meta record types and `smelt.models.*` / `smelt.sources.*` accessors (`with_tag`, listing, filtering).

### Phase E1 — Records, `Map<K,V>`, schema-typed config loaders *(deferred to Phase E1)*

Will define record meta type (inline `Record<{...}>` and named `smelt.record Name = { fields }`); `Map<K,V>` with `entries`/`keys`/`values`/`get`/`has`; and the schema-typed YAML/JSON/TOML loaders (full surface in `meta_config_loading.md`).

### Phase E2 — Multi-model production *(deferred to Phase E2)*

Will define the `generates: models` frontmatter directive; the `ModelDef` record meta type; the meta-`Text`-as-identifier lift in path positions; and the workspace-shape change ("one file may produce N models").

### Phase F — Polish *(deferred to Phase F)*

Will define parameterised reducers (e.g. `concat_with(sep)`); multi-arg lambdas; the meta-world ternary `if cond then a else b`; and `zip_with` if any shipped example demands it.

### Phase G — LSP completeness *(deferred to Phase G)*

Will define rename support for new constructs and guarantee hover/goto-def/completion/diagnostics-with-frame-stacks across every shipped meta-language surface element. (No new syntactic surface; LSP capability is part of the spec because the user-visible behaviour of editor tooling is part of "how this feature works".)

## Semantics

### Two worlds, one program

The meta-language extends smelt with a **compile-time evaluation layer**. Every program is checked and evaluated in two interlocking layers:

- **Meta-World.** Compile-time values (lists, lambdas, records, reflection results, config values). Types are fragment sorts (`Expr<T>`, `TableExpr`, `SelectItems<…>`, `OrderSpec`) plus the new types `List<T>`, `Lambda<…>`, `Record<…>`, `Map<K,V>`. Meta values exist only during compilation.
- **Data-World.** SQL the database engine sees. Types are the `DataType` vocabulary in `types.md`. Data values exist at query runtime.

The two worlds intersect at **splice points** — places where a meta value materialises into Data-World syntax. Splice points already exist (every `smelt.<path>(...)` call is one); the meta-language adds: list literal positions, spread positions, and (Phase E2) generated-model positions.

### Meta-evaluation rules (load-bearing across phases)

1. **Termination.** Meta evaluation must terminate without user-visible recursion. Lists are finite; HOFs walk once. Reflection results are bounded by workspace state. The compiler must reject any construct that admits unbounded recursion at meta level.
2. **Determinism.** Meta evaluation given the same workspace state must produce the same result. No clock, no random, no network. Environment variables are accessible only via gated APIs (Phase E1 spec touch) that opt the file out of pure determinism.
3. **Purity.** Meta evaluation has no side effects. The compiler may evaluate the same expression multiple times for caching reasons; user code may not depend on observable side effects.
4. **Single-pass type-check.** Every meta program must type-check in a single pass without execution. The type checker may invoke a (bounded) meta evaluator to compute reflection results during checking, but this evaluator is itself pure and terminates.
5. **No string templating.** Meta values are CST nodes (or values that produce CST nodes). The compiler never re-parses the output of meta evaluation.

### Per-phase semantic rules

*[deferred to phases A–F]*

## Design

### Why a meta-language at all

`smelt.define` (`functions.md`) closed the gap on **fragment-level** reuse — predicates, expressions, table transformers, select-list shapes can be parameterised, called by name, and inlined with full type checking. It does not address the class of dbt patterns where the *input to the SQL is computed from the project itself*: union all models matching a tag, coalesce all numeric columns, generate one staging model per source-table entry in a YAML file. These patterns require iterating over compile-time data — a list of models, a list of columns, a list of config rows — and reducing the result into a SQL fragment that splices into a model.

dbt does this through Jinja string-substitution. That choice forfeits typing, navigation, and LSP feedback inside macros: `{{ col.name }}` resolves to nothing, errors anchor to the post-substitution SQL, refactoring is text-only. smelt's meta-language proposes to give every meta value a type, every cross-reference an LSP-resolvable definition, every diagnostic a source span. The unifying claim, restated from the research doc: smelt already has a meta-world (fragment sorts, two-tiered expansion, splice contexts); making it user-visible is a layering exercise, not a new language inside the language.

The full design rationale, alternatives considered at every level (lambda surface, reducer registry, list literal disambiguation, reflection API shape, multi-model production mechanism), and the framing of the meta-/data-world boundary live in `docs/research/20260507-typed-meta-programming.md`. This spec records the decisions; the research doc records why they look like this.

### Why a single spec rather than per-construct specs

The constructs are deeply interdependent. Lambdas have no use without HOFs; HOFs have no use without lists; lists have limited use without reflection; reflection has limited use without records; records have limited use without loaders. Splitting this into seven specs would force every later spec to repeat the framing of the meta-/data-world boundary and the meta-evaluation rules above. One spec, one framing, multiple Surface entries that grow phase by phase.

The exception is `meta_config_loading.md` — the file-loading family is large enough (formats, schema authoring, validation diagnostics, per-target overlay) to warrant its own spec, with this one referencing it.

### Per-phase design rationale

*[deferred to phases A–F; expanded as each phase lands]*

## Constraints & Invariants

### Meta-world invariants (always hold)

- **Meta evaluation never reaches the database engine.** Every meta value is consumed during type checking or codegen-time expansion. The DB-facing SQL contains only Data-World constructs.
- **`smelt-db/src/type_inference.rs` remains pure.** New HOF and reflection rules are added as pure functions; Salsa queries call them, they do not call Salsa queries. (See `CLAUDE.md` Pure Function Rule.)
- **Termination is structural, not check-and-error.** The grammar admits no construct that requires runtime fixed-point iteration. If a syntax extension would admit unbounded recursion, it is rejected at the spec level, not allowed and policed at evaluation time.
- **No expansion-frame regression.** Adding HOF and multi-model production must not weaken the `expansion.md` frame-stack contract. Diagnostics from inside a `map(xs, fn x => …)` body must surface with a frame stack that names `map` and the per-element index.
- **Bidirectional checking remains decidable.** New types interlock with existing widening rules without introducing non-deterministic checks.

### Out-of-scope by deliberate choice

- **Pipe-SQL extension** (research §4.6 alternative b) — porting the pipe operator into Data-World queries is a separate paper.
- **Tuples** — rejected in favour of records; `zip_with` (if shipped) takes a multi-arg lambda rather than producing a `List<Tuple<…>>`.
- **Generators-of-generators** — Phase E2 forbids one generator file consuming another generator's output. Cycles in workspace-shape evaluation are rejected at the spec level.
- **Heterogeneous lists / sum types** — meta lists are monomorphic. A list with mixed element types is a type error; sum types are out of scope.
- **User-defined reducers** — the v1 reducer registry is closed. Extension requires a compiler change (revisit when concrete pain emerges and a soundness-verification approach exists).
- **`infer_schema` codegen mode** — schema authoring for config loaders is required; tools that infer schemas from sample data are post-plan.

## Known Divergences / Open Questions

- **Spec is incremental.** Sections A–G are filled phase by phase. Until the corresponding phase ships, the section says `[deferred to Phase X]`. Code may not exist yet for those sections — that is the intended state.
- **Lambda surface — `fn x => body` keyword vs. positional disambiguation.** Research §4.5 leans `fn`, with positional disambiguation as backup. Phase B finalises the choice; until then the spec describes the leaned shape only.
- **Reflection API namespace organisation.** Research §4.8 lands on functional accessors under `smelt.*`. The exact namespace layout (`smelt.models` vs `smelt.workspace.models`) is a Phase D decision.
- **Multi-model production mechanism.** Research §4.10.4 leans on a frontmatter directive (`generates: models`) plus a body returning `List<ModelDef>`. Phase E2 finalises; the meta-plan §10 stop-the-line condition catches scope expansion.
- **Meta-`Text`-as-identifier lift narrowness rule.** Phase E2 must commit to which SQL grammar slots admit a meta-`Text` lift (model path components, column aliases, CTE names) and which do not (arbitrary keywords). The narrowness is what keeps the lift from drifting into Jinja territory.

## References

- **Code**: *(populated as phases land)*
  - Phase A onwards: `crates/smelt-parser/src/{lexer,parser,ast}.rs`, `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs`, `crates/smelt-db/src/function_body_check.rs`, `crates/smelt-db/src/lib.rs`, `crates/smelt-lsp/src/lib.rs`
- **Tests**: *(populated as phases land)*
  - Phase A onwards: `crates/smelt-cli/tests/example_diagnostics.rs` (acceptance gate); per-crate unit tests in the directories above
- **User docs**: *(populated as phases land)*
  - Phase A onwards: `docs-site/docs/meta-language/{index,lists,hofs,lambdas,pipes,reflection,generators,reference}.md`
- **Plans (history)**:
  - `docs/plans/20260509-meta-language-overall.md` — meta-plan / phase status table
  - `docs/plans/20260509-meta-language-A.md` — Phase A *(when written)*
  - `docs/plans/20260509-meta-language-B.md` — Phase B *(when written)*
  - `docs/plans/20260509-meta-language-C.md` — Phase C *(when written)*
  - `docs/plans/20260509-meta-language-D.md` — Phase D *(when written)*
  - `docs/plans/20260509-meta-language-E1.md` — Phase E1 *(when written)*
  - `docs/plans/20260509-meta-language-E2.md` — Phase E2 *(when written)*
  - `docs/plans/20260509-meta-language-F.md` — Phase F *(when written)*
  - `docs/plans/20260509-meta-language-G.md` — Phase G *(when written)*
- **Related specs**:
  - `docs/specs/functions.md` — `smelt.define`, fragment sorts, named arguments (parser disambiguation surface)
  - `docs/specs/types.md` — `DataType` vocabulary, fragment-sort grammar, strict-by-default doctrine
  - `docs/specs/expansion.md` — codegen-time expansion, frame stacks, `Caller`/`Callee`/`Synthesized` provenance — extended by Phase B (HOFs) and Phase E2 (multi-model)
  - `docs/specs/scoping.md` — body scoping, splice contexts, parameters-first; lambda parameter scoping (Phase B) and generator-file scoping (Phase E2) plug in here
  - `docs/specs/architecture.md` — `smelt.<path>` resolution, project layout — Phase E2 amends the "1 file = 1+ models" invariant
  - `docs/specs/gradual_typing.md` — `Unknown` widening — Phase A spec touch documents `List<Unknown>` rules
  - `docs/specs/meta_config_loading.md` — file-loader family for `smelt.config.load_yaml` etc. (Phase E1)
  - `docs/specs/model_selection.md`, `incremental_models.md`, `python_models.md`, `data_catalog.md`, `schema_evolution.md`, `cli.md`, `datagen.md` — Phase E2 cross-feature touches (see meta-plan §6)
  - `docs/specs/lsp.md` — LSP support obligations per phase
- **Research**:
  - `docs/research/20260507-typed-meta-programming.md` — design oracle: framing, alternatives at every choice point, sequencing, worked examples, open questions
  - `docs/research/20260413-smelt-functions.md` — parent paper for fragment sorts and `smelt.define`
