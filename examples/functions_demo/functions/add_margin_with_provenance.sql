---
provenance: { margin: [source.revenue, source.cost] }
deterministic: true
---
smelt.define add_margin_with_provenance(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)

