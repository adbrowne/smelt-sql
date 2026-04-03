# Research: LSP Python Model Integration

**Date**: 2026-04-03
**Topic**: How Python models are handled in the LSP vs CLI, and what the integration gap looks like
**Branch**: worktree-next
**Commit**: 22c1a19

## Summary

The LSP **already discovers and registers Python models** at startup and on file changes. It executes Python files via subprocess, gets the generated SQL, and registers it in Salsa as virtual `.sql` paths. The key gap is that the **LSP does not pass a `ProjectContext`** to Python models, meaning models that use `project.find_models()` receive an empty context and may generate incomplete or incorrect SQL. The CLI builds a full `ProjectContext` with model lists/tags/directories and does iterative fixed-point validation — neither of these exist in the LSP path.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/python_scan.rs` | LSP-specific Python discovery, caching, execution | L1-249 |
| `crates/smelt-lsp/src/main.rs` | LSP server: init, file watching, Python change handler | L815-1008, L1158-1221 |
| `crates/smelt-cli/src/python.rs` | CLI Python discovery with ProjectContext + fixed-point | L231-442 |
| `crates/smelt-core/src/python_models.rs` | PyO3 bridge for embedded execution | L36-145 |
| `python/smelt/core.py` | `@model` decorator, `ProjectContext`, `find_models()` | L1-29 |
| `python/smelt/runner.py` | Subprocess entry point (`python -m smelt.runner`) | L1-73 |

## Architecture & Data Flow

### CLI Flow (complete)
```
discover_python_files()                    # scan for .py with @model
  → build_project_context(sql + py models) # JSON with model names, tags, directories
  → run_python_model(file, context_json)   # subprocess or PyO3
  → parse generated SQL
  → repeat up to 5 rounds (fixed-point)    # handles models that depend on find_models()
  → validate_fixed_point()                 # ensure no circular meta-dependencies
```

### LSP Flow (partial)
```
discover_python_models(models_path, project_root, cache)  # python_scan.rs
  → walk for .py files with @model
  → check content-hash cache
  → execute_python_file(path, project_root)               # subprocess
  → create virtual .sql path: <dir>/<model_name>.sql
  → register in Salsa via set_file_text()
  → map virtual path → (source .py path, decorator_line)  # for goto-definition
```

### What the LSP does well

1. **Virtual path registration**: Python models become `<dir>/<name>.sql` in Salsa, so `resolve_ref()` finds them and they participate in type inference (`main.rs:1170-1193`).
2. **Content-hash caching**: Avoids re-executing unchanged files (`python_scan.rs:50-116`).
3. **File watching**: Registers `**/models/**/*.py` watcher, handles changes via `handle_python_file_change()` (`main.rs:815-1008`).
4. **Last-known-good fallback**: On execution failure, keeps previous SQL in Salsa (`main.rs:905-916`).
5. **Error diagnostics**: Python execution errors appear as LSP diagnostics on the `.py` file (`main.rs:870-901`).
6. **Goto-definition**: Maps virtual SQL paths back to `.py` source with decorator line (`main.rs:1188-1191`).

## Current Behavior

### Gap 1: No ProjectContext

The LSP's `execute_python_file()` in `python_scan.rs` calls the runner **without project context**. Looking at the execution:

- **CLI** (`python.rs:231-256`): `build_project_context()` constructs JSON with all model names, tags, and directories. Passed as second argument to `python -m smelt.runner`.
- **LSP** (`python_scan.rs`): Calls runner without context. Python models that call `project.find_models()` get an empty `ProjectContext` with no models listed.

**Impact**: Python models that dynamically generate SQL based on other models (e.g., union-all-tagged-models pattern) will produce incorrect/empty SQL in the LSP.

### Gap 2: No Iterative Discovery

The CLI runs up to 5 rounds of Python model execution (`python.rs:317-442`). Each round rebuilds ProjectContext with newly discovered models from previous rounds. The LSP runs Python models exactly once.

**Impact**: Python models that depend on other Python models via `find_models()` may not see all available models.

### Gap 3: No Fixed-Point Validation

The CLI's `validate_fixed_point()` (`python.rs:463-503`) ensures no Python model queries match its own output (circular meta-dependency). The LSP has no equivalent check.

**Impact**: Low risk in practice since this is a validation check, not a functional requirement.

### Gap 4: Duplicated Code

`python_scan.rs` in smelt-lsp duplicates significant logic from `python.rs` in smelt-cli:
- `has_model_decorator()` — same regex check
- `build_decorator_map()` — same algorithm (but 0-indexed in LSP vs 1-indexed in CLI)
- `find_python_sdk()` — same walk-up-to-5-levels logic
- `build_pythonpath()` — same PYTHONPATH construction
- `find_python()` — same interpreter resolution

## Related Patterns

### How SQL models flow through the LSP

SQL models are registered directly via `register_sql_content()` (`main.rs:674`). This handles both single-model and multi-model files (via `::` virtual paths). The Salsa DB then provides `parse_model()`, `model_refs()`, `file_diagnostics()`, and `type_diagnostics()` for all registered files — including Python-generated virtual SQL files.

Once a Python model's SQL is registered in Salsa, it participates fully in:
- Reference resolution (`resolve_ref()` finds it by name)
- Type inference (column types propagate to downstream models)
- Diagnostics (parse errors in generated SQL are reported)

The gap is purely in the **Python execution** step, not in downstream Salsa processing.

### `did_change()` handler skips `.py` files

`main.rs:1402` explicitly skips `.py` files for the `did_change` handler (inline edits). Python model updates only come through the file watcher path (`did_change_watched_files`), which triggers `handle_python_file_change()`. This means:
- If a user has a `.py` file open in the editor and edits it, updates only happen on disk save (when the file watcher fires), not on keystroke.

## Test Coverage

- `crates/smelt-cli/tests/test_workspace_validation.rs` — integration tests for Python model discovery via CLI path
- `crates/smelt-cli/tests/example_diagnostics.rs` — verifies example workspaces produce no LSP diagnostics (includes Python models)
- `crates/smelt-core/src/python_models.rs` — 5 unit tests for PyO3 execution
- No dedicated tests for `python_scan.rs` in the LSP crate

## Implementation Approach

The fix is to thread a real `context_json` string through the Python execution path instead of the hardcoded empty `{"models": []}`.

### Changes to `python_scan.rs`

Both `discover_python_models()` and `execute_single_python_file()` gain a `context_json: &str` parameter, replacing the hardcoded empty JSON at lines 411 and 546. The `CacheEntry` struct gets a `context_hash: String` field so cached results invalidate when the model list changes (e.g., a new SQL model is added). `PythonModelCache::get()` and `put()` updated to check/store the context hash.

### Changes to `main.rs`

A `build_python_context()` helper function builds the JSON from the `all_files` list accumulated during SQL model scanning. For each file path:
- Extract model name (handle `::` virtual paths for multi-model files)
- Get directory from parent path
- Get tags via `config.get_tags(name, None)` (config-level tags; frontmatter tag extraction deferred)
- Serialize as `{"models": [{"name": "...", "tags": [...], "directory": "..."}]}`

In the **init path** (line ~1097): retain the full `Config` object (currently only `model_paths` is extracted), build context from `all_files` before calling `discover_python_models()`.

In the **file-change path** (`handle_python_file_change`, line ~842): lock the DB, get `all_files()`, load Config, build context, pass into the blocking spawn for `execute_single_python_file()`.

### Not in scope (for now)

- **Iterative discovery**: CLI does up to 5 rounds for Python→Python `find_models()` dependencies. Single pass with all SQL models is sufficient initially.
- **Frontmatter tag extraction**: Using `config.get_tags(name, None)` gets tags from `smelt.yml`. Parsing each SQL file's frontmatter for inline tags can be added later.
- **Code deduplication**: `python_scan.rs` duplicates helpers from CLI's `python.rs`. Extracting to `smelt-core` is a separate cleanup.

## Open Questions

1. **Should `did_change()` handle `.py` files?** Currently only file-watcher events trigger re-execution. This means no live feedback while editing Python models (only on save). May be acceptable given subprocess execution cost.

2. **Future: iterative discovery in LSP?** If Python models commonly depend on other Python models via `find_models()`, a second pass could be added. Monitor whether single-pass is sufficient.
