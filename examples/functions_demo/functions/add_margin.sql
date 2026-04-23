smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
