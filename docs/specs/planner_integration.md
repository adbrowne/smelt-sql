---
feature: planner_integration
status: experimental
last_reviewed: 2026-04-29
owners: [andrew]
---

# Planner Integration

> **Scope.** Normative spec for how transparent functions, models, and frontmatter properties feed the smelt planner. Defines the three planner levels (L1 logical-to-logical, L2 logical-to-physical, L3 physical-to-execution-plan), the transparent vs black-box optimization boundary, the frontmatter properties the planner consumes, the validation rules the planner emits against declared metadata, and the `smelt build --show-plan` CLI surface. The architectural rule that the planner is value-producing and never mutates CSTs lives in `architecture.md`; the unified-model framing lives there too. The frontmatter keys' grammar lives in `functions.md`. This spec specifies the *consumption* of those keys — what the planner does with `deterministic`, `idempotent`, `append_only`, `backends`, `joins`, and `provenance`.

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

9. **Pushdown advisory rule.** When a transparent function is called from a SELECT that has a WHERE clause and the callee lacks declared `provenance:`, the planner emits `MissingProvenancePushdownAdvisory` at the WHERE clause range. The advisory is informational — it does not block compilation. Adding `provenance:` enables `PushFilterIntoTransparentFunction` to fire.

10. **Fixed-point loop semantics.** `apply_rules_to_fixed_point` runs the rule list in order; if any rule returns `Changed`, the loop restarts from the beginning. Termination is guaranteed by:
    - The acyclic transparent-function call graph (`functions.md` rule 3 — no recursion).
    - A per-pass `visited: HashSet<FnId>` in `ExpandTransparentFunctionCalls` that suppresses bodies of in-flight expansions.
    - Each rule being individually monotone over a finite plan size.

11. **Rule purity.** Planner rules must be pure functions of the input plan and the `RuleContext` registry lookups. No I/O, no Salsa calls, no mutation of shared state. The `RuleContext` is the only side channel and is read-only.

### Interactions with adjacent specs

- **Models-as-functions equivalence** (`architecture.md`): a model body is itself a transparent function from this spec's perspective. The planner consumes models and `smelt.define`s by the same rules.
- **Frontmatter grammar and key catalogue** (`functions.md`): the keys consumed here are defined there. Planner-relevant diagnostics from frontmatter parsing (`UnstableSchemaRequired`, `BackendsWideningNotAllowed`, `FrontmatterParseError`) are emitted by the parsing layer; this spec covers only the four codes in the Surface table.
- **Incremental strategies** (`incremental_models.md`): L2 strategy selection reads `materialization`, `incremental.enabled`, `partition_column`, etc. from model frontmatter. This spec specifies the planner's *consumption*; the configuration surface lives there.
- **Expansion mechanics** (`expansion.md`, when written): how `ExpandTransparentFunctionCalls` substitutes argument expressions and attaches `ProvenanceTag` is an internal invariant. This spec assumes correct expansion; it does not respec it.

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
- **`joins:` cardinality is a raw string.** The `JoinSpec.cardinality` field stores the raw string from frontmatter (`"1:1"`, `"1:N"`); mapping into the structured `Cardinality` enum is deferred. `EliminateUnusedLeftJoin` operates on `Cardinality` values produced by upstream logical-plan construction, not on `JoinSpec.cardinality` directly. The mapping rule is unspecified and is an open question — likely "exact string match" but no normative claim yet.
- **End-to-end `smelt build` does not yet pass L1-optimised plans to the executor.** `--show-plan` proves the rules run; the production build path's integration with rule-optimised plans is in progress (Phases 56–57 of the smelt-functions plan). Today, the executor consumes a SQL string emitted from the CST, not from the optimised logical plan.
- **`backends:` narrowing is enforced at parse time, not at L2 strategy selection.** The narrow-only check (`BackendsWideningNotAllowed`) is wired in `smelt-db::backends`. The downstream consumption — refusing to lower a body to a backend not in its declared set at L2 — has no code path yet because L2 strategy selection has no code path yet.
- **No L1 rule fires on bare model `SELECT`s today.** The four L1 rules pattern-match on `FunctionCall { transparent: true, .. }` (i.e., `smelt.fn.*` call sites). Models, despite being conceptually transparent functions per `architecture.md`, are constructed as `Select { from: TableRef { ... } }` in the current logical-plan builder rather than as `FunctionCall` nodes. Aligning the model construction path with the unified-model framing is open work; the spec fixes the *intent* (planner reasons across model boundaries) without claiming that intent is fully wired.

## References

### Code

- `crates/smelt-planner/src/logical.rs` — `LogicalNode`, `FunctionProperties`, `Provenance`, `JoinSpec`, `Cardinality`, `ProvenanceTag`, `parse_function_properties`
- `crates/smelt-planner/src/logical_plan_rules.rs` — `PlannerRule` trait, `RuleContext`, `apply_rules_to_fixed_point`, `show_plan_rules`, the four L1 rules
- `crates/smelt-planner/src/plan_printer.rs` — `format_plan` (deterministic renderer used by `--show-plan`)
- `crates/smelt-planner/src/rules/` — graph-level (model-graph) rules; distinct from logical-plan rules above
- `crates/smelt-planner/src/lowering/` — Phase 42 lowering helpers (`as_struct_to_sql`, etc.)
- `crates/smelt-planner/src/analysis/temporal.rs` — temporal-dependency analysis feeding incremental strategy selection
- `crates/smelt-db/src/lib.rs` — `logical_plan` Salsa query, `DiagnosticCode::{ProvenanceMismatch, JoinsMismatch, DeclaredCardinalityUnverifiable, MissingProvenancePushdownAdvisory}`, the missing-provenance-pushdown emission site
- `crates/smelt-db/src/provenance_validator.rs` — pure validators for the four diagnostic codes
- `crates/smelt-db/src/backends.rs` — `infer_body_backends`, `apply_narrow_rule`
- `crates/smelt-cli/src/main.rs` — `--show-plan` CLI argument
- `crates/smelt-cli/src/commands/build.rs` — `show_plan` driver (gather workspace, build plan, run rules, print)

### Tests

- `crates/smelt-cli/tests/show_plan.rs` — end-to-end `smelt build --show-plan` integration tests
- `crates/smelt-planner/tests/logical_plan_tests.rs` — logical-plan construction tests
- `crates/smelt-planner/tests/logical_plan_rule_tests.rs` — per-rule unit tests
- `crates/smelt-planner/tests/pushdown_tests.rs` — combined-rule fixed-point ordering test
- `crates/smelt-planner/tests/join_elimination_tests.rs` — `EliminateUnusedLeftJoin` cases
- `crates/smelt-planner/tests/phase41_body_splice_tests.rs` — body-splicing and provenance-tag tests

### User docs

- `docs-site/docs/concepts/planner.md` (and adjacent rule pages) — to be reconciled with this spec via `/smelt:validate planner_integration`

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — Phases 30–52 land the logical-plan IR, the four L1 rules, frontmatter-property parsing, the validators, and `--show-plan`
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/architecture.md` — the planner's place in the compilation pipeline; transparent-vs-black-box at the system level; models-as-functions
- `docs/specs/functions.md` — `smelt.define` / `smelt.extern` / frontmatter key catalogue
- `docs/specs/incremental_models.md` — model-frontmatter keys consumed by L2 strategy selection
- `docs/specs/types.md` — type vocabulary referenced by `FunctionProperties`-adjacent fields
- `docs/specs/expansion.md` — internal invariants for AST expansion (when written)

### Research

- `docs/research/20260413-smelt-functions.md` §12 (Three-level planner integration), §16 #22 (unified frontmatter — the property surface this spec consumes), §13 (canonical built-in registry — relevant for `backends:` semantics)
