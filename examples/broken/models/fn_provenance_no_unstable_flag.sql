---
provenance: { margin: [source.revenue, source.cost] }
---
smelt.define fn_provenance_no_flag(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
