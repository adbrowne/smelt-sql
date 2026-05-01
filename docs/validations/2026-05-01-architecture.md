## Drift Report: architecture

**Spec**: `docs/specs/architecture.md` (last_reviewed: 2026-04-29)
**Date**: 2026-05-01

### Automated checks
- `cargo fmt --all -- --check` — **PASS**
- `cargo clippy --all-targets` — **PASS** (no warnings)
- `cargo test` — **PASS** (all binary suites green; exit 0)
- `cargo test -p smelt-cli --test example_diagnostics` — **PASS**

### Surface drift

- ✅ Compilation pipeline stages (Parse / Analyze / Plan / Generate / Execute) and their producer crates match. All listed crates exist under `crates/`.
- ✅ Spec-listed Salsa queries exist as functions in `crates/smelt-db/src/lib.rs`: `parse_file` (510), `model_refs` (540), `resolve_ref` (2474), `file_diagnostics` (2507), `model_schema` (2954), `resolved_model_schema` (3924).
- ✅ `BackendCapabilities` and `SqlDialect` exported from `crates/smelt-dialect/src/lib.rs:10`.
- ✅ Cross-dialect function-name normalization (`EXPLODE`→`UNNEST`, `EVERY`→`BOOL_AND`) implemented at `crates/smelt-dialect/src/printer.rs:429-448`.
- ✅ `ExecutionResult` exported by `crates/smelt-backend/src/lib.rs:14`.
- ❌ **`Transformation` enum diverges from spec.** Spec lines 176-191 vs `crates/smelt-planner/src/types.rs:9-46`:
  - Spec: `CreateModel { name, sql, materialization }` — code: `CreateNode { name, sql, dependencies, origin, materialization }` (different variant name; extra `dependencies`/`origin`)
  - Spec: `RemoveModel { name }` — code: `RemoveNode { model }`
  - Spec: `RedirectRef { model, old_ref, new_ref }` — code: `RedirectRef { from, to }` (totally different fields)
  - Code has `SetIncremental { model, event_time_column, partition_column, granularity }` not mentioned in the spec snippet
  - `SetMaterialization` and `ReplaceWithPlan` match.
- ❌ **`smelt-cli` labelled `sync (entry point)` but is async.** Spec table line 47 vs `crates/smelt-cli/src/main.rs:398-399` (`#[tokio::main] async fn main`) and `crates/smelt-cli/Cargo.toml:32` (`tokio.workspace = true`). The CLI legitimately needs to drive async backends — likely the spec label is wrong, not the code.
- ⚠️ **docs-site uses the legacy `smelt.ref()`/`smelt.source()` addressing scheme.** Spec Surface §"Resolution" makes `smelt.<path>` the universal addressing scheme, but every example in `docs-site/` uses the legacy syntax:
  - `docs-site/docs/concepts/how-it-works.md:5,15,28,59,98,111,133,136`
  - `docs-site/docs/concepts/project-structure.md:77,96,128`
  - `docs-site/docs/developing/architecture.md:46,131,226,242,244,247`

  This is acknowledged in spec Known Divergences §1 ("pre-implementation"). Flagged as ⚠️ rather than ❌ because the divergence is declared, but docs-site still teaches the legacy form to readers as if it's current. A docs-only refresh once the path migration lands is the right follow-up.
- ⚠️ Examples under `examples/` use legacy `smelt.ref()`/`smelt.source()`/`smelt.fn.<path>` — same status as above; spec explicitly notes "examples will be migrated as part of the implementation plan".

### Semantics drift

- ✅ **Identity property #1 (pg_query anchor)** — test infrastructure exists at `crates/smelt-parser-compat/tests/parse_equivalence.rs` (proptests for both directions of equivalence).
- ✅ **Identity property #2 (DuckDB byte-identity)** — covered by `crates/smelt-dialect/tests/snapshots.rs:40+` (`duckdb_identity_simple`, `duckdb_identity_complex`, `duckdb_identity_with_cte`).
- ✅ **Salsa purity rule** — `crates/smelt-db/src/type_inference.rs` and `schema.rs` contain no Salsa imports or `db.*_query()` calls (verified by grep). Pure functions take AST + plain data.
- ✅ **Stage 4 (Generate is single-pass)** — covered structurally by the printer's recursive walk and the byte-identity tests above.
- ⚠️ **Stage rules 1–3 (Parse total, Analyze incremental, Plan additive)** — no single test exercises these as named invariants. They are upheld by the architecture (CST never mutated, Salsa drives Analyze, planner returns `Vec<Transformation>` values), but no test explicitly asserts "Parse is total on invalid input" or "Plan does not mutate existing CSTs". Flag for a sentinel test.
- ⚠️ **Planner scope (in-scope vs out-of-scope rule list)** — there is no test that prevents a future PR from adding a predicate-pushdown rule to `smelt-planner`. The boundary is upheld by convention only.

### Invariant drift

1. ✅ **Rowan CST is the single representation** — no `LogicalPlan` analogue; planner operates on CST.
2. ✅ **`smelt-db` analysis logic is pure** — verified above for `type_inference.rs`/`schema.rs`.
3. ✅ **CSTs are not mutated** — planner output is `Transformation` values (`crates/smelt-planner/src/types.rs:9-46`).
4. ❌ **Sync core, async edges** — held for `smelt-types`, `smelt-parser`, `smelt-core`, `smelt-db`, `smelt-dialect`, `smelt-planner` (no `tokio` / `async-trait` deps; no `async fn`). **Violated by `smelt-cli`** (see Surface drift). Either spec table or CLI must change to reconcile.
5. ✅ **`smelt-dialect` is lightweight** — `crates/smelt-dialect/Cargo.toml` declares only `smelt-parser` + `smelt-types` deps; no Arrow / Tokio / DuckDB.
6. ⚠️ **No circular crate dependencies** — not directly verified beyond `cargo build` succeeding; would need `cargo-depgraph` or equivalent for proof.
7. ⚠️ **Parser produces usable CST on invalid input** — covered indirectly by parse_equivalence tests and pervasive use in LSP, but no explicit "parser doesn't panic on a corpus of malformed inputs" test.

### Freshness
- last_reviewed: **2026-04-29**
- most recent code change to spec's Reference → Code paths: **2026-04-27** (`git log` against the listed crates returns no commits since 2026-04-29).
- **Verdict: fresh.** No code drift introduced after the review date.

### Summary
- Drift items: **9 total** — 4 surface (2 ❌ + 2 ⚠️), 2 semantics (⚠️), 3 invariants (1 ❌ + 2 ⚠️).
- Material drift requiring action:
  1. `Transformation` enum names/fields in spec do not match `smelt-planner` (❌ surface).
  2. `smelt-cli` listed as `sync` but is `async` (❌ surface + invariant #4).
  3. docs-site addressing scheme is the legacy form (acknowledged ⚠️, but still misleads readers).

### Recommended next step
- **`/smelt:plan architecture`** with a tight scope: (a) reconcile the `Transformation` enum — either update the spec to reflect `CreateNode`/`RemoveNode`/`SetIncremental`/`from`+`to` field names, or rename the code to match the spec; (b) decide whether `smelt-cli` is `async (entry point)` (update spec) or `sync` (untangle the CLI from `tokio::main`). The docs-site refresh is naturally bundled with the `smelt.<path>` migration plan once that lands — it does not need its own plan today.
