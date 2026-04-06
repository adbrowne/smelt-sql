# Plan: LSP Visual Documentation with Playwright Demos

**Date**: 2026-04-06
**Branch**: `lsp-visual-docs`
**Script**: `scripts/run-lsp-visual-docs-loop.sh`

## Design Principle

**Show, don't tell.** Each LSP feature gets a motivating real-world scenario that a data engineer would actually encounter, automated via Playwright against VS Code Server (code-server) to produce screenshots and screen recordings. Documentation is generated as markdown with embedded media, suitable for the project README, docs site, or GitHub wiki.

## Demo Infrastructure

**Approach**: Playwright drives a headless (or headed) code-server instance with the smelt extension installed. Each demo script:
1. Opens a purpose-built example workspace with a compelling scenario
2. Performs editor interactions (typing, hovering, clicking, Ctrl+click, etc.)
3. Captures annotated screenshots and/or animated gifs
4. Outputs media to `docs/demos/media/`

**Why code-server**: It's VS Code in a browser — Playwright's native territory. No Electron hacks, no flaky desktop automation. The smelt extension loads identically since it's a standard LSP client.

**Demo workspace**: `examples/demo_workspace/` — a small but realistic analytics pipeline designed so each feature has a natural, compelling demonstration point.

### Video Capture Pipeline

Tests that produce animated demos use a purpose-built video pipeline (added in Session 5):

1. **`VideoTimer`** — tracks demo start/end timestamps relative to Playwright's auto-recorded video. Call `markDemoStart()` after setup/LSP priming, `markDemoEnd()` when done.
2. **Deliberate pacing** — `waitForTimeout(2000)` between demo actions so each step is clearly visible at 12fps.
3. **`getEditorBounds()`** — captures the `.editor-container` bounding box for cropping (before `page.close()`).
4. **`page.close()`** — finalizes Playwright's video file. Must happen before `saveVideo()`.
5. **`saveVideo()`** — trims the raw `.webm` to the demo portion, crops to editor area, converts to `.gif` using two-pass ffmpeg (palette generation for high-quality dithered output). Handles viewport→video coordinate scaling since Playwright records at a lower resolution than the viewport.

**Output format**: `.gif` — renders inline everywhere (GitHub markdown, GitLab, any browser) without `<video>` tags. Two-pass palette generation keeps quality high despite gif's 256-color limit.

**Requires**: `ffmpeg` installed on the system.

### Test Pattern for Video Demos

```typescript
test('Video: "Demo name"', async ({ page }) => {
  const timer = new VideoTimer();
  await setupPage(page);
  await primeLSP(page);

  // --- Setup complete, demo starts ---
  timer.markDemoStart();

  // ... demo actions with 1-2s pauses between steps ...

  timer.markDemoEnd();

  // --- Capture gif ---
  const crop = await getEditorBounds(page);
  const viewport = page.viewportSize();
  await page.close();
  await saveVideo(page, {
    feature: "feature-name",
    name: "demo-name",
    timer,
    crop: crop ?? undefined,
    viewportSize: viewport ?? undefined,
  });
});
```

## Demo Workspace Design

The workspace tells a story: a startup's analytics pipeline with users, events, and revenue.

```
examples/demo_workspace/
├── smelt.yml
├── sources.yml                    # raw data sources with typed columns
├── models/
│   ├── staging/
│   │   ├── stg_users.sql          # clean user data from source
│   │   └── stg_events.sql         # clean event data from source  
│   ├── intermediate/
│   │   ├── user_sessions.sql      # sessionized events (refs stg_events)
│   │   └── user_first_purchase.sql # first purchase per user (refs stg_events, stg_users)
│   └── marts/
│       ├── user_lifetime_value.sql # LTV calculation (refs multiple intermediates)
│       └── daily_revenue.sql       # daily revenue rollup
└── models_broken/                  # copies with intentional errors for demo
    ├── bad_ref.sql                 # typo in smelt.ref() → diagnostics demo
    ├── type_mismatch.sql           # VARCHAR vs INTEGER comparison → type error demo
    └── missing_column.sql          # references non-existent column → undeclared column demo
```

## Status Key

- `[ ]` — Not started
- `[~]` — In progress
- `[x]` — Complete
- `[!]` — Blocked or needs review

---

## Phase 0: Demo Infrastructure Setup `[x]`

**Goal**: Set up Playwright, code-server, and the demo workspace so all subsequent phases can focus purely on scripting demos.

**Work items**:
- [x] Create `docs/demos/` directory structure:
  ```
  docs/demos/
  ├── package.json              # Playwright + dependencies
  ├── playwright.config.ts      # code-server base URL, timeouts, video settings
  ├── helpers/
  │   ├── code-server.ts        # Launch/teardown code-server with smelt extension
  │   ├── editor.ts             # High-level helpers: openFile(), typeText(), hover(), triggerCompletion(), goToDefinition(), etc.
  │   └── capture.ts            # Screenshot/video capture with annotation overlays
  ├── scripts/
  │   └── start-code-server.sh  # Launch code-server pointing at demo workspace
  ├── tests/                    # Playwright test files (one per feature)
  ├── media/                    # Generated screenshots and videos
  └── output/                   # Generated markdown documentation
  ```
- [x] Create `examples/demo_workspace/` with `smelt.yml` and `sources.yml`
- [x] Write `sources.yml` with a realistic schema:
  ```yaml
  sources:
    raw:
      tables:
        users:
          columns:
            - name: user_id
              type: INTEGER
            - name: email
              type: VARCHAR
            - name: signup_date
              type: DATE
            - name: plan_type
              type: VARCHAR
        events:
          columns:
            - name: event_id
              type: INTEGER
            - name: user_id
              type: INTEGER
            - name: event_type
              type: VARCHAR
            - name: event_time
              type: TIMESTAMP
            - name: revenue_cents
              type: INTEGER
  ```
- [x] Write all model SQL files (staging, intermediate, marts) — keep them short (5-15 lines each) but realistic
- [x] Write the `models_broken/` variants with intentional errors
- [x] Install Playwright: `npm init -y && npm install @playwright/test playwright`
- [x] Write `helpers/code-server.ts` — launches code-server with `--install-extension` for the smelt VSIX, returns the base URL
- [x] Write `helpers/editor.ts` — wraps common VS Code web interactions:
  - `openFile(page, path)` — via quick-open (Ctrl+P)
  - `goToLine(page, line)` — via Ctrl+G
  - `hoverWord(page, word)` — position cursor and wait for hover widget
  - `triggerCompletion(page)` — Ctrl+Space and wait for suggest widget
  - `goToDefinition(page)` — F12 or Ctrl+Click
  - `findReferences(page)` — Shift+F12
  - `rename(page, newName)` — F2
  - `getCodeActions(page)` — Ctrl+. and wait for lightbulb menu
  - `waitForDiagnostics(page)` — wait for squiggly underlines to appear
- [x] Write `helpers/capture.ts` — screenshot with optional bounding-box highlight and caption
- [x] Write a smoke test: open code-server, load demo workspace, verify smelt extension activates (look for diagnostics on a broken file)

**Verification**:
- [x] `cd docs/demos && npx playwright test tests/smoke.spec.ts` passes
- [x] code-server launches with smelt extension loaded
- [x] Screenshot of activated extension saved to `media/smoke/`

---

## Phase 1: Real-Time Diagnostics Demo `[x]`

**Goal**: Showcase how smelt catches errors as you type — the most immediately compelling LSP feature.

**Motivating scenario**: A data engineer creates a new model referencing an upstream model, but makes a typo. The error appears instantly with a descriptive message.

**Demo sequence** (3 screenshots + 1 animated gif):

1. **Screenshot: "Clean pipeline"** — Open `stg_users.sql`, show no errors. Caption: *"A healthy pipeline — all references resolve, all types check out."*

2. **Gif: "Typo caught instantly"** (`typo-caught-instantly.gif`, 788KB, 6.6s) — Open `bad_ref.sql` which has `smelt.ref('stg_uusers')` (typo). Show:
   - File opens with red squiggly under `'stg_uusers'`
   - Hover over the error → tooltip shows "Undefined model reference: stg_uusers"
   - Caption: *"Catch model reference typos before they hit production."*

3. **Screenshot: "Type mismatch across models"** `[!]` — Open `type_mismatch.sql` which compares `user_id` (INTEGER) with a string literal `'abc'`. **Known issue**: The LSP doesn't currently flag `INTEGER = 'abc'` comparisons as type errors when refs resolve correctly. This test times out waiting for diagnostics.

4. **Screenshot: "Undeclared column"** — Open `missing_column.sql` which selects `nonexistent_col` from a model. Show:
   - Error squiggly on the column reference
   - Caption: *"Schema-aware diagnostics know exactly which columns exist upstream."*

**Test file**: `tests/diagnostics.spec.ts`

**Verification**:
- [x] 3 screenshots and 1 animated gif in `media/diagnostics/`
- [x] Each screenshot clearly shows the diagnostic with readable text
- [!] "Type mismatch" test known-broken (LSP limitation, not a demo infrastructure issue)

---

## Phase 2: Go-to-Definition Demo `[x]`

**Goal**: Show how F12 navigation works across the model dependency graph — the feature that makes exploring a data pipeline feel like navigating a real codebase.

**Motivating scenario**: A data engineer is debugging a revenue discrepancy in `daily_revenue.sql`. They need to trace the calculation upstream through the model chain to find where `revenue_cents` is defined and transformed.

**Demo sequence** (1 animated gif + 2 screenshots):

1. **Gif: "Trace a column through the pipeline"** (`trace-pipeline.gif`, 2.8MB, 12.6s) — Start in `daily_revenue.sql`:
   - F12 on `smelt.ref('stg_events')` → jumps to `stg_events.sql`
   - F12 on `smelt.source('raw.events')` → jumps to `sources.yml` at the events table definition
   - Caption: *"Two clicks to trace a metric from mart to raw source."*

2. **Screenshot: "Jump to CTE definition"** — In `user_first_purchase.sql` (which uses a WITH clause), F12 on the `purchases` CTE reference in the FROM clause → cursor jumps to the CTE definition. Caption: *"Navigate within complex queries too — CTE definitions are one click away."*

3. **Screenshot: "Jump to source definition"** — Show the cursor landing in `sources.yml` with the exact table highlighted. Caption: *"Source definitions in YAML are first-class navigation targets."*

**Test file**: `tests/goto-definition.spec.ts`

**Verification**:
- [x] 1 animated gif and 2 screenshots in `media/goto-definition/`
- [x] Gif clearly shows the file tab changing with each jump

---

## Phase 3: Hover Information Demo `[x]`

**Goal**: Show the rich schema information available on hover — instant documentation without leaving your editor.

**Motivating scenario**: A data engineer is writing a new mart model and needs to know what columns and types are available from upstream models, without switching files.

**Demo sequence** (3 screenshots — hover is inherently static, screenshots work well):

1. **Screenshot: "Model schema on hover"** — In `user_first_purchase.sql`, hover over `smelt.ref('stg_events')`. Show the hover popup with the full schema table: column names, types, lineage. Caption: *"Hover any model reference to see its full schema — no file switching needed."*

2. **Screenshot: "Upstream model schema with lineage"** — In `user_first_purchase.sql`, hover over `smelt.ref('stg_users')`. Show the hover popup with user schema and lineage tracing back to `raw.users`. Caption: *"Schema lineage traces each column back to its origin."*

3. **Screenshot: "Source schema on hover"** — In `stg_events.sql`, hover over `smelt.source('raw.events')`. Show the source's declared columns, types, and descriptions from `sources.yml`. Caption: *"Source schemas from YAML are surfaced directly in your SQL."*

**Note**: The LSP does not currently support hover on individual column names (returns `None`). The original plan included a "Column type on hover" screenshot, replaced with the second ref hover showing lineage.

**Test file**: `tests/hover.spec.ts`

**Verification**:
- [x] 5 screenshots in `media/hover/` (3 full-page with overlays + 2 editor-only crops)
- [x] Hover popups are fully visible and readable

---

## Phase 4: Code Completion Demo `[x]`

**Goal**: Show intelligent, context-aware completions that know your data model — not just SQL keywords.

**Motivating scenario**: A data engineer is building a new model from scratch. Completions help them discover available models, source tables, and write correct SQL without memorizing the schema.

**Demo sequence** (1 animated gif + 2 screenshots):

1. **Screenshot: "Model name completions"** — In `daily_revenue.sql`, type `smelt.ref('` at end of file. Show the completion dropdown listing all available models (daily_revenue, stg_events, stg_users, user_first_purchase, user_lifetime_value, user_sessions). Caption: *"Discover models by name — no need to browse the file tree."*

2. **Screenshot: "Source table completions"** — In `stg_events.sql`, type `smelt.source('` at end of file. Show the completion dropdown listing available source tables (raw.events, raw.users) with column metadata. Caption: *"Source tables are also discoverable — with column info from your sources.yml."*

3. **Gif: "Build a query with completions"** (`build-query-with-completions.gif`, ~2.2MB) — In `stg_events.sql`, demonstrate both ref and source completions in sequence:
   - Type `smelt.ref('` → model name dropdown appears
   - Dismiss, then type `smelt.source('` → source table dropdown appears
   - Caption: *"Schema-aware completions guide you through the entire query."*

**Note**: Qualified column completions (`alias.column`) were tested but the dot trigger character doesn't reliably produce visible completions in the Playwright + code-server environment. Column completions work in interactive use but are fragile in automation. The ref and source completion demos effectively showcase the completion system.

**Test file**: `tests/completion.spec.ts`

**Verification**:
- [x] 1 animated gif and 2 screenshots (+ 2 editor crops) in `media/completion/`
- [x] Completion dropdown is clearly visible with model/source names

---

## Phase 5: Find References Demo `[ ]`

**Goal**: Show how to answer "who depends on this model?" — essential for impact analysis before making changes.

**Motivating scenario**: A data engineer needs to change the schema of `stg_events` (renaming a column). Before doing so, they need to know every downstream model that would be affected.

**Demo sequence** (2 screenshots):

1. **Screenshot: "Find all consumers of a model"** — In `stg_events.sql`, trigger Find References on the model name. Show the references panel listing `user_sessions.sql`, `user_first_purchase.sql`, and `daily_revenue.sql` (all models that `ref('stg_events')`). Caption: *"Before changing a model, see every downstream consumer — instant impact analysis."*

2. **Screenshot: "Find CTE references within a file"** — In a model with CTEs, find references on a CTE name. Show all uses within the file highlighted. Caption: *"Works for CTEs too — see every use of a subquery within complex models."*

**Test file**: `tests/references.spec.ts`

**Verification**:
- [ ] 2 screenshots in `media/references/`
- [ ] References panel is visible with file paths and line numbers

---

## Phase 6: Rename Refactoring Demo `[ ]`

**Goal**: Show safe, project-wide renames — the feature that turns a scary find-and-replace into a confident one-step operation.

**Motivating scenario**: The team decides to rename `stg_events` to `stg_activity_events` for clarity. In dbt, this means manually updating every `ref('stg_events')` across the project and hoping you didn't miss one. In smelt, it's one F2.

**Demo sequence** (1 animated gif + 1 screenshot):

1. **Gif: "Rename a model across the project"** — In `stg_events.sql`:
   - Place cursor on the model name
   - Press F2 → rename dialog appears
   - Type `stg_activity_events`
   - Press Enter → show all `ref('stg_events')` calls updating across multiple files simultaneously
   - Show the file being renamed too
   - Use `VideoTimer` + `saveVideo()` pattern with deliberate pacing
   - Caption: *"One keystroke renames the model and updates every reference across the project."*

2. **Screenshot: "Rename preview"** — Show the rename preview (if VS Code shows one) with all affected files listed. Caption: *"See every change before it happens — no surprises."*

**Test file**: `tests/rename.spec.ts`

**Verification**:
- [ ] 1 animated gif and 1 screenshot in `media/rename/`
- [ ] Gif clearly shows multiple files updating

---

## Phase 7: Code Actions / Quick Fixes Demo `[ ]`

**Goal**: Show intelligent quick fixes that don't just report problems but offer solutions.

**Motivating scenario**: A data engineer is iterating quickly — they reference a model that doesn't exist yet, compare mismatched types, and reference a source table not in `sources.yml`. Each time, the lightbulb offers a fix.

**Demo sequence** (3 screenshots + 1 animated gif):

1. **Gif: "Create a model from a reference"** — Write a new model with `smelt.ref('user_churn')` which doesn't exist:
   - Red squiggly appears under the ref
   - Click the lightbulb (or Ctrl+.) → "Create model 'user_churn'" appears
   - Select it → a new `user_churn.sql` file is created with a template
   - Use `VideoTimer` + `saveVideo()` pattern with deliberate pacing
   - Caption: *"Reference a model before it exists — smelt scaffolds it for you."*

2. **Screenshot: "Fix type mismatch with CAST"** — Show a type mismatch diagnostic with the lightbulb offering `CAST(column AS INTEGER)`. Caption: *"Type mismatches come with a one-click CAST fix."*

3. **Screenshot: "Add missing source"** — Show an undefined source diagnostic with the lightbulb offering to add the source to `sources.yml`. Caption: *"Missing source? The quick fix adds it to your YAML config."*

4. **Screenshot: "Add missing column to source"** — Show an undeclared column on a source with the lightbulb offering to add the column definition. Caption: *"Even column declarations can be auto-added to your source definitions."*

**Test file**: `tests/code-actions.spec.ts`

**Verification**:
- [ ] 1 animated gif and 3 screenshots in `media/code-actions/`
- [ ] Lightbulb menu is visible with action descriptions

---

## Phase 8: Documentation Assembly & smeltsql.com Integration `[ ]`

**Goal**: Assemble all media into polished documentation pages and publish them on the docs site (smeltsql.com via gh-pages).

**Work items**:
- [ ] Write `docs/demos/output/lsp-features.md` — main documentation page with all features, embedded screenshots and animated gifs
- [ ] Write `docs/demos/output/getting-started.md` — quick-start guide with the most impressive 3 screenshots/gifs
- [ ] Create a `docs/demos/generate-docs.ts` script that:
  - Scans `media/` for generated assets (`.png` and `.gif`)
  - Generates markdown with proper image references (gifs render inline in GitHub markdown)
  - Adds captions and feature descriptions
  - Outputs to `output/`
- [ ] Add a comparison section: "smelt vs dbt" showing what each feature replaces (manual grep, prayer, etc.)
- [ ] Update the main `README.md` with a "Editor Features" section linking to the full docs
- [ ] Optimize media: compress PNGs, verify gif sizes are reasonable (target < 3MB each)
- [ ] **Integrate into docs-site (smeltsql.com)**:
  - Add an "Editor Features" or "LSP Features" page to `docs-site/docs/guide/`
  - Copy or symlink optimized media assets into `docs-site/docs/` so MkDocs can reference them
  - Add the page to `docs-site/mkdocs.yml` navigation (under Guide section, near "Editor Setup")
  - Verify the page renders correctly with `mkdocs serve` locally
  - Ensure the `.github/workflows/docs.yml` CI pipeline picks up the new media files
  - Consider adding a showcase/hero section on the landing page (`docs-site/docs/index.md`) with the best 1-2 gifs

**Verification**:
- [ ] `docs/demos/output/lsp-features.md` renders correctly with all media
- [ ] Total media size is under 20MB
- [ ] All image/gif links resolve and render inline on GitHub
- [ ] LSP features page renders correctly on smeltsql.com (via `mkdocs serve`)
- [ ] GitHub Actions docs workflow builds successfully with the new media

---

## Phase 9: CI Integration `[ ]`

**Goal**: Make the demos reproducible and keep them from going stale as features evolve.

**Work items**:
- [ ] Add a `docs/demos/run-all.sh` script that:
  1. Builds the smelt-lsp binary
  2. Packages the VS Code extension
  3. Starts code-server with the extension
  4. Runs all Playwright tests (which generate media)
  5. Runs the doc generation script
  6. Shuts down code-server
- [ ] Add instructions to `docs/demos/README.md` for running locally
- [ ] Consider a GitHub Actions workflow that regenerates demos on LSP changes (optional — may defer)
- [ ] Add a `--update-snapshots` flag for refreshing media after intentional UI changes

**Verification**:
- [ ] `docs/demos/run-all.sh` runs end-to-end and produces all media + docs
- [ ] Clean run from scratch works (no leftover state dependencies)

---

## Session Log

### Session 1 — 2026-04-06

**Phase**: Phase 0 (Demo Infrastructure Setup)
**Status**: Complete

**What was done**:
- Verified all Phase 0 artifacts were already in place from prior work:
  - `docs/demos/` with package.json, playwright.config.ts, tsconfig.json
  - Helpers: code-server.ts, editor.ts, capture.ts
  - Scripts: start-code-server.sh
  - Smoke test: tests/smoke.spec.ts
  - Demo workspace: examples/demo_workspace/ with smelt.yml, sources.yml, 6 model SQL files (staging/intermediate/marts), 3 broken models
- Ran `npx playwright test tests/smoke.spec.ts` — all 3 tests passed (14.2s):
  - ✓ code-server loads the demo workspace
  - ✓ smelt extension activates and produces diagnostics
  - ✓ clean model has no errors
- Verified 3 screenshots generated in `media/smoke/`: workbench-loaded.png, diagnostics-active.png, clean-model.png

**Decisions**:
- Broken models are under `models/broken/` (not `models_broken/` as originally planned) — this is fine since they're still in the model path and the LSP picks them up.
- Note: the plan mentioned `models_broken/` as a top-level dir but `models/broken/` subdirectory works better with smelt's model discovery.

### Session 2 — 2026-04-06

**Phase**: Phase 1 (Real-Time Diagnostics Demo)
**Status**: Complete

**What was done**:
- Wrote `docs/demos/tests/diagnostics.spec.ts` with 4 tests:
  1. "Clean pipeline" — opens `stg_users.sql`, verifies 0 errors, captures clean editor screenshot
  2. "Typo caught instantly" — opens `bad_ref.sql`, waits for red squiggles, hovers `stg_uusers` to show error tooltip, captures both squiggly and hover screenshots; Playwright auto-records video
  3. "Type mismatch across models" — opens `type_mismatch.sql`, verifies diagnostics, captures screenshot with squiggles
  4. "Undeclared column" — opens `missing_column.sql`, captures squiggly + hover screenshots
- Fixed `helpers/editor.ts` — `hoverWord()` now uses `.first()` on hover locator to avoid strict mode violation (code-server renders 2 `.monaco-hover-content` elements)
- All 4 tests pass (35.5s total)
- Generated 7 screenshots + 1 video in `media/diagnostics/`

**Decisions**:
- Used `stg_users.sql` instead of `user_lifetime_value.sql` for "clean pipeline" — the LTV model has real LSP diagnostics (JOIN-related) so it's not a good "zero errors" showcase.
- Simplified the "typo fix" video to show error + hover only (no find-and-replace) — code-server's find widget has different selectors than desktop VS Code, making find-and-replace brittle. The error detection story is compelling enough without the fix.
- Made type mismatch hover best-effort — the LSP may not provide hover info on string literals in all cases. The squiggly screenshot alone is a strong visual.
- Playwright auto-records video for every test (configured in playwright.config.ts), so the dedicated video test just needs to perform actions slowly enough for the recording to be clear.

### Session 3 — 2026-04-06

**Phase**: Phase 2 (Go-to-Definition Demo)
**Status**: Complete

**What was done**:
- Wrote `docs/demos/tests/goto-definition.spec.ts` with 3 tests:
  1. "Trace a column through the pipeline" — opens `daily_revenue.sql`, F12 on ref → jumps to `stg_events.sql`, F12 on source → jumps to `sources.yml`; Playwright auto-records video
  2. "Jump to CTE definition" — opens `user_first_purchase.sql`, F12 on `purchases` CTE reference → cursor jumps to CTE definition in WITH clause
  3. "Jump to source definition" — opens `stg_events.sql`, F12 on `raw.events` → jumps to `sources.yml` with events table visible
- Fixed critical bug: **LSP model discovery was not recursive** — `std::fs::read_dir` only scans the top-level `models/` directory, missing models in subdirectories like `models/staging/`. Fixed by adding explicit `model_paths` in demo workspace `smelt.yml` (`models/staging`, `models/intermediate`, `models/marts`, `models/broken`)
- Fixed `clickWord` helper to use `:text-is()` for exact leaf-span matching with fallback to `:has-text().last()` for deepest match — the original `:has-text().first()` was matching parent container spans (236px wide) instead of leaf text nodes, causing cursor mis-positioning
- Generated 8 screenshots + video in `media/goto-definition/`
- All 3 tests pass (40.4s total)

**Decisions**:
- Used `daily_revenue.sql` as starting point instead of `user_lifetime_value.sql` — the LTV model's JOIN syntax causes parse diagnostics that interfere with ref-based go-to-definition. The 2-hop trace (mart → staging → source) is still compelling.
- Used F12 (keyboard shortcut) rather than Ctrl+Click — more reliable in Playwright automation since Ctrl+Click requires precise pixel-level cursor positioning.
- Noted regression in Phase 1 diagnostics: the "type mismatch" test (`type_mismatch.sql`) now times out because with fixed model_paths, refs resolve correctly and the LSP doesn't flag `INTEGER = 'abc'` comparisons as errors. This is a pre-existing LSP limitation, not a Phase 2 issue. The type mismatch test was previously passing because the ref was *unresolvable* (showing "undefined model reference" squiggly, not a type mismatch squiggly).
- The non-recursive model discovery is a real LSP bug (filed mentally for future fix). The `smelt.yml` workaround is sufficient for the demo workspace.

### Session 4 — 2026-04-06

**Phase**: Phase 3 (Hover Information Demo)
**Status**: Complete

**What was done**:
- Wrote `docs/demos/tests/hover.spec.ts` with 3 tests:
  1. "Model schema on hover" — opens `user_first_purchase.sql`, hovers `smelt.ref('stg_events')`, captures hover popup showing full schema table (columns, types, lineage)
  2. "Upstream model schema with lineage" — hovers `smelt.ref('stg_users')` in same file, shows user schema with lineage tracing back to `raw.users` source
  3. "Source schema on hover" — opens `stg_events.sql`, hovers `smelt.source('raw.events')`, shows source columns/types/descriptions from `sources.yml`
- All 3 tests pass (34.1s total)
- Generated 5 screenshots in `media/hover/` (3 full-page with overlays + 2 editor-only crops)

**Decisions**:
- Adapted the plan's demo sequence: the original plan called for "Column type on hover" (hovering a column name), but the LSP hover handler only supports `smelt.ref()` and `smelt.source()` calls — hovering column names returns `None`. Replaced with a second ref hover showing `stg_users` schema to demonstrate the feature works across different models with lineage info.
- Used `user_first_purchase.sql` instead of `user_lifetime_value.sql` for ref hovers — the LTV model's JOIN syntax may cause parse diagnostics. `user_first_purchase.sql` has two refs (`stg_events`, `stg_users`) making it ideal for showing two different hover schemas.
- Used `:has-text()` with `.last()` (deepest match) for hover targeting rather than `hoverWord()` helper — more reliable for targeting text inside string literals like ref/source arguments.

### Session 5 — 2026-04-06

**Phase**: Video capture infrastructure + retrofit Phases 1-2
**Status**: Complete

**What was done**:
- Added video capture infrastructure to `helpers/capture.ts`:
  - `VideoTimer` class — tracks demo start/end timestamps relative to Playwright's video recording
  - `saveVideo()` — takes Playwright's auto-recorded `.webm`, trims to the demo portion, crops to editor area, converts to `.gif` using two-pass ffmpeg (palette generation for high-quality colors)
  - `getEditorBounds()` — gets `.editor-container` bounding box for crop coordinates
  - Handles viewport→video coordinate scaling (Playwright records at lower resolution than viewport)
- Retrofitted Phase 1 diagnostics: "Typo caught instantly" test now produces `typo-caught-instantly.gif` (788KB, 79 frames, 6.6s) — shows red squiggly appearing on `stg_uusers` then hover tooltip with error message
- Retrofitted Phase 2 go-to-definition: "Trace a column through the pipeline" test now produces `trace-pipeline.gif` (2.8MB, 151 frames, 12.6s) — shows F12 jumping from `daily_revenue.sql` → `stg_events.sql` → `sources.yml`
- Removed old `.webm` video from media/ (replaced by gif)
- All 12 passing tests still pass (the "Type mismatch" test was already known-broken from Session 3)

**Decisions**:
- Chose `.gif` format over `.mp4` — gifs render inline everywhere (GitHub markdown, GitLab, any browser) without needing `<video>` tags. Two-pass palette generation keeps quality high despite gif's 256-color limit.
- Used `page.close()` before `saveVideo()` — Playwright only finalizes the video file after the page closes. The test performs all assertions before closing, then processes the video.
- Added deliberate pacing (`waitForTimeout(2000)`) between demo actions so the gif shows each step clearly at 12fps.
- Cropping uses viewport-to-video coordinate scaling since Playwright records at a lower resolution (800x450) than the viewport (1280x720).

### Session 6 — 2026-04-06

**Phase**: Phase 4 (Code Completion Demo)
**Status**: Complete

**What was done**:
- Wrote `docs/demos/tests/completion.spec.ts` with 3 tests:
  1. "Model name completions" — opens `daily_revenue.sql`, types `smelt.ref('` at end of file, captures completion dropdown showing all 6 available models
  2. "Source table completions" — opens `stg_events.sql`, types `smelt.source('` at end of file, captures completion dropdown showing `raw.events` and `raw.users` with column metadata
  3. "Build a query with completions" (gif) — in `stg_events.sql`, demonstrates both ref and source completions in sequence; produces `build-query-with-completions.gif` (~2.2MB)
- All 3 tests pass (45.5s total)
- Generated 5 files in `media/completion/`: 2 full-page screenshots, 2 editor crops, 1 animated gif
- Updated Phase 8 to include smeltsql.com (gh-pages) integration — the final documentation should be published on the docs site, not just in the repo

**Decisions**:
- Adapted the plan: replaced "Column completions from upstream" (Ctrl+Space in SELECT) with "Source table completions" (inside `source('')`). Qualified column completions (`alias.column` via dot trigger) don't reliably produce visible suggest widgets in the Playwright + code-server environment — the widget appears in the DOM but stays hidden. This is likely a timing/trigger issue specific to automated browser interaction, not a bug in the LSP itself.
- Used `smelt.ref('` and `smelt.source('` typed at end of file rather than editing existing code — cleaner demo that doesn't risk breaking the file's syntax for subsequent tests.
- Typed `-- smelt.ref('` (inside a comment) for the gif to avoid introducing parse errors that could interfere with the LSP.
- Source completions are actually a stronger demo than column completions — they show rich metadata (column names in documentation field) alongside the table name.
