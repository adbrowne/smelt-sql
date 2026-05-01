## Drift Report: architecture (post-fix)

**Spec**: `docs/specs/architecture.md` (last_reviewed: 2026-05-01)
**Date**: 2026-05-01
**Pairs with**: [`2026-05-01-architecture.md`](./2026-05-01-architecture.md) — the report that motivated commit `8581111`.

### Automated checks
- `cargo fmt --all -- --check` — **PASS**
- `cargo clippy --all-targets` — **PASS** (no warnings)
- `cargo test` — **PASS** (exit 0)
- `cargo test -p smelt-cli --test example_diagnostics` — **PASS**

### Surface drift
- ✅ Compilation pipeline stages and producer crates match (`crates/` tree).
- ✅ Spec-listed Salsa queries exist as functions in `crates/smelt-db/src/lib.rs`: `parse_file` (510), `model_refs` (540), `resolve_ref` (2474), `file_diagnostics` (2507), `model_schema` (2954), `resolved_model_schema` (3924).
- ✅ `BackendCapabilities` and `SqlDialect` exported from `crates/smelt-dialect/src/lib.rs:10`.
- ✅ Cross-dialect function-name normalization (`EXPLODE`→`UNNEST`, `EVERY`→`BOOL_AND`) at `crates/smelt-dialect/src/printer.rs:429-448`.
- ✅ `ExecutionResult` exported by `crates/smelt-backend/src/lib.rs:14`.
- ✅ **`Transformation` enum now matches code.** Spec lines 177-189 list `ReplaceWithPlan`/`SetIncremental`/`SetMaterialization`/`CreateNode`/`RemoveNode`/`RedirectRef{from,to}` — identical to `crates/smelt-planner/src/types.rs:9-46`.
- ✅ **`smelt-cli` correctly labelled `async (entry point)`** — spec table line 47 vs `crates/smelt-cli/src/main.rs:398-399` (`#[tokio::main] async fn main`).
- ⚠️ **docs-site uses legacy `smelt.ref()`/`smelt.source()` addressing** (acknowledged in Known Divergences §1, "pre-implementation"). Locations: `docs-site/docs/concepts/{how-it-works,project-structure}.md` and `docs-site/docs/developing/architecture.md`. Will be cleared by the future `smelt.<path>` migration plan.
- ⚠️ Examples under `examples/` use legacy syntax — same status as above; spec explicitly notes "examples will be migrated as part of the implementation plan".

### Semantics drift
- ✅ **Identity property #1 (pg_query anchor)** — proptest infrastructure at `crates/smelt-parser-compat/tests/parse_equivalence.rs`.
- ✅ **Identity property #2 (DuckDB byte-identity)** — `crates/smelt-dialect/tests/snapshots.rs:40+` (`duckdb_identity_simple`/`_complex`/`_with_cte`).
- ✅ **Salsa purity rule** — `crates/smelt-db/src/{type_inference.rs,schema.rs}` are Salsa-free pure functions.
- ✅ **Generate is single-pass** — covered structurally by the dialect printer + byte-identity tests.
- ⚠️ **Stage rules 1–3 (Parse total, Analyze incremental, Plan additive)** — no sentinel test asserts these as named invariants; upheld by architecture but not enforced. Pre-existing flag.
- ⚠️ **Planner scope (in-scope vs out-of-scope rule list)** — no test prevents a future PR from adding predicate-pushdown to `smelt-planner`. Convention-only. Pre-existing flag.

### Invariant drift
1. ✅ **Rowan CST is the single representation** — no `LogicalPlan` analogue; planner operates on CST.
2. ✅ **`smelt-db` analysis logic is pure** — verified in `type_inference.rs`/`schema.rs`.
3. ✅ **CSTs are not mutated** — planner output is `Transformation` values.
4. ✅ **Sync core, async edges** — sync crates (`smelt-{types,parser,core,db,dialect,planner}`) carry no `tokio`/`async-trait` deps; async lives at execution backends, LSP, UI, and the CLI entry point — and the spec now reflects that.
5. ✅ **`smelt-dialect` is lightweight** — `crates/smelt-dialect/Cargo.toml` declares only `smelt-parser` + `smelt-types`.
6. ⚠️ **No circular crate dependencies** — not directly verified beyond `cargo build` succeeding; would need `cargo-depgraph` for proof. Pre-existing flag.
7. ⚠️ **Parser produces usable CST on invalid input** — covered indirectly by parse_equivalence tests; no explicit "no-panic on a malformed-input corpus" test. Pre-existing flag.

### Freshness
- last_reviewed: **2026-05-01**
- most recent code change to spec's Reference → Code paths: **2026-04-27** (no commits to those crates after spec review date).
- **Verdict: fresh.**

### Summary
- Drift items: **6 total** — 2 surface (⚠️ acknowledged), 2 semantics (⚠️ pre-existing test gaps), 2 invariants (⚠️ pre-existing inspection limits). **Zero ❌.**
- Material drift requiring action: **none.** All ❌ items from the previous report are resolved.

### Recommended next step
**None.** The spec and code are reconciled. Remaining ⚠️ flags are:
- The docs-site/examples migration to `smelt.<path>` — already declared as Known Divergence §1; will close out as part of that future implementation plan.
- Sentinel tests for stage rules and planner-scope guardrails — would harden the spec but are not drift; a small "spec-enforcement tests" plan could pick these up if/when they bite.
- `cargo-depgraph` and a malformed-input parser corpus — same story, hardening rather than reconciliation.
