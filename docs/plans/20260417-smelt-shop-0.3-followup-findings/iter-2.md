# Smelt Shop — Iteration 2 Findings

**Wheel:** `smelt_sql-0.3.0-cp312-cp312-manylinux_2_39_x86_64.whl`
**Date:** 2026-04-18
**Outcome:** `smelt build` runs cleanly twice in a row; validation script passes 21/21 acceptance checks at both 0.05x and 1.0x scale.

## Summary

The pipeline contains 6 staging models, 5 intermediate models, and 8 marts (one per spec section). Total full-scale build time ~13s, well under the 5 min NFR. Two real bugs were uncovered during implementation; both required SQL workarounds and are documented below.

## Issues found

### 1. Aggregate type narrowing: `SUM(DOUBLE)` materializes as `BIGINT` *(major)*

**Reproducer:**
```bash
$ uv run smelt run --select mart_sales_cube --dry-run -v | head
... CAST(gross_revenue AS BIGINT) AS gross_revenue,
    CAST(net_revenue   AS DECIMAL(38,10)) AS net_revenue,
    ...
    CAST(returned_revenue AS BIGINT) AS returned_revenue, ...
```
The inner SQL has `SUM(line_revenue)` where `line_revenue` is `DOUBLE`. The
`_smelt_typed` outer wrapper inferred the column as `BIGINT`, silently
truncating fractional cents on every monetary aggregate.

**Surface area observed:** widespread. Without explicit casts, `int_order_enriched.{gross,returned,adjusted}_revenue`, `int_customer_orders.total_*_revenue`, `mart_product_performance.{revenue,returned_revenue}`, `mart_channel_attribution.attributed_revenue`, and `mart_sales_cube.{gross,returned}_revenue` were all narrowed to `BIGINT`. Interestingly `net_revenue` in `mart_sales_cube` was inferred as `DECIMAL(38,10)` — different SUM expressions land on different incorrect types.

**Workaround applied:** explicit `CAST(SUM(double_col) AS DOUBLE)` on every monetary aggregate (12 sites across `int_order_enriched.sql`, `int_customer_orders.sql`, `mart_sales_cube.sql`, `mart_product_performance.sql`, `mart_channel_attribution.sql`).

**Severity:** major — silent loss of precision on financial data that the spec calls out explicitly. A user who simply writes idiomatic SQL gets wrong answers.

### 2. `CASE … ELSE 0.0 END` over `DOUBLE` is narrowed to `DECIMAL(2,1)` *(major)*

**Reproducer:** the natural expression
```sql
CASE WHEN oi.returned_flag THEN oi.line_revenue ELSE 0.0 END AS returned_revenue
```
fails at runtime:
```
Conversion Error: Could not cast value 74.260000 to DECIMAL(2,1) when casting from
source column returned_revenue
LINE 1: ... CAST(returned_revenue AS DECIMAL(2,1)) AS returned_revenue ...
```
The literal `0.0` is given the smallest DECIMAL type that fits (`DECIMAL(2,1)`), and the CASE result is unified to that — even though the THEN branch is `DOUBLE`. The outer `_smelt_typed` wrapper then casts the whole column to `DECIMAL(2,1)`, which blows up the first time the THEN branch produces a value > 9.9.

**Workaround applied:** `CASE WHEN ... THEN oi.line_revenue ELSE CAST(0.0 AS DOUBLE) END` in `int_order_items_enriched.sql`. The same pattern is needed on every divide-by-zero guard / nullable monetary fallback (`mart_executive_summary.sql`, `mart_funnel_conversion.sql`, `mart_sales_cube.sql`, `mart_channel_attribution.sql`, `mart_product_performance.sql`).

**Severity:** major — a runtime error, not a type-check error. The model builds successfully via dry-run and only fails when DuckDB tries to apply the inferred wrapper cast against real data.

### 3. Successful builds emit no stdout *(minor)*

`uv run smelt build` exits 0 with zero output on success. There is no per-model "OK" line, no count of models built, no timing summary. Adding `--verbose` only prints compiled SQL; there is no equivalent of dbt's per-model status banner. This made it hard to tell at a glance whether a build had run anything at all.

**Severity:** minor — build still works; only an ergonomics issue.

### 4. Datagen `string_pattern` is the only way to get a VARCHAR id, and it does not coordinate with `foreign_key` types *(minor)*

**Reproducer:** I initially declared `visitors.visitor_id` with `string_pattern: "v{sequential_id}"` to get human-readable visitor strings, then declared `page_events.visitor_id` as a `foreign_key` of `visitors`. The foreign_key generator emits an INTEGER (the underlying row index), so the join `stg_page_events e LEFT JOIN stg_visitors v ON e.visitor_id = v.visitor_id` failed at staging build time with `Conversion Error: Could not convert string 'v1' to INT32`.

**Workaround applied:** drop the string template — make `visitor_id` an integer `sequential_id` in both datasets (and update `sources.yml` to declare INTEGER).

**Severity:** minor — it was my mistake, but the failure mode (the FK generator silently emits a different type than the dataset's id column) is a sharp edge worth documenting in `--list-generators`.

## Workarounds applied

1. Explicit `CAST(SUM(...) AS DOUBLE)` on 12 monetary aggregates across 5 model files (issue #1).
2. Explicit `CAST(0.0 AS DOUBLE)` on every `CASE … ELSE 0.0 END` and `COALESCE(.., 0.0)` over a DOUBLE column (issue #2). Roughly 9 sites total in marts.
3. Switched visitor ids from `string_pattern` to `sequential_id` so they match `foreign_key`'s INTEGER output (issue #4).

No other workarounds were applied. CUBE + GROUPING(), seed-as-ref, idempotent re-run, multi-CTE chains, window functions (LAG/RANK/NTILE), `date_diff`, `date_trunc`, `EXTRACT(EPOCH FROM ...)`, self-joins, scalar subqueries, and CROSS JOIN all worked first try.

## Scale and timing (1.0× scale)

- 50K customers, 5K products, 80K visitors, 500K orders, 1.2M order items, 5M page events.
- `smelt-datagen`: ~3s to write parquet.
- `load_raw.py`: ~1.4s to load into DuckDB.
- `smelt build` (cold): ~13s.
- `smelt build` (re-run, idempotency check): ~13s.
- `validate.py`: <1s.

## Files of interest

- Pipeline config: `/tmp/smelt_shop_iter_2/{smelt.yml,sources.yml,datagen.yaml}`
- Loader: `/tmp/smelt_shop_iter_2/load_raw.py`
- Validator: `/tmp/smelt_shop_iter_2/validate.py`
- Models: `/tmp/smelt_shop_iter_2/models/{staging,intermediate,marts}/*.sql`
- Seeds: `/tmp/smelt_shop_iter_2/seeds/*.csv`
