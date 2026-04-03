# Plan: sources.yml Live Updates in LSP

**Date**: 2026-04-03
**Research**: docs/research/2026-04-03-sources-yml-live-updates.md
**Status**: Validated

## Overview

The LSP server already has correct handling for sources.yml changes (`did_change` at main.rs:1482-1487), but VSCode never sends notifications for YAML files because the extension's `documentSelector` only matches `**/models/**/*.sql`. This plan adds sources.yml/yaml to the extension's document selector and file watchers so edits propagate to the LSP in real time.

## Current State

- `editors/vscode/src/extension.ts:112-114` — `documentSelector` only matches `**/models/**/*.sql`
- `editors/vscode/src/extension.ts:116-119` — `fileEvents` watchers only match `.sql` and `.py` in `models/`
- `crates/smelt-lsp/src/main.rs:1482-1487` — `did_change` handler correctly processes sources.yml but never fires
- `crates/smelt-lsp/src/main.rs:1524-1535` — `did_change_watched_files` only processes `.py` files

## Desired End State

When a user edits `sources.yml` in VSCode, the LSP immediately re-evaluates source schemas. Go-to-definition for newly added source columns works without reloading.

## What We're NOT Doing

- Adding new LSP server-side logic (it already exists and is correct)
- Supporting sources.yml in other editors (Neovim, JetBrains) — separate work
- Adding tests for live-update flow (would require an integration test harness for the extension)

## Implementation Phases

### Phase 1: Add sources.yml to VSCode extension document selector and file watchers

**Files to modify**:
- `editors/vscode/src/extension.ts` — Add sources patterns to `documentSelector` and `fileEvents`

**Changes**:

1. Add two entries to the `documentSelector` array at line 112-114:
   ```typescript
   documentSelector: [
       { scheme: 'file', pattern: '**/models/**/*.sql' },
       { scheme: 'file', pattern: '**/sources.yml' },
       { scheme: 'file', pattern: '**/sources.yaml' },
   ],
   ```

2. Add a file system watcher for sources.yml/yaml to the `fileEvents` array at line 116-119:
   ```typescript
   fileEvents: [
       vscode.workspace.createFileSystemWatcher('**/models/**/*.sql'),
       vscode.workspace.createFileSystemWatcher('**/models/**/*.py'),
       vscode.workspace.createFileSystemWatcher('**/sources.{yml,yaml}'),
   ]
   ```

**Verification**:
- [x] `cd editors/vscode && npm run compile` (no errors)
- [ ] Manual test: open test_workspace, edit sources.yml to add a column, verify go-to-definition works without reload

### Phase 2: Handle sources.yml in did_change_watched_files

The file watcher sends `did_change_watched_files` events (not `did_change`) when the file is modified outside the editor or saved externally. The current handler at main.rs:1524-1535 only processes `.py` files.

**Files to modify**:
- `crates/smelt-lsp/src/main.rs` — Add YAML handling to `did_change_watched_files`

**Changes**:

1. In the `did_change_watched_files` method (line 1524-1535), add a branch for sources.yml after the `.py` check:
   ```rust
   if path.extension().and_then(|s| s.to_str()) == Some("py") {
       self.handle_python_file_change(&path).await;
   } else if is_sources_file(&path) {
       // Re-read sources.yml from disk when changed outside the editor
       if let Ok(content) = std::fs::read_to_string(&path) {
           if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
               let mut db = self.db.lock().await;
               db.set_project_sources_yaml(project_root, Arc::new(content));
               drop(db);
               self.publish_all_diagnostics().await;
           }
       }
   }
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass — pre-existing duckdb lib issue in smelt-backend-duckdb unrelated)
- [x] `cargo test -p smelt-cli --test example_diagnostics`

## Testing Strategy

This is primarily a wiring fix. Verification is manual:
1. Open `examples/test_workspace/` in VSCode with the updated extension
2. Confirm go-to-definition works for existing source columns
3. Add a new column to `sources.yml`, save
4. Open a model that references that source, add the new column to a SELECT
5. Verify go-to-definition navigates to the source definition without reloading

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Adding YAML to documentSelector might cause the language client to try to parse YAML as SQL | The `did_change` handler already checks `is_sources_file()` and routes correctly — non-SQL files won't be parsed as SQL |
| File watcher for `**/sources.{yml,yaml}` is too broad (matches unrelated files) | `is_sources_file()` validates the filename; unrelated YAML files will be ignored by the handler |
