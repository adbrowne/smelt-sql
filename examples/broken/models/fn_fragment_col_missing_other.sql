-- Companion for `fn_fragment_col_missing.sql`. Provides the upstream model
-- `fragment_col_source` with columns {id, amount}.
SELECT
  CAST(NULL AS INTEGER) AS id,
  CAST(NULL AS DOUBLE) AS amount
