# Phase 11e summary (attempt 1) — smoke run fixed four ambient-session bugs, blocked on a fifth

**Status: blocked.** Task 1 (offline test scaffold) done and committed. Task 2 (`dbx-verify.sh`)
green. Tasks 3-4 (deploy, seed, manual smoke run) done, and the smoke run itself did its job:
it surfaced and let us fix four real, load-bearing ambient-session bugs before any cadence
compression. Task 4 then hit a fifth, unrelated defect (wheel platform incompatibility) that
stops `smelt_run` from ever installing smelt on Databricks serverless compute. Tasks 5-13 not
reached.

## Shipped

- `python/smelt/databricks_adapter.py` — four fixes to the ambient (`host=None`) session path:
  `.serverless(True)` moved inside `if host:` (was forcing the wrong `databricks-connect`
  builder branch unconditionally); `setCurrentCatalog`/`setCurrentDatabase` deleted (every caller
  already fully qualifies `catalog.schema.table`, and both raised `NO_ACTIVE_SESSION` on the
  ambient session specifically); `table_exists` reimplemented via `information_schema.tables`
  instead of `spark.catalog.tableExists` (same RPC-class failure); `close()` now calls
  `.stop()` only when `self.host` is set (the ambient session is the job runtime's own shared
  session, not something this adapter built — stopping it broke every subsequent `_connect()`
  in the same process).
- `examples/github_activity/dbx_job/load_next_day.py` — switched from `subprocess.run` to an
  in-process `importlib` load + direct `cmd_next_day()` call (a serverless job task's ambient
  Connect channel is bound to the exact process Databricks launches, not inheritable by a child
  process).
- `scripts/dbx-dogfood-loader.py` — `duckdb_query_arrow` normalizes a `RecordBatchReader` (what
  the serverless environment's duckdb/pyarrow pin returns) to a `Table` before
  `pa.ipc.new_stream(...).write_table(...)`, which only accepts a `Table`.
- `crates/smelt-cli/tests/dbx_dogfood_loader.rs` — `adapter_omits_host_builder_call_when_ambient`
  corrected (it asserted the old, wrong `.serverless(True)`-always behavior); two new tests,
  `close_stops_only_a_session_this_adapter_built` and
  `duckdb_query_arrow_normalizes_a_record_batch_reader`.
- `crates/smelt-cli/tests/github_activity_dbx_scheduled.rs` — new file, criterion 11's report-
  driven gates (task 1 of the plan): three tests reading not-yet-committed
  `phases/11e-runs.json`/`11e-equivalence.json`, skipping until a future phase lands them.
- `crates/smelt-cli/tests/parity_support/mod.rs` — `EQUIVALENCE_DIVERGENCE_REGISTRY` extracted
  from `github_activity_dbx_oracle.rs` into a shared `DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY`
  const, so the new scheduled-run suite reuses the same registry rather than restating it.
- `.claude/large-file-baseline.txt` — `dbx_dogfood_loader.rs` re-baselined (1087 → 1210 lines),
  the new tests' legitimate growth.

## Decisions

- All five bugs were fixed via live iteration (each redeploy+run round trip took ~40-60s), not
  deferred, because they block the smoke run's entire purpose. See `outcome.md` `## Blocked`'s
  phase 11e entry for the full root-cause chain and why bugs 3 and 4 were initially easy to
  conflate (same `[NO_ACTIVE_SESSION]` signature, different triggers).
- The wheel-compatibility blocker (bug/defect 5, unrelated to the ambient-session class) was
  **not** fixed here: it needs a manylinux-compliant build environment (Docker or `maturin build
  --zig`), which is an infrastructure decision — which manylinux floor to target, Docker vs.
  zig, CI availability — not a patch. Confirmed by testing `maturin build --compatibility
  manylinux_2_28` directly: it failed, naming the actual offending glibc symbol versions
  (`GLIBC_2.29`-`2.39` in `libc.so.6`/`libm.so.6`), proving the binary genuinely needs an
  older-glibc build host rather than just a wrong tag to relabel.
- The three new `github_activity_dbx_scheduled.rs` tests skip rather than hard-fail when their
  evidence files are absent, matching phase 9b's precedent (`the_committed_equivalence_report_
  shows_no_violation` etc. were "NOT landed" until evidence existed) — committing them as
  unconditional hard gates would have left `cargo test` red on `main` for a fact this phase
  didn't finish measuring.

## For the next planner

- **The wheel-compatibility blocker needs its own offline row** before resuming 11e's live legs:
  decide the manylinux floor (likely `manylinux_2_28`, a common serverless-compatible baseline —
  verify against Databricks' own documented supported tags rather than assuming), then either
  add a Docker-based build step or evaluate `maturin build --zig` cross-compilation to
  `databricks.yml`'s `smelt_wheel` artifact `build:` command, gated offline (a structural test
  that the build command targets the chosen manylinux floor) plus a live confirmation that the
  produced wheel actually installs on serverless compute.
- **Workspace state**: 12 fixture days now loaded (`2026-08-05`..`2026-08-16`, `github_events`
  at 40,911 rows) — one more than 9f's committed 11-day end state. The next live phase's `--var
  schedule_cron` compressed-cadence run (task 5 onward) should account for this rather than
  assume 9f's state, though it does not block anything since the loader's own `--next-day` logic
  reads the live ledger, not a hardcoded day count.
- **11d's own offline gate had a real blind spot**: `adapter_omits_host_builder_call_when_ambient`
  passed while asserting the wrong behavior, because its stub builder recorded calls without
  modelling `databricks.connect.session`'s actual branching logic (any of `.serverless`/`.host`/
  `.token` forces the same non-ambient `Config()` path). A stub that faithfully reproduces a
  library's control flow, not just its call surface, would have caught bug 1 offline — worth
  keeping in mind for any future `databricks-connect` stub.
- Once the wheel is fixed, resume exactly at 11e-plan.md's task 4 (the smoke run) — tasks 1-3 are
  done and stay done.
- Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `cargo test -p smelt-cli --test dbx_dogfood_loader --test databricks_bundle --test
  github_activity_dbx_scheduled` — all green (the three scheduled-run tests skip, printing why).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — stays green (18/18) after the
  registry extraction into `parity_support`.
- `bash scripts/dbx-verify.sh` — green (workspace reachable, both schemas visible, out-of-scope
  `CREATE SCHEMA` correctly refused).
- `bash scripts/dbx-bundle.sh validate` — green.
- `databricks bundle deploy` (live, six iterations across the five bugs) — green each time.
- `bash scripts/dbx-bundle.sh run github_activity_daily` (live) — `load_next_day` succeeded after
  bug 4's fix; `smelt_run` fails on the wheel blocker.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`; large-file ratchet re-baselined).
