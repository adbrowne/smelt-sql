smelt.define with_helper(source: TableExpr) -> TableExpr AS (
  WITH helper AS (
    SELECT source.* FROM source
  )
  SELECT * FROM helper
)
