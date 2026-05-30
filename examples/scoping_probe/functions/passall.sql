smelt.define passall(source: TableExpr<{revenue: Numeric, region: Text}>) -> TableExpr AS (
  SELECT source.* FROM source
)
