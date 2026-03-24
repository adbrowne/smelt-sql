# Plan: smelt-ui Dashboard Expansion

## Context

The `smelt ui` was a read-only snapshot viewer — it loaded all data at startup and served it statically. The goal is to transform it into a live, interactive dashboard that:

1. Shows all explain/type data for every model
2. Lets you plan runs interactively (preview batches, time ranges, model selection)
3. Lets you trigger and monitor runs in real-time
4. Auto-updates when model files change on disk (leveraging Salsa like the LSP)
5. Works on mobile (responsive layout)

## Architecture

**Before**: CLI builds 3 static response structs at startup → passes to `start_server()` → Axum serves immutable `Arc<AppState>`

**After**: CLI passes live `Database` + `Config` + `project_dir` to `start_server()` → file watcher updates Salsa inputs on disk changes → API handlers query Salsa live → WebSocket pushes change/run events to browser

Key pattern: the LSP (`crates/smelt-lsp/src/main.rs`) already wraps `Database` in `Arc<Mutex<Database>>` with `set_file_text()` on changes. The UI backend replicates this for the file watcher.

## Phases

### ✅ Phase 1: Live Backend + WebSocket Foundation (March 24, 2026)

Replaced static snapshot with live Salsa queries and file watching.

- `AppState` holds live `Database` + `DependencyGraph` behind `tokio::sync::Mutex`
- File watcher (`notify-debouncer-mini`) monitors model dirs + config files, 200ms debounce
- On change: re-discovers models, updates Salsa via `set_file_text()`, rebuilds graph
- WebSocket `/ws` pushes `ChangeEvent::ModelsUpdated` to all clients
- Frontend `useWebSocket` hook auto-reconnects and invalidates React Query caches

### ✅ Phase 2: Explain Data + Full Types (March 24, 2026)

Surfaced all model metadata, incremental config, batch safety, and type information.

- `ModelDetailResponse` extended with `incremental`, `batch_safety`, `diagnostics`
- `build.rs` computes batch safety via `analyze_batch_safety()` from smelt-optimizer
- Diagnostics pulled from Salsa's `file_diagnostics()`
- Frontend: incremental config section, batch safety badges (green/yellow/red), diagnostics panel, nullable column indicator

### ✅ Phase 3: Run Planner — Preview (March 24, 2026)

Interactive UI to configure run options and preview execution plan.

- `POST /api/run/plan` accepts time range, batch size, per-partition, model selection
- `build_run_plan()` computes plan using batch safety analysis, generates batches
- Frontend: date inputs, batch size override, per-partition toggle, model selector chips
- Plan preview table with expandable per-batch detail rows
- Navigation tabs: Graph | Run Planner

### 🔮 Phase 4: Run Execution + Monitoring

Execute runs from the UI with real-time progress streaming via WebSocket.

**New: `RunManager`** (`crates/smelt-ui/src/run_manager.rs`):
- Singleton with `Mutex<RunState>` (Idle | Running)
- On execute: spawns tokio task, runs the model execution loop (adapted from `crates/smelt-cli/src/main.rs` run flow)
- Streams `RunProgressEvent` through broadcast channel → WebSocket → browser
- Cancellation via `CancellationToken` checked between batches
- Only one run at a time (returns 409 if busy)

**Events**: `RunStarted`, `ModelStarted`, `BatchCompleted`, `ModelCompleted`, `RunCompleted`, `RunFailed`, `RunCancelled`

**New API endpoints:**
- `POST /api/run/execute` — start run (same body as plan)
- `POST /api/run/cancel` — cancel current run
- `GET /api/run/status` — current state (idle/running)
- `GET /api/runs` — run history from `smelt-state` FileStore
- `GET /api/runs/{id}` — single run manifest

**Frontend:**
- `useRunStatus` hook subscribing to WebSocket run progress events
- `RunProgress` component: current model, batch N/M, row counts, elapsed time, progress bars
- `RunHistory` page: table of past runs

### 🔮 Phase 5: Mobile Responsive + Polish

**Layout changes (Tailwind breakpoints):**
- Graph + detail panel: stack vertically on mobile (`flex-col`), side-by-side on desktop (`md:flex-row`)
- ModelDetail: bottom sheet on mobile (slide up), fixed side panel on desktop
- Navigation: bottom tabs on mobile, top bar on desktop
- Run planner: stack inputs vertically on mobile
- React Flow: enable touch gestures, hide minimap on mobile

**Polish:**
- Dark mode (Tailwind `dark:` classes + toggle)
- Loading skeletons
- Toast notifications for events (model updated, run completed)

### 🔮 Phase 6: Interval Status + Gap Visualization

**New API:**
- `GET /api/status` — interval coverage for all incremental models (from `IntervalStore`)
- `GET /api/status/{model}` — detailed intervals + gaps

**Frontend:**
- Status page showing coverage heatmap/timeline per model
- Gap detection with "fill gap" action (pre-fills run planner)
- Link from run history to interval updates

## Key Files Reference

| File | Role |
|------|------|
| `crates/smelt-ui/src/server.rs` | AppState, routes, WebSocket, server startup |
| `crates/smelt-ui/src/api.rs` | All API handlers |
| `crates/smelt-ui/src/build.rs` | Build responses from Salsa/config, run plan computation |
| `crates/smelt-ui/src/types.rs` | All serializable request/response types |
| `crates/smelt-ui/src/watcher.rs` | File watcher, Salsa refresh, graph rebuild |
| `crates/smelt-cli/src/main.rs` | `ui()` function (startup) |
| `crates/smelt-optimizer/src/rules/incremental.rs` | `analyze_batch_safety()` |
| `crates/smelt-lsp/src/main.rs` | Reference for Salsa + file watching pattern |
| `ui/src/App.tsx` | Frontend entry, navigation, WebSocket |
| `ui/src/components/ModelDetail.tsx` | Model detail panel with types/diagnostics |
| `ui/src/pages/RunPlanner.tsx` | Run planner page |
| `ui/src/hooks/useWebSocket.ts` | WebSocket connection + auto-reconnect |
