---
deterministic: true
provenance: { margin: [source.revenue, source.cost
---
smelt.define fn_frontmatter_malformed(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
