smelt.define add_margin(source: TableExpr) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
