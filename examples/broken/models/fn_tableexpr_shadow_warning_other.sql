-- Companion for `fn_tableexpr_shadow_warning.sql`. Supplies a caller
-- model whose schema contains a `user_id` column so the shadow warning
-- on the `shadow_demo` function declaration fires.
SELECT
  CAST(NULL AS BIGINT) AS user_id,
  CAST(NULL AS VARCHAR) AS event_type
