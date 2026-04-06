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
3. Captures annotated screenshots and/or short video clips
4. Outputs media to `docs/demos/media/`

**Why code-server**: It's VS Code in a browser — Playwright's native territory. No Electron hacks, no flaky desktop automation. The smelt extension loads identically since it's a standard LSP client.

**Demo workspace**: `examples/demo_workspace/` — a small but realistic analytics pipeline designed so each feature has a natural, compelling demonstration point.

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

## Phase 1: Real-Time Diagnostics Demo `[ ]`

**Goal**: Showcase how smelt catches errors as you type — the most immediately compelling LSP feature.

**Motivating scenario**: A data engineer creates a new model referencing an upstream model, but makes a typo. The error appears instantly with a descriptive message. They also see type mismatches caught across model boundaries.

**Demo sequence** (3 screenshots + 1 short video):

1. **Screenshot: "Clean pipeline"** — Open `user_lifetime_value.sql`, show no errors. Caption: *"A healthy pipeline — all references resolve, all types check out."*

2. **Video: "Typo caught instantly"** — Open `bad_ref.sql` which has `smelt.ref('stg_uusers')` (typo). Show:
   - File opens with red squiggly under `'stg_uusers'`
   - Hover over the error → tooltip shows "Undefined model reference: stg_uusers"
   - Fix the typo to `'stg_users'` → squiggly disappears in real-time
   - Caption: *"Catch model reference typos before they hit production."*

3. **Screenshot: "Type mismatch across models"** — Open `type_mismatch.sql` which compares `user_id` (INTEGER) with a string literal `'abc'`. Show:
   - Yellow/red squiggly on the comparison
   - Hover tooltip showing the type mismatch details
   - Caption: *"Cross-model type checking catches mismatches that dbt can't."*

4. **Screenshot: "Undeclared column"** — Open `missing_column.sql` which selects `nonexistent_col` from a model. Show:
   - Error squiggly on the column reference
   - Caption: *"Schema-aware diagnostics know exactly which columns exist upstream."*

**Test file**: `tests/diagnostics.spec.ts`

**Verification**:
- [ ] 3 screenshots and 1 video in `media/diagnostics/`
- [ ] Each screenshot clearly shows the diagnostic with readable text

---

## Phase 2: Go-to-Definition Demo `[ ]`

**Goal**: Show how Ctrl+Click navigation works across the model dependency graph — the feature that makes exploring a data pipeline feel like navigating a real codebase.

**Motivating scenario**: A data engineer is debugging a revenue discrepancy in `daily_revenue.sql`. They need to trace the calculation upstream through the model chain to find where `revenue_cents` is defined and transformed.

**Demo sequence** (1 video + 2 screenshots):

1. **Video: "Trace a column through the pipeline"** — Start in `daily_revenue.sql`:
   - Ctrl+Click on `smelt.ref('user_first_purchase')` → jumps to `user_first_purchase.sql`
   - Ctrl+Click on `smelt.ref('stg_events')` → jumps to `stg_events.sql`
   - Ctrl+Click on `smelt.source('raw.events')` → jumps to `sources.yml` at the events table definition
   - Caption: *"Three clicks to trace a metric from dashboard to raw source."*

2. **Screenshot: "Jump to CTE definition"** — In `user_sessions.sql` (which uses a WITH clause), Ctrl+Click on a CTE reference in the final SELECT → cursor jumps to the CTE definition. Caption: *"Navigate within complex queries too — CTE definitions are one click away."*

3. **Screenshot: "Jump to source definition"** — Show the cursor landing in `sources.yml` with the exact table highlighted. Caption: *"Source definitions in YAML are first-class navigation targets."*

**Test file**: `tests/goto-definition.spec.ts`

**Verification**:
- [ ] 1 video and 2 screenshots in `media/goto-definition/`
- [ ] Video clearly shows the file tab changing with each jump

---

## Phase 3: Hover Information Demo `[ ]`

**Goal**: Show the rich schema information available on hover — instant documentation without leaving your editor.

**Motivating scenario**: A data engineer is writing a new mart model and needs to know what columns and types are available from upstream models, without switching files.

**Demo sequence** (3 screenshots):

1. **Screenshot: "Model schema on hover"** — In `user_lifetime_value.sql`, hover over `smelt.ref('user_first_purchase')`. Show the hover popup with the full schema: column names, types, nullability. Caption: *"Hover any model reference to see its full schema — no file switching needed."*

2. **Screenshot: "Column type on hover"** — Hover over a column name like `revenue_cents` in a SELECT. Show type information (INTEGER). Caption: *"Every column carries its type — hover to inspect."*

3. **Screenshot: "Source schema on hover"** — Hover over `smelt.source('raw.events')`. Show the source's declared columns and types from `sources.yml`. Caption: *"Source schemas from YAML are surfaced directly in your SQL."*

**Test file**: `tests/hover.spec.ts`

**Verification**:
- [ ] 3 screenshots in `media/hover/`
- [ ] Hover popups are fully visible and readable

---

## Phase 4: Code Completion Demo `[ ]`

**Goal**: Show intelligent, context-aware completions that know your data model — not just SQL keywords.

**Motivating scenario**: A data engineer is building a new model from scratch. Completions help them discover available models, columns, and write correct SQL without memorizing the schema.

**Demo sequence** (1 video + 2 screenshots):

1. **Video: "Build a query with completions"** — Create a new empty model file, then:
   - Type `SELECT ` then trigger completion → see column names from context
   - Type `FROM smelt.ref('` → see all available model names as completions
   - Select `stg_users` from the list
   - On the next line, type a column name prefix → see matching columns from `stg_users` schema
   - Caption: *"Schema-aware completions guide you through the entire query."*

2. **Screenshot: "Model name completions"** — Inside `ref('`, show the completion dropdown listing all available models with their paths. Caption: *"Discover models by name — no need to browse the file tree."*

3. **Screenshot: "Column completions from upstream"** — After a FROM clause referencing a model, trigger completion in SELECT to show columns pulled from that model's schema. Caption: *"Completions know the upstream schema — you get the right columns, not just SQL keywords."*

**Test file**: `tests/completion.spec.ts`

**Verification**:
- [ ] 1 video and 2 screenshots in `media/completion/`
- [ ] Completion dropdown is clearly visible with model/column names

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

**Demo sequence** (1 video + 1 screenshot):

1. **Video: "Rename a model across the project"** — In `stg_events.sql`:
   - Place cursor on the model name
   - Press F2 → rename dialog appears
   - Type `stg_activity_events`
   - Press Enter → show all `ref('stg_events')` calls updating across multiple files simultaneously
   - Show the file being renamed too
   - Caption: *"One keystroke renames the model and updates every reference across the project."*

2. **Screenshot: "Rename preview"** — Show the rename preview (if VS Code shows one) with all affected files listed. Caption: *"See every change before it happens — no surprises."*

**Test file**: `tests/rename.spec.ts`

**Verification**:
- [ ] 1 video and 1 screenshot in `media/rename/`
- [ ] Video clearly shows multiple files updating

---

## Phase 7: Code Actions / Quick Fixes Demo `[ ]`

**Goal**: Show intelligent quick fixes that don't just report problems but offer solutions.

**Motivating scenario**: A data engineer is iterating quickly — they reference a model that doesn't exist yet, compare mismatched types, and reference a source table not in `sources.yml`. Each time, the lightbulb offers a fix.

**Demo sequence** (3 screenshots + 1 video):

1. **Video: "Create a model from a reference"** — Write a new model with `smelt.ref('user_churn')` which doesn't exist:
   - Red squiggly appears under the ref
   - Click the lightbulb (or Ctrl+.) → "Create model 'user_churn'" appears
   - Select it → a new `user_churn.sql` file is created with a template
   - Caption: *"Reference a model before it exists — smelt scaffolds it for you."*

2. **Screenshot: "Fix type mismatch with CAST"** — Show a type mismatch diagnostic with the lightbulb offering `CAST(column AS INTEGER)`. Caption: *"Type mismatches come with a one-click CAST fix."*

3. **Screenshot: "Add missing source"** — Show an undefined source diagnostic with the lightbulb offering to add the source to `sources.yml`. Caption: *"Missing source? The quick fix adds it to your YAML config."*

4. **Screenshot: "Add missing column to source"** — Show an undeclared column on a source with the lightbulb offering to add the column definition. Caption: *"Even column declarations can be auto-added to your source definitions."*

**Test file**: `tests/code-actions.spec.ts`

**Verification**:
- [ ] 1 video and 3 screenshots in `media/code-actions/`
- [ ] Lightbulb menu is visible with action descriptions

---

## Phase 8: Documentation Assembly `[ ]`

**Goal**: Assemble all media into polished documentation pages.

**Work items**:
- [ ] Write `docs/demos/output/lsp-features.md` — main documentation page with all features, screenshots, and video embeds
- [ ] Write `docs/demos/output/getting-started.md` — quick-start guide with the most impressive 3 screenshots
- [ ] Create a `docs/demos/generate-docs.ts` script that:
  - Scans `media/` for generated assets
  - Generates markdown with proper image/video references
  - Adds captions and feature descriptions
  - Outputs to `output/`
- [ ] Add a comparison section: "smelt vs dbt" showing what each feature replaces (manual grep, prayer, etc.)
- [ ] Update the main `README.md` with a "Editor Features" section linking to the full docs
- [ ] Optimize media: compress PNGs, convert videos to GIF where short enough, ensure reasonable file sizes

**Verification**:
- [ ] `docs/demos/output/lsp-features.md` renders correctly with all media
- [ ] Total media size is under 20MB
- [ ] All image/video links resolve

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
