# Phase 6 summary — first live Databricks run: full refresh, refusals recorded

**Deployed against:** `workspace.smelt_dogfood`, the same schema phase 5's loader populated
(two fixture days: 2026-08-05, 2026-08-06), via the OAuth M2M credential phase 4c verified.
Compiled with `cargo build -p smelt-cli --features databricks` (the default build has no
Databricks backend — see finding below). No table outside `smelt_dogfood` was touched.

## Task 1 — the target and source-name reconciliation

Added a `databricks` target to `examples/github_activity/smelt.yml` (`type: databricks`,
`host: ${SMELT_DBX_HOST}`, `token: ${SMELT_DBX_TOKEN}`, `catalog: workspace`,
`schema: smelt_dogfood`), `target: dev` left pinned. Added `databricks:` entries to both
source YAMLs' `name:` maps, mirroring the `bigquery:`/`bigquery_oracle:` entries exactly
(`smelt_dogfood.github_events` / `smelt_dogfood.github_events_arrival` — the physical
tables phase 5's loader wrote, not the default `sources_raw_*` mapping). The oracle target
is deliberately not declared — that is phase 9's.

Three new offline tests in `crates/smelt-cli/tests/github_activity_databricks.rs` (mirroring
`github_activity_bq_oracle`'s pattern): source resolution to the loaded tables, the
no-`--target` default staying `dev` (`databricks` now sorts alphabetically first, ahead of
even `bigquery`), and config parsing needing no credential. All green, no network touched.

## Task 4 — pre-existing gates, re-checked

`github_activity_replay` and `example_diagnostics` both broke, for reasons that are
consequences of correctly declaring a third real target — not bugs, and the BigQuery
phase 11 precedent for exactly this is `docs/outcomes/20260906-bigquery-dogfood-spine/
phases/11-summary.md`.

1. **Env-var interpolation is target-blind by spec** (`docs/specs/smelt_yml.md` §Semantics
   item 8: "runs exactly once, in the config-load pass", before any target is resolved).
   Every test that spawns `smelt` against the staged/real `examples/github_activity` needs
   `SMELT_DBX_HOST`/`SMELT_DBX_TOKEN` set to *something*, even for a `--target dev`
   invocation, or config load hard-fails before doing anything target-related. Fixed by
   adding dummy `.env(...)` values (`unused-in-tests...`) at the four call sites that spawn
   the `smelt` binary against this project: `github_activity_support::smelt_run`,
   `github_activity_replay.rs`'s two `smelt_explain*` helpers and
   `smelt_run_expect_failure`, and `list_external_step.rs`'s `run()`.
2. **`MaintenanceStateDowngraded` reappeared, correctly.** That diagnostic is computed
   against the union of every declared target's backend (`crates/smelt-db/src/queries/
   maintenance/diagnostics.rs`'s own "Checked against every declared backend" comment).
   `databricks` maps to the `SparkSQL` dialect (`write_pin.rs::backend_dialect_for`), which
   realises **no** state structure at all (`state_structure.rs`: `SparkSQL => vec![]`) —
   unlike BigQuery, which now realises the full ledger set. So the same four cells that
   downgraded before BigQuery's own ledger support landed (phase 15 of the BigQuery outcome)
   downgrade again, this time because of `databricks`. Fixed as a **test** change, following
   the established precedent exactly: `github_activity_no_diagnostics` now calls
   `check_workspace_diagnostics_are_exactly` with the four expected messages (reusing the
   helper BigQuery's own phase 11 added and phase 15 later stopped needing — restored to
   active use, its `#[allow(dead_code)]` removed). `smelt-lsp`'s `example_workspaces.rs`
   needed no change: its `github_activity` case already excludes the
   `maintenance-state-downgraded` code by design.

Both are genuinely expected, not compile-time refusals, and neither blocks a run.

## Two fixes made under the "no run completes at all" exception

The plan's brief is record-don't-fix, with one named exception. Both of these were required
just to get *any* live call to complete — nothing downstream was reachable without them.

1. **`smelt-cli` had no `databricks` Cargo feature.** `smelt-backends` already gates the
   Databricks backend behind its own `databricks` feature (phase 2), but nothing in
   `smelt-cli/Cargo.toml` turned it on — the closest existing flag, `spark`, only requests
   `smelt-backends/spark`. Added `databricks = ["smelt-backend-spark", "python",
   "smelt-backends/databricks"]`, mirroring `spark`'s shape but pointing at the Databricks
   capability profile. Without this, every live call failed immediately with "Databricks
   backend not available. Rebuild with --features databricks" — no compile refusal to record
   at all, since nothing ever reached the compiler.
2. **The PyO3-embedded interpreter never processes the Databricks venv's `.pth` shim**,
   so `databricks-connect` (via `pyspark`) failed outright with `ModuleNotFoundError: No
   module named 'distutils'` on Python 3.12 (stdlib `distutils` was removed; setuptools's
   `distutils-precedence.pth` reroutes `import distutils` to `setuptools._distutils`, but
   `.pth` files only run for directories the `site` module itself registers at startup —
   `scripts/dbx-dogfood-env.sh` only appends the venv's site-packages via `PYTHONPATH`, which
   bypasses that mechanism entirely). Phase 3/5's loader script never hit this because it
   invokes `.smelt-dbx-venv/bin/python` directly — a real venv activation, where `.pth`
   processing does happen. `smelt run`'s embedded interpreter is a different code path.
   Fixed with a 6-line try/except in `python/smelt/databricks_adapter.py`'s `__init__`
   (`import _distutils_hack; _distutils_hack.add_shim()`, no-op wherever the shim already
   ran). Verified the mechanism directly with a standalone `python3 -c` reproduction before
   changing the adapter. No test added — this is a Python-environment integration detail with
   no meaningful offline assertion; the live run is its own proof.

## A finding surfaced but NOT fixed (recorded per the plan's brief)

**`scripts/dbx-key.sh` stores the workspace host WITH its `https://` scheme** (used directly
by the Python adapter's `DatabricksSession.builder.host(...)` and by `dbx_dogfood_query.py`),
but the `type: databricks` target's `host:` field requires a **bare** hostname — no scheme,
no trailing slash (`config.rs`'s `databricks_target_requires_host`-adjacent validation,
phase 1/2's spec). Sourcing `scripts/dbx-dogfood-env.sh` and pointing `smelt.yml` straight at
`$SMELT_DBX_HOST` therefore fails config load with `` `host` must be a bare hostname with no
scheme and no trailing slash, got `https://dbc-...` ``. Worked around for this phase's live
calls only, by stripping the scheme in the shell (`SMELT_DBX_HOST="${SMELT_DBX_HOST#https://}"`,
trailing `/` trimmed) immediately before invoking `smelt` — **no committed script or
`smelt.yml` was changed for this**. This is a real format mismatch between the wizard's
stored value and the target contract; the reconciliation (either the target accepting a
scheme, or the wizard/env script also exporting a bare-host variant) is left for phase 10's
findings handoff or a follow-on outcome, not decided here.

## Task 6 — live, compile only first

`--dry-run` reproduces the same immediate refusal BigQuery phase 11 already documented and
which is expected, unrelated behavior:

```
Error: ExternalStepNotInvocable: step 'sources.raw.github_loader' cannot be invoked this
run — this is a dry run — `smelt explain` is the non-refusing preview surface for a step
```

`--show-plan` (without `--dry-run`, invoking the loader step for real — a no-op, since both
fixture days were already loaded) reached real compile+execution and surfaced the first
finding below on `bronze.events`.

## Task 7 — live, full refresh

```
smelt run --target databricks --full-refresh --allow-full-refresh \
  --event-time-start 2026-08-05 --event-time-end 2026-08-07
```

Outcome (from the committed run report,
`.smelt/targets/databricks/reports/20260912-085732-66040b.json`, and the run manifest):
**1 success, 3 failed, 12 skipped**, all sixteen models accounted for, one root cause.

| model | outcome | rows |
|---|---|---|
| `bronze.events` | **failed** | 0 |
| `silver.actor_naming` | **failed** | 0 |
| `silver.repo_naming` | **failed** | 0 |
| `silver.events_deduped` | success | 5,915 |
| every other model (12) | skipped (downstream of a failed model) | 0 |

## Finding 1 — the append-only baseline-snapshot fingerprint emits `sha256(...)` literally, which does not exist on Spark/Databricks

**Model / statement:** `bronze.events`, `silver.actor_naming`, `silver.repo_naming` — each
model's `NewData`/append-only-baseline maintenance cell over `raw.github_events` or
`raw.github_events_arrival`. One root cause, three call sites (every model whose driving
source is one of the two raw sources and needs the baseline snapshot to establish "new
data").

**Smallest reproducing SQL** (the actual failing statement, `bronze.events`'s baseline
snapshot):

```sql
SELECT
  CAST(DATE_TRUNC('day', created_at) AS STRING) AS partition_value,
  COUNT(*) AS current_count,
  sha256(CONCAT_WS('', SORT_ARRAY(COLLECT_LIST(
    sha256(CONCAT(
      sha256(CASE WHEN id IS NULL THEN 'N' ELSE CONCAT('V', CAST(id AS STRING)) END),
      -- ...one sha256(...) per column...
    ))
  )))) AS current_fingerprint
FROM smelt_dogfood.github_events
GROUP BY CAST(DATE_TRUNC('day', created_at) AS STRING)
```

**Live error:**

```
AnalysisException: [UNRESOLVED_ROUTINE] Cannot resolve routine `sha256` on search path
[`system`.`session`, `system`.`builtin`, `system`.`ai`, `workspace`.`smelt_dogfood`].
SQLSTATE: 42883
```

**Root cause, precisely located:** `crates/smelt-logical/src/maintenance/emit/
fingerprint.rs` builds the whole-row and per-column fingerprint expressions as **literal,
dialect-unaware SQL text** — `format!("sha256({concatenated})")` (line ~113) and
`format!("sha256(CASE WHEN {column} IS NULL THEN ...)")` (line ~54) call `sha256(...)`
regardless of target dialect. DuckDB has a native `sha256()` scalar function and BigQuery's
GoogleSQL accepts `SHA256()` case-insensitively, so this has been silently correct on both
engines this project has run on so far. **Spark/Databricks has no `sha256` function at
all** — its equivalent is `sha2(expr, bitLength)` (`sha2(expr, 256)` for a SHA-256 digest).
This is a genuine dialect-emission gap in the maintenance layer's fingerprint mechanism, and
by the letter of the Function-Registry single-ownership invariant
(`CLAUDE.md` §"Function-registry single ownership") it should never have been hand-spelled
at all — every other built-in's per-dialect spelling is registry `Signature::emission` data,
interpreted by the generic printer, precisely so a gap like this can't reach a live engine
unnoticed. `fingerprint.rs` bypasses that path entirely, which is also why this surfaced as
a **runtime** `AnalysisException` rather than a compile-time `UnsupportedOnBackend` refusal —
smelt's own dialect-conformance machinery never got a chance to catch it.

**Blast radius:** every maintained model whose driving source needs an append-only baseline
snapshot against a `SparkSQL`-dialect target is unconditionally broken on first run. On this
fixture that is 3 of 16 models directly, cascading to 12 more (only `silver.events_deduped`,
whose `KeyedFold`→`PerGroupRecompute` downgrade needs no baseline snapshot, escaped).
**Not fixed here** — this is squarely a `databricks-correctness` finding, and fixing the
maintenance-emit layer's dialect handling is out of this phase's "record, don't fix" scope.

## Task 8 — read-back row counts

```
SHOW TABLES IN workspace.smelt_dogfood
→ _loader_days, github_events, github_events_arrival, silver_events_deduped
SELECT count(*) FROM workspace.smelt_dogfood.silver_events_deduped → 5915
```

Matches the run report's `row_count` for `silver.events_deduped` exactly. No other model
table exists in the schema — consistent with the report's `skipped`/`failed` verdicts; there
is nothing to under- or over-count.

## New Free Edition fact for `free-edition-facts.md`

`dbx-query.sh` reproduced the same `INVALID_HANDLE.SESSION_CLOSED` warning phase 4c already
recorded (serverless session teardown between per-invocation sessions), this time
immediately after a `smelt run` — confirms it is a property of the serverless session
lifecycle generally, not specific to the verify script. No new quota discovered; not added as
a new row since phase 4c's entry already covers it.

## Gates

- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  shellcheck, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_databricks --quiet` — 3 passed.
- `cargo test -p smelt-cli --test github_activity_replay --quiet` — 21 passed (previously
  all failed on the env-var issue above; fixed, not skipped).
- `cargo test -p smelt-cli --test example_diagnostics --quiet` — 128 passed, 1 ignored
  (`github_activity_no_diagnostics` now asserts the exact four expected messages).
- `cargo build -p smelt-cli --features databricks` — succeeds (new feature, task above).
- Live evidence: quoted above (compile-only refusal reproduction, full-refresh run report,
  read-back row counts).

## For phase 7 (next: incremental windows)

- The `sha256`/`sha2` gap must be worked around (not fixed) to get past window 1: any model
  whose driving source needs the append-only baseline snapshot will fail identically on every
  subsequent window too, since the root cause is dialect-invariant across runs. Phase 7
  should either `--exclude` the three affected models plus their 12 dependents and scope its
  incremental-window claim to `silver.events_deduped` only, or treat "get past this" as the
  phase's own "no run completes at all" exception if the outcome wants broader coverage.
  Recorded here rather than decided, per this phase's own brief.
- `.claude/large-file-baseline.txt` was not touched — no file crossed its tracked threshold
  this phase (unlike BigQuery's phase 11, which needed a bump for `example_workspaces.rs`).
- The host bare-vs-scheme mismatch (see above) will need resolving before phase 11's
  Asset Bundle work, since a bundle's own `databricks.yml` will need the same target config
  and won't have a human shell to work around it in.
