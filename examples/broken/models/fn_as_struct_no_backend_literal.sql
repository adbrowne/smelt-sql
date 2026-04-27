-- Phase 38 (smelt-functions) — broken fixture for the
-- `AsStructUnsupportedBackend` diagnostic.
--
-- The function declares `backends: [no_struct_db]`. The body uses
-- `smelt.as_struct(source)`, which requires struct-literal SQL support.
-- Since `no_struct_db` is not a known struct-literal backend, the
-- `AsStructUnsupportedBackend` diagnostic fires.

---
backends: [no_struct_db]
---
smelt.define fn_as_struct_no_backend_literal(source: TableExpr) -> TableExpr AS (
    SELECT smelt.as_struct(source) AS s FROM source
)
