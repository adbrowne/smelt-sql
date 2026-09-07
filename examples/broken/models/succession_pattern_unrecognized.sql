---
materialization: table
refresh: incremental
---
-- SuccessionPatternUnrecognized: `GROUP BY` on the scope is not this shape
-- — a `refresh: incremental` model with no `unique_key`, no `timeseries:`,
-- and a SQL shape none of the other succession rules individually names
-- (`docs/specs/model_properties.md` §"Keyed-succession classification").
-- Fix: declare `refresh: full`, `refresh: materialized_view`, or reach
-- another grain via `unique_key`/`timeseries`.
SELECT customer_id, COUNT(*) AS n
FROM smelt.sources.succession_changes
GROUP BY customer_id
