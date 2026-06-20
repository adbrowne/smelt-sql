# Plan: Drive down cold Initial-Load CPU (path-resolution & per-file diagnostic overhead)

- **Spec:** [`docs/specs/architecture.md`](../specs/architecture.md) — "Salsa purity rule (analysis)", "Project isolation rule", "Workspace loading parity rule". This is a **behavior-preserving performance refactor**; it must not change any diagnostic, resolution, or schema output.
- **Spec diff:** none. No user-visible surface or semantics change.
- **Docs:** code-only.
- **Tracking commit (prerequisite, already landed):** `d28a26ba` — `project_sql_address_index` removed the O(N²) `resolve_ref_path` file scan (Initial Load ~850 → 663 ms locally).

## Why

A flamegraph of the 2000-model **Salsa / Initial Load** benchmark (warm `all_models` + `file_diagnostics` for every file) showed **~41–48 % of cold CPU self-time in `std::path`** (`compare_components`, `Components::next`, `strip_prefix`) — not type inference, not parsing. The first fix removed one source (per-ref file rescans). Two structural sources remain, plus a per-file diagnostic-dispatch tax:

1. **Repeated `files.sort_by(|a,b| a.path(db).cmp(b.path(db)))`** across many queries — three of them run **per file** during cold load, so the workspace file list is re-sorted N times → O(N² log N) `PathBuf` comparisons.
2. **`check_file_diagnostics` runs ~30–60 sub-passes per file**, many of them separate `#[salsa::tracked]` queries and several re-walking the same CST, even for plain SQL models where ~20 passes are guaranteed no-ops.
3. **Residual `PathBuf` comparison/hashing** in hot maps and `starts_with`/`strip_prefix` project-isolation checks.

`SourceFile` is already a cheap interned `Copy` salsa id (`Hash`/`Eq`), so the fixes are: sort/scan **once per workspace**, gate per-file passes behind a one-shot file classification, and key hot structures by `SourceFile` rather than `PathBuf`.

## Verification harness (every phase uses this)

```bash
export DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb
export LD_LIBRARY_PATH=$DUCKDB_LIB_DIR:$LD_LIBRARY_PATH

# Correctness gates (must stay green — these are the equivalence oracle):
cargo test -p smelt-db --test path_resolution
cargo test -p smelt-db                                   # (pre-existing fail: smoke_integer_div_integer — unrelated, ignore)
cargo test -p smelt-cli --test example_diagnostics
cargo test -p smelt-lsp --test example_workspaces
cargo clippy --all-targets

# Perf measurement (before/after each phase — committed bins):
cargo build -p smelt-bench --bin decompose_initial_load --bin profile_initial_load --release
./target/release/decompose_initial_load 8      # phase breakdown
./target/release/profile_initial_load 10       # best-of initial load

# Re-flamegraph to find the next hotspot (perf_event_paranoid must be ≤1):
flamegraph -c "record -F 497 --call-graph dwarf,16384 -g" -o initial_load.svg -- ./target/release/profile_initial_load 6
```

**Baseline to beat (post-`d28a26ba`, local):** Initial Load best ≈ 663 ms; `decompose` diag-beyond-type_context ≈ 537 ms.

## How to execute (subagent-driven)

Run each phase via `/smelt:implement`-style pairs: an **implementer** subagent does red-green TDD on the listed tests + the equivalence gates, then a **reviewer** subagent checks the phase against the invariants below before commit. One phase per commit/push. **Re-profile between phases** — phase order may change if a re-flamegraph reranks the hotspots. Do not batch phases; each must independently pass all correctness gates and show a non-regressing perf number.

**Invariants every reviewer must check:**
- Salsa purity: new analysis logic is pure; tracked queries only gather inputs + call pure code.
- Project isolation: any workspace-wide index/sort stays correctly project-scoped at the lookup boundary (cross-project tests in `example_workspaces` must stay green).
- No behavior change: diagnostic sets, resolution kinds, and schemas are byte-identical to pre-phase (the example gates + property tests are the oracle).
- Fail-loud discipline unchanged; no new `unwrap`/`expect`/`println!`.

---

## Phase 1 — `sorted_workspace_files`: hoist the per-file re-sorts  *(status: done — Initial Load 663 → 297 ms)*

**Highest-confidence win, lowest risk; mirrors the `workspace_function_signatures` / `project_sql_address_index` pattern.**

Add one workspace-keyed tracked query and route the per-file re-sorts through it:

```rust
#[salsa::tracked]
pub fn sorted_workspace_files(db, workspace: Workspace) -> Arc<Vec<SourceFile>> {
    let mut files = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    Arc::new(files)
}
```

Replace the `files.sort_by(path.cmp)` bodies at these sites with a call to it (filter post-sort where project-scoped):

| File:line | Function | Per-file? |
|---|---|---|
| `function_diagnostics.rs:172` | `function_body_diagnostics_for_file` | **yes (N×)** |
| `function_diagnostics.rs:1252` | `smelt_fn_call_diagnostics_for_file` | **yes (N×)** |
| `function_diagnostics.rs:994` | `missing_provenance_advisory_for_file` | yes (N×, unstable only) |
| `functions.rs:86` | `workspace_function_signatures` | once/ws |
| `functions.rs:182,223` | `resolve_function`, `resolve_function_path` | per-call; filter post-sort |
| `function_diagnostics.rs:43` | `workspace_function_diagnostics` | once/ws |
| `project.rs:605` | `generator_files` | once/ws |

Leave sites that sort by a **non-path key** (`schema.rs:1879/1911`, `project.rs:2361/2458/2509/2524` sort by name/ref_name or sort `ModelRefValue`/`EmittedModelDef`) untouched.

**TDD tests (write first, watch fail):**
- `sorted_workspace_files_is_sorted_and_memoized` — returns files in path order; two calls return the same `Arc` (pointer-eq) within a revision.
- Equivalence: existing `function_diagnostics` unit tests + `example_diagnostics`/`example_workspaces` stay green (the oracle).

**Reviewer checklist:** post-sort project filtering preserves `resolve_function`'s "first match" semantics; no site changed its output ordering; cross-project isolation intact.

**Commit:** `perf(smelt-db): share one sorted workspace-file list across diagnostic queries`

**Expected:** removes the bulk of the remaining `PathBuf::cmp` self-time; re-measure with `decompose`.

---

## Phase 2 — `file_classification` gate: skip no-op passes on plain SQL models  *(status: pending)*

~80 % of files are plain SQL models (no `smelt.define`/`extern`, not a generator, `unstable_schema:false`, no `smelt.config.*`/loader calls), yet pay ~20 per-file diagnostic passes (each a tracked-query call) that can only fire on the other shapes.

Add one cheap per-file query computed from the already-cached parse:

```rust
#[salsa::tracked]
pub fn file_classification(db, file: SourceFile) -> FileClassification
// { is_generator, has_defines, has_fn_calls, has_loader_calls, has_config_vars }
// unstable_schema comes from the existing project-level query.
```

Gate these `check_file_diagnostics` passes behind the relevant flag (lines approximate — re-locate at edit time):
- `has_defines` → the 9 function-definition passes (`lib.rs:~1477–1528`).
- `has_fn_calls` → `smelt_fn_call_diagnostics_for_file` (`~1536`).
- `is_generator` → generator W2–W4 + loader passes (`~1205–1290`).
- `has_loader_calls` → `loader_call_diagnostics_for_file` (`~1618`).
- `has_config_vars` → config-var pass (`~1602`).
- `unstable_schema` already gates provenance passes — keep.

**Critical correctness rule:** a gate may only skip a pass when the pass is *provably* a no-op for that classification. The reviewer must confirm each gated pass emits **zero** diagnostics for files lacking the flagged construct. The oracle: run `example_diagnostics`/`example_workspaces` and assert identical diagnostic sets; add a focused test that a plain SQL model with a deliberately-present `smelt.define` in a *sibling* file is unaffected.

**TDD tests:**
- `file_classification_flags_match_content` — table of small files → expected flags.
- `gating_preserves_diagnostics` — for each gated pass, a positive fixture (flag true → diagnostic still emitted) and a negative fixture (flag false → identical output to ungated run).

**Reviewer checklist:** no gate hides a diagnostic; classification is one Salsa read (no extra parse); purity preserved.

**Commit:** `perf(smelt-db): classify files once and skip inapplicable diagnostic passes`

**Expected (agent estimate):** ~5–15 % on plain-SQL-heavy workspaces.

---

## Phase 3 — Unified CST collection + single parse handle in `check_file_diagnostics`  *(status: pending)*

`check_file_diagnostics` re-fetches `parse_file`/`parse.syntax()`/`AstFile::cast` ~8× and runs ~9 independent `syntax.descendants()`/`children()` walks (for `SMELT_PATH_CALL`, `RECORD_LITERAL`, `TABLE_REF`, `CTE`, `SELECT_STMT`). Collapse to one walk + one parse handle threaded through the structural passes:

```rust
struct CstCollection { record_literals, smelt_path_calls, table_refs, ctes, select_stmts: Vec<SyntaxNode> }
fn collect_cst_nodes(syntax: &SyntaxNode) -> CstCollection // single descendants() pass
```

Thread the single `&parse` (already held for the type-check block at `lib.rs:~1896`) through the early structural passes instead of re-calling `parse_file`.

**TDD tests:**
- `collect_cst_nodes_partitions_by_kind` — a fixture with one of each node kind lands in the right bucket; ordering matches `descendants()` order (the passes depend on document order).
- Equivalence gates green.

**Reviewer checklist:** node iteration order preserved (diagnostics anchored by document order); no pass silently dropped; purely mechanical.

**Commit:** `perf(smelt-db): single CST walk + parse handle in check_file_diagnostics`

**Expected:** ~2–3 % CST-walk + reduced Salsa memo lookups. Lower priority than 1–2; reorder if re-profiling disagrees.

---

## Phase 4 (stretch, gated on re-profiling) — cheap path identity for residual hot sites  *(status: pending)*

Only undertake if, after Phases 1–3, a fresh flamegraph still shows material `PathBuf` `compare`/`hash`/`starts_with` self-time. Options, smallest-blast-radius first:

- **4a.** Key low-ripple hot maps/sets by `SourceFile` (interned id) instead of `PathBuf` where the path string isn't semantically needed (e.g. dedup sets, `all_models` *consumers* that only need identity). **Do not** touch the `Database::files: HashMap<PathBuf, SourceFile>` registry (it's the path→file lookup) or `EmittedModelDef::generator_file` (path needed for diagnostics).
- **4b.** Add `#[salsa::tracked] sourcefile_order_key(db, file) -> u32` (ordinal assigned from `sorted_workspace_files`) for the remaining `.path(db).cmp()` / `== &path` / `starts_with(project_root)` project-isolation checks (`schema.rs:693`, `project.rs:2348`, etc.), replacing component-walk comparisons with integer compares.

**TDD tests:** ordinal is stable within a revision and consistent with path order; isolation checks (`example_workspaces` cross-project tests) green; a microbench in `decompose` shows reduced `compare_components`.

**Reviewer checklist:** no place that needs the actual path string was switched to an opaque key; ordinal recomputation on workspace edit is correct.

**Commit:** `perf(smelt-db): identify workspace files by interned key on hot comparison paths`

---

## Phase 5 — Completeness re-profile & ROADMAP update  *(status: pending)*

Re-run the flamegraph + `decompose`, record the new Initial Load number and the new top hotspot. If a new O(N²)/per-file re-scan surfaces, append a phase. Update `docs/ROADMAP.md` with the cumulative Initial Load improvement and date. Confirm the CI **Salsa / Initial Load** dashboard metric reflects the drop.

**Commit:** `docs: record initial-load perf results and next hotspot`

---

## Progress

| Phase | Description | Status |
|---|---|---|
| 0 | `project_sql_address_index` (resolve_ref_path O(N²) scan) | **done** (`d28a26ba`) |
| 1 | `sorted_workspace_files` hoist | **done** (Initial Load 663 → 297 ms) |
| 2 | `file_classification` skip-gate | pending |
| 3 | Unified CST walk + single parse handle | pending |
| 4 | Cheap path identity (gated on re-profile) | pending |
| 5 | Completeness re-profile + ROADMAP | pending |

## Risk register

- **Gating hides a diagnostic (Phase 2)** — highest-risk item. Mitigation: positive+negative fixture per gated pass; `example_diagnostics`/`example_workspaces`/property tests as the byte-identical oracle. A reviewer sign-off note is required for every gate.
- **Ordering drift** — several passes depend on document/path order for diagnostic anchoring and "first match wins". Mitigation: preserve `descendants()` order in `CstCollection`; preserve sort order in `sorted_workspace_files`; equivalence tests.
- **Project isolation** — any shared workspace index must filter by project at the boundary. Mitigation: keep the existing cross-project tests green; reviewer checks every shared-query consumer.
- **Over-investing in low-gain phases** — Phases 3–4 are smaller wins; re-profile between phases and stop when Initial Load plateaus.
