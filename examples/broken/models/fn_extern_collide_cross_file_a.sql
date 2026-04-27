-- Phase 53 fixture (cross-file extern same-name collision — file A).
-- Alphabetically first: no diagnostic fires here; the error anchors on _b.sql.
---
backends: [duckdb]
---
smelt.extern cross_file_extern(x: Expr<Integer>) -> Expr<Integer>
