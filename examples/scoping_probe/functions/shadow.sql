smelt.define filter_rev(source: TableExpr<{revenue: Numeric, region: Text}>, revenue: Expr<Numeric>) -> TableExpr AS (
  SELECT source.* FROM source WHERE source.revenue > revenue
)
