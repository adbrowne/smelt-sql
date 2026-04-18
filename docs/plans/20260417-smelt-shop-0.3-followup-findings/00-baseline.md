# smelt-sql 0.3.0 — Implementation Findings

Second implementation of the Smelt Shop reporting pipeline (SPEC.md / spec.md)
against `smelt-sql==0.3.0`. Previous notes referenced problems from 0.2.0 that
the user said were fixed; this report covers what is still broken, what is
fixed, and anything new that surfaced.

## Pipeline status

All 8 marts required by the spec are implemented and the `validate.py` script
exercises every stated acceptance criterion. **21/21 checks pass** on a fresh
build.

| Layer       | Models                                                                                                       |
|-------------|--------------------------------------------------------------------------------------------------------------|
| staging     | stg_customers, stg_products, stg_orders, stg_order_items, stg_page_events                                    |
| intermediate| int_order_enriched, int_customer_orders, int_customer_lifetime, int_session_boundaries/events/summary        |
| marts       | executive_dashboard, sales_cube, funnel_conversion, customer_rfm, cohort_retention, product_performance, product_affinity, channel_attribution |

## What's fixed in 0.3 (vs 0.2)

- **Frontmatter materialization is respected.** In 0.2 the `materialization: table`
  frontmatter was being silently ignored; in 0.3 `smelt explain` shows each
  model's materialization correctly and physical tables are written.
- **CTEs no longer break type inference.** Every mart in this pipeline uses WITH
  clauses (often multiple) and the output column types are concrete rather than
  `?` placeholders.
- **`smelt.ref()` inside CTEs/subqueries resolves.** Previously refs were only
  substituted in the top-level FROM clause.
- **JOINs with sources no longer generate wrong CAST wrappers.** `stg_customers`,
  `stg_products`, `stg_orders` all join the raw source with a seed and the type
  wrapper is now reasonable (though see narrow-decimal bug below).

## Bugs still present / still have to be worked around

### 1. `smelt run` is not idempotent — second run fails with catalog error

Most serious bug. A fresh build succeeds, but running `smelt run` again without
dropping the database fails with:

```
Catalog Error: Existing object stg_orders is of type Table, trying to drop type View
```

The object _is_ a `BASE TABLE`, the frontmatter _does_ say `materialization: table`,
and `smelt explain` agrees. It looks like smelt issues `DROP VIEW IF EXISTS`
before the `CREATE OR REPLACE TABLE`, which errors on DuckDB when a table exists
with the same name. This also affects `smelt build`.

**Workaround:** delete `smelt_shop.duckdb` between runs (or drop every `main.*`
model object). This rules out incremental development ergonomics.

### 2. Seeds are not visible as `smelt.ref()` targets

The docs say: _"Top-level seeds (files directly in the seed directory) are
available as `smelt.ref()` targets in models."_ In practice, `smelt run` fails
with:

```
Dependency resolution failed:
  Model 'stg_products' references undefined model/source 'category_hierarchy'
```

Seeds load fine (`smelt seed` produces the tables in `main`), but the
dependency resolver doesn't know about them.

**Workaround:** declare every seed as a source in `sources.yml` under the `main`
schema and reference with `smelt.source('main.<seed_name>')`. This is the same
workaround as 0.2.

### 3. Type wrapper chooses narrow integer / decimal types for aggregate SUMs

Without explicit casts the `_smelt_typed` wrapper that wraps every model will
cast aggregate monetary columns to absurd types. Examples observed in this
project before I added `CAST(... AS DOUBLE)`:

- `SUM(line_gross)` where `line_gross` is `DOUBLE` → wrapper cast to `BIGINT`.
  This truncated a $10.8M `net_revenue` into an integer dollar value and a
  `return_rate` of `0.0609…` into `0`.
- `SUM(line_net)` (DECIMAL after multiplication) → DECIMAL(38,10) at the
  intermediate and DECIMAL(2,1) at the mart, which overflows.
- `COUNT(DISTINCT order_id)` → SMALLINT in one mart — would overflow above
  32,767 rows per group.

**Workaround:** wrap every aggregate expression in `CAST(... AS DOUBLE)` (for
money) or `CAST(... AS BIGINT)` (for counts) before the wrapper can narrow it.
Doing this consistently makes types survive end-to-end.

### 4. Data generator: `geometric` without `min: 1` still produces zeros

Same behaviour as 0.2. The spec says quantities are never zero; without
`min: 1` the generator happily emits `quantity = 0`. Working as documented,
but it remains an easy foot-gun for a spec that explicitly forbids zero
quantities. `smelt-datagen --help` doesn't surface `min` at all — I only found
it from the docs.

## Other observations

- **Unknown error / diagnostic stream is inconsistent.** `smelt run --dry-run
  --verbose` produced zero output and exit 0 even when the live run fails.
  `smelt explain` and `smelt run` both print errors to stderr only when they
  actually try to execute. During development it was common to see a totally
  silent "success" and have to re-check the DB.
- **`smelt seed` does not clean up on schema change.** Changing a seed CSV and
  re-running produces `IF EXISTS` / drop issues related to the non-idempotent
  bug above. Easier to just delete the .duckdb file.
- **CUBE + COALESCE pattern has a subtle data-quality trap.** `COALESCE(x,
  'ALL')` is wrong when the source column is nullable, because actual NULLs
  and CUBE-rolled-up NULLs both collapse to `'ALL'`, producing multiple "grand
  total" rows. Use `CASE WHEN GROUPING(x) = 1 THEN 'ALL' ELSE x END` to keep
  rollup rows distinct. (This is a SQL thing, not a smelt bug, but the spec's
  "rollup rows use 'ALL' as sentinel" phrasing makes it easy to do the wrong
  thing.)
- **Test data from `smelt-datagen` does not produce funnel-ordered events.**
  `event_type` is an independent weighted_choice per row, so a session can have
  `purchase=1` but `checkout_start=0`. The funnel mart has to impute step N−1
  whenever step N is present (implemented via GREATEST()) to satisfy the
  "funnel rates must decrease or stay equal" acceptance criterion.

## Reproducing

```bash
uv sync
uv run smelt-datagen --config datagen.yaml
rm -f smelt_shop.duckdb            # because of bug #1
uv run python load_raw.py
uv run smelt build                 # loads seeds + runs all models
uv run python validate.py          # 21/21 checks
```
