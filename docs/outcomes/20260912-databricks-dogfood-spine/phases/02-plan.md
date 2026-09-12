# Phase 2 plan — The `databricks` backend, offline

## Objective

Make `type: databricks` a real backend in code: a `BackendType::Databricks` variant, the
`host`/`token` target keys with their refusals and redaction, a
`BackendCapabilities::databricks()` profile matching the matrix phase 1 published, and a pure
session-plan function that selects the `DatabricksSession` builder for a `databricks` target and
the plain `SparkSession.remote` builder for a `spark` one. Every assertion runs with **no
workspace, no credential and no Python** — this advances success criteria 1 and 2, and is the
precondition for phases 3–11 being a test of Databricks rather than of smelt.

## Spec delta

None expected — phase 1 (`e242fce4f`) settled the contract in `docs/specs/smelt_yml.md`
§"Target shape" and `docs/specs/multi_backend.md` §Surface / §"Connection security" /
§"Cross-engine data exchange". If implementation forces a contract change (e.g. `host` turns
out to need a scheme), edit the spec **first**, in the same commit, and note it in the summary.

## Tests

Red-green, in this order:

1. `backend_type_resolves_databricks` (`smelt-core/src/config.rs` tests) — `type: databricks`
   resolves to `BackendType::Databricks`; `table_format()` is `None` (format is a Spark concept);
   an unknown `type:` still errors.
2. `databricks_target_requires_host` — a `databricks` target with no `host` is a load error
   naming the key; a `host` carrying a scheme or trailing slash is rejected.
3. `databricks_target_refuses_spark_only_keys` — each of `connect_url`, `warehouse`, `format`,
   `database`, `settings`, `project`, `dataset`, `location` on a `databricks` target is a hard
   error naming both the offending key and the backend. Table-driven over all eight.
4. `databricks_literal_token_is_rejected` — `token: dapi…` (literal) is a hard error;
   `token: ${SMELT_DATABRICKS_TOKEN}` loads; an **absent** `token` loads (the ambient form).
   The check must see the raw YAML, so it runs before `interpolate_env_vars`.
5. `databricks_capabilities_match_matrix` — extend
   `smelt-dialect/tests/capability_conformance.rs::every_flag_matches_matrix` with a
   `databricks` column cell for **every** row of the spec matrix: equal to Spark (Delta)
   throughout, `null_safe_equality = Spaceship`, `supports_native_ivm = false`,
   `dialect = SqlDialect::SparkSQL`.
6. `databricks_target_config_render_redacts_token` — `Debug` and serde renderings of a `Target`
   holding a resolved token contain no substring of the token value; the backend-construction
   error path does not either.
7. `databricks_session_plan_selects_databricks_builder` — the pure session-plan function maps a
   `databricks` target to the `smelt.databricks_adapter` entry point with host + serverless and
   a token-present / token-absent (ambient) arm, and a `spark` target to
   `smelt.spark_adapter` with its connect URL. No interpreter is started.
8. `cross_backend_edge_to_databricks_is_refused` — a cross-backend edge whose producer or
   consumer is a `databricks` target fails with a diagnostic naming both targets, instead of
   the `read_parquet()` substitution or a silent fall-through when `materialized_path` is `None`.

## Tasks

1. Add `BackendType::Databricks`; extend `Target::backend_type()` and `Target::table_format()`.
   Fix every resulting non-exhaustive `match` across the workspace (compiler-driven).
2. Add `host: Option<String>` and `token: Option<String>` to `Target`; `token` is
   `skip_serializing`, and `Target` gets a hand-written `Debug` that prints `token: <redacted>`.
3. Add a pure `fn validate_targets(&self) -> Result<(), Vec<String>>` on `Config` covering
   tasks-2 requiredness and the eight key refusals; call it from `Config::load` after parse.
4. Add a pure `fn check_literal_secrets(text: &str) -> Vec<String>` over the **raw** YAML (a
   `databricks` target's `token` must be exactly `${VAR}`); call it in `Config::load` before
   `interpolate_env_vars`. Interpolation is lossy about origin, hence the pre-pass.
5. Add `BackendCapabilities::databricks()` in `smelt-dialect/src/dialect.rs`, doc-commented as
   inherited from `spark_delta()` and not yet live-verified (spec §Known Divergences).
6. In `smelt-backend-spark`, add a `flavor` discriminator (`Spark` | `Databricks`) driving
   `capabilities()` and the adapter entry point, plus a `new_databricks(host, token, catalog,
   schema)` constructor. Extract the entry-point decision into the pure session-plan function
   test 7 drives. No new crate — the SQL surface is identical.
7. Add `python/smelt/databricks_adapter.py`: `DatabricksAdapter` building
   `DatabricksSession.builder.host(...).serverless(True)`, `.token(...)` only when supplied,
   otherwise the ambient credential; same method surface as `SparkAdapter` (including
   `load_arrow_table`). Document that `databricks-connect` conflicts with `pyspark`.
8. Wire `BackendType::Databricks` in `smelt-backends::create_backend` behind a `databricks`
   cargo feature that pulls `smelt-backend-spark`; catalog defaults to `workspace`.
9. Refuse a cross-backend edge touching a `databricks` target where `execute/project/mod.rs`
   consumes `cross_engine_edges`.
10. Update `.claude/hardening-baseline.txt` only if the new code adds a classified
    `unwrap`/`expect`; prefer `Result` and add none.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full test,
  example_diagnostics).
- `cargo test -p smelt-dialect --test capability_conformance --quiet`
- `cargo test -p smelt-core --lib config --quiet`
- `cargo test -p smelt-backend-spark --quiet` — the offline `sql_*` tier must stay green; the
  Spark parity tier is untouched (criterion 10).
- `cargo test -p smelt-core --test hardening_budget --quiet`
- `rg -n 'databricks' crates/ --type rust -l` — sanity check that no live-workspace dependency
  crept in; nothing in this phase may require `SMELT_DATABRICKS_HOST`.

## Commit message

`feat(backend): add the databricks target — dispatch, capabilities, refusals, redaction`
