# Phase 1 summary — Spec delta: the `databricks` target

**Shipped:**
- `docs/specs/smelt_yml.md` §"Target shape": `type: databricks`, new `host` (required) and
  `token` (optional, `${ENV}`-only, literal is a hard error) rows, `catalog` widened to
  Spark+Databricks, the per-target key-placement refusal list
  (`connect_url`/`warehouse`/`format`/`database`/`settings`/`project`/`dataset`/`location`),
  and a matching Known Divergences entry noting the check is Databricks-only today.
- `docs/specs/multi_backend.md`: backends bullet now lists `databricks`; capability matrix
  gains a Databricks column (identical to Spark (Delta) except `null_safe_equality` = `<=>`,
  `supports_native_ivm` = ✗); `SMELT_DATABRICKS_HOST`/`SMELT_DATABRICKS_TOKEN` skip-not-fail
  bullet; §"Connection security" describes the distinct `token`-key carrying path and its
  hard-error-on-literal rule (vs. Spark's URL-embedded smell); §"Session initialization" states
  the catalog-qualified `CREATE SCHEMA IF NOT EXISTS`; §"Loading data into a backend" states the
  Arrow-only path is the *only* option (no filesystem at all, not just an unreliable one);
  §"Cross-engine data exchange" refuses a `databricks` edge outright; new §Design paragraph
  "Why Databricks is a distinct target type"; the old two-line "not yet a distinct backend"
  Known Divergence replaced with five gap-first entries (matrix inheritance, no exchange, no
  native IVM, no Statement Execution API path, no paid-tier shapes).

**Decisions:**
- Kept the existing `type: spark` example in §"Connection security" as-is and added the
  Databricks path alongside it rather than rewriting the Spark example — the two paths carry
  the secret differently (URL-embedded vs. dedicated key) and both need to stay legible.
- Databricks shares `SqlDialect::SparkSQL` (not a new dialect variant) since Databricks Connect
  compiles to Spark SQL; only the capability profile and connection shape are distinct.

**For the next planner:**
- Phase 2 (backend dispatch, session builder, capability profile, refusals, redaction) should
  drive its tests directly off the six named in phase 1's plan
  (`backend_type_resolves_databricks`, `databricks_target_refuses_spark_only_keys`,
  `databricks_literal_token_is_rejected`, `databricks_capabilities_match_matrix`,
  `databricks_target_config_render_redacts_token`, `cross_backend_edge_to_databricks_is_refused`).
- No code changed in this phase, so `hardening_budget`/ratchet baselines are untouched — nothing
  to update there for phase 2 either unless it adds new `unwrap`/`expect`.
- Nothing found out of scope; the phase stayed within its stated boundary (spec text only).

**Gates:**
- `cargo test -p smelt-dialect --test capability_conformance --quiet` — 2 passed (spec-only
  column addition, no code touched, gate green as expected).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, example_diagnostics).
- `rg -n 'not yet a distinct backend' docs/` — no hits outside outcome/plan files (which are
  historical record, not spec/user-doc body).
