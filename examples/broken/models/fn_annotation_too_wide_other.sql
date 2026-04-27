-- Companion for `fn_annotation_too_wide.sql`. Provides `annot_source`
-- with columns {id, amount, name} so the annotation check has real
-- column schemas to compare against the HAVING-restricted inferred context.
SELECT
  CAST(NULL AS INTEGER) AS id,
  CAST(NULL AS DOUBLE) AS amount,
  CAST(NULL AS VARCHAR) AS name
