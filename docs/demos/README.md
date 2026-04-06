# LSP Visual Demos

Automated screenshots and animated gifs of smelt's LSP features, driven by Playwright against code-server.

## Prerequisites

- **Rust toolchain** (to build smelt-lsp)
- **Node.js 18+**
- **code-server** (`npm install -g code-server`)
- **ffmpeg** (for gif generation from video captures)
- **Playwright browsers**: `cd docs/demos && npx playwright install chromium`

## Quick Start

From the repo root:

```bash
bash docs/demos/run-all.sh
```

This builds the LSP, packages the VS Code extension, starts code-server, runs all Playwright tests to capture media, and generates markdown documentation.

## Running Individual Tests

```bash
cd docs/demos
# Make sure code-server is already running:
bash scripts/start-code-server.sh &

# Run a single demo
npx playwright test tests/diagnostics.spec.ts
npx playwright test tests/goto-definition.spec.ts
```

## Updating Media

After intentional UI changes, clear old screenshots and regenerate everything:

```bash
bash docs/demos/run-all.sh --update-snapshots
```

This deletes the `media/` directory before running tests, so all assets are freshly captured.

## Output

- `media/` — Screenshots (`.png`) and animated gifs (`.gif`) organized by feature
- `output/` — Generated markdown documentation (`lsp-features.md`, `getting-started.md`)
- `test-results/` — Playwright test artifacts (videos, traces on failure)
- `playwright-report/` — HTML test report

## Environment Variables

- `CODE_SERVER_PORT` — Port for code-server (default: `18080`)
- `CODE_SERVER_URL` — Full URL for Playwright to connect to (default: `http://localhost:18080`)
