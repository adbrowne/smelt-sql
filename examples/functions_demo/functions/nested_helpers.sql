-- Phase 12: multi-level expansion chain for the happy-path. The paired
-- broken fixture `examples/broken/models/fn_nested_call_error.sql`
-- exercises the three-frame cascade.
--
-- Parameters are intentionally untyped so the call sites type-check
-- without a constraint violation. On the happy path
-- (see `models/uses_nested_helpers.sql`) the chain yields no
-- diagnostics; the broken fixture replaces `inner_unary`'s body with an
-- `undefined_var` reference to force a three-frame UnknownIdentifier
-- cascade.
---
backends: [duckdb]
---
smelt.define inner_unary(x) AS (x + 1)

---
backends: [duckdb]
---
smelt.define middle(z) AS (smelt.functions.inner_unary(z))

---
backends: [duckdb]
---
smelt.define outer_call(y) AS (smelt.functions.middle(y))

-- Phase 14 (§16 #24): a window-function call in scalar position. The
-- SELECT-list splice point accepts every kind, so a `ROW_NUMBER() OVER
-- (...)` call type-checks cleanly. The fixture exists to keep the
-- happy-path diagnostic-clean while the broken pair
-- (`examples/broken/models/fn_window_in_where.sql`) exercises the
-- WHERE-clause rejection.
---
backends: [duckdb]
---
smelt.define ranked_row() AS (ROW_NUMBER() OVER (ORDER BY 1))

