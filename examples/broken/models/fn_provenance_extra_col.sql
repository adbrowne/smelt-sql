---
provenance: { margin: [source.revenue, source.cost, dim.extra] }
---
smelt.define fn_provenance_extra_col(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT revenue - cost AS margin FROM source
)
