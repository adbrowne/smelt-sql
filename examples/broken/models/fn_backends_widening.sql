-- Phase 11 (smelt-functions) — broken fixture for the
-- `BackendsWideningNotAllowed` diagnostic.
--
-- Body calls `duckdb.read_parquet(...)`, so the inferred backend set is
-- `[duckdb]`. The declared `backends: [duckdb, spark]` is a widening
-- (spark is not implied by the body). The §16 #23 narrow-only rule
-- rejects this.

---
backends: [duckdb, spark]
---
smelt.define load_broken(path: Expr<Text>) -> Expr<Text>
    AS (duckdb.read_parquet(path))
