# Phase 2 summary — The `databricks` backend, offline

**Shipped:**
- `BackendType::Databricks` (`smelt-core/src/config.rs`); `Target` gains `host`/`token`
  fields, a hand-written redacting `Debug` impl, and a `serialize_with = "redact_token"`
  serde field so no rendering path leaks a resolved token.
- `Config::validate_targets()` — post-parse: `databricks` requires `host` (bare hostname,
  no scheme/trailing slash) and hard-errors on the eight foreign keys
  (`connect_url`/`warehouse`/`format`/`database`/`settings`/`project`/`dataset`/`location`),
  table-driven via `DATABRICKS_FOREIGN_KEYS`.
- `check_literal_secrets()` — pure pre-interpolation pass over raw YAML: a `databricks`
  `token` must be exactly `${VAR}`; wired into `Config::load` before `interpolate_env_vars`.
- `BackendCapabilities::databricks()` (`smelt-dialect/src/dialect.rs`) = `spark_delta()`
  verbatim; `capability_conformance.rs` extended with a full Databricks column.
- `crates/smelt-backend-spark/src/session.rs` (new) — pure `SparkFlavor` enum and
  `plan_session()` selecting `smelt.spark_adapter.SparkAdapter` vs
  `smelt.databricks_adapter.DatabricksAdapter` + args, no interpreter started.
- `SparkBackend::new_databricks(host, token, catalog, schema)`; `flavor` field drives
  `capabilities()` and `materialized_path()` (always `None` for Databricks — no warehouse).
- `python/smelt/databricks_adapter.py` (new) — mirrors `SparkAdapter`'s method surface,
  builds `DatabricksSession.builder.host(...).serverless(True)`, `.token(...)` only when
  supplied.
- `smelt-backends::create_backend` dispatches `BackendType::Databricks` behind a new
  `databricks` cargo feature (pulls `smelt-backend-spark`, no new crate); catalog defaults
  to `workspace`.
- `refuse_databricks_cross_edges()` in `smelt-runtime/src/execute/project/mod.rs` — a
  cross-backend edge touching a `databricks` target on either side fails loud naming both
  targets, before the `read_parquet()` substitution loop (which previously silently dropped
  a ref when `materialized_path()` returned `None`).

**Decisions:**
- `BackendCapabilities::databricks()` is `Self::spark_delta()` with no field overrides —
  every flag the plan named (`null_safe_equality: Spaceship`, `supports_native_ivm: false`,
  `dialect: SparkSQL`) already matches Spark(Delta) exactly, so there was nothing to diverge.
- The Databricks Python constructor takes `(host, catalog, token)` positionally; `new_databricks`
  builds the plan via `plan_session` then calls the Python class with `plan_token.as_deref()`
  so `None` crosses the PyO3 boundary as Python `None` (ambient-credential form).

**For the next planner:**
- ~81 pre-existing `Target { .. }` struct literals across the workspace needed `host`/`token`
  fields added — done via a scripted regex pass (77 sites) plus 4 manual `BackendType` match
  arms (`compile.rs`, `execute/targets.rs` x2, `smelt-ui/src/build.rs`, `s_tracker.rs`,
  `smelt-cli/src/commands/explain.rs`, `smelt-ui/tests/api.rs`). Grep for `BackendType::` if a
  future backend variant is added — the compiler finds every site but there are many.
  `crates/smelt-core/src/config.rs` grew past its large-file baseline (3909→4304 lines) from the
  new tests; baseline updated via `--update`, flagged here rather than split since the tests are
  cohesive with the `Target`/`Config` module they exercise.
- Phase 3 (loader/venv/`dbx-dogfood-env.sh`) can now assume `type: databricks` parses, dispatches,
  and refuses cleanly — no workspace touched by anything in this phase.
- Not done (out of phase-2 scope, left for live phases): no live Databricks Connect session has
  ever been opened; `databricks-connect`'s actual Python API surface (method names, `.serverless()`
  availability, exception shapes) is unverified against the real package — phase 3/5 should sanity
  check `python/smelt/databricks_adapter.py` imports cleanly against a pinned `databricks-connect`
  venv before the first live run.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, example_diagnostics).
- `cargo test -p smelt-dialect --test capability_conformance --quiet` — 2 passed.
- `cargo test -p smelt-core --lib config --quiet` — 103 passed.
- `cargo test -p smelt-backend-spark --quiet` — 31+5+1+1 passed (incl. new `session` module tests).
- `cargo test -p smelt-core --test hardening_budget --quiet` — 5 passed, no baseline change.
- `rg -n 'databricks' crates/ --type rust -l` — no `SMELT_DATABRICKS_HOST`/`_TOKEN` env read
  anywhere; confirmed via separate grep.
- `.claude/scripts/large-file-check.sh --update` run once (legitimate growth from new tests);
  re-verified green on rerun.
