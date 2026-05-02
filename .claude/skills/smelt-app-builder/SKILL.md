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
- Materializations: `table`, `view`, `incremental` — see `smelt docs show guide/materializations`. **If you omit `materialization:` from frontmatter the model is built as a `table`** (so most marts and staging models can leave it off).

## Build loop

```bash
smelt build              # seed + run, idempotent — re-running is safe and won't error
smelt build --verbose    # extra detail when models actually run; a no-op rebuild prints nothing extra
smelt build --show-plan path/to/model.sql   # compile a single model without executing (positional arg required)
```

`smelt build` does **not** accept `--dry-run`; do not pass it. There is currently no project-wide "compile only" flag — `--show-plan` works per-model.
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

The bundled venv's `python` ships DuckDB but **not** `numpy`/`pandas`, so `con.execute(...).fetchdf()` raises `ModuleNotFoundError: numpy`. Use `.fetchall()` (or `.arrow()` if you really need a dataframe and install pyarrow yourself) when scripting validation queries. Copy-pasteable validation shape:

```python
import duckdb
con = duckdb.connect("my-project.duckdb")
rows = con.execute("SELECT customer_id, revenue FROM mart_top_customers ORDER BY revenue DESC").fetchall()
for r in rows:
    print(r)
# tuple-of-tuples; no pandas required

# Expected-schema check (DuckDB DESCRIBE returns (name, type, ...) per column):
got = [(r[0], r[1]) for r in con.execute("DESCRIBE mart_top_customers").fetchall()]
expected = [("customer_id", "INTEGER"), ("customer_name", "VARCHAR"), ("total_revenue", "DOUBLE")]
assert got == expected, f"schema mismatch: {got}"
```

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
- **Type errors on aggregates** → `SUM`/`COUNT` infer as non-null, and `COUNT(*)` lands as `BIGINT` (not `INTEGER`). For `LEFT JOIN`-fed sums where the right side may be empty, wrap in `COALESCE(SUM(...), 0)`; if a downstream column or test expects `INTEGER`, add an outer `CAST(... AS INTEGER)`. A worked mart pattern: `SELECT c.customer_id, COALESCE(SUM(CASE WHEN o.status = 'shipped' THEN o.amount END), 0) AS revenue FROM smelt.models.raw_customers c LEFT JOIN smelt.models.stg_orders o USING (customer_id) GROUP BY c.customer_id` — ensures every customer appears with `0` revenue instead of `NULL`.
- **`smelt diff` reports phantom nullability changes after a clean build** → known issue; safe to ignore for app correctness, but don't use `smelt diff` as a CI gate yet.
- **Stale model cache after deleting a `.sql` file** → `rm .smelt/schemas/<deleted_model>.json` manually.

## Iteration discipline

- Build a *minimum* model first (one seed → one staging model → `smelt build`) before adding the rest. Verify output with `duckdb my-project.duckdb` + `SELECT * FROM stg_orders LIMIT 5`.
- After the first `smelt build` (which materializes seeds), run `duckdb my-project.duckdb -c 'DESCRIBE raw_<seed>'` to see physical types, **and** `smelt table <staging_model>` after building each staging model to see smelt's *inferred* types. The two can disagree even on a passthrough `SELECT col` — e.g. DuckDB may store a column as `DATE` while smelt infers `TEXT`, and smelt's inferred types govern downstream type-checking and the materialized column types. When the spec dictates a target type, `CAST` explicitly in staging rather than trusting the seed's type to flow through. Date-shaped strings landing as `VARCHAR`, and numeric CSVs landing as `DOUBLE` rather than `DECIMAL`, want the same fix.
- Add models in dependency order: seeds → staging → intermediate → marts.
- After every 1-2 new models, `smelt build` again. Don't write the whole project blind.
- **If the spec asks for `smelt.define` functions**, read `smelt docs show guide/functions` first — the call-path rule is easy to get wrong, and `smelt build --show-plan models/<m>.sql` is the fastest way to confirm a call resolves before doing a full build. The key rule: the filename stem does **NOT** appear in the call path. `functions/revenue.sql` → `smelt.functions.safe_revenue(...)` (NOT `smelt.functions.revenue.safe_revenue(...)`). Including the stem causes `UnknownSmeltFn` and `smelt build` exits non-zero.
  - A function returning `Expr<Boolean>` composes fine inside `CASE WHEN smelt.functions.<...>(...) THEN ... END` and inside aggregate wrappers like `SUM(CASE WHEN ... )` — no extra cast needed.
  - A function declared `-> Expr<Double>` forces the materialized column to `DOUBLE` regardless of the seed CSV's apparent precision. If a spec says "DECIMAL or DOUBLE", DOUBLE-via-function satisfies it; if it requires DECIMAL specifically, type the function as `Expr<Decimal<...>>` instead.
- For plan inspection without execution, use `smelt build --show-plan <model.sql>` (one model at a time). `smelt build --verbose` only emits extra detail when models actually run.
- **Validate schema, not just rows.** Before declaring done, `DESCRIBE` each output table (or `smelt table <model>`) and compare column types against the spec. Harness validators often check row counts and value sums but not column types, so a `VARCHAR`-vs-`DATE` mismatch will silently pass row-level checks.

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
