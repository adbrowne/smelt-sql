# CLAUDE.md — Playwright Demo Pipeline

This directory contains the automated demo recording system for smelt's LSP features. Playwright drives code-server to capture GIFs and screenshots that appear on the docs site.

## Quick Start

```bash
# Full pipeline: build, record, generate docs (from repo root)
bash docs/demos/run-all.sh --update-snapshots

# Run a single test suite (requires code-server already running)
CODE_SERVER_URL=http://localhost:18080 npx playwright test tests/diagnostics.spec.ts

# Run one specific test
CODE_SERVER_URL=http://localhost:18080 npx playwright test tests/smoke.spec.ts -g "produces diagnostics"
```

**Prerequisites:** code-server, ffmpeg, Node.js, Playwright browsers (`npx playwright install`)

## Pipeline (`run-all.sh`)

1. `cargo build -p smelt-lsp` — Build the LSP binary
2. Package the VS Code extension (`editors/vscode/smelt-0.1.0.vsix`)
3. `npm install` — Install Playwright and deps
4. Start code-server with isolated temp workspace and user-data-dir
5. `npx playwright test` — Run all tests, generating media
6. `npx tsx generate-docs.ts` — Assemble markdown from media

Use `--update-snapshots` to clear old media before regenerating.

## Key Architecture

- **Disposable workspace copies** — The source `examples/demo_workspace` is never modified. Each run copies it to a temp dir. Tests that create/rename files (code-actions, rename) operate on the copy.
- **Isolated user-data-dir** — Each run creates a fresh code-server user-data-dir with preset settings (no welcome page, trust disabled, chat panel off). This avoids stale state from previous sessions breaking tests.
- **Video-to-GIF conversion** — Playwright auto-records WebM video for every test. `saveVideo()` in `capture.ts` trims to the demo window and converts to GIF via ffmpeg 2-pass palette generation.

## Directory Structure

```
docs/demos/
├── run-all.sh              # End-to-end pipeline script
├── playwright.config.ts    # Playwright config (1280x720, serial, video on)
├── generate-docs.ts        # Assembles markdown from media/
├── helpers/
│   ├── code-server.ts      # Server lifecycle, workspace copy, extension install
│   ├── editor.ts           # VS Code interaction (open file, hover, goto-def, etc.)
│   └── capture.ts          # Screenshots, video trimming, GIF conversion
├── tests/
│   ├── smoke.spec.ts       # Sanity checks (workspace loads, LSP activates)
│   ├── diagnostics.spec.ts # Error detection, squiggly underlines, hover tooltips
│   ├── goto-definition.spec.ts  # F12 navigation through pipeline
│   ├── hover.spec.ts       # Schema info on hover
│   ├── completion.spec.ts  # Ctrl+Space code completions
│   ├── references.spec.ts  # Shift+F12 find references
│   ├── rename.spec.ts      # F2 rename refactoring
│   └── code-actions.spec.ts # Quick fixes (create model, cast, etc.)
├── media/                  # Generated GIFs and PNGs (committed)
│   ├── diagnostics/
│   ├── goto-definition/
│   ├── hover/
│   ├── completion/
│   ├── references/
│   ├── rename/
│   └── code-actions/
└── output/                 # Generated markdown (gitignored)
```

## Helper Modules

**`code-server.ts`** — `launchCodeServer()` creates temp workspace copy + user-data-dir, installs the VSIX, spawns code-server, waits for port readiness. `stop()` cleans up temp dirs.

**`editor.ts`** — High-level VS Code automation:
- `openFile(page, name)` — Ctrl+P quick open with retry for index readiness
- `waitForDiagnostics(page)` — Waits for squiggly underlines (LSP ready signal)
- `hoverWord(page, word)` — Hover and wait for tooltip
- `dismissDialogs(page)` — Closes welcome tabs, trust prompts, chat panel
- `enableScreencastMode(page)` — Shows keystrokes in recordings
- `runCommand(page, cmd)` — Ctrl+Shift+P command palette

**`capture.ts`** — Media output:
- `screenshotEditor(page, opts)` — Editor-only PNG (no sidebars)
- `screenshotWithOverlay(page, opts)` — Full page (captures hover tooltips)
- `VideoTimer` + `saveVideo(page, opts)` — Trim and convert video to GIF

## Asset Flow to Docs Site

After `run-all.sh` generates media, the GIFs and key screenshots must be copied to `docs-site/docs/assets/editor-features/`. See `docs-site/CLAUDE.md` for the exact copy commands. The filenames match what `docs-site/docs/guide/editor-features.md` references.

## Known Issues

- **Type mismatch test skipped** — `diagnostics.spec.ts` skips the "Type mismatch across models" test because the LSP doesn't yet flag `WHERE int_column = 'string'` comparisons. Re-enable when type checking for comparisons is implemented.
- **Code-server version sensitivity** — The tab/dialog DOM structure varies across code-server versions. `dismissDialogs()` handles known variants but may need updating after code-server upgrades.
