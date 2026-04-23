-- Companion for `fn_row_requirement_missing.sql`. Declares a model
-- with a `revenue` column but deliberately no `cost`, so the Phase 16
-- row-requirement check on `add_margin_req` fails at the call site
-- with `RowRequirementUnsatisfied` on the missing `cost` column.
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS DECIMAL(18, 2)) AS revenue
