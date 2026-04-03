# Research: sources.yml Changes Not Reflected in LSP Until Reload

**Date**: 2026-04-03
**Topic**: Why adding columns to sources.yml doesn't update go-to-definition until VSCode is reloaded
**Branch**: main
**Commit**: 16e65d4

## Summary

The LSP server has full code to handle sources.yml changes via `did_open` and `did_change` notifications, but VSCode never sends these notifications for YAML files. The VSCode extension's `documentSelector` only matches `**/models/**/*.sql`, so the language client doesn't activate for sources.yml. Additionally, the file system watcher only watches `.sql` and `.py` files. On reload, the LSP re-reads sources.yml from disk during `initialize()`, which is why changes appear after restart.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `editors/vscode/src/extension.ts` | VSCode extension - language client config | L111-123 |
| `crates/smelt-lsp/src/main.rs` | LSP server - file change handlers | L1328-1347, L1473-1522 |
| `crates/smelt-lsp/src/main.rs` | LSP server - sources.yml handling in did_change | L1482-1487 |
| `crates/smelt-lsp/src/main.rs` | LSP server - initialization sources loading | L1120-1154 |
| `crates/smelt-db/src/lib.rs` | Salsa queries - sources_config parsing | L250-259 |

## Architecture & Data Flow

### How sources.yml is loaded at startup (works)

```
LSP initialize() → find_smelt_projects() → find_config_file("sources.yml")
  → std::fs::read_to_string(sources_path)
  → db.set_project_sources_yaml(project_root, content)
```

This happens at `main.rs:1120-1154` during initialization. The file is read directly from disk.

### How sources.yml changes should propagate (has the code, never triggered)

```
User edits sources.yml → VSCode did_change notification
  → Backend::did_change() at L1473
  → is_sources_file(&path) check at L1482 → true
  → db.set_project_sources_yaml(project_root, new_content) at L1485
  → publish_all_diagnostics() at L1487
  → Salsa re-evaluates sources_config() → updated SourceTableDef with new columns
  → Go-to-definition now resolves new columns via resolve_source()
```

### Why it fails: the notification gap

The VSCode extension configures the language client with:

```typescript
// extension.ts L112-114
documentSelector: [
    { scheme: 'file', pattern: '**/models/**/*.sql' }
],
```

This tells VSCode to only send `didOpen`/`didChange`/`didClose` text document notifications for SQL files matching `**/models/**/*.sql`. Sources.yml files:
1. Are not SQL files
2. Live at the project root, not inside `models/`

So VSCode never notifies the LSP about sources.yml changes.

### File watchers also miss sources.yml

The extension registers file system watchers for:
- `**/models/**/*.sql` (extension.ts L117)
- `**/models/**/*.py` (extension.ts L118)
- `**/models/**/*.py` again via dynamic registration (main.rs L1340)

None of these match `sources.yml` at the project root.

The `did_change_watched_files` handler (main.rs L1524-1535) also only processes `.py` files, even if a YAML change somehow arrived.

## Current Behavior

1. **On startup**: sources.yml is read from disk → go-to-definition works for existing columns
2. **User adds column to sources.yml**: LSP is never notified → Salsa still has old content → go-to-definition fails for new columns
3. **User reloads VSCode**: LSP restarts → re-reads sources.yml from disk → new columns work

## Fix

### Add sources.yml to VSCode `documentSelector`

SQL model files use `documentSelector` so the language client sends `didOpen`/`didChange` notifications with buffer content. The LSP's `did_change` handler already has the correct logic for sources.yml at main.rs L1482-1487 — it just never fires because VSCode doesn't send the notification.

The fix is to match the SQL pattern: add sources.yml/yaml to the `documentSelector` in extension.ts. Note that sources.yml lives at the project root (not inside `models/`), so the glob needs to be `**/sources.yml` and `**/sources.yaml` (not `**/models/**/sources.yml`).

## Related Patterns

### Python file watching (working example)
Python files use `did_change_watched_files` (not `did_change`) because Python models need to be executed, not just parsed. The pattern:
1. Dynamic watcher registered in `initialized()` at L1334-1347
2. `did_change_watched_files` handler at L1524-1535 calls `handle_python_file_change()`
3. Python file is re-executed and virtual SQL is updated

### did_change handler for sources.yml (exists but unreachable)
The `did_change` handler at L1482-1487 already has the correct logic for sources.yml - it just never fires because VSCode doesn't send the notification. Adding sources.yml to `documentSelector` will make it reachable.

## Test Coverage

No tests specifically cover live-update of sources.yml content. The `example_diagnostics` test reads sources.yml at initialization time (like the LSP does on startup).

## Open Questions

None — the approach matches the existing SQL file pattern.
