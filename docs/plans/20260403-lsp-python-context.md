# Plan: LSP Python Model ProjectContext + Helper Deduplication

**Date**: 2026-04-03
**Research**: docs/research/2026-04-03-lsp-python-model-integration.md
**Status**: Validated

## Context

Python models that call `project.find_models()` receive an empty `ProjectContext` in the LSP because `python_scan.rs` hardcodes `{"models": []}` instead of building a real context from discovered SQL models. The CLI already does this correctly. Additionally, `python_scan.rs` duplicates 5 helper functions from the CLI's `python.rs` — this work extracts them into `smelt-core` first, then threads the real context through.

## Desired End State

- Python models executed by the LSP receive a `ProjectContext` populated with all known SQL model names, tags, and directories
- Cache invalidates when the model list changes (new `context_hash` field)
- Shared helpers (`find_python`, `find_python_sdk`, `build_pythonpath`, `scan_for_model_decorators`, `build_decorator_map`) live in `smelt-core/src/python_utils.rs` and are used by both LSP and CLI

## What We're NOT Doing

- Iterative (multi-round) Python model discovery — single pass is sufficient initially
- Frontmatter tag extraction — `config.get_tags(name, None)` (config-only tags) is enough for now
- Fixed-point validation in the LSP
- Changing `did_change()` to handle `.py` files inline (file-watcher-on-save is acceptable)

## Implementation Phases

### Phase 1: Extract shared helpers into smelt-core

**Files to modify**:
- `crates/smelt-core/src/python_utils.rs` — **new file** with shared helpers
- `crates/smelt-core/src/lib.rs` — add `pub mod python_utils;`
- `crates/smelt-lsp/src/python_scan.rs` — replace local helpers with imports from `smelt_core::python_utils`
- `crates/smelt-cli/src/python.rs` — replace local helpers with imports from `smelt_core::python_utils`

**New module `python_utils.rs`** contains these functions (no feature gate — pure Rust, no PyO3):

1. **`find_python(config_python: Option<&str>) -> Option<String>`** — Based on CLI version (lines 111-143 of `python.rs`). Checks `SMELT_PYTHON` env var, then `config_python`, then tries `python3`/`python`. Returns `Option` (callers handle not-found differently).

2. **`find_python_sdk(project_dir: &Path) -> Option<PathBuf>`** — Based on CLI version (lines 74-107). Checks `SMELT_PYTHON_SDK` env var, then `project_dir/python/smelt`, then walks up 5 levels. Returns `Option`.

3. **`build_pythonpath(sdk_path: &Path, file_path: &Path) -> OsString`** — Identical in both (LSP lines 216-227, CLI lines 261-272). Move as-is.

4. **`scan_for_model_decorators(content: &str) -> Vec<u32>`** — Based on CLI version (lines 57-70). Returns 0-indexed line numbers of all `@model` decorators. The LSP's `has_model_decorator()` becomes `!scan_for_model_decorators(content).is_empty()`. CLI callers adjust from 1-indexed `Vec<usize>` to 0-indexed `Vec<u32>` (add +1 where needed for display).

5. **`build_decorator_map(content: &str) -> HashMap<String, u32>`** — From LSP version (lines 135-168). Returns **0-indexed** line numbers. CLI callers add +1 for display.

**Updating callers**:
- `python_scan.rs`: Remove local `find_python()`, `find_python_sdk()`, `build_pythonpath()`, `has_model_decorator()`, `build_decorator_map()`. Import from `smelt_core::python_utils`.
- `python.rs` (CLI): Remove local `find_python()`, `find_python_sdk()`, `build_pythonpath()`, `scan_for_model_decorators()`, `build_decorator_map()`. Import from `smelt_core::python_utils`. Adjust for 0-indexed return values (add +1 where 1-indexed values are needed for display).
- `discovery.rs` (CLI): Update `scan_for_model_decorators` import path and adjust for `Vec<u32>` return type.

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (no warnings)
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (all pass)
- [ ] No behavioral changes — pure refactor

### Phase 2: Add `context_json` parameter to `python_scan.rs`

**Files to modify**:
- `crates/smelt-lsp/src/python_scan.rs`

**Changes**:

1. **Add `context_hash` to `CacheEntry`** (line ~54):
   ```rust
   struct CacheEntry {
       content_hash: String,
       context_hash: String,  // NEW
       models: Vec<CachedModel>,
       timestamp: u64,
   }
   ```

2. **Update `PythonModelCache::get()`** (line ~95): Accept `context_hash: &str` parameter. Return `None` if `entry.context_hash != context_hash` (cache miss when model list changes).

3. **Update `PythonModelCache::put()`** (line ~102): Accept `context_hash: &str` parameter. Store it in the `CacheEntry`.

4. **Update `discover_python_models()` signature** (line ~356):
   ```rust
   pub fn discover_python_models(
       models_path: &Path,
       project_dir: &Path,
       cache: &mut PythonModelCache,
       context_json: &str,       // NEW
   ) -> PythonScanResult
   ```
   - Remove hardcoded `let context_json = r#"{"models": []}"#;` at line 411
   - Compute `context_hash` from `context_json` using `content_hash()` (already exists)
   - Pass `context_hash` to cache `get()`/`put()` calls

5. **Update `execute_single_python_file()` signature** (line ~468):
   ```rust
   pub fn execute_single_python_file(
       file_path: &Path,
       project_dir: &Path,
       cache: &mut PythonModelCache,
       context_json: &str,       // NEW
   ) -> PythonScanResult
   ```
   - Remove hardcoded `let context_json = r#"{"models": []}"#;` at line 546
   - Pass `context_hash` to cache calls

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (no warnings)
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (all pass — callers updated with placeholder `"{\"models\": []}"` temporarily)

### Phase 3: Build and pass real context in LSP `main.rs`

**Files to modify**:
- `crates/smelt-lsp/src/main.rs`

**Changes**:

1. **Add `build_python_context()` helper** (new private function):
   ```rust
   fn build_python_context(all_files: &[PathBuf], config: &Config) -> String
   ```
   - Iterate `all_files`, extract model name from each path:
     - For `::` virtual paths (multi-model files): split on `::`, use last segment
     - For regular paths: use file stem
   - Get directory from parent path's file name
   - Get tags via `config.get_tags(name, None)` (config-level only)
   - Serialize as `{"models": [{"name": "...", "tags": [...], "directory": "..."}]}`
   - Reuse `ProjectContextData` / `ProjectModelInfo` structs — either import from CLI or define locally (they're simple serde structs)

2. **Update init path** (~line 1097):
   - Retain full `Config` object (currently only `model_paths` is extracted at line 1098-1100)
   - After SQL model scanning completes (~line 1156), call `build_python_context(&all_files, &config)`
   - Pass resulting `context_json` to `discover_python_models()` call at line 1161

3. **Update `handle_python_file_change()`** (~line 842):
   - Inside the handler, lock DB and get `all_files()`
   - Load `Config` from project root
   - Call `build_python_context()` to build context
   - Pass `context_json` to `execute_single_python_file()` call

4. **Move `ProjectContextData` and `ProjectModelInfo` structs to `smelt-core`** (or define locally in `main.rs` — they're 2 small serde structs). If moved to core, both CLI and LSP can share them.

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (no warnings)
- [ ] `cargo test --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb` (all pass)
- [ ] `cargo test -p smelt-cli --no-default-features --features duckdb --test example_diagnostics` (no LSP diagnostics in examples)

## Testing Strategy

1. **Unit tests**: Existing tests in `smelt-core/src/python_models.rs` and `smelt-cli/tests/test_workspace_validation.rs` continue to pass
2. **Integration test**: `example_diagnostics` test verifies Python models in example workspaces still produce correct SQL and no diagnostics
3. **Manual verification**: If a test workspace has a Python model using `find_models()`, verify it generates correct SQL in the LSP (check via diagnostics on downstream models that reference it)
4. **Cache invalidation**: Add/remove a SQL model file and verify the Python model cache invalidates and re-executes

## Risks & Mitigations

1. **CLI `build_decorator_map` uses 1-indexed lines**: After dedup to 0-indexed, CLI callers need `+1` adjustment. Verify all CLI usage sites.
2. **CLI `scan_for_model_decorators` returns line numbers**: Used in `discovery.rs` (line 121) to get decorator lines AND as boolean check. The shared version returns `Vec<u32>` (0-indexed); CLI's `discovery.rs` stores these alongside files — verify downstream usage handles the type change.
3. **Cache format change**: Adding `context_hash` to `CacheEntry` will invalidate existing caches on first run (deserialization fails → cache miss). This is fine — models re-execute once.
4. **Config loading in file-change handler**: `Config::load()` reads `smelt.yml` from disk on every Python file change. This is acceptable since file changes are infrequent (save-triggered only).
