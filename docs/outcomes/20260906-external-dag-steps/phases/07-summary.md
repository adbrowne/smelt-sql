# Phase 7 summary — github_activity dogfoods its loader as an external step

**Shipped:**
- `examples/github_activity/load_day.sh` — the one day-loader implementation, replacing
  three drifting copies. Idempotent per day via `main._loader_days`; previous-day 2%
  redelivery computed via a SQL interval (no shell date arithmetic, no `first_day` flag).
- `examples/github_activity/models/sources/raw/github_loader.yml` — `external_step:`
  declaring the loader, producing both `raw.github_events` and `..._arrival`.
- `run_incremental.py`: `load_day` → `redelivered_count` (post-run parquet read, since the
  load itself now happens inside `smelt run`); loop no longer pre-loads.
- `crates/smelt-cli/tests/github_activity_support/mod.rs`: `load_day` now shells out to
  `load_day.sh` (call sites unchanged); `replay_days` drops its pre-load, relying on the
  step.
- `.github/actions/setup-duckdb/action.yml` and `scripts/mise-setup-duckdb.sh` now also
  provision the `duckdb` CLI.
- 9 new tests: 3 in `github_activity_loader.rs` (idempotency, redelivery, table creation),
  2 in `github_activity_replay.rs` (smelt-drives-the-load, step-failure-blocks-downstream),
  1 in `list_external_step.rs` (declared step on the real fixture), plus the 3 existing
  gated suites (replay/oracle/example_workspaces) confirmed green under the rewiring.
- `examples/github_activity/README.md` documents the step as a DAG node.

**Decisions:**
- Redelivery via `DATE '{d}' - INTERVAL 1 DAY` in SQL, not shell `date` arithmetic — avoids
  GNU/BSD portability issues and the `first_day` special case entirely (out-of-range dates
  just match zero rows).
- The oracle's *direct* multi-day staging loops (building the full-refresh comparison
  side) were left untouched; only the *incremental* replay/oracle legs stopped pre-loading.
  Both paths converge safely because the loader is idempotent.
- Test 6 uses `smelt explain --json`, not `smelt list --format json` (see below).

**For the next planner:**
- **Genuine pre-existing gap, unrelated to external steps**: `smelt list --format json`
  hard-fails (`ListError::ParseErrors`) on `examples/github_activity`,
  `examples/web_analytics`, and `examples/retail_analytics` alike — `load_workspace`
  discovers root-level utility scripts (`sample.sql`, `setup_sources.sql`) as SQL models
  project-wide, and `smelt list` treats parse errors anywhere as fatal, not scoped to the
  selected set. Verified pre-existing via a temporary stash of this phase's changes.
  Someone should either scope `ListError::ParseErrors` to selection, or give
  `load_workspace` callers a way to exclude non-model root SQL. Not fixed here — orthogonal
  to this phase and could have wide blast radius.
- Phase 8 (docs-site page) and phase 9 (close-out) remain.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-cli --test github_activity_loader --test github_activity_replay --test github_activity_oracle --test list_external_step --test explain_external_step` — all pass (37+10+3+10+3... see individual runs; no failures)
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — pass
- `cargo test -p smelt-runtime --test execute_parity` — pass
- `bash .claude/scripts/large-file-check.sh` — OK, no baseline change needed
- Hardening ratchets unmoved (only test/example/CI files touched)
