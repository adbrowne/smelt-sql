# Phase 8 plan — dual-target parity: DuckDB vs Databricks over the same rows

**Live phase.** Check reachability FIRST (`bash scripts/dbx-verify.sh`, after
`bash scripts/dbx-auth.sh` — the OAuth token lives an hour). If the workspace is
unreachable, still land tasks 1–5 (they are offline, per-PR gates) and their tests, commit,
then emit `<<PHASE_BLOCKED>>` naming the live remainder (tasks 6–10). Never skip green.

## Objective

Advance criterion 7. The BigQuery dual-target comparator gains a **Databricks leg,
generalised over the target rather than duplicated**: one shared support module, one landing
seam, one relation-discovery + exclusion rule, two sweeps with their own side labels,
exclusion sets and divergence registries. Then run it live: every model's state in
`workspace.smelt_dogfood` equals the DuckDB leg's over the same eight fixture days, or the
difference is a registered divergence with a root-caused reason. An unregistered difference
fails.

## Spec delta

None. No user-visible feature behaviour changes — this is test/tooling work over the
already-specified `databricks` target.

## Tests

Offline (per-PR, no credential), in `crates/smelt-cli/tests/github_activity_dual_target.rs`
unless noted:
- `databricks_sweep_compares_every_model` — `DATABRICKS_EXCLUDED_MODELS` is empty and is
  asserted equal to the exclusion list the new shell driver passes, the way the BigQuery pair
  is already kept in lockstep; a model excluded from the BigQuery leg is compared by this one.
- `an_unregistered_databricks_divergence_fails` — negative control over a synthetic perturbed
  pair, reported in both directions with `SIDES_DBX = (duckdb, databricks)` labels.
- `the_databricks_sweep_fails_closed_on_an_empty_registry` — the registry-consulting path is
  driven over a real mismatch with `DBX_DIVERGENCE_REGISTRY` emptied, so the control holds
  whatever the registry happens to contain.
- `databricks_relation_set_mismatch_fails` — coverage totality under the empty exclusion set:
  a relation present on only one target names the relation and the side.
- `landing_seam_is_target_agnostic` (in the support module's tests) — a synthetic NDJSON dir
  in the *declared export encoding* (timestamps as epoch seconds, everything else as text)
  lands byte-identically to its source table, proving the seam carries no BigQuery-specific
  branch.
- `parity_manifest_shape_is_shared` — both sweeps deserialise the same `ParityManifest` struct
  from the support module (compile-level + one round-trip assertion).

Live (gated on `SMELT_DBX_DOGFOOD_LIVE=1`; **fails**, never skips, when set without a
manifest):
- `duckdb_and_databricks_agree_on_every_model` — reads the phase-8 manifest, lands the
  Databricks NDJSON under the DuckDB leg's declared types, compares, writes
  `phases/08-parity.json`, and fails naming every relation whose difference is unregistered.
- `dbx_registry_entries_are_all_live` — `DBX_DIVERGENCE_REGISTRY` checked in **both**
  directions against the committed `08-parity.json`: no entry without evidence, no evidence
  without an entry. Landed only together with that artifact (it panics when the artifact is
  absent, by design).

## Tasks

1. Rename `crates/smelt-cli/tests/bq_parity_support/` → `parity_support/` and update its two
   `#[path]` includes and all `bq_parity_support::` paths (`github_activity_dual_target.rs`,
   `github_activity_bq_oracle.rs`). No behaviour change.
2. In the support module: rename `load_bigquery_snapshot` → `load_exported_snapshot` and
   document the **export encoding contract** it assumes (timestamp columns arrive as epoch
   seconds; every other column as text a `CAST` accepts) — that contract, not the exporter, is
   what makes the seam target-agnostic.
3. Move the `ParityManifest`/`Checkpoint` structs out of `github_activity_dual_target.rs` into
   the support module so both sweeps and the new Databricks sweep share one manifest shape.
4. Split the model-exclusion constant: `EXCLUDED_MODELS` → `BIGQUERY_EXCLUDED_MODELS` (same
   two GoogleSQL refusals, same doc comment) plus `DATABRICKS_EXCLUDED_MODELS: &[] `, whose
   doc comment records that phases 6b–6f closed every construct and the target runs 16/16.
   `EXCLUDED_PREFIXES`/`SUFFIXES`/`EXACT` are unchanged and shared — the Databricks schema
   carries the same `github_events`/`github_events_arrival` source spelling.
5. Add the Databricks sweep to `github_activity_dual_target.rs`: `SIDES_DBX`,
   `DBX_DIVERGENCE_REGISTRY` (starts empty), `check_databricks_agree`, and the offline tests
   above.
6. Add `scripts/dbx_dogfood_export.py` — one Databricks Connect session, discovers relations
   from `${SMELT_DBX_CATALOG}.information_schema.tables` for `${SMELT_DBX_SCHEMA}`, applies
   the same exclusion rules, and writes one `<relation>.ndjson` per relation in the encoding
   task 2 declares (pyarrow timestamp columns → epoch seconds, else `default=str`). Read-only:
   it issues `SELECT`s and nothing else.
7. Add `scripts/dbx-dogfood-parity.sh` with stages `duck`, `dbx-snapshot`, `manifest`,
   `report`, modelled on `bq-dogfood-parity.sh` but with **no destructive stage at all** — the
   Databricks state phases 5–7b built is the thing under test and this script never drops it.
   `duck` drives `examples/github_activity/run_incremental.py --start-date 2026-08-05 --days 8
   --window-days 1 --first-full-refresh --snapshot-after 8`, mirroring the live target's
   full-refresh-then-windows sequence; `dbx-snapshot` calls task 6's exporter.
8. **Live, first:** backfill `gold.events_enriched`'s coverage gap that `07-summary.md`
   flagged — `smelt run --target databricks --event-time-start 2026-08-07 --event-time-end
   2026-08-10 -s gold.events_enriched+` (or the whole set if the selector is unavailable) — and
   confirm `intervals.json` shows contiguous `[2026-08-05, 2026-08-13)` for **every** model
   before snapshotting. Parity over non-contiguous coverage would be a finding about the
   backfill, not about the engines.
9. **Live:** run `duck`, then `dbx-snapshot`, then `manifest`, then the live test with
   `SMELT_DBX_DOGFOOD_LIVE=1`; re-mint the token first (one-hour lifetime).
10. Root-cause every difference the sweep reports. Each one is either a fix that is the only
    way the comparison completes at all, or a `DBX_DIVERGENCE_REGISTRY` entry naming the
    relation, the column, the bound and the reason — never a comparator tolerance. Commit
    `phases/08-parity.json`, the `report` stage's markdown twin, and add each finding to
    `free-edition-facts.md` or the row-10 punch-list material in the summary.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_dual_target` — offline sweep tests green
  with no credential.
- `cargo test -p smelt-cli --test github_activity_bq_oracle` — the rename did not disturb the
  BigQuery equivalence sweep.
- Live: `SMELT_DBX_DOGFOOD_LIVE=1 cargo test -p smelt-cli --test github_activity_dual_target
  duckdb_and_databricks_agree_on_every_model -- --nocapture`.

## Commit message

`outcome(databricks-dogfood-spine): phase 8 proves DuckDB/Databricks parity via the generalised comparator`
