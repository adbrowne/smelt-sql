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
smelt.define middle(z) AS (smelt.fn.inner_unary(z))

---
backends: [duckdb]
---
smelt.define outer_call(y) AS (smelt.fn.middle(y))
