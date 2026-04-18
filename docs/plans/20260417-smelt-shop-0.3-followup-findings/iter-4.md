# Findings — iter 4 (smelt-sql 0.3.0 wheel cp311, post B8/B9/B11)

## Summary

Built the Smelt Shop pipeline end-to-end on a fresh worktree using
`smelt_sql-0.3.0-cp311-cp311-manylinux_2_39_x86_64.whl` (rebuilt with
B11 — `Optional<...>::is_nullable()` propagation for entity columns).

**24/24 acceptance checks pass at full scale, twice consecutively**
(5M page events, 500K orders, 1.2M line items). Two consecutive
`smelt build` runs without deleting `target/dev.duckdb` both exit 0;
`validate.py` reports `24/24 checks passed` after each.

Approximate runtime on this machine: datagen ~15s, raw load ~1.4s,
`smelt build` ~12.5s (1st) / ~12.7s (2nd), validate <1s.

**Loop exit criteria met:**
- ✅ ≥21/21 acceptance checks pass at full scale
- ✅ Idempotent (two consecutive builds clean)
- ✅ Zero SQL workarounds for narrowing / NULL coercion / lexer / seed-as-ref
- ✅ Baseline scaffold unmodified (cp311 wheel matches `>=3.11,<3.12`)
- ✅ B8, B9, B11 all confirmed fixed by direct probes

## Workflow

1. `uv add ./smelt_sql-0.3.0-cp311-cp311-manylinux_2_39_x86_64.whl`
   (single `[tool.uv.sources]` entry — no `requires-python` bump)
2. `uv run smelt-datagen --config datagen.yaml`
3. `uv run python load_raw.py`
4. `uv run smelt build` (twice; second run is the idempotency check)
5. `uv run python validate.py`

## Bug-fix verification

### B11 (Optional+ForeignKey emits real NULLs) — FIXED
`raw.page_events.customer_id` has 2,251,311 NULLs and **0 zeros**
(out of 5M rows at p=0.55). `stg_page_events.sql` is a plain
`SELECT customer_id` with **no `CASE WHEN col = 0 THEN NULL` workaround**.
Confirmed end-to-end through `int_sessions`.

### B8 (qualified upstream column types) — FIXED
`SUM(oi.gross_line_revenue)` over a DOUBLE upstream column yields
`DECIMAL(38,10)` (no silent BIGINT narrowing); `SUM(oi.net_line_revenue)`
yields `DOUBLE`. **No explicit CASTs in any aggregation.**

### B9 (CASE/COALESCE widening) — FIXED
`SUM(CASE WHEN is_successful THEN net_revenue ELSE 0.0 END)` builds and
runs cleanly — the previous `Could not cast value to DECIMAL(2,1)`
runtime error does not reproduce.

## Issues encountered (all minor, none requiring SQL workarounds)

### 1. `smelt.yml` `version` field must be `u32`, not semver string (minor / UX)
* **Reproduce:** writing `version: "0.1.0"` (mirroring `pyproject.toml`)
  fails to parse. The field expects an integer (`version: 1`).
* **Workaround:** `version: 1`. Clear error message; quick fix.
* **Severity:** minor (docs/UX).

### 2. `smelt build` is silent on success (minor / UX)
* Same as iter-3 #3. No "run summary" line on a successful build.
* **Severity:** minor.

### 3. `partition_by:` YAML key silently ignored (minor / docs)
* **Reproduce:** Using `partition_by:` (a plausible-sounding name) in
  `datagen.yaml` does not produce Hive-partitioned output — instead a
  single `data.parquet` per dataset. The correct key is `partition:`.
* **Impact:** Benign for the pipeline (`load_raw.py` reads `**/*.parquet`
  either way) but can confuse users who try the wrong key name.
* **Workaround:** use `partition:` (which is what iter-3 did). Could be
  improved by warning on unknown keys (serde `deny_unknown_fields`).
* **Severity:** minor.

### 4. Spec ambiguity on funnel "visit" definition (minor / spec)
* Same as iter-3 #6. Resolved by treating later-step reach as implying
  earlier steps. No smelt bug.
* **Severity:** minor.

## Workarounds applied

**None** for SQL semantics. The only adjustments were (a) the YAML
key-name fix in #3 above, and (b) the funnel monotonicity interpretation
in #4. No `CASE WHEN ... = 0 THEN NULL`, no narrowing CASTs, no seed
re-implementations.

## Pipeline at a glance

```
seeds/                   models/staging/             models/intermediate/        models/marts/
  category_hierarchy       stg_customers               int_sessions                mart_executive_summary
  channel_groups           stg_products                int_session_summary         mart_sales_cube
  country_codes            stg_orders                  int_order_items_enriched    mart_funnel
  status_codes             stg_order_items             int_orders_enriched         mart_customer_rfm
                           stg_page_events                                         mart_cohort_retention
                                                                                   mart_product_performance
                                                                                   mart_product_affinity
                                                                                   mart_channel_attribution
```
