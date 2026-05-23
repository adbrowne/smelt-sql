# Research: LSP / CLI / UI consumer divergence

**Date**: 2026-05-23
**Topic**: Recurring bug class where one of LSP / CLI / UI reimplements (or forgets) work the others do. Catalog the current divergence, sketch architectural rules that prevent the class.
**Motivating incident**: Today's panic at `crates/smelt-ui/src/run_manager.rs:508` — UI executed test-materialized models because it skipped the `is_test()` filter that CLI applies at `commands/run.rs:103`. Same bug shape as the LSP `functions/` discovery miss (Q1 2026), the LSP `set_loader_file` miss, and the LSP flat-resolver multi-project leak — *the loud failure mode of having three parallel reimplementations of "process a smelt project."*

## Summary

smelt has three consumer crates that each do a different slice of the model lifecycle:

| | LSP | CLI | UI |
|---|---|---|---|
| Discover | ✓ (via `load_workspace`) | ✓ (via `load_workspace` + extras) | partial (graph pre-loaded) |
| Analyze (parse, types, diagnostics) | ✓ | ✓ | ✓ |
| Plan (batch safety, frontmatter, windows) | — | ✓ | ✓ |
| Compile (refs, fn expansion, ephemeral inlining, type casts) | — | ✓ (full) | partial (thin) |
| Pre-execute diagnostic gate | — | ✓ | ✗ |
| Execute (backend, manifests, intervals) | — | ✓ | ✓ (reimplemented) |
| Surface (CLI args / HTTP / LSP RPC) | ✓ | ✓ | ✓ |

The lower the lifecycle stage, the better the sharing: parsing, type inference, and workspace loading are cleanly shared via `smelt-parser`, `smelt-db`, and `smelt_core::workspace::load_workspace`. The two existing parity rules in CLAUDE.md (Workspace Loading Parity, Project Isolation) lock these in.

The higher stages — compilation, pre-execution gating, execution orchestration — are *not* shared cleanly. They live in `smelt-cli`'s private modules and have been re-implemented (incompletely) inside `smelt-ui`. That is where the bug class still lives and where the next architectural rule needs to land.

## Verified divergences (current state)

Each finding cites the file:line where the divergence is visible.

### Selection / filtering of executable models

- CLI: `crates/smelt-cli/src/commands/run.rs:100-103` filters out generator files (`.gen` suffix or `.gen.` in path) AND `!m.is_test()`.
- UI: `crates/smelt-ui/src/run_manager.rs:195` (before today's fix) ran whatever the graph returned. **Today's fix adds `is_test()` filtering to `run_manager.rs:195` and `build.rs:356`** but does *not* filter `.gen` generator files. If the UI ever sees a workspace with `.gen.sql` generators in the graph, it will try to execute them.
- LSP: doesn't run anything, so filtering is N/A — but its analysis surface should match what CLI/UI consider "runnable" for things like "is this model unused" lints.

### SQL compilation pipeline

- CLI: `crates/smelt-cli/src/compiler.rs:620-680` builds a full `PrintContext` with `smelt_as_struct: Some(...)`, `smelt_fn: Some(fn_expander)`, `smelt_path_ref: Some(...)`, `smelt_path_call: Some(path_call_expander)`. Function bodies come from `build_fn_body_map(&db, workspace)` (`compiler.rs:1443`). Ephemeral CTEs are inlined. `apply_type_casts()` wraps SELECT projections with CASTs based on inferred types.
- UI: `crates/smelt-ui/src/run_manager.rs:627-642` has its own `compile_sql()` that constructs `PrintContext` with **all four emitter slots set to `None`** and **empty `ephemeral_models` and `cross_engine_refs`**. There is no call to `build_fn_body_map`, no `apply_type_casts`, no ephemeral inlining.
- Consequence: a model that uses `smelt.fn.foo(...)` works under `smelt run` but the same model fails (or produces wrong SQL) under `smelt ui` → Run. Similarly, ephemeral models in the dependency chain are not inlined.
- Why the divergence exists structurally: `SqlCompiler` and `build_fn_body_map` live in `smelt-cli`, which is fundamentally the CLI binary's crate. The UI can't depend on it without dragging in CLI concerns, so it reimplemented the bits it needed.

### Pre-execution diagnostic gate

- CLI: `crates/smelt-cli/src/commands/run.rs:643-656` runs `smelt_db::file_diagnostics(...)` on every model file *before* executing and fails fast on `DiagnosticCode::UnknownSmeltFn`. The comment at `run.rs:643` explicitly notes this is a backstop for invalid function paths that the compiler would pass through silently.
- UI: `crates/smelt-ui/src/build.rs:221` does call `file_diagnostics` — but only to *display* diagnostics in the UI surface. The execution path in `run_manager.rs` doesn't gate on diagnostic codes; an `UnknownSmeltFn` would reach the backend and fail there with a less helpful message.
- LSP: surfaces diagnostics to the editor but doesn't gate execution (correct — LSP doesn't execute).

### Workspace discovery beyond `load_workspace`

This is mostly clean now thanks to the existing Workspace Loading Parity Rule. Verified:
- `load_workspace` at `crates/smelt-core/src/workspace.rs` covers SQL models, function files in `functions/`, multi-model frontmatter expansion, and YAML/JSON/TOML loader files.
- LSP's `Backend::initialize` consumes `load_workspace` and additionally registers a glob watcher for `**/functions/**/*.sql` at `backend.rs:1108`.
- CLI's `init_db` consumes `load_workspace`.

Remaining gap: **generator-file expansion** (`*.gen.sql` / `generates: models` frontmatter) is not part of `load_workspace`. CLI's `commands/run.rs:86` calls `discover_emitted_model_files()` to expand generators into virtual model files via the Salsa pipeline. The UI doesn't do this — its graph is built from non-generator models only. So a UI run in a project that uses generators silently ignores the generated models.

### Execution orchestration

`crates/smelt-ui/src/run_manager.rs` (709 lines) and `crates/smelt-cli/src/commands/run.rs` (1404 lines) both implement: select+exclude resolution, per-model target assignment, cross-engine edge detection, backend creation per target, time-range parsing, batch planning per `BatchSafety`, time filter injection, per-batch execution with `MaterializationStrategy::Incremental`, manifest writing, interval-store updates. They are line-for-line similar in places (the batch-safety match arms, the `inject_time_filter` helper). They differ in the surface details (CLI prints to stdout, UI emits `RunProgressEvent`).

## Why this keeps happening

The pattern: a feature gets built end-to-end in the CLI first (because that's where the test suite is richest), with helpers landing in `smelt-cli` modules that are visible to `smelt-cli`'s `lib.rs` but not exported as a real crate. When the UI grows a similar feature, the engineer either:
1. Copies the helper into the UI crate (compilation pipeline), or
2. Wires up the UI without the helper at all (test filter, generator expansion, diagnostic gate), or
3. Calls a thin shared helper for the part that *is* extracted (`smelt_core::load_workspace`, `smelt_planner::analyze_batch_safety`).

Only path (3) is a sustainable pattern. The first two have been the source of every divergence incident.

The root cause is that **there is no crate that owns "the full compile + execute pipeline for one model"**. `smelt-cli` owns it as a private detail of the CLI; `smelt-ui` reimplements a thinner version; `smelt-lsp` doesn't need it but would benefit from being able to dry-run compilation for richer diagnostics.

## Existing parity rules (recap from CLAUDE.md)

Two architectural invariants are already documented and gated by CI:

1. **Workspace Loading Parity Rule (CLI ↔ LSP)**: eager init-time discovery lives only in `smelt_core::workspace::load_workspace`. Salsa-ingest is centralised in `smelt_db::workspace_ingest::ingest_loaded_workspace`. Standing CI gate: `cargo test -p smelt-lsp --test example_workspaces`.

2. **Project Isolation Rule**: workspace folder may contain multiple smelt projects; resolvers must thread `ProjectInput` through. Standing CI gate: multi-project case in `example_workspaces`.

Both follow the same shape: *one place owns the logic; all consumers call it; CI runs the consumer that historically drifted against real fixtures.*

## Proposed direction (research, not prescriptive)

The fix is **layered ownership**, not just "extract one more crate." The existing parity rules already establish that analysis-shared-with-LSP belongs in the lower layers (`smelt-parser`, `smelt-db`, `smelt-core`, `smelt-planner`). A new crate for CLI+UI-only concerns sits *above* those layers and depends on them — it does not replace or duplicate them.

### Layer ownership matrix

| Layer | Crate | Shared with LSP? | Status |
|---|---|---|---|
| Parse (lexer, CST, AST, error recovery) | `smelt-parser` | ✓ | clean |
| Incremental analysis (Salsa queries, type inference, schemas, diagnostics) | `smelt-db` | ✓ | clean (pure-function rule) |
| Workspace discovery (`load_workspace`, ingest) | `smelt-core` | ✓ | clean (workspace parity rule) |
| Planning (frontmatter parse, batch safety, time windows) | `smelt-planner` | ✓ (LSP could surface as diagnostics) | clean |
| Compile (refs, fn expansion, ephemeral inlining, type casts, time filter) | **today: `smelt-cli` private; proposed: `smelt-runtime`** | partially — LSP would benefit from dry-compile but doesn't today | **divergent** |
| Execute (backend dispatch, batch loop, manifests, intervals) | **today: split between `smelt-cli` and `smelt-ui`; proposed: `smelt-runtime`** | ✗ (LSP doesn't execute) | **divergent** |
| Surface (CLI args, HTTP, LSP RPC, progress UX) | `smelt-cli` / `smelt-ui` / `smelt-lsp` | n/a | correct (each owns its own) |

**The rule that follows from this matrix**: a new piece of logic is placed at the *lowest* layer it can be shared from. If LSP needs it, it goes in `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner`. If only CLI and UI need it, it goes in the new `smelt-runtime`. It never lives in a consumer crate.

This is important because the LSP-shared work is *already* in good shape — we don't want a `smelt-runtime` extraction to accidentally pull analysis logic up out of `smelt-db` where it's reachable by the LSP. The existing pure-function rule and workspace parity rule must continue to apply unchanged; `smelt-runtime` is purely *additive* for the layers that aren't shared with LSP.

### Move 1: Extract the compile + execute pipeline into `smelt-runtime`

Working name: `smelt-runtime` (other candidates: `smelt-pipeline`, `smelt-runner`, `smelt-execute`). It depends on `smelt-db`, `smelt-core`, `smelt-planner`, `smelt-backend`, `smelt-parser`. It owns:

- `SqlCompiler` and `build_fn_body_map` (today in `smelt-cli/src/compiler.rs`).
- `build_fn_body_map_from_model_files` for the non-Salsa variant.
- `apply_type_casts`, ephemeral inlining, cross-engine ref resolution.
- `inject_time_filter` (today duplicated in both crates).
- The selection-+-filter pass that resolves selectors, applies excludes, drops tests and `.gen` files, expands emitted models, returns the executable model list.
- The per-model execute loop (full refresh vs. incremental batches vs. cumulative dispatch).
- The `RunManifest`/interval-store writes.

It explicitly does NOT own:
- Parsing, type inference, schema extraction → stays in `smelt-parser` / `smelt-db`.
- Workspace discovery / ingest → stays in `smelt-core::workspace`.
- Batch-safety analysis / frontmatter → stays in `smelt-planner`.
- Diagnostic checks themselves → the *gate* is in `smelt-runtime`, the *checks* stay in `smelt-db`.

Consumers reduce to surface code:
- `smelt-cli`: CLI args → `smelt_runtime::ExecuteRequest` → stdout reporter.
- `smelt-ui`: HTTP body → `smelt_runtime::ExecuteRequest` → `RunProgressEvent` reporter.
- `smelt-lsp`: doesn't depend on `smelt-runtime`. Continues to depend only on `smelt-db` / `smelt-core` / `smelt-parser` / `smelt-planner` for analysis.

A `RunReporter` trait abstracts "tell the user about progress" — CLI implements stdout/spinner, UI implements broadcast events, tests implement a captured-log variant. This is the only abstraction the runtime crate needs; everything else is plain data.

### Move 2: Codify a layered parity rule covering all three consumers

Once `smelt-runtime` exists, the architectural rule in `docs/specs/architecture.md` and CLAUDE.md is **two-part**, not one — and explicitly preserves the LSP-shared layering:

> **Layered single-ownership rule.** Smelt is organised as a stack where each layer has one owning crate, and consumers may only depend downward. A new piece of logic is placed at the lowest layer that needs it:
>
> 1. **Shared with LSP** (parsing, analysis, type inference, schemas, diagnostics, workspace discovery, planning) — lives in `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner`. All three consumers (`smelt-cli`, `smelt-ui`, `smelt-lsp`) call into these. This is already enforced by the Workspace Loading Parity Rule, the Project Isolation Rule, and the smelt-db Pure Function Rule.
>
> 2. **Shared with CLI+UI only** (compile pipeline, execute loop, manifests, interval-store writes, selection/filter pass) — lives in `smelt-runtime`. Both `smelt-cli` and `smelt-ui` consume it via a single `execute_project(request, reporter)` entry point and contribute only surface concerns (argument parsing, progress reporting, HTTP serialization). LSP does not depend on `smelt-runtime`.
>
> 3. **Surface only** (CLI argument shapes, HTTP request/response, LSP RPC, progress UX) — lives in the consumer crate.
>
> *Why*: incidents trace to two failure modes. **Mode A** (e.g., LSP `functions/` discovery, `set_loader_file` miss): a consumer reimplements analysis logic instead of calling the shared layer. **Mode B** (e.g., today's test-model execution, UI `smelt.fn.*` non-expansion): a consumer reimplements execution logic because there's no shared layer for it to call. Layered single-ownership closes both.
>
> *How to apply*: when adding a feature, classify it by who needs it. If LSP needs it, it goes in `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner` — never in `smelt-runtime` or a consumer crate. If only CLI and UI need it, it goes in `smelt-runtime`. Never duplicate across layers; never reach across consumer crates.

Standing CI gates, layered the same way:
- Layer 1 (already exists): `cargo test -p smelt-lsp --test example_workspaces` — drives the real LSP `Backend` against every example workspace and asserts zero diagnostics. Catches "LSP forgets to consume the shared analysis layer."
- Layer 2 (new): a fixture-driven test that runs the same project through `smelt-cli` and `smelt-ui` entry points and asserts identical model outputs, manifest contents, and selection sets. The fixtures must include test models (filter), generators (expansion), and `smelt.fn.*` calls (compile). Catches "UI reimplements execute differently from CLI."

### Move 3: Make consumer crates structurally read-only against shared internals

To prevent the rule from being eroded in practice, both shared layers expose narrow surfaces:

- `smelt-runtime` exposes only its entry point + a few opaque types (`ExecuteRequest`, `RunReporter` trait, `RunOutcome`). Helpers like `SqlCompiler`, `build_fn_body_map`, `inject_time_filter`, `apply_type_casts` are `pub(crate)` inside `smelt-runtime`. Consumers can't reach in and use half the pipeline.
- The lower-layer crates already follow this shape (`smelt-db`'s pure-function rule keeps Salsa traits out of analysis APIs; `smelt-core`'s `load_workspace` is the only public discovery entry point).
- Result: "add it to the right layer or do nothing" is the only path. Re-implementing half is no longer the path of least resistance.

This is the same shape as the `smelt-db` pure-function rule and the Workspace Loading Parity Rule: make the "right thing" the only thing exposed, at every layer.

## Tradeoffs and risks

- **Extraction cost**: `smelt-cli/src/compiler.rs` is ~1500 lines that are intertwined with CLI-specific types (`CompiledModel`, `ModelFile` shape). The extraction is not mechanical — it requires picking the right shared types. Likely a multi-PR refactor before the rule can be enforced.
- **Risk of pulling analysis up into `smelt-runtime` by accident**: during extraction, it's tempting to grab anything `SqlCompiler` touches and put it in the new crate. The discipline is that *if a piece of logic is also needed by the LSP* (type inference, schema resolution, ref resolution, diagnostic checks), it stays in `smelt-db` / `smelt-core` and `smelt-runtime` calls down to it. The layering matrix above is the test: anything in column "shared with LSP = ✓" stays where it is.
- **Reporter abstraction risk**: the `RunReporter` trait could grow to leak surface concerns into the runtime crate (e.g., HTTP-specific event shapes). Keep it minimal — abstract progress, not transport.
- **Generators are special**: the emitted-models pipeline lives behind a Salsa query in `smelt-db`. `smelt-runtime` can depend on `smelt-db` directly and call the existing query, or accept a pre-expanded model list as input. The first preserves the layering (one source of truth for generator expansion, also reachable by LSP for future use); the second is more decoupled. Probably the first.
- **Plan-mode UI**: today's `build.rs::build_run_plan` is a separate "preview without executing" path. The runtime crate should support `execute_request.dry_run = true` rather than maintaining two parallel pre-execution paths.
- **LSP eventually consuming `smelt-runtime`**: even though Move 2 says LSP doesn't depend on `smelt-runtime` today, the layering doesn't *forbid* it. If we later want the LSP to dry-compile a model to surface type-cast diagnostics, it can depend on `smelt-runtime` for that — but only if `smelt-runtime`'s analysis-touching parts have been kept pure enough to be called without executing. Worth keeping the compile path callable as a pure function returning `CompiledModel` (i.e., not coupled to the execute loop) so this option stays open.

## Open questions

- Should `smelt-runtime` own the **diagnostic gate** (running `file_diagnostics` and failing on a configured set of codes) or should that stay in the surface layer? Arguments either way: in-runtime means consistent gating; in-surface means CLI and UI can pick different gates (UI might want to show warnings inline rather than fail-fast). Current lean: in-runtime, with a `gate_codes: Vec<DiagnosticCode>` field on the request so consumers can configure but not skip.
- Does this crate need a new name, or can it be `smelt-cli`'s `lib` portion renamed and the binary moved to `smelt-cli-bin`? A rename is cheaper but `smelt-cli` carries a lot of CLI-specific identity (`CliError`, `commands/`). Probably cleaner to extract.
- What's the migration order? Likely: (1) move pure helpers (`build_fn_body_map`, `inject_time_filter`) to a new crate first; (2) extract `SqlCompiler` once its dependencies are out; (3) extract the execute loop last (it has the most cross-cutting concerns: cancellation, progress, interval store). Each step is independently shippable and shrinks the divergence surface.

## References

- Predecessor incidents (documented in CLAUDE.md → Architecture):
  - Workspace Loading Parity Rule (LSP `functions/` discovery and `set_loader_file` misses).
  - Project Isolation Rule (LSP flat resolver leak across `web_analytics` and `functions_demo`).
- Today's fix: `crates/smelt-ui/src/run_manager.rs:195`, `crates/smelt-ui/src/build.rs:356` — test-model filter added.
- Pure Function Rule (smelt-db): analogous structural rule that keeps analysis logic out of Salsa internals so it can be extracted to a future `smelt-check` crate. The proposed Run Pipeline Parity Rule is the same idea applied to the compile+execute layer.
