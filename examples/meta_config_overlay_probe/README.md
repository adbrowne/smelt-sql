# meta_config_overlay_probe

Adversarial fixture for `docs/specs/meta_config_loading.md` §"Per-target overlay"
and §"Validation diagnostics", authored in the B5 feature-sweep phase.

`models/cohorts.gen.sql` loads `cohorts.yaml` as `List<{name, region, min_revenue}>`
and maps each row to a model that filters `orders` by `revenue >= min_revenue`.
A sibling overlay `cohorts.prod.yaml` raises `min_revenue` to `999` for `--target prod`.

## What it reproduces (both `needs-review` in `docs/bug-hunt/2026-05-30-findings.md`)

- **BUG-014** — per-target overlay is unwired in the production run/generator pipeline.
  `smelt build --target prod` emits the *base* value (`revenue >= 100`, not the
  overlay's `>= 999`); `duckdb target/prod.duckdb "select * from cohorts_west"`
  returns the row the overlay should have filtered out. Replacing the overlay with a
  schema-violating file also builds exit-0 with no diagnostic (overlay validation
  never runs).

- **BUG-015** — making the base `cohorts.yaml` invalid (omit a required field) makes
  `smelt build` report `built 1 model(s)` exit-0: the generated `cohorts_west`
  silently vanishes with no `ConfigLoaderRequiredFieldMissing`.

## Manual repro

```bash
cd examples/meta_config_overlay_probe
../../target/debug/smelt build --target prod
duckdb target/prod.duckdb \
  "select sql from duckdb_views() where view_name='cohorts_west'"
# Observed: WHERE … revenue >= 100   (BUG-014: should be >= 999 from the overlay)
```
