# Plan: Author the missing specs from `docs/research/20260413-smelt-functions.md`

**Date**: 2026-04-28
**Tracking branch**: `worktree-spec`
**Output**: a sequence of new specs under `docs/specs/`, plus a small extension to `docs/specs/architecture.md`.

## Context

`docs/specs/types.md` (committed 2026-04-28) covers the type vocabulary, fragment sorts, promotion, and constraints, but the smelt-functions research has substantial spec-worthy material untouched: the `smelt.define` / `smelt.fn.*` / `smelt.extern` / `PASSING` surface, scoping rules, the three-tier gradual typing model, planner integration, and the models-as-functions equivalence. This plan walks that material into normative specs in dependency order.

This is a documentation-authoring plan, not an implementation plan. Each phase calls `/smelt:spec <slug>` and produces one spec file. No code changes.

## Phase order

```
1. functions          ← biggest, foundation for the rest
2. scoping            ← depends on functions
3. gradual_typing     ← depends on functions
4. (extend) architecture  ← small addition: models-as-functions
5. planner_integration ← depends on functions
6. expansion          ← internal invariant, lowest priority
```

## Progress tracking

| Phase | Spec slug                   | Status   | Commit | Date |
|-------|-----------------------------|----------|--------|------|
| 1     | functions                   | done     | 3e042cd | 2026-04-29 |
| 2     | scoping                     | done     | 3195ae9 | 2026-04-29 |
| 3     | gradual_typing              | done     | 7f23f15 | 2026-04-29 |
| 4     | architecture (extend)       | done     | 6b50cc8 | 2026-04-29 |
| 5     | planner_integration         | done     |        | 2026-04-29 |
| 6     | expansion                   | pending  |        |      |

---

## Phase 1 — `docs/specs/functions.md`

**Goal.** Normative spec for the user-facing function surface: `smelt.define`, `smelt.fn.*`, `smelt.extern`, `PASSING`, `smelt.as_struct`, frontmatter properties, default values, cycle rules.

**Source material.**
- Research §3 (Functions over Fragments), §4 (Models as functions — taxonomy bullet only; full treatment in Phase 4), §5 (Black box functions), §10 (PASSING), §13 (canonical registry, backend tiers), §16 #6/#11/#18/#19/#20/#21/#22/#23.
- Code: `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/{lib.rs,function_body_check.rs,functions.rs}`, `crates/smelt-parser/src/...` for `SmeltDefine`/`SmeltExtern`/`SmeltFnCall`/`PASSING`.

**Surface to capture.**
- **File structure (per-`.sql`-file grammar).** A file is a sequence of `(frontmatter? declaration)` items. Declarations are: `smelt.define`, `smelt.extern`, or one bare model `SELECT`. They may interleave freely. At most one bare model `SELECT` per file. Whitespace separates items; no separator token. (§16 #11, §16 #22.) Conflict diagnostic: `DuplicateFunctionDefinition` for same-name defines in the same directory.
- **Function-declaration frontmatter.** Unified attachment rule (§16 #22): each frontmatter block attaches to the immediately following declaration, and each declaration may carry its own. Keys catalogued in this spec:
  - `smelt.define`: `deterministic`, `idempotent`, `append_only`, `backends`. Structured `joins` / `provenance` gated behind `smelt.yml: unstable_schema: true` (`UnstableSchemaRequired`).
  - `smelt.extern`: same as `smelt.define`, plus extern-specific spelling decisions.
  - Model frontmatter keys (e.g. `materialization`) are catalogued in `incremental_models.md` and the architecture extension (Phase 4) — *not* duplicated here.
  Diagnostics: `FrontmatterParseError` (Error for malformed YAML; Warning for unknown keys), `BackendsWideningNotAllowed` (declared `backends:` widens what the body supports).
- `smelt.define` grammar: `smelt.define <name>(<params>) [-> <Type>] AS (<body>) [;]`.
- `smelt.fn.<path>(...)` call syntax. Directory-derived namespacing under `functions/` (project layout itself is documented in `architecture.md` Phase 4 extension; this spec only references it). Named args via `=>`. No overloading, no recursion (cycle detection emits `FunctionCallCycle`).
- Parameter spec: `name [: <Type>] [= <default>]`. Trailing commas. Default-value rules (§16 #20): self-contained, type-checked at definition, fragment defaults = "splice nothing", `= TRUE` for boolean filters.
- `smelt.extern <name>(...)` (§16 #21): full grammar, fragment-param rejection (`ExternFragmentParamUnsupported`), name collision (`ExternCollidesWithBuiltin`).
- `PASSING <name> AS (...)` clauses (§10, §16 #18): grammar, attachment to a call site, multiple PASSING clauses, `UnknownPassingParameter`.
- `smelt.as_struct(alias EXCEPT ...)` (§6, §16 #19): compile-time struct namespacing, `AsStructUnsupportedBackend`.
- Backend namespacing: `duckdb.read_parquet(...)`, `postgres.sum(...)`. Three portability tiers (§13).
- Engine-agnostic bodies, backend specificity via namespace + `backends:` (§16 #23).
- `smelt.metric()` explicitly out of scope (§16 #6).

**Semantic rules to make normative.**
- All functions are public (no visibility modifier in v1).
- No recursion, no overloading, no nesting (defines may not appear inside SELECT/CTE/another body).
- One canonical built-in registry, not per-dialect (§13).
- Canonical return types are CAST-enforced; backend-namespace calls opt out.

**Diagnostic codes covered.** `DuplicateFunctionDefinition`, `DuplicateParameterName`, `UnknownSmeltFn`, `MissingArgument`, `ArgTypeMismatch`, `FunctionBodyTypeMismatch`, `ReturnTypeMismatch`, `InvalidFunctionTypeRef`, `FunctionCallCycle`, `ExternCollidesWithBuiltin`, `ExternFragmentParamUnsupported`, `UnknownPassingParameter`, `BackendsWideningNotAllowed`, `FrontmatterParseError`, `UnstableSchemaRequired`, `AsStructUnsupportedBackend`.

**Likely Known Divergences.**
- Phase coverage gaps from `docs/plans/20260422-smelt-functions.md` (e.g., `joins`/`provenance` parsing not all wired).
- Whether end-to-end `smelt build` runs `smelt.fn.*` (Phase 56/57 of that plan).

**Split decision (decide in Step 3 of `/smelt:spec`).** If the draft exceeds ~350 lines, split out one of: `extern.md` (smelt.extern + UDF declaration), `passing.md` (block syntax), `as_struct.md` (compile-time struct namespacing). Default: keep them together — they share frontmatter/diagnostic infrastructure and reading them as one document is more useful.

**Command.** `/smelt:spec functions`

---

## Phase 2 — `docs/specs/scoping.md`

**Goal.** Normative spec for name resolution inside `smelt.define` bodies: parameters-first ordering, no-overlap rule, context bindings, splice-point inference.

**Source material.**
- Research §6 (Context Bindings), §7 (Parameters-First Scoping), §16 #1, #4, #5, #7.
- Code: `function_body_check.rs` (parameter binding, shadow detection); `TypeContext` `function_params` / `tableexpr_param_schemas` / `fragment_param_kinds` / `opaque_ctes`.

**Surface.**
- `Expr<T, ctx>` annotation grammar (context names a sibling parameter).
- Parameter-name lookup before any SQL FROM scope; `ParameterShadowsColumn` warning.
- Bare columns from `TableExpr` parameters resolve when unambiguous (§16 #7).

**Semantic rules.**
- Resolution order: function params → CTE columns → FROM-scope columns → upstream model/source schemas.
- No-overlap rule (§16 #4): parameter contexts must have unique column names. Three escape hatches: explicit CTE rename, typed `TableExpr` parameter, `smelt.as_struct(... EXCEPT)`.
- Context inference (§16 #5): inferred from splice points; multi-splice = intersection of schemas. Explicit `Expr<T, ctx>` is documentation/validation.
- Annotation-too-wide rule: explicit context cannot claim columns missing from inferred splice context.

**Diagnostic codes covered.** `ParameterShadowsColumn`, `UnknownIdentifier`, `UnknownContext`, `ContextMismatch`, `FragmentColumnMissing`, `AnnotationTooWide`, `FragmentKindMismatch`, `CteCycle`.

**Likely Known Divergences.**
- CTE alpha-renaming is a v2 item (§16 #12); v1 emits a collision diagnostic.
- Bare-column resolution from JOIN aliases inside `TableExpr` bodies — Phase 45 of the smelt-functions plan.

**Command.** `/smelt:spec scoping`

---

## Phase 3 — `docs/specs/gradual_typing.md`

**Goal.** Normative spec for the three annotation tiers, bidirectional checking, and the error-tracing contract that ties errors back to call sites.

**Source material.**
- Research §8 (Three Tiers), §9 (Bidirectional Checking), §16 #16 (Tier 1 single-level traces), §16 #17 (Tier 2 calling Tier 1).
- Code: `function_body_check.rs` (Tier dispatch), `signatures.rs::Tier`, `DiagnosticData::ExpansionFrames`.

**Surface.**
- Tier rules:
  - Tier 1 (no annotations): expand at each call site, check expanded body in synthesis mode, errors mapped back via frame trace.
  - Tier 2 (parameters annotated, no return type): body checked in isolation; row variables unify locally per call.
  - Tier 3 (parameters + return type): body checked against return type in checking mode; `ReturnTypeMismatch` if synthesised type disagrees.
- Error format guarantees: "expected X, got Y"; row variables never appear in user-facing messages; errors are local.
- Frame stack data structure (`FrameInfo`, `ExpansionFrames`) — single-level rendered in v1, multi-level in v2.
- LSP stability under broken bodies: which features keep working when a body fails to type-check.

**Semantic rules.**
- Bidirectional rule: at call sites, declared types push down (checking mode); inside bodies, types flow up (synthesis); return annotations re-introduce checking.
- No cross-boundary inference: Tier 1 return types are computed per call, never propagated.
- No higher-rank polymorphism, no implicit coercion across concrete types, no global constraint solving.
- Engine aliases (`Text`/`Varchar`) treated as equal at unification.

**Diagnostic codes referenced.** `FunctionBodyTypeMismatch`, `ReturnTypeMismatch`, `ArgTypeMismatch`, `MissingArgument` — same surface as `functions.md` but here we spec the *checking process* rather than the syntax.

**Likely Known Divergences.**
- Multi-level frame rendering deferred (§16 #16) — single-level renderer in v1.
- LSP hover for return types of Tier 1 calls: research promises stability; check live behaviour.

**Command.** `/smelt:spec gradual_typing`

---

## Phase 4 — Extend `docs/specs/architecture.md` (models-as-functions)

**Goal.** Add a section on the unified-model insight to `architecture.md` rather than spinning up a small standalone `models.md`.

**Source material.**
- Research §4 (The Unified Model), §16 #6 (multiple defines).
- Code: `smelt-core` (model discovery), `smelt-db::resolved_model_schema`.

**What gets added.**
- **Project layout.** Standard directory roles:
  - `models/` — model `.sql` files (zero or more `smelt.define` + at most one bare model `SELECT`).
  - `functions/` — function `.sql` files (zero or more `smelt.define` / `smelt.extern`; bare model `SELECT` allowed but unusual).
  - `seeds/` — seed CSVs.
  - `tests/` — unit-test SQL.
  - `sources.yml`, `smelt.yml` — project-level config.
  Note: file *kind* is grammar, not directory — a `models/foo.sql` may contain `smelt.define`s, and a `functions/bar.sql` may contain a model `SELECT`. Directory layout drives `smelt.fn.*` namespacing only.
- **Unified frontmatter rule (§16 #22).** A frontmatter block (YAML between `---` fences) attaches to the immediately following declaration: model `SELECT`, `smelt.define`, or `smelt.extern`. Each declaration may carry its own. Per-feature key catalogues live in the relevant feature spec (`functions.md` for function/extern keys; `incremental_models.md` for model materialization keys).
- A "Models as functions" subsection under Surface or Semantics.
- The taxonomy table (transparent vs black-box × persisted vs inline).
- The materialization-orthogonal-to-transparency rule.
- Parameterized models (model takes explicit `TableExpr` parameters in addition to default refs).
- `smelt.ref()` / `smelt.source()` semantics: parameter-with-DAG-default.
- Pointer to `functions.md` for the function half of the equivalence and for function-frontmatter keys.

**Why extend, not new file.** The unified model is a system-level claim about how models and functions relate at the architecture layer; it belongs in the architecture spec next to crate boundaries and the compilation pipeline. Splitting into `models.md` would make `architecture.md` lie about its own scope.

**Command.** Manual edit (no `/smelt:spec` invocation needed for an extension); follow up with `/smelt:validate architecture`.

---

## Phase 5 — `docs/specs/planner_integration.md`

**Goal.** Normative spec for how transparent functions and frontmatter properties feed the planner: three levels of rules, declared metadata, transparency boundary.

**Source material.**
- Research §12 (Planner Integration — Three Levels), §16 #22 (frontmatter property surface for the planner).
- Code: `smelt-planner` (transformations, rule API), `smelt-db::logical_plan`, `function_body_check.rs::ExpandedCall`.

**Surface.**
- Three planner levels:
  - L1 logical-to-logical (filter pushdown, function fusion, join elimination).
  - L2 logical-to-physical (strategy selection, expansion).
  - L3 physical-to-execution-plan (multi-statement lowering, cross-engine).
- Transparent vs black-box optimization boundary.
- Frontmatter properties acted on by the planner: `deterministic`, `idempotent`, `append_only`, `backends`, plus `joins`/`provenance` (gated behind `unstable_schema`).
- `--show-plan` CLI surface (Phase 39 of smelt-functions plan).

**Semantic rules.**
- Planner reasons across transparent boundaries; treats black-box calls as atomic.
- `idempotent: true` ⇒ retry-safe at L3; `deterministic: true` ⇒ re-execution produces same result.
- Properties are author-declared in v1; auto-derivation is deferred.
- Validation rules: `provenance:` is checked against body (`ProvenanceMismatch`); `joins:` is checked against body's outermost FROM (`JoinsMismatch`); `cardinality` is trusted with a warning (`DeclaredCardinalityUnverifiable`).

**Diagnostic codes covered.** `ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable`, `MissingProvenancePushdownAdvisory`.

**Likely Known Divergences.**
- L2 / L3 mostly aspirational in current code; only L1 rules ship in MVP. The spec should be honest about how much is wired vs declared.
- `provenance:` / `joins:` parsing is partially landed.

**Command.** `/smelt:spec planner_integration`

---

## Phase 6 — `docs/specs/expansion.md` (lowest priority)

**Goal.** Normative spec for AST-level expansion mechanics, provenance frames, and hygiene.

**Source material.**
- Research §16 #12 (Expansion mechanics).
- Code: `function_body_check.rs::ExpandedCall`, `DiagnosticData::ExpansionFrames`, `FrameInfo`.

**Surface.** Mostly internal — the only user-visible surfaces are diagnostic frame messages and CTE-collision diagnostics. Spec is short.

**Semantic rules.**
- Expansion is AST-level (clone CST, substitute placeholders, attach provenance). No textual substitution.
- Two senses of expansion: type-check-time (binding without rewrite) vs codegen-time (CST rewrite).
- Provenance origin tags: `Caller(span)`, `Callee(fn_id, span)`, `Synthesized(fn_id, reason)`.
- Frame stack pushed on every call expansion; renderer reads it.
- Hygiene v1: parameter resolution at type-check time, CTE-collision diagnostic. Alpha-rename deferred to v2.

**Why defer.** This is mostly an implementation invariant; the user-visible surface is small (frame trace formatting + the hygiene diagnostic). It is worth spec'ing eventually so future planner work doesn't silently break the provenance contract, but it is the lowest-leverage of the six.

**Command.** `/smelt:spec expansion`

---

## Verification (per spec)

After each phase:
- `wc -l docs/specs/<slug>.md` ≤ 300 (soft target).
- `git diff` shows no unintended changes to other specs.
- Where the spec references a code path, confirm the path exists (`ls`).
- Commit with message `specs: add <slug>.md — <one-line summary>`.

After all phases:
- `ls docs/specs/` shows the new files.
- Run `/smelt:validate <slug>` against each new spec to surface drift between spec, code, and `docs-site/`. Drift is expected — track in a follow-up plan, do not fix inside the spec PRs.

## Deferred / out of scope

- Implementation plans for any of the divergences these specs surface — those come from `/smelt:plan <slug>` against each spec's diff, not from this plan.
- Any user-doc (`docs-site/`) reconciliation — surfaced by `/smelt:validate`, fixed in follow-up plans.
- Splitting `functions.md` into sub-specs — decision made at Phase 1 Step 3 based on draft length.
