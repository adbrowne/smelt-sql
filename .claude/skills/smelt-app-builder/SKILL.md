---
name: smelt-app-builder
description: Use this skill whenever you are scaffolding, building, or debugging a smelt data project from scratch — i.e., the user wants you to set up a smelt.yml, write SQL models, run `smelt build`, or otherwise produce a working analytics pipeline using the smelt CLI. Also use it when a spec describes "build a data pipeline", "build an analytics layer", or similar even if smelt isn't named explicitly. The smelt CLI ships its own user-facing docs — read them via `smelt docs list` and `smelt docs show <topic>` rather than guessing.
---

# Building smelt projects

You are starting from an empty directory and need to deliver a working smelt project. This skill covers the fast path plus the gotchas that aren't obvious from the docs alone. Lean on `smelt docs show <topic>` for the canonical reference; this file is a meta-guide for the build *flow*.

## First moves (do these before reading the spec deeply)

1. `smelt docs list` — see what topics exist. Bookmark `getting-started/quickstart`, `reference/smelt-yml`, `reference/sources-yml`, `reference/cli`, `guide/sql-models`, `guide/seeds`, `guide/sources`.
2. `smelt --help` and `smelt build --help` — understand the CLI surface.
3. `smelt docs show getting-started/quickstart` — minimum viable project. Build this first, even if the spec is bigger.

## Project skeleton

A minimum-viable smelt project looks like this:

```
my-project/
├── smelt.yml          # project config
├── seeds/             # CSV seed data
│   └── *.csv
├── sources.yml        # (optional) declares external tables
└── models/
    └── *.sql
```

**`smelt.yml`** (verbatim minimum):

```yaml
name: my-project
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  dev:
    type: duckdb
    database: my-project.duckdb
    schema: main
```

**Model file format** — every `.sql` model has YAML frontmatter:

```sql
---
name: stg_orders
materialization: table
---
SELECT
    CAST(o.order_id AS INTEGER) AS order_id,
    ...
FROM smelt.sources.raw.orders o
LEFT JOIN smelt.models.seed_order_statuses s ON o.status_code = s.status_code
```

- Reference seeds and other models with `smelt.models.<name>` (seeds are first-class ref targets — name = filename minus `.csv`).
- Reference declared sources with `smelt.sources.<schema>.<table>`.
- Materializations: `table` (default for marts), `view`, `incremental` — see `smelt docs show guide/materializations`.

## Build loop

```bash
smelt build              # seed + run, idempotent — re-running is safe and won't error
smelt build --verbose    # show compiled SQL per model (great for debugging)
smelt build --dry-run --verbose   # parse + compile, no execution
```

`smelt build` is idempotent on DuckDB targets — it will not error if tables already exist. You do *not* need to delete the `.duckdb` file between iterations.

To rebuild a subset:

```bash
smelt build --select stg_orders --select int_revenue   # repeat the flag, do NOT do `--select a b c`
```

The repeated-flag form is mandatory; positional values after `--select` will fail with "unexpected argument".

## Install gotchas (only relevant outside this dir's pre-installed venv)

When installing smelt yourself in a fresh venv:

```bash
uv venv --python 3.11      # 0.3.1 wheels exist for cp311 manylinux only
uv pip install --only-binary=smelt-sql smelt-sql
```

The `--only-binary=smelt-sql` is required: the source distribution on PyPI is broken (it tries to `cargo metadata` against missing workspace members). If you skip the flag and pip falls back to sdist, expect `failed to load manifest for dependency 'smelt-backend-spark'`.

`smelt-datagen` is bundled inside the `smelt-sql` wheel — do not try to `pip install smelt-datagen` separately.

## Sources (only if the spec mentions external tables)

If the spec says "data lands as parquet" or "raw tables come from elsewhere", you'll need a `sources.yml`:

```yaml
version: 1
sources:
  raw:
    tables:
      orders:
        description: "Raw orders"
        columns:
          - { name: order_id, type: INTEGER }
          - { name: customer_id, type: INTEGER }
          - { name: order_timestamp, type: VARCHAR }
```

Then reference with `smelt.sources.raw.orders`.

**Parquet caveat:** the bundled DuckDB cannot auto-fetch the parquet extension (HTTP 404 from `extensions.duckdb.org`). If your spec's raw data is parquet, **materialize it into DuckDB tables before running smelt** — write a tiny Python script:

```python
import duckdb
con = duckdb.connect("my-project.duckdb")
con.execute("CREATE SCHEMA IF NOT EXISTS raw")
con.execute("CREATE OR REPLACE TABLE raw.orders AS SELECT * FROM read_parquet('output/orders/*.parquet')")
```

…then `smelt build` reads from those tables happily. Trying to use `read_parquet` *inside* a smelt model will fail at runtime.

## Stuck-points checklist

If `smelt build` fails, work through these before changing approach:

- **"Unknown ref / source"** → run `smelt docs show concepts/project-structure`. Confirm seed CSV is under `seeds/` and the model frontmatter `name:` matches what other models call via `smelt.models.<name>`. Seed names = seed filename minus `.csv`.
- **YAML frontmatter parse error** → the `---` fences must be on their own lines, with valid YAML between. No tabs.
- **Type errors on aggregates** → `SUM`/`COUNT` infer as non-null. If your downstream model assumes nullable for `LEFT JOIN`-fed sums, wrap in `COALESCE(SUM(x), 0)`.
- **`smelt diff` reports phantom nullability changes after a clean build** → known issue; safe to ignore for app correctness, but don't use `smelt diff` as a CI gate yet.
- **Stale model cache after deleting a `.sql` file** → `rm .smelt/schemas/<deleted_model>.json` manually.

## Iteration discipline

- Build a *minimum* model first (one seed → one staging model → `smelt build`) before adding the rest. Verify output with `duckdb my-project.duckdb` + `SELECT * FROM stg_orders LIMIT 5`.
- Add models in dependency order: seeds → staging → intermediate → marts.
- After every 1-2 new models, `smelt build` again. Don't write the whole project blind.
- When debugging compiled SQL, `smelt build --verbose` is your friend; for plan inspection without execution, `smelt build --dry-run --verbose`.

## When you finish (or get stuck)

Write `retro.md` in the run directory with structured sections:

```markdown
# Retro

## confusion
What was hard to figure out, and where you eventually found the answer.

## doc_gaps
Things the docs *should* cover but don't, with the docs topic that was closest to where you needed help.

## skill_gaps
Knowledge you wish was in this skill — concrete, actionable additions.

## suspected_tool_bugs
Behaviour that looked like a smelt bug rather than a misunderstanding. Include the exact command and the unexpected output.
```

Be honest in the retro — it feeds the loop that improves this skill and the docs.
