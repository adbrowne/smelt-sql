---
date: 2026-04-17
status: red-baseline
test: crates/smelt-cli/tests/smelt_shop_idempotency.rs::smelt_shop_min_idempotency_and_types
workspace: examples/smelt_shop_min/
phase: A2 (regression scaffold; no fixes applied)
---

# A2 baseline failure capture

This is the verbatim failure of the new
`smelt_shop_min_idempotency_and_types` test against `main` (commit
`f8e1ec4` at the time of capture), proving the test is red for the right
reasons before Phase B fixes start.

The test exercises bugs #1, #2, #3 from `~/smelt_shop/FINDINGS.md`
simultaneously by shelling out to the compiled `smelt` binary. The bugs
fire in dependency order — fixing one reveals the next. This document
captures the bug that fires *first* on `main` (#2), and notes which
assertion will surface bug #1 and bug #3 once each preceding fix lands.

## Run command

```sh
cargo test -p smelt-cli --test smelt_shop_idempotency -- --nocapture
```

(System DuckDB; `DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb`.)

## Current failure on `main` — bug #2 (CLI dependency validator ignores seeds)

```
thread 'smelt_shop_min_idempotency_and_types' panicked at
crates/smelt-cli/tests/smelt_shop_idempotency.rs:132:9:
`smelt build` (run #1) failed (exit ExitStatus(unix_wait_status(256))); stderr:
Error: Dependency validation failed

Caused by:
    Dependency resolution failed:
      Model 'stg_orders' references undefined model/source 'order_statuses'
```

This matches the bug #2 wording in `FINDINGS.md` and the root cause
documented in `docs/research/20260417-0.3-regression-triage.md` (the
`LogicalGraph::validate` path at
`crates/smelt-cli/src/logical_graph.rs:101-123` never sees the seed list).

The seeding step itself succeeds — the first `smelt build` invocation
loads `main.order_statuses` (3 rows) before the dependency validator
rejects `stg_orders`'s reference to it. So the failure is specifically in
graph resolution, not seed loading. This rules out a "seed missing"
explanation and confirms bug #2 as classified.

## Bugs that will surface as B-phase fixes land

The test contains three independent assertion clusters. They fire in
order; each B-phase flips one to green:

| Order | Assertion | Bug | Failure mode | Phase that fixes |
|-------|-----------|-----|--------------|------------------|
| 1     | `run_smelt_build(.., "run #1")` succeeds | #2 (seeds-as-refs) | Dependency validator rejects `smelt.ref('order_statuses')` | **B2** |
| 2     | `run_smelt_build(.., "run #2")` succeeds | #1 (idempotency) | DuckDB Catalog Error: `DROP VIEW IF EXISTS stg_orders` against an existing Table | **B1** |
| 3a    | `typeof(net_revenue) = 'DOUBLE'` | #3 (aggregate narrowing) | Returns `'BIGINT'` because `_smelt_typed` wrapper builds an empty `TypeContext`, so `SUM(unknown)` falls through to BIGINT | **B3** |
| 3b    | `typeof(unique_orders) = 'BIGINT'` | #3 (aggregate narrowing) | May return `'SMALLINT'` in configs where the wrapper observes the literal range | **B3** |
| 3c    | `typeof(gross_value) = 'DOUBLE'` | #3 (aggregate narrowing) | Returns `'BIGINT'` for the same reason as 3a | **B3** |
| 3d    | `(actual_net_revenue - expected).abs() < 0.01` | #3 (silent data loss) | When SUM is narrowed to BIGINT, the ~$18 500 of fractional cents on 50 000 rows disappears | **B3** |

Bug #1's catalog error (`Existing object stg_orders is of type Table,
trying to drop type View`) cannot be observed today because run #1 fails
first. Bug #3's narrowing similarly cannot be observed without runs #1
and #2 succeeding. As B2 → B1 → B3 each land, the next assertion in this
table becomes the load-bearing one.

## Why this scaffold is the right one

Per the cross-cutting principle in
`docs/plans/20260417-smelt-shop-0.3-followup.md`:

> Tests must use the compiled CLI binary, not `smelt-cli` as a library.
> The prior plan's library-level tests caught none of bugs #1, #2, or #3.

This test does exactly that. `Command::new(env!("CARGO_BIN_EXE_smelt"))`
runs the same binary an end user installs, against a workspace shaped
like `~/smelt_shop`. The `_smelt_typed` wrapper, the dependency
validator, and the drop-then-create logic all execute through their
production code paths.
