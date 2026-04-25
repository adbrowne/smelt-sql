---
deterministic: true
unknown_property: foo
---
smelt.define fn_frontmatter_unknown_key(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
