# Phase 1 — Spec delta: the `databricks` target

**Outcome:** `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`
**Serves criteria:** 1 (wholly), and fixes the contract phases 2, 3 and 11 implement against
**Driver:** Claude-executable — spec text only, no workspace, no credential
**Spec delta:** yes, and it is the entire phase (spec-first rule).

## Objective

Make `type: databricks` a specified backend before any code exists: the target's keys and
their refusals, the `BackendCapabilities::databricks()` column, the connection-security rule
for a token that lives in its own key rather than inside a URL, Unity-Catalog session init,
the Arrow-only load path, and the refusal of cross-engine exchange. Replace the standing
Known Divergence "Databricks is not yet a distinct backend" with a statement of what *is*
modelled plus the gaps that genuinely remain.

## Spec delta

Timeless-oracle rule applies: no phase vocabulary, no "not yet implemented" framing outside
§Known Divergences.

**`docs/specs/smelt_yml.md` §"Target shape"**
- `type` gains `databricks`.
- New rows: `host` (Databricks only, required — workspace hostname, no scheme, no trailing
  slash); `token` (Databricks only, optional — a `${ENV}` reference **only**; a literal token
  is a hard error, not a warning, because the value is a whole-workspace credential sitting in
  a committed file); when `token` is absent the client's ambient Databricks authentication is
  used, which is the form a workload running inside the workspace takes.
- `catalog` widens from "Spark only" to "Spark and Databricks"; on a Databricks target it
  names the Unity Catalog catalog and defaults to `workspace`. `schema` keeps its meaning and
  is catalog-qualified in emitted SQL.
- State the refusals: `connect_url`, `warehouse`, `format`, `database`, `settings`, `project`,
  `dataset`, `location` on a `databricks` target are **hard errors naming the key and the
  backend**, not ignored keys — serverless Free Edition has no host-visible warehouse path and
  no format choice, so a silently-ignored `warehouse:` would mean a user's file-layout
  intention was dropped. Add to §Known Divergences that this per-target key-placement check is
  specified for `databricks` only; other target types still tolerate misplaced keys.

**`docs/specs/multi_backend.md`**
- §Surface backends bullet: `duckdb | spark | bigquery | databricks`; a `databricks` target
  declares `SqlDialect::SparkSQL` and `BackendCapabilities::databricks()`, and names
  `host`/`catalog`/`schema` in place of Spark's `connect_url`/`warehouse`.
- §Surface capability matrix: add a **Databricks** column with a cell for every flag. Values
  equal `Spark (Delta)` except `supports_native_ivm` = ✗ (Enzyme emission does not exist, and
  the flag states what smelt emits); `null_safe_equality` = `<=>`.
- §Surface: a `SMELT_DATABRICKS_HOST` / `SMELT_DATABRICKS_TOKEN` bullet — when unset,
  Databricks-targeted tests **skip**, not fail, exactly as Spark's and BigQuery's do.
- §"Connection security": the Databricks rule. The token is read from the interpolated `token`
  key and handed to the Databricks Connect session builder; it is never written to a log line,
  a run report, a diagnostic, or an error message — any rendering of the target config redacts
  it. Contrast with Spark, where the secret rides inside `connect_url` and a literal is only a
  smell: here the key is the secret, so a literal is refused.
- §"Session initialization": a `databricks` target is `requires_schema_init = true` and issues
  `CREATE SCHEMA IF NOT EXISTS <catalog>.<schema>` in Unity Catalog before selecting it.
- §"Loading data into a backend": Databricks loads rows through the session's own
  `createDataFrame` from Arrow. A host-path read is not merely unreliable here but impossible —
  serverless compute shares no filesystem with the client, and neither DBFS root nor a Volume
  path is assumed.
- §"Cross-engine data exchange": a cross-backend edge into or out of a `databricks` target is
  **refused with a diagnostic** rather than compiled to a `read_parquet()` substitution; the
  substitution's precondition (a `warehouse` path both processes can read) cannot hold.
- §Design: a short "Why Databricks is a distinct target type" paragraph — serverless-only,
  Unity-Catalog-mandatory, no host-visible warehouse; `builder.remote()` on an `sc://` URL
  cannot reach serverless compute; the rejected alternative was a `serverless:` flag on
  `type: spark`, which would have left `warehouse`/`format` legal-but-broken.
- §Known Divergences: delete the two-line "Databricks is not yet a distinct backend" entry.
  Replace with what is modelled, plus the residual gaps stated behaviourally and linked to
  this outcome: the Databricks capability cells are inherited from Spark (Delta) and not yet
  each executed against a live workspace; no cross-engine exchange; no native IVM; the
  Statement Execution API is not a connection path; paid-tier compute shapes are unmodelled.

## Tests

No new Rust tests: this phase changes no production behaviour, so there is nothing to drive
red. The assertions the text above obliges are named here so phase 2 cannot lose them:

- `backend_type_resolves_databricks` — `Target::backend_type()` maps `"databricks"`.
- `databricks_target_refuses_spark_only_keys` — `warehouse`/`format`/`connect_url` each a hard
  error naming key and backend.
- `databricks_literal_token_is_rejected` — a non-`${VAR}` `token` fails config load.
- `databricks_capabilities_match_matrix` — the new column in
  `crates/smelt-dialect/tests/capability_conformance.rs`.
- `databricks_target_config_render_redacts_token` — no rendering path emits the secret.
- `cross_backend_edge_to_databricks_is_refused` — exchange refusal.

## Tasks

1. Edit `docs/specs/smelt_yml.md` §"Target shape" (+ its Known Divergences) per the delta.
2. Edit `docs/specs/multi_backend.md` §Surface (backends bullet, matrix column, env bullet).
3. Edit §"Connection security", §"Session initialization", §"Loading data into a backend",
   §"Cross-engine data exchange".
4. Add the §Design paragraph; replace the §Known Divergences Databricks entry.
5. Re-read both specs for timeless-oracle violations (`rg -n 'Phase [A-Z0-9]'` over the edits)
   and for a stale claim that `type:` has exactly three values (`rg -n 'duckdb.*spark.*bigquery'`).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-dialect --test capability_conformance --quiet 2>&1 | tail -20` — the
  hand-transcribed matrix gate must stay green across a spec-only column addition.
- `rg -n 'not yet a distinct backend' docs/` returns nothing.

## Commit message

`docs(spec): specify the `databricks` target — shape, capabilities, connection security, loading`
