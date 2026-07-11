---
materialization: view
bounded_domain:
  column: category
  max_cardinality: 10000
---

-- `category` is asserted to have a bounded active domain (at most 10,000
-- distinct values) — a world-fact this exact `MEDIAN` aggregate cannot
-- decide statically. The `bounded_domain` declaration above widens the
-- otherwise-refused exact-holistic-aggregate verdict for multiset
-- maintenance; no transform consumes it here (this phase builds the
-- declaration + its widening/guard only).
SELECT
    category,
    MEDIAN(amount) AS median_amount
FROM (VALUES
    ('electronics', 100.0),
    ('electronics', 200.0),
    ('groceries', 10.0)
) AS t(category, amount)
GROUP BY category
