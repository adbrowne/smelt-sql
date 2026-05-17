# Meta-language final reference — Phase G `/smelt-loop` tier-3 outcome

Reference doc landed alongside the Phase G `/smelt-loop` tier-3 run. Captures
the concrete workflow gotchas that the tier-3 build agent surfaced and that
the skill body now warns about. Use this when the per-cohort union demo (or
any generator + downstream union pattern) is part of the spec.

## What the tier-3 fixture exercises

The `large` tier asks for the per-cohort union pattern end-to-end:

1. `configs/cohorts.yaml` — three cohorts keyed by country code.
2. `models/cohorts.gen.sql` with `generates: models` frontmatter — loads the
   YAML via `smelt.config.load_yaml`, maps each cohort to a `ModelDef`
   filtering shipped orders for that country.
3. `models/all_orders.sql` — UNION ALL of the three emitted models.
4. `tests/cohort_count.test.sql` — boolean assertion that the union row count
   matches the sum of per-cohort shipped counts (9 = 3×3).

The Phase G iteration-1 build agent passed all 7 acceptance checks on the
first `smelt build`.

## Workflow gotchas (non-obvious; not in the docs)

### Generator emitted-model naming

The emitted smelt path is `<file-stem>.<emitted-name>`:

- `name: 'us'` in `models/cohorts.gen.sql` → `smelt.cohorts.us` (NOT
  `smelt.us`).
- DuckDB substitutes `_` for `.`, so the physical table is `cohorts_us`.

`smelt build --show-plan models/cohorts.gen.sql` is currently terse for
generator files — it shows the top-level `load_yaml` call but does not list
the emitted ModelDefs. After `smelt build`, verify the emitted tables exist
via the catalog:

```bash
duckdb cohort_pipeline.duckdb -c \
  "SELECT table_name FROM information_schema.tables WHERE table_name LIKE 'cohorts_%'"
```

### Two test layouts coexist; one may silently skip

Specs and fixtures sometimes use a `materialization: test` file with a body
that is a single boolean `SELECT … AS passes`. The documented form (see
`smelt docs show guide/testing`) is a `test:`/`inputs:`/`expect:` YAML block.

Form (a) compiles into the DuckDB artifact silently during `smelt build`, but
`smelt test` may report `0 passed, 0 failed, 0 total` without mentioning the
file at all. When in doubt, validate the assertion directly:

```bash
duckdb cohort_pipeline.duckdb -c "SELECT passes FROM cohort_count"
```

### `tests/` is just another scanned directory

If the spec wants a `tests/` directory, add `tests` to `paths:` in
`smelt.yml` alongside `models` and `seeds`. Any `.sql` file under a listed
path is discovered regardless of subdirectory or filename suffix
(`*.test.sql` is convention, not a discovery rule).

## Cross-references

- The skill body's "Generator files & tests" subsection (added 2026-05-17)
  encodes the discovery rules above.
- For full meta-language surface, see `smelt docs show meta-language/index`
  and the dedicated pages (`generators`, `config-loaders`, `reflection`,
  etc.).
- The phase A–F reference docs in this directory cover incremental surface:
  - `20260509-meta-lists.md` — `List<T>`, literals, spread.
  - `20260510-meta-hofs.md` — HOFs, lambdas, pipe, reducers, `smelt.config.var`.
  - `20260510-meta-columns.md` — `smelt.columns_of`, `ColumnRef`.
  - `20260512-meta-workspace.md` — `smelt.models.*` / `smelt.sources.*`.
  - `20260513-meta-records-maps-loaders.md` — records, `Map<K,V>`, YAML/JSON loaders.
  - `20260515-meta-multi-model-production.md` — `generates: models`, `ModelDef`.
  - `20260516-meta-polish.md` — multi-arg lambdas, parameterised reducers, ternary.
