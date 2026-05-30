# crates/smelt-core/CLAUDE.md

Shared data types and workspace loading — `Config`, `ModelId`, `ModelDiscovery`, `DependencyGraph`, project-root detection, seed handling, and the centralised `load_workspace` entry point consumed by both the CLI and LSP.

## How to test

```bash
cargo test -p smelt-core
```

## Gotchas

- **`src/workspace.rs` is the single discovery entry point.** `load_workspace` performs all eager init-time filesystem discovery: SQL models under `config.paths`, function files under `functions/`, and sources YAML text. Both `smelt-cli`'s `init_db` and `smelt-lsp`'s `Backend::initialize` call it. Adding a new eager-discovery step anywhere else creates the asymmetric-discovery bug class. See root `CLAUDE.md` §Architectural invariants — **Workspace Loading Parity** is load-bearing here.
- **`src/project.rs`** owns `find_smelt_projects`, `find_project_root`, `find_project_root_for_file`, and the project-root detection helpers. A workspace may contain multiple projects; callers must not assume one-to-one workspace-to-project mapping.
- **`Config` (in `src/config.rs`) is the parsed `smelt.yml` shape.** It is shared between the LSP and CLI — changes here affect both. The `ModelConfig` embedded in it holds per-model materialization, incremental config, and backend assignment.
- **`src/seeds/`** is a sub-directory of modules handling seed loading, CSV parsing, Arrow conversion, and ephemeral seed construction. The entry points are re-exported from `src/lib.rs`.
- **`python` feature flag.** `src/python_models.rs` is gated on `#[cfg(feature = "python")]`. The default build does not include it; the CLI enables it.

## Where things live

- `src/workspace.rs` — `load_workspace`, `LoadedWorkspace`, `WorkspaceLoadErrors`
- `src/config.rs` — `Config`, `ModelConfig`, `Materialization`, `IncrementalConfig`
- `src/project.rs` — `find_smelt_projects`, `find_project_root`, `find_project_root_for_file`
- `src/discovery.rs` — `ModelDiscovery`, `ModelFile`, `discover_function_file_paths`
- `src/graph.rs` — `DependencyGraph`
- `src/seeds/` — seed loading sub-modules
