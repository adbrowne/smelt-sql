-- Phase 11: two declarations in the same file, each preceded by its own
-- per-declaration frontmatter block.
--
-- 1. `coerce_to_text` — a `smelt.define` that casts any `Expr<Numeric>` to
--    Text. Declared `backends: [duckdb]` narrows the body's inferred set
--    (`all`, since CAST is generic SQL). Accepted under the narrow-only rule.
--
-- 2. `read_parquet` — a `smelt.extern` using the `duckdb.<name>` backend-
--    namespace sugar. Equivalent to the explicit
--    `---\nbackends: [duckdb]\n---\nsmelt.extern read_parquet(...)` form.
--    Registered under the name `read_parquet`, callable as
--    `smelt.fn.read_parquet(...)`.

---
backends: [duckdb]
---
smelt.define coerce_to_text(x: Expr<Numeric>) -> Expr<Text> AS (CAST(x AS TEXT))

smelt.extern duckdb.read_parquet(path: Expr<Text>) -> Expr<Text>

