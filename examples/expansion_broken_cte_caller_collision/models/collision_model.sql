-- BUG-007 fixture: this model declares a CTE named `helper` and passes
-- it to a function whose body also declares a CTE named `helper`.
-- Expected diagnostic: CteShadowsCallerCte on the smelt.functions.with_helper call.
WITH helper AS (
  SELECT 1 AS value
)
SELECT * FROM smelt.functions.with_helper(helper)
