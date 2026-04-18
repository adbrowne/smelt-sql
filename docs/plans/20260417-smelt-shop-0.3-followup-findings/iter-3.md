# Findings — iter 3 (smelt-sql 0.3.0 wheel cp312)

## Summary

Built the Smelt Shop pipeline end-to-end on a fresh worktree using the
candidate `smelt_sql-0.3.0-cp312-cp312-manylinux_2_39_x86_64.whl`.
All 21 acceptance checks pass at full scale (5M page events, 500K orders,
1.2M line items). Two consecutive `smelt build` runs (without deleting the
DuckDB file) succeed and `validate.py` reports `21/21 checks passed`.

Approximate runtime on this machine: datagen 2.6s, raw load 2.4s,
`smelt build` 17.8s (consistent across both runs).

## Workflow

1. `uv sync` (install wheel via `[tool.uv.sources]`)
2. `uv run smelt-datagen --config datagen.yaml`  (Hive-partitioned parquet)
3. `uv run python load_raw.py` (reads each dataset's `**/*.parquet` into `raw.*`)
4. `uv run smelt build` (seeds + 5 staging + 3 intermediate + 7 marts)
5. `uv run python validate.py`

## Issues encountered

### 1. Wheel/python version mismatch in the baseline scaffold (minor)

* **Reproduce:** the scaffold ships `requires-python = ">=3.11,<3.12"` and a
  `.python-version` of `3.11`, but the candidate wheel is tagged
  `cp312-cp312-manylinux_2_39_x86_64`. Running `uv sync` against the
  unmodified scaffold fails to resolve the wheel.
* **Workaround:** bumped `requires-python` to `>=3.12,<3.13` and
  `.python-version` to `3.12`. uv then provisioned CPython 3.12.13 and the
  wheel installed cleanly.
* **Severity:** minor (scaffold-only; not a bug in smelt-sql itself).

### 2. `smelt --version` flag not implemented (minor)

* **Reproduce:** `uv run smelt --version` exits non-zero with
  `error: unexpected argument '--version' found`. The prompt asks the user
  to confirm `smelt --version` matches the wheel; that flag does not exist.
* **Workaround:** verified the installed version via
  `python -c "import importlib.metadata; print(importlib.metadata.version('smelt-sql'))"`
  (returns `0.3.0`). No project-side workaround beyond skipping that step.
* **Severity:** minor.

### 3. `smelt build` is silent on success (minor / UX)

* **Reproduce:** a successful `smelt build` (clean or warm DB) produces no
  stdout/stderr at all — exit code 0, zero bytes.
* **Impact:** during initial development this made it hard to tell whether
  the command was a no-op, hung, or genuinely succeeded. Resolved by
  inspecting `target/dev.duckdb` with DuckDB directly. `smelt build -v`
  does emit the compiled SQL but there is no "run summary" line.
* **Workaround:** none required for correctness; just mention it.
* **Severity:** minor.

### 4. `optional` + `foreign_key` emits 0 instead of NULL (minor)

* **Reproduce:** the datagen `page_events` dataset uses
  `optional { prob: 0.55, inner: foreign_key(customers) }`. The resulting
  parquet file has `customer_id = 0` (45 % of rows) instead of NULL.
* **Workaround:** `stg_page_events` rewrites the column with
  `CASE WHEN customer_id = 0 THEN NULL ELSE customer_id END AS customer_id`.
  This goes in the workarounds list below.
* **Severity:** minor (predictable, easily neutralised, but undocumented).

### 5. Source columns must reflect post-load DuckDB types, not the parquet schema (minor / docs)

* **Reproduce:** datagen emits `event_date` as a Hive partition column
  parsed by DuckDB as `DATE`. My initial `sources.yml` declared it as
  `VARCHAR` (matching the documented behaviour of the `date` generator,
  which writes a string in unpartitioned columns). `smelt build` ran but
  the inferred types were inconsistent with the actual table.
* **Workaround:** declared all Hive partition columns and the order_date /
  item_date partitions as `DATE` in `sources.yml`. Pure documentation
  workaround — no SQL changes required.
* **Severity:** minor (docs gap, not a smelt bug).

### 6. Spec ambiguity on funnel "visit" definition (minor / spec)

* **Reproduce:** funnel spec lists steps as
  `Visit -> Product View -> Add to Cart -> Checkout Start -> Purchase` and
  requires that each step be <= the prior step. If "visit" is read as
  "session contained a `page_view` event", sessions that start on a
  product page (no page_view event) violate the monotonicity rule.
* **Resolution applied (NOT a workaround for a smelt bug):** treated
  `visits = total sessions` and each subsequent step as "session reached
  this OR any later step" (e.g. `purchase` implies `add_to_cart`). This
  satisfies the monotonicity acceptance criterion.
* **Severity:** minor (spec interpretation note).

## Workarounds applied

1. Pinned `requires-python` to 3.12 in `pyproject.toml` (issue #1).
2. `stg_page_events` rewrites `customer_id = 0` to `NULL` to undo a quirk
   of the `optional + foreign_key` generator (issue #4).
3. `sources.yml` declares Hive partition columns as `DATE` to match
   what DuckDB infers when reading the partitioned parquet (issue #5).

No SQL workarounds were needed for type inference, narrowing, lexer
quoting, seed-as-ref, idempotency, or CUBE-with-COALESCE. The smelt-sql
0.3.0 wheel handled every model in this pipeline (CTEs, window functions,
GROUP BY CUBE, NTILE, FILTER (WHERE ...), FULL OUTER JOIN, RANK over LEFT
JOIN nulls, etc.) on the first try.

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
