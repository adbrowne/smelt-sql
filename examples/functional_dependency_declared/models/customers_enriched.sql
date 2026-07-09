---
materialization: view
functional_dependencies:
  - key: [customer_id]
    determines: customer_region
---

-- `customer_region` is a per-key constant under `customer_id` — a fact this
-- plain pass-through SELECT cannot prove statically (no join, no aggregate).
-- The `functional_dependencies` declaration above widens that undecidable
-- verdict for once-write enrichment; no transform consumes it here (DC2
-- builds the declaration + its widening/guard only).
SELECT
    customer_id,
    customer_region
FROM (VALUES (1, 'us-east'), (2, 'eu-west')) AS t(customer_id, customer_region)
