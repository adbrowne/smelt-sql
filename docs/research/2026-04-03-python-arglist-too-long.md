# Research: Python Model "Argument list too long" Error

**Date**: 2026-04-03
**Topic**: LSP errors like `Python model error in .../py_l4_387.py: Failed to run Python: Argument list too long (os error 7)`
**Branch**: main
**Commit**: 5f5b347

## Summary

Python models are executed via subprocess (`python -m smelt.runner <file> <context_json>`), where `context_json` is a JSON string containing all known model names passed as a **command-line argument**. In the `examples/huge` workspace (2000 models), this JSON is ~114 KB, which approaches the Linux `MAX_ARG_STRLEN` limit of 128 KB per single argument. Combined with environment variables, this triggers `E2BIG` (os error 7). Both the LSP and CLI share this pattern.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/python_scan.rs` | Subprocess execution of Python models | L141-193 |
| `crates/smelt-lsp/src/main.rs` | Builds context JSON and calls scan | L570-600, L882-896, L1228-1236 |
| `crates/smelt-cli/src/python.rs` | CLI's equivalent subprocess execution | L47-59 |
| `python/smelt/runner.py` | Python entry point that reads context from `sys.argv[2]` | L24-27 |
| `crates/smelt-core/src/python_utils.rs` | Shared helpers (find_python, build_pythonpath) | L82-92 |

## Architecture & Data Flow

### Python Model Execution Flow

1. **Context building** (`main.rs:570` `build_python_context()`):
   - Reads `all_files()` from Salsa DB (all registered model paths)
   - For each path, extracts model name (file stem or virtual path segment after `::`)
   - Extracts directory from parent path's file name
   - Gets tags from `Config::get_tags()`
   - Serializes as `{"models": [{"name": "...", "tags": [...], "directory": "..."}]}`

2. **Subprocess invocation** (`python_scan.rs:148-153`):
   ```rust
   Command::new(python)
       .arg("-m")
       .arg("smelt.runner")
       .arg(file_path)
       .arg(context_json)     // <-- the large argument
       .env("PYTHONPATH", pythonpath)
       .output()
   ```

3. **Python consumption** (`runner.py:25`):
   ```python
   project_json = sys.argv[2]
   ```

### Two Call Sites in LSP

- **Initial scan** (`main.rs:1228`): `build_python_context()` is called with `all_files` containing SQL models only (~1000 in `huge`). Context is ~54 KB. This likely succeeds.
- **File-change handler** (`main.rs:882`): `build_python_context()` is called with `all_files` containing SQL + Python virtual paths (~2000 in `huge`). Context is ~114 KB. This is near the limit.

## Current Behavior

### Size Analysis for `examples/huge` (2000 models)

| Metric | Value |
|--------|-------|
| SQL model files | 1000 |
| Python model files | 1000 |
| Context JSON (1000 models, initial scan) | ~54 KB |
| Context JSON (2000 models, after init) | ~114 KB |
| Linux `MAX_ARG_STRLEN` (single arg limit) | 128 KB |
| Linux `ARG_MAX` (total args + env) | ~2 MB (1/4 stack) |

The ~114 KB context JSON is 89% of the 128 KB single-argument limit. Any additional overhead (longer paths, extra model metadata, env vars) can push it over.

### Error Path

When `Command::new(...).output()` returns `Err(e)` at `python_scan.rs:169`, the error is formatted as `"Failed to run Python: {e}"` and collected into `PythonModelError`. These errors are then surfaced as LSP diagnostics (warning severity) at `main.rs:1267-1290` and logged via the client.

### Caching Interaction

The cache (`PythonModelCache`) keys on `(file_path, content_hash, context_hash)`. When context changes (e.g., new models added), the context hash changes, invalidating all cached entries. This means that after adding a new model, ALL Python files are re-executed, each receiving the full (larger) context as a command-line argument.

## Related Patterns

- **CLI** (`smelt-cli/src/python.rs:54-58`): Uses the exact same subprocess pattern with `context_json` as argv. Subject to the same limit.
- **PyO3 path** (`python_scan.rs:197-239`): When compiled with `feature = "python"`, uses embedded Python via PyO3 instead of subprocess. This path passes `context_json` as a function parameter, so it is **not affected** by the `E2BIG` limit.

## Test Coverage

- `python_scan.rs:470-514`: Unit tests for `content_hash` and `PythonModelCache` put/get. No tests for the subprocess execution path or large-context scenarios.
- `examples/huge/`: The 2000-model stress-test workspace exists but there are no specific tests that verify Python model execution at this scale.

## Fix Applied

Two changes were made to eliminate the E2BIG error:

### 1. PyO3 as default for smelt-lsp

`crates/smelt-lsp/Cargo.toml` now has `default = ["python"]`, so the LSP uses the embedded PyO3 interpreter by default. This path passes `context_json` as a Rust function parameter — no subprocess, no OS argument limits.

### 2. Subprocess path uses stdin instead of argv

For the non-PyO3 fallback (used when built without `python` feature), both `python_scan.rs` (LSP) and `python.rs` (CLI) now pass context JSON via stdin instead of as a command-line argument. `runner.py` accepts both modes:
- **2 args** (`<file>`): reads context from stdin (preferred)
- **3 args** (`<file> <context_json>`): legacy argv mode (backward compatible)

### Files changed

| File | Change |
|------|--------|
| `crates/smelt-lsp/Cargo.toml` | Added `default = ["python"]` |
| `crates/smelt-lsp/src/python_scan.rs` | Subprocess uses `spawn()` + stdin pipe; fixed `with_gil` → `attach` deprecation |
| `crates/smelt-cli/src/python.rs` | Subprocess uses `spawn()` + stdin pipe |
| `python/smelt/runner.py` | Accept context via stdin when only 2 args provided |

## Open Questions

1. **Pre-existing PyO3 test failures**: 3 tests in `smelt-core::python_models` fail with `TypeError("'dict_items' object cannot be converted to 'Sequence'")` — unrelated to this change, existed before.
2. **Environment contribution**: The inherited environment variables contribute to the total arg+env size. A user with a large environment could hit the limit at a lower context size (now mitigated by stdin).
3. **Worktree path length**: The user's worktree path (`/home/andrew/smelt-sql/.claude/worktrees/gtd/...`) adds length to the file_path argument but not significantly to the context_json (which only stores model names and directory names, not full paths).
