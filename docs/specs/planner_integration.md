---
feature: planner_integration
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Planner Integration

> **What this is.** Normative spec for how transparent functions, models, and frontmatter properties feed the smelt planner. Defines the three planner levels (L1 logical-to-logical, L2 logical-to-physical, L3 physical-to-execution-plan), the transparent vs black-box optimization boundary, the frontmatter properties the planner consumes, the validation rules the planner emits against declared metadata, and the `smelt build --show-plan` CLI surface. The architectural rule that the planner is value-producing and never mutates CSTs lives in `architecture.md`; the unified-model framing lives there too. The frontmatter keys' grammar lives in `functions.md`. This spec specifies the *consumption* of those keys — what the planner does with `deterministic`, `idempotent`, `append_only`, `backends`, `joins`, and `provenance`.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### Three planner levels

The planner operates as three conceptual levels of progressive lowering. The level a rule lives at is a property of the rule, not a separate pass — they all run inside the same fixed-point loop today, but the staging is intentional and observable in the rule list.

| Level | Input → Output                       | Rule examples (current code)                                          |
|-------|--------------------------------------|-----------------------------------------------------------------------|
| **L1** | Logical → Logical                   | `PushFilterIntoTransparentFunction`, `ExpandTransparentFunctionCalls`, `ElideEmptySelectItemsSplices`, `EliminateUnusedLeftJoin` |
| **L2** | Logical → Physical                  | Materialization-strategy choice (table / view / ephemeral / incremental); transparent-function expansion under a chosen strategy |
| **L3** | Physical → Execution Plan           | Multi-statement lowering (`CREATE TEMP → DELETE → INSERT → DROP`); cross-engine choreography (write Parquet on Spark, read on DuckDB); validated-write swap |

Levels are not directories in the crate today. Level membership of a rule is documented per-rule and asserted by ordering tests (e.g., `combined_rule_set_reaches_fixed_point` in `crates/smelt-planner/tests/pushdown_tests.rs`).

### Optimization boundary: transparent vs black-box

Every node in a logical plan is either **transparent** (planner may rewrite across the boundary) or **black-box** (planner treats as an atomic node and may only rewrite *around* it). The boundary is encoded on `LogicalNode::FunctionCall { transparent: bool, .. }`:

- `transparent: true` — `smelt.define`-declared functions and model bodies. Body is reachable; planner rules splice, push filters into, and reason across the boundary.
- `transparent: false` — `smelt.extern` declarations, canonical built-ins, source references. Body is unavailable to the planner; rules that recurse into bodies stop at the boundary.

The transparency flag is set during logical-plan construction (`smelt-db::logical_plan`); planner rules do not change it.

### Frontmatter properties the planner consumes

These keys live on the frontmatter block immediately preceding a `smelt.define`, `smelt.extern`, or model declaration (see `architecture.md` "Unified frontmatter rule" and `functions.md` for full key catalogue). The planner reads them via `LogicalNode::FunctionCall.properties: FunctionProperties`:

| Key             | Surface          | Default               | Planner effect                                                                        |
|-----------------|------------------|-----------------------|---------------------------------------------------------------------------------------|
| `deterministic` | `true` / `false` | `false`               | At L3, enables re-execution / replay reasoning (re-run yields same result).            |
| `idempotent`    | `true` / `false` | `false`               | At L3, marks the node retry-safe (mid-failure resume / partial-write recovery).        |
| `append_only`   | `true` / `false` | `false`               | At L2, eligible for incremental-append strategy under partition-based selection.       |
| `backends`      | `all` or list    | inferred from body    | At L2, narrows physical strategy choice; at L3, narrows engine routing.                |
| `joins`         | structured map   | absent                | At L1, enables `EliminateUnusedLeftJoin` when an entry declares `cardinality: 1:1`.    |
| `provenance`    | structured map   | absent                | At L1, enables `PushFilterIntoTransparentFunction` (filters can rewrite using the declared output→input column map). |

`joins:` and `provenance:` are gated behind `smelt.yml: unstable_schema: true`. When the flag is absent, the keys parse but the planner-visible field is reset to `Unknown` / empty and a `FrontmatterParseError` warning is emitted (see `functions.md`).

### `smelt build --show-plan` CLI surface

```
smelt build <model-file> --show-plan [--project-dir <dir>]
```

- **Required positional**: a model file path. `--show-plan` errors out without one.
- **Effect**: builds the logical plan for the given file (`smelt-db::logical_plan`), runs `apply_rules_to_fixed_point` over it with the `show_plan_rules()` rule list, prints the deterministic textual rendering produced by `smelt-planner::plan_printer::format_plan`, and exits.
- **No execution side effects.** No backend is contacted, no DDL is emitted, no state is written. The flag is read-only in the strictest sense.
- **Output shape is stable.** `format_plan` is byte-deterministic across runs of the same input — no `HashMap`, no `Instant`, no pointer addresses. Tests grep on the output; format changes are observable as test failures.

The `show_plan_rules()` rule list, in order:

1. `PushFilterIntoTransparentFunction` (L1)
2. `ExpandTransparentFunctionCalls` (L1)
3. `ElideEmptySelectItemsSplices` (L1)
4. `EliminateUnusedLeftJoin` (L1)

This rule list is also the v1 production rule list — there is no separate "show only" subset.

### Diagnostic codes

User-visible diagnostics emitted by the planner-validation pure functions in `smelt-db::provenance_validator`. Anchored at the declaration's name range.

| Code                                  | Severity | Triggered by                                                                                                              |
|---------------------------------------|----------|----------------------------------------------------------------------------------------------------------------------------|
| `ProvenanceMismatch`                  | Error    | Declared `provenance:` lists a source column not read by the body's outermost SELECT, or vice versa.                       |
| `JoinsMismatch`                       | Error    | A declared `joins:` entry names a table that does not appear as a join alias in the body's outermost FROM clause.          |
| `DeclaredCardinalityUnverifiable`     | Warning  | Every declared join with a non-empty `cardinality:` field. Cardinality is trusted, not verified against data.              |
| `MissingProvenancePushdownAdvisory`   | Hint     | A transparent function called from a SELECT with a WHERE clause where the callee has no declared `provenance:`. Filter pushdown into the body is skipped; declaring `provenance:` enables the optimisation. |

## Semantics

These rules are normative. Each phrasing of the form "the planner must ..." applies to the rule pipeline assembled by `show_plan_rules()`; future rules added to that list inherit these obligations.

1. **Transparency gates traversal.** A planner rule that recurses into function bodies must terminate the recursion at any node where `transparent: false`. Black-box nodes are visible only by their signature, properties, and arguments — never by their body. Equivalently: rules may rewrite *around* a black-box node, never *through* it.

2. **Materialization is independent of transparency.** A `materialization: table` model is still transparent; an `ephemeral` model is still transparent. The planner's optimization boundary does not change with materialization. Materialization decides how the output is realised at execution time (L2/L3), not whether the body is visible (L1). See `architecture.md` "Models as functions" for the underlying rule.

3. **L1 rules preserve correctness; L2/L3 rules preserve equivalent output schema.** L1 rewrites must produce a logical plan that computes the same set of rows for any input. L2/L3 rewrites must produce an execution that yields the same final output schema and contents — including for incremental strategies where intermediate state is reshaped.

4. **Property semantics at execution.**
   - `deterministic: true` ⇒ at L3, re-execution of the same input must produce byte-identical output (modulo non-canonical timestamp encodings). The planner may rely on this for retry-without-state-rewind.
   - `idempotent: true` ⇒ at L3, re-execution after partial write must converge to the same final state. The planner may resume from mid-failure.
   - `append_only: true` ⇒ at L2, the function (or model) is eligible for incremental-append strategy selection. Combined with `partition_column` from `incremental_models.md`, this enables the partition-DELETE+INSERT path on backends that support it.
   - `backends:` ⇒ at L2, restricts which engine the body may be lowered to. The declared set may only narrow the body's inferred set — see `functions.md` `BackendsWideningNotAllowed`.

5. **Properties are author-declared in v1.** The planner does not auto-derive `deterministic`, `idempotent`, `append_only`, `joins`, or `provenance` from body inspection. Auto-derivation would require a full lineage analyser; v1 ships explicit declarations and the validation rules below. Auto-derivation may be added later as a pure DX improvement; nothing in this spec forecloses it.

6. **Provenance validation rule.** When `unstable_schema: true` and `provenance:` is declared, the planner-validation pass (`smelt-db::provenance_validator::check_provenance`) compares each declared `(output_col, [source_cols])` tuple against the body's outermost SELECT:
   - If `output_col` aliases a SELECT item, the source columns of that item must equal the declared `source_cols` set. Any column declared but not read, or read but not declared, emits `ProvenanceMismatch`.
   - If `output_col` does not alias any SELECT item, `ProvenanceMismatch` is emitted.

7. **Joins validation rule.** When `unstable_schema: true` and `joins:` is declared, each entry's `table` must appear as a join alias in the body's outermost FROM. Otherwise `JoinsMismatch` is emitted. The `cardinality` field is **not** verified against data — instead, every declaration with a non-empty `cardinality` triggers `DeclaredCardinalityUnverifiable` at Warning severity, telling the author that the planner trusts but does not verify the claim.

8. **Cardinality soundness caveat.** `EliminateUnusedLeftJoin` may elide a `LeftJoin` only when its `cardinality == OneToOne` *and* none of the join's RHS-only output columns appear in the parent projection. The planner trusts the declaration; if the author misdeclares, the optimisation is unsound. This is the only known soundness gap in v1 and is encoded in the warning above.

   **Cardinality string→enum mapping (normative, fail-safe).** The `cardinality:` frontmatter field is a raw string; its mapping into the `Cardinality` enum that gates the rewrite is exact and fail-safe:
   - The exact spelling `1:1` maps to `OneToOne` — the *only* value that enables `EliminateUnusedLeftJoin`.
   - **Any other string** (including `1:N`, `N:1`, `N:M`, mixed case like `1:1 `, alternate spellings like `one_to_one`, or an unrecognised token) maps to a **non-`OneToOne`** value and therefore **never** enables the elision. The default on an unrecognised string is fail-safe: when in doubt, do not elide. There is no error for an unrecognised cardinality string — it simply does not unlock the soundness-bearing rewrite (the join is kept).

9. **Pushdown advisory rule.** When a transparent function is called from a SELECT that has a WHERE clause and the callee lacks declared `provenance:`, the planner emits `MissingProvenancePushdownAdvisory` at the WHERE clause range. The advisory is informational — it does not block compilation. Adding `provenance:` enables `PushFilterIntoTransparentFunction` to fire.

10. **Fixed-point loop semantics.** `apply_rules_to_fixed_point` runs the rule list in order; if any rule returns `Changed`, the loop restarts from the beginning. Termination is guaranteed by:
    - The acyclic transparent-function call graph (`functions.md` rule 3 — no recursion).
    - A per-pass `visited: HashSet<FnId>` in `ExpandTransparentFunctionCalls` that suppresses bodies of in-flight expansions.
    - Each rule being individually monotone over a finite plan size.

11. **Rule purity.** Planner rules must be pure functions of the input plan and the `RuleContext` registry lookups. No I/O, no Salsa calls, no mutation of shared state. The `RuleContext` is the only side channel and is read-only.

### Interactions with adjacent specs

- **Models-as-functions equivalence** (`architecture.md`): a model body is itself a transparent function from this spec's perspective. The planner consumes models and `smelt.define`s by the same rules.
- **Frontmatter grammar and key catalogue** (`functions.md`): the keys consumed here are defined there. Planner-relevant diagnostics from frontmatter parsing (`UnstableSchemaRequired`, `BackendsWideningNotAllowed`, `FrontmatterParseError`) are emitted by the parsing layer; this spec covers only the four codes in the Surface table.
- **Incremental strategies** (`incremental_models.md`): L2 strategy selection reads `materialization`, `incremental.enabled`, `partition_column`, etc. from model frontmatter. This spec specifies the planner's *consumption*; the configuration surface lives there. See `incremental_shapes.md` §"Functions inside partition-grain bodies" for how transparent-function expansion (via `ExpandTransparentFunctionCalls`) composes with the framework's per-model WHERE injection and batch-safety classification.
- **Expansion mechanics** (`expansion.md`): how `ExpandTransparentFunctionCalls` substitutes argument expressions and attaches `ProvenanceTag` is an internal invariant. This spec assumes correct expansion; it does not respec it.

## Design

This section captures the load-bearing rationale behind the three-level pipeline, the transparency-vs-materialization split, and the property-declaration discipline above. Where deeper justification exists, it lives in `docs/research/20260413-smelt-functions.md` §12 and §16 #22, and is cross-linked. The crate-boundary and "value-producing planner" rationale lives in `architecture.md`'s Design section; this spec covers only the rationale specific to planner integration.

**Three levels with distinct trust contracts, not one fixed-point pass.** L1 (logical→logical), L2 (logical→physical), L3 (physical→execution) each have a different contract on what the rule is allowed to change: L1 must preserve the user-visible row set, L2 picks an execution strategy under a fixed output schema, L3 lowers a strategy to runtime steps. Collapsing them onto one stage was rejected because a single rule would have to reason across all three abstractions at once — a filter-pushdown rule and a multi-statement-lowering rule would share a trait and a return type, making it harder to author either one and harder for `--show-plan` to render either one usefully. A two-stage variant (collapse L1+L2) was rejected for the same reason at smaller scale: strategy selection reads frontmatter that L1 rules never need, and ordering becomes implicit. The MLIR-style progressive-lowering framing makes the three-tier structure observable in the rule list rather than hidden inside one giant rule (research §12). See `architecture.md` "CSTs are not mutated" for the value-producing rule that all three levels share.

**The optimization boundary is transparency, not materialization.** Whether a body is reachable to planner rewrites is decided by `transparent: true/false` on `LogicalNode::FunctionCall`, not by whether the result is persisted. A `materialization: table` model is still transparent — the planner sees through it just like an ephemeral one. The alternative ("treat persisted bodies as black-box; only ephemeral bodies are transparent") was rejected because materialization is a deployment choice (how big is the data, how is it scheduled, what does the runtime cache) — not a correctness property of the body. Conflating the two would force users wanting view-materialized output to lose planner visibility, and ephemeral users would gain optimization only as a side effect of the storage choice. Keeping the two axes orthogonal lets `incremental_models.md` evolve materialization independently of this spec, and matches the architecture-level invariant in `architecture.md` "Materialization is orthogonal to transparency."

**Properties are author-declared in v1, not auto-derived.** `deterministic`, `idempotent`, `append_only`, `joins`, and `provenance` are declared in frontmatter (research §16 #22) rather than inferred by walking the body. Auto-derivation requires a full lineage analyser the compiler does not yet have; a wrong auto-derivation would silently widen rewrite eligibility — e.g., a body that calls `now()` would be wrongly tagged `deterministic: true` and the planner would replay it across retries. Conservative explicit declaration is honest: the user states the contract, the planner trusts it, and the validators surface mismatches between declaration and body where they can be checked structurally (`ProvenanceMismatch`, `JoinsMismatch`). Auto-derivation may land later as a pure DX win without breaking this spec — declared values would simply override inferred ones, the same shape as today's `backends:` narrowing.

**`joins:` and `provenance:` gated behind `unstable_schema`.** The structured-map surface for these keys is still being prototyped against real planner-rule pressure (currently only `EliminateUnusedLeftJoin` and `PushFilterIntoTransparentFunction` consume them). Locking the syntax in before the second consumer arrives risks shipping a one-way door. Gating behind `smelt.yml: unstable_schema: true` lets eager users wire up the optimisations now while signalling clearly that the surface may shift; the simple-boolean keys (`deterministic`, `idempotent`, `append_only`, `backends`) are stable and ungated. Once `planner_integration` stabilises the structured shape, the gate is dropped without breaking existing usage (research §12, §16 #22). See `functions.md` Design "`joins:` and `provenance:` gated behind `unstable_schema`."

**`MissingProvenancePushdownAdvisory` is a hint, not a warning.** Filter pushdown into a transparent body requires a declared `provenance:` map; without it, the filter sits at the call site and the body runs without the predicate. This is a missed optimisation, not a correctness bug — the function still computes the right result, just slower. Surfacing it as a hint tells authors "you can opt in" without nagging users who deliberately skipped the declaration. Promoting it to a warning was rejected because users who don't care about the optimisation would have to suppress noise on every `WHERE`-bearing call site. The asymmetry — declaring `provenance:` enables a rewrite, omitting it disables one — is captured in the property-to-effect monotonicity invariant (Constraints & Invariants 6).

**`DeclaredCardinalityUnverifiable` is a warning because cardinality is empirical.** The planner has no statistics, no profile data, and no runtime probe — it cannot verify a `cardinality: 1:1` claim against actual data. `EliminateUnusedLeftJoin` trusts the declaration; if the author misdeclares, the rewrite is unsound. Requiring runtime verification (probe row counts during build, fail if cardinality is violated) was rejected because it runs at the wrong layer (build vs execution) and shifts the cost model from "type-check-time" to "data-scan-time" — fundamentally changing what `smelt build` does. The honest middle ground is to warn at every declaration, telling the author "the planner trusts but does not verify this claim." Authors who want stricter checks can layer their own tests; authors who want the optimisation accept the trust contract.

**`--show-plan` is L1-only in v1.** Only L1 rules ship today, so `show_plan_rules()` is also the v1 production rule list — there is no hidden lowering for the printer to skip, and no per-level filter is needed. Designing a `--level` selector or a printed annotation now would commit the CLI to a surface decision before L2 and L3 rules exist to validate the shape. Rejected alternative: ship `--show-plan --level=L1|L2|L3` now with stub L2/L3 outputs. That bakes the surface in before the rules teach us what it should be (e.g., maybe L2 wants per-strategy diff output rather than per-level). Deferring the surface decision to when L2/L3 land is the same throw-away discipline `architecture.md` codifies for design under uncertainty.

**Single-engine planning in v1; cross-engine deferred.** L3's framework intent includes cross-engine choreography ("write Parquet on Spark, read on DuckDB"), but no rule implements it. Cross-engine execution requires a coordination layer — a runtime that can issue statements to two backends, block on the first, and feed its output to the second — which doesn't exist today. Shipping an L3 rule that emitted multi-engine plans the executor couldn't run would be a fiction. Constraining v1 to single-engine plans keeps the surface honest; the L3 rule contract in Semantics is what cross-engine rules must honour when they land, not what they do today. The spec's framework intent is preserved here so future plans can land cross-engine support without re-deriving the contract.

## Constraints & Invariants

1. **Black-box opacity is absolute.** No planner rule may inspect, splice, or rewrite the body of a node with `transparent: false`. The body field on `FunctionCall` is `None` for opaque calls by construction.
2. **Pure rules.** Every `impl PlannerRule` must be `Send + Sync`, and `apply` must be a pure function of `(plan, ctx)`. No async, no I/O, no Salsa.
3. **`format_plan` is deterministic.** Two equal `Plan` values render to byte-identical strings. The renderer must not include `HashMap` iteration, `Instant`, or pointer addresses. Test fixtures grep on output; non-determinism is a test break.
4. **`--show-plan` is read-only.** The flag must not write to disk, contact a backend, or mutate workspace state. Property-testable: running `--show-plan` twice must yield identical output and identical filesystem state.
5. **Cycle pre-pass is upstream of the planner.** `smelt-db` runs the workspace-wide call-graph cycle check (`FunctionCallCycle`) before constructing any logical plan that the planner sees. Plans containing cycle-tainted `fn_id`s reach the planner with `body: None`, suppressing splicing.
6. **Property-to-effect monotonicity.** Setting a property to its non-default value must not block any optimisation that was available at the default. The planner may *enable* additional rewrites in response to declared properties; it must never disable an existing one. (This is what makes `MissingProvenancePushdownAdvisory` an advisory rather than an error — adding `provenance:` is purely additive.)
7. **Out of scope for v1** (intent — preserved here so future plans honour it):
   - Cost-based rule selection. Rules are currently deterministic detectors; statistics input is future work.
   - Auto-derivation of `provenance` / `joins` / `deterministic` / `idempotent` / `append_only`.
   - Cross-model L2 strategy fusion (treating two transparent models as one execution unit).
   - L3 cross-engine choreography rules — declared but not implemented.

## Known Divergences / Open Questions

The plan that produced this spec acknowledges the wired-vs-aspirational gap explicitly. The current state, as of `last_reviewed`:

- **L1 is the only level with shipped rules.** The four rules in `show_plan_rules()` (`PushFilterIntoTransparentFunction`, `ExpandTransparentFunctionCalls`, `ElideEmptySelectItemsSplices`, `EliminateUnusedLeftJoin`) all live at L1. **L2 and L3 are framework intent, not implemented rule code.** The materialization-strategy selection and multi-statement lowering described above describe how those levels are *meant* to operate; the rule trait, fixed-point loop, and `RuleContext` are reusable, but no L2/L3 rule has been written yet. Treat the L2/L3 sections of Surface and Semantics as the contract any future rule must honour, not as a description of code that runs today.
- **`--show-plan` only invokes L1 rules.** Until L2/L3 rules exist, `smelt build --show-plan` is faithful to the full optimised plan: there is no hidden lowering that the printer skips. When L2/L3 rules ship, `--show-plan` may need a `--level` selector or a printed annotation; that surface decision is open.
- **`provenance:` and `joins:` parsing is partially landed.** The keys parse, the `unstable_schema` gate is enforced, and `FunctionProperties` carries the values into the planner. The `ProvenanceMismatch` / `JoinsMismatch` / `DeclaredCardinalityUnverifiable` validation pass landed in Phase 51 of `docs/plans/20260422-smelt-functions.md`; the `MissingProvenancePushdownAdvisory` hint landed in Phase 52. Behaviour upstream of those phases (e.g., on older branches) does not match this spec.
- **End-to-end `smelt build` does not yet pass L1-optimised plans to the executor.** `--show-plan` proves the rules run; the production build path's integration with rule-optimised plans is in progress (Phases 56–57 of the smelt-functions plan). Today, the executor consumes a SQL string emitted from the CST, not from the optimised logical plan.
- **`backends:` narrowing is enforced at parse time, not at L2 strategy selection.** The narrow-only check (`BackendsWideningNotAllowed`) is wired in `smelt-db::backends`. The downstream consumption — refusing to lower a body to a backend not in its declared set at L2 — has no code path yet because L2 strategy selection has no code path yet.
- **No L1 rule fires on bare model `SELECT`s today.** The four L1 rules pattern-match on `FunctionCall { transparent: true, .. }` (i.e., `smelt.<path>(...)` call sites that resolve to a `smelt.define`). Models, despite being conceptually transparent functions per `architecture.md`, are constructed as `Select { from: TableRef { ... } }` in the current logical-plan builder rather than as `FunctionCall` nodes — and the body's `smelt.<path>` references resolve through the dependency graph, not through a function-call node. Aligning the model construction path with the unified-model framing (so a `smelt.<path>` reference resolving to a model is also a `FunctionCall { transparent: true }` node with `body: Some(_)`) is open work; the spec fixes the *intent* (planner reasons across model boundaries) without claiming that intent is fully wired.
- **User-authored planner-rule API — pre-spec.** Today, only built-in rules ship (the four L1 rules in `show_plan_rules()`). The `Rule` trait and `RuleContext` are reusable, but the surface for a **user-authored** rule — registration, lifecycle, stability guarantees, the `RuleContext` extension surface, error handling and validation hooks — is not specified. The `README.md` / `CLAUDE.md` differentiator "engineer controls planning" describes intent; the working design lives at `docs/planner_rule_api_design.md` and predates the 2026-05-01 universal-addressing rework, so it needs review before becoming normative. A future `planner_api.md` spec is in scope (see `architecture.md` §"Specs not yet authored").
- **Diagnostic code ownership.** This spec owns the *semantics* of the diagnostic codes it lists — when each fires and what it anchors to. [`diagnostics.md`](diagnostics.md) is the cross-feature catalogue that indexes every code's severity and canonical trigger; the two must agree, with the owning feature spec governing semantics and `diagnostics.md` governing the catalogue row. The four planner-validation codes (`ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable`, `MissingProvenancePushdownAdvisory`) are catalogued there under their owning feature spec.

## References

- **Code**:
  - `crates/smelt-logical/src/logical.rs` — `LogicalNode`, `FunctionProperties`, `Provenance`, `JoinSpec`, `Cardinality`, `ProvenanceTag`, `parse_function_properties` (the logical model lives in `smelt-logical`; `smelt-planner` re-exports it — see architecture.md §"Constraints & Invariants" (Layered single-ownership))
  - `crates/smelt-logical/src/rules/rule_diagnostics.rs` — `RuleContext`, `detect_builtin_rules` (the pure rule-data interface, in `smelt-logical`)
  - `crates/smelt-planner/src/logical_plan_rules.rs` — `PlannerRule` trait, `apply_rules_to_fixed_point`, `show_plan_rules`, the four L1 rules (rule *application* stays in `smelt-planner`)
  - `crates/smelt-planner/src/plan_printer.rs` — `format_plan` (deterministic renderer used by `--show-plan`)
  - `crates/smelt-planner/src/rules/` — graph-level (model-graph) rules; distinct from logical-plan rules above
  - `crates/smelt-logical/src/lowering/` — lowering helpers (`as_struct_to_sql`, etc.)
  - `crates/smelt-logical/src/analysis/temporal.rs` — temporal-dependency analysis feeding incremental strategy selection
  - `crates/smelt-db/src/lib.rs` — `logical_plan` Salsa query, `DiagnosticCode::{ProvenanceMismatch, JoinsMismatch, DeclaredCardinalityUnverifiable, MissingProvenancePushdownAdvisory}`, the missing-provenance-pushdown emission site
  - `crates/smelt-db/src/provenance_validator.rs` — pure validators for the four diagnostic codes
  - `crates/smelt-db/src/backends.rs` — `infer_body_backends`, `apply_narrow_rule`
  - `crates/smelt-cli/src/main.rs` — `--show-plan` CLI argument
  - `crates/smelt-cli/src/commands/build.rs` — `show_plan` driver (gather workspace, build plan, run rules, print)
- **Tests**:
  - `crates/smelt-cli/tests/show_plan.rs` — end-to-end `smelt build --show-plan` integration tests
  - `crates/smelt-planner/tests/logical_plan_tests.rs` — logical-plan construction tests
  - `crates/smelt-planner/tests/logical_plan_rule_tests.rs` — per-rule unit tests
  - `crates/smelt-planner/tests/pushdown_tests.rs` — combined-rule fixed-point ordering test
  - `crates/smelt-planner/tests/join_elimination_tests.rs` — `EliminateUnusedLeftJoin` cases
  - `crates/smelt-planner/tests/phase41_body_splice_tests.rs` — body-splicing and provenance-tag tests
- **User docs**:
  - `docs-site/docs/concepts/planner.md` (and adjacent rule pages) — to be reconciled with this spec via `/smelt:validate planner_integration`
- **Plans (history)**:
  - `docs/plans/20260422-smelt-functions.md` — Phases 30–52 land the logical-plan IR, the four L1 rules, frontmatter-property parsing, the validators, and `--show-plan`
  - `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file
- **Related specs**:
  - `docs/specs/architecture.md` — the planner's place in the compilation pipeline; transparent-vs-black-box at the system level; models-as-functions
  - `docs/specs/functions.md` — `smelt.define` / `smelt.extern` / frontmatter key catalogue
  - `docs/specs/incremental_models.md` — model-frontmatter keys consumed by L2 strategy selection
  - `docs/specs/types.md` — type vocabulary referenced by `FunctionProperties`-adjacent fields
  - `docs/specs/expansion.md` — internal invariants for AST expansion

### Research

- `docs/research/20260413-smelt-functions.md` §12 (Three-level planner integration), §16 #22 (unified frontmatter — the property surface this spec consumes), §13 (canonical built-in registry — relevant for `backends:` semantics)
