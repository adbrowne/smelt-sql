# Salsa Upgrade: 0.16 → 0.26

## Session Status (2026-04-15)

**Phase 1: DONE** (uncommitted, on disk in the `salsa-upgrade` worktree):
- `Cargo.toml` bumped `salsa = "0.16"` → `salsa = "0.26"` (latest stable — latest at time of work).
- `cargo update -p salsa` ran; new transitive deps added (`salsa-macro-rules`, `thin-vec`, `boxcar`, `intrusive-collections`, `inventory`, `crossbeam-queue`, `allocator-api2`). Old `parking_lot 0.11` dropped.
- Toolchain OK with cargo 1.94.1; no `rust-toolchain.toml` changes needed.

**Pre-upgrade bench baseline** (0.16, `cargo bench -p smelt-bench --bench salsa_incremental -- --quick`):
- `salsa_initial_load_2000`: **9.13 ms**
- `salsa_leaf_edit_diagnostics`: **3.11 µs**
- `salsa_full_diagnostics_2000`: **59.5 µs**

Post-upgrade must be equal or better on all three.

**Phase 2: STARTED, NOT DONE.** `cargo build -p smelt-db` currently fails with 15 errors — expected. Nothing else committed; smelt-db/smelt-lsp/bench still use the 0.16 API.

**API shifts discovered while surveying 0.26:**
- `#[salsa::database(...)]` macro is **gone**. Replaced by `#[salsa::db]` attribute on the struct AND on `impl salsa::Database for Database {}`.
- `salsa::Database` now requires `Send + ZalsaDatabase + AsDynDatabase`. `HasStorage` is the real trait to satisfy — there's a stock `salsa::DatabaseImpl` that does this, and a custom DB needs to `derive` or hand-impl it. Simplest path: use `salsa::DatabaseImpl` directly if we don't need custom state on the DB, OR keep our own `Database` struct and ensure it implements `HasStorage` via the `#[salsa::db]` derive.
- Query groups are gone — all queries become free `#[salsa::tracked]` fns taking `&dyn salsa::Database`.
- Inputs are structs via `#[salsa::input]`. Singleton pattern: `#[salsa::input(singleton)]`.
- `#[salsa::accumulator]` available for diagnostics.
- Cycle handling: `#[salsa::tracked(cycle_fn=..., cycle_initial=...)]` — fixpoint iteration, no more `catch_unwind` needed.

## How to resume (fresh session, clean context)

1. **cd into the worktree:** `/home/andrew/smelt-sql/.claude/worktrees/salsa-upgrade`.
2. Read this plan file + `/home/andrew/.claude/plans/dynamic-dreaming-stroustrup.md`.
3. Read `crates/smelt-db/src/lib.rs:1-165` to re-anchor on the query surface. The 28 queries below line 165 are mostly pure-fn wrappers that barely change except their signatures.
4. Skim `crates/smelt-lsp/src/lib.rs:680-720` (Backend + `Arc<Mutex<Database>>`) and `:908-971` (setter loop + `query_diagnostics` panic-catch workaround — DELETE this).
5. Start work on Phase 2 below; the design sketch is now included.

## Target Design Sketch (Phase 2)

```rust
// crates/smelt-db/src/lib.rs — new top

#[salsa::db]
#[derive(Clone, Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
    // Path → SourceFile input registry (external, not a salsa query).
    // LSP uses this to look up the right SourceFile to set_text on when a file changes.
    files: Arc<DashMap<PathBuf, SourceFile>>,
    projects: Arc<DashMap<PathBuf, ProjectInput>>,
}

#[salsa::db]
impl salsa::Database for Database {}

#[salsa::input]
pub struct SourceFile {
    #[id] pub path: PathBuf,
    #[returns(ref)] pub text: String,
    pub project_root: PathBuf,
}

#[salsa::input]
pub struct ProjectInput {
    #[id] pub path: PathBuf,
    #[returns(ref)] pub sources_yaml: String,
}

#[salsa::input(singleton)]
pub struct Workspace {
    #[returns(ref)] pub files: Vec<SourceFile>,
    #[returns(ref)] pub projects: Vec<ProjectInput>,
}

#[salsa::accumulator]
pub struct DiagnosticAcc(pub Diagnostic);

// Queries — free fns, no trait:
#[salsa::tracked(returns(ref))]
fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> smelt_parser::Parse {
    let text = file.text(db);
    let clean = smelt_parser::strip_frontmatter(text);
    smelt_parser::parse(&clean)
}

#[salsa::tracked(returns(ref), cycle_fn = typed_schema_cycle_fn, cycle_initial = typed_schema_cycle_initial)]
fn typed_model_schema(db: &dyn salsa::Database, file: SourceFile) -> ModelSchema { ... }

fn typed_schema_cycle_initial(_db: &dyn salsa::Database, _file: SourceFile) -> ModelSchema {
    ModelSchema::empty()
}
fn typed_schema_cycle_fn(...) -> CycleRecoveryAction<ModelSchema> {
    CycleRecoveryAction::Iterate  // or ::Fallback(ModelSchema::empty())
}
```

**Key migration moves inside smelt-db/src/lib.rs (below line 165):**
- Every fn signature: `fn foo(db: &dyn Syntax, path: PathBuf) -> Arc<T>` → `#[salsa::tracked] fn foo(db: &dyn salsa::Database, file: SourceFile) -> Arc<T>`.
- Every call site in-file: `db.file_text(path)` → `file.text(db)`; `db.parse_file(path.clone())` → `parse_file(db, file)`; `db.resolve_ref(name)` → `resolve_ref(db, name)` etc.
- `resolve_ref` takes a `String` model name — still works as a query arg, but the *return* should become `Option<SourceFile>` not `Option<PathBuf>` so downstream queries get input identity.
- Strip the `Arc<Vec<Diagnostic>>` returns; push via `DiagnosticAcc::accumulate(db, d)` and read via `file_diagnostics::accumulated::<DiagnosticAcc>(db, file)` at the top level.

**smelt-lsp migration (Phase 3):**
- `Backend.db: Arc<Mutex<Database>>` → `Backend.db: Database` (cheap Clone, internally concurrent). Read-only callers get `&self.db`; writers need `&mut`, so introduce a single-writer `tokio::sync::Mutex` around a small "writer handle" or use `parking_lot::Mutex` only on the writer path. The read path must not lock.
- Setter pattern: instead of `db.set_file_text(path, Arc::new(content))`, do:
  ```rust
  let file = db.files.entry(path.clone()).or_insert_with(|| SourceFile::new(&mut db, path.clone(), content.clone(), root));
  file.set_text(&mut db).to(content);
  ```
- Delete `query_diagnostics` panic workaround (`smelt-lsp/src/lib.rs:942-971`) + its comment.
- Wire `db.synthetic_write(Durability::LOW)` or a cancellation handle on `did_change` so concurrent diagnostic computations unwind cleanly.

**Bench migration (Phase 4):**
- `crates/smelt-bench/benches/salsa_incremental.rs` — switch to new `SourceFile::new` / `set_text` / top-level tracked fn calls. Keep iteration structure identical so numbers are comparable.

## TODOs carried forward to fresh session

- [x] Phase 1: bump Cargo.toml, baseline bench
- [x] Phase 2: rewrite `crates/smelt-db/src/lib.rs` (top 165 lines + all 28 query impls + 3 cycle fns + in-file tests)
- [x] Phase 3: rewrite `crates/smelt-lsp/src/lib.rs` DB handling and delete panic-catch workaround
- [x] Phase 4: rewrite `crates/smelt-bench/benches/salsa_incremental.rs`; run `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-db --test type_property_tests`
- [x] Add explicit cycle-regression test under `smelt-db/tests/` using a circular model graph; must return the cycle_initial value without panicking
- [ ] Phase 5: purge 0.16 references from `docs/architecture_overview.md`, `docs/lsp_architecture.md`, `CLAUDE.md`, update `docs/ROADMAP.md` with completion date

## Original Context

`smelt-db` is pinned to `salsa = "0.16"` (2020-era). Salsa has since had a near-total rewrite ("salsa-2022" / "salsa 3", latest published as `salsa` 0.22.x). The old `#[salsa::query_group]` macro-based API is gone; modern Salsa uses `#[salsa::tracked]` functions, `#[salsa::input]` / `#[salsa::tracked]` structs, and a `#[salsa::db]` trait.

Motivations:
1. **Kill a live workaround.** `smelt-lsp/src/lib.rs:942-971` wraps diagnostic queries in `catch_unwind` because salsa 0.16.1 panics during memo validation when circular model refs exist. Modern salsa has first-class fixpoint cycle handling and no longer poisons the DB on cycles.
2. **Remove `Arc<Mutex<Database>>` bottleneck.** LSP currently holds `Arc<Mutex<Database>>` (`smelt-lsp/src/lib.rs:687,710`) and serialises every read. Modern salsa supports concurrent reads via `&dyn Database` + internal parking-lot locks; a mutex around the whole DB is unnecessary and hurts LSP latency at scale (see `examples/huge`, 2000 models).
3. **Better diagnostics ergonomics.** `salsa::Accumulator` replaces our hand-rolled `Arc<Vec<Diagnostic>>` return-value pattern (`file_diagnostics`, `type_diagnostics`) with push-based collection — diagnostics bubble up from nested tracked calls without an explicit aggregator.
4. **`#[salsa::tracked]` structs for `Model` / `ModelSchema`** enable cross-query identity and cheaper invalidation (currently returned as `Arc<...>` by value).
5. **Durability / LRU / interning** are still available and largely compatible.

## Current state (inventory)

**Crates touching salsa:**
- `crates/smelt-db/src/lib.rs` — 5 query groups, 5 inputs, ~28 queries, 3 `#[salsa::cycle]` recovery fns (lines 43-165, 963-992).
- `crates/smelt-lsp/src/lib.rs` — holds `Arc<Mutex<Database>>`, calls setters + queries, has panic-catch workaround (line 942).
- `crates/smelt-bench/benches/salsa_incremental.rs` — incremental-edit benchmarks.
- `Cargo.toml` — workspace dep `salsa = "0.16"`.

**Query groups:** `InputsStorage`, `SyntaxStorage`, `SemanticStorage`, `SchemaStorage`, `TypeCheckingStorage` (`smelt-db/src/lib.rs:43,68,97,117,128`).

**Cycle recovery fns:** `recover_typed_model_schema`, `recover_type_context`, `recover_resolved_model_schema` (`smelt-db/src/lib.rs:965-992`).

## Approach

Single-branch upgrade, phased commits. No compat shim — the API break is total, so partial migration isn't possible.

### Phase 1 — Preparation
- Bump `salsa = "0.22"` (or latest stable at implementation time) in root `Cargo.toml`.
- Verify MSRV — modern salsa typically requires recent stable Rust; bump `rust-toolchain` if needed.
- Run the bench suite (`cargo bench -p smelt-bench --bench salsa_incremental`) on 0.16 and save a baseline (cold load, leaf edit, full diagnostics on 2000 models). We'll compare post-upgrade.

### Phase 2 — Rewrite `smelt-db` core
File: `crates/smelt-db/src/lib.rs`.

- Replace each `#[salsa::query_group(...)] trait X { ... }` with a `#[salsa::db] trait Db { ... }` single supertrait (or plain free `#[salsa::tracked]` fns that take `&dyn salsa::Database`). Flatten the 5-group hierarchy — it exists only because 0.16 required grouping. In new salsa, queries are standalone `#[salsa::tracked]` fns.
- Replace the `Database` struct:
  ```rust
  #[salsa::db]
  #[derive(Default, Clone)]
  pub struct Database { storage: salsa::Storage<Self> }
  #[salsa::db] impl salsa::Database for Database {}
  ```
- Convert inputs. Candidates for `#[salsa::input]` structs:
  - `SourceFile { path: PathBuf, text: Arc<String> }`
  - `ProjectRoot { path: PathBuf, sources_yaml: Arc<String> }`
  - Workspace-level input tracking `Vec<SourceFile>` and `Vec<ProjectRoot>`.
  This replaces the keyed `file_text(PathBuf)` style. Callers look up the `SourceFile` by path via an interned `FilePath` or an `all_files` input returning `Vec<SourceFile>`.
- Convert queries to free functions: `#[salsa::tracked] fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> Parse { ... }`. Each existing fn body stays almost identical — only the signature changes (swap `&dyn Syntax` → `&dyn salsa::Database`, swap `PathBuf` → `SourceFile`).
- **Diagnostics via `Accumulator`.** Define `#[salsa::accumulator] struct DiagnosticAcc(Diagnostic);`. Inside tracked checking fns, call `DiagnosticAcc(d).accumulate(db)` instead of returning `Vec<Diagnostic>`. Top-level `file_diagnostics(file)` calls the checker then collects via `file_diagnostics::accumulated::<DiagnosticAcc>(db, file)`. This removes the need for `Arc<Vec<Diagnostic>>` returns and the manual `.chain()` in `query_diagnostics`.
- **Cycle handling.** Replace `#[salsa::cycle(recover_*)]` with modern salsa's fixpoint iteration (`#[salsa::tracked(cycle_fn=..., cycle_initial=...)]`). The three current recovery fns map 1:1 to `cycle_initial` returning the same empty defaults. No more memo-validation panics.
- Keep the pure-function layer (`type_inference.rs`, `schema.rs`) entirely unchanged — per `CLAUDE.md`'s Pure Function Rule, tracked fns are thin wrappers that call these. This is the main reason the migration is tractable.

### Phase 3 — Rewrite LSP integration
File: `crates/smelt-lsp/src/lib.rs`.

- Remove `Arc<Mutex<Database>>`. New salsa `Database` is `Clone` (storage is internally `Arc`'d and concurrency-safe); share as `Database` directly on `Backend`, or `Arc<Database>` if multiple owners need `&self`. Reads no longer need `.lock().await`.
- Replace `db.set_file_text(path, Arc::new(content))` calls (lines 909-1621) with input-struct setters: look up or create the `SourceFile` for that path, then `source_file.set_text(&mut db).to(Arc::new(content))`. Writes take `&mut Database` — use a short-lived mutable scope or the standard LSP pattern of a single writer task.
- **Delete the panic workaround** (`query_diagnostics`, `smelt-lsp/src/lib.rs:942-971`). Replace with a direct call to `file_diagnostics(&db, file)` + accumulator collection. Remove `std::panic::catch_unwind` import if no other uses.
- Cancellation: new salsa has `db.unwind_if_cancelled()` / `db.synthetic_write(Durability::LOW)` for LSP cancellation on input change. Wire this into the LSP `did_change` handler so in-flight diagnostic computations cancel cleanly — an actual improvement over today's unconditional full recompute.

### Phase 4 — Update bench and tests
- `crates/smelt-bench/benches/salsa_incremental.rs`: update setter calls and query invocations to the new API. Compare against the Phase 1 baseline; the LSP path should be faster due to mutex removal and better cycle handling.
- `cargo test -p smelt-db` and `cargo test -p smelt-cli --test example_diagnostics` must pass. The `examples/broken/` workspace is the acid test for cycle handling — it currently triggers the `catch_unwind` path.
- Run the property tests: `cargo test -p smelt-db --test type_property_tests`. These test pure functions so should be unaffected, but confirm.

### Phase 5 — Cleanup
- Remove `salsa = "0.16"` references from comments/docs, notably the workaround comment at `smelt-lsp/src/lib.rs:943`.
- Update `docs/ROADMAP.md` with completion date.
- Update `CLAUDE.md`'s "Architecture / Salsa" notes if the query-group terminology is referenced.
- Check `docs/architecture_overview.md` and `docs/lsp_architecture.md` for stale 0.16 API descriptions.

## Critical files

| File | Change |
|---|---|
| `Cargo.toml` | bump `salsa` version |
| `crates/smelt-db/src/lib.rs` | full rewrite of query definitions (not the pure-fn bodies) |
| `crates/smelt-lsp/src/lib.rs` | remove mutex, remove `catch_unwind`, new setter API, add cancellation |
| `crates/smelt-bench/benches/salsa_incremental.rs` | adapt to new API |
| `docs/ROADMAP.md` | log completion |

## Verification

1. `cargo fmt --all && cargo clippy --all-targets` — clean.
2. `cargo build` — clean.
3. `cargo test` workspace-wide — passes.
4. `cargo test -p smelt-cli --test example_diagnostics` — all example workspaces diagnostic-clean.
5. `cargo test -p smelt-db --test type_property_tests` — no regressions.
6. **Cycle regression test:** open `examples/broken/` with circular refs in the LSP (or a direct `smelt-db` test) and confirm diagnostics return **without** panic-catch, with accumulated cycle-recovery defaults. Add an explicit test in `smelt-db` that calls `resolved_model_schema` on a cyclic graph and asserts the initial/fixpoint value is returned rather than a panic — this locks in the workaround removal.
7. `cargo bench -p smelt-bench --bench salsa_incremental` — compare against Phase 1 baseline; expect equal or better on all three scenarios, with larger wins on leaf-edit under concurrent LSP load.
8. Manual LSP smoke test against `examples/test_workspace/` and `examples/huge/` (2000 models) — edit latency feels at least as fast; no hangs.

## Risks / notes

- **Scope.** smelt-db is ~5k lines but only ~200 lines are salsa-specific surface area (the `#[salsa::query_group]` blocks + the `Database` struct). The pure-fn bodies are untouched. Realistic 1–2 day job, not a week.
- **Input modeling choice.** Keyed-input style (`file_text(PathBuf)`) still works in modern salsa via `#[salsa::input]` structs keyed by interned path — but `SourceFile` struct inputs are more idiomatic and give stable identity across queries. Plan chooses the latter; if it produces API churn we don't like, fall back to keyed inputs with zero behavior change.
- **Cancellation wiring** is optional for correctness but the biggest latent LSP win — include it in the same PR since we're already touching all the call sites.
- **No backwards-compat stage.** 0.16 and 0.22 cannot coexist in the same crate graph; this is one atomic upgrade.
