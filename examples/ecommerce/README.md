# Ecommerce Example Workspace

A realistic ecommerce analytics pipeline demonstrating smelt's core features. This workspace exercises the SQL patterns most common in production data pipelines.

## Structure

```
ecommerce/
  smelt.yml              # Project configuration
  sources.yml            # Raw source table definitions
  seeds/
    category_hierarchy.csv   # Product category lookup (smelt.ref() target)
    order_statuses.csv        # Order status reference (smelt.ref() target)
  models/
    staging/
      stg_products.sql        # Products + category join (seed ref, division, JOINs)
      stg_orders.sql          # Orders + status join (subquery, seed ref)
      stg_events.sql          # Clickstream events (EXTRACT, TIMESTAMP)
    intermediate/
      int_order_enriched.sql  # Order + product details (CTEs, multiple JOINs)
      int_sessions.sql        # Session aggregation (CASE WHEN with OR, GROUP BY)
    marts/
      mart_funnel.sql         # Funnel conversion metrics (CASE in aggregates)
```

## Patterns demonstrated

| Pattern | Model |
|---------|-------|
| Seed as `smelt.ref()` target | `stg_products.sql`, `stg_orders.sql` |
| Integer / decimal division | `stg_products.sql` (`unit_price_cents / 100.0`) |
| `EXTRACT(EPOCH FROM ...)` | `stg_events.sql` |
| CTE with OR in CASE WHEN | `int_sessions.sql` |
| Multi-table JOIN type inference | `int_order_enriched.sql` |
| Subquery in FROM with seed ref | `stg_orders.sql` |
| CASE in aggregate (`COUNT(CASE ...)`) | `mart_funnel.sql` |

## Running

```bash
# Load seeds and run all models
smelt build

# Run with results preview
smelt build --show-results

# Run a specific model
smelt run --select stg_products

# Check for type errors (LSP)
smelt check
```

## What this tests

This workspace was originally built as part of the smelt_shop validation report to expose real-world bugs in smelt 0.2.0. All 6 critical/major bugs that required workarounds in the original report have been fixed; the models in this workspace are written as clean SQL without workarounds.

The `ecommerce_no_diagnostics` test in `crates/smelt-cli/tests/example_diagnostics.rs` verifies there are no LSP errors. The `test_ecommerce_models_compile_and_execute` test in `crates/smelt-cli/tests/ecommerce_execution.rs` verifies all models compile to valid DuckDB SQL and execute without errors.
