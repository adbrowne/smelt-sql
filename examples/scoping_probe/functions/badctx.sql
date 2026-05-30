smelt.define pick(source: TableExpr<{amount: Numeric}>, pred: Expr<Boolean, source>) -> TableExpr AS (
  SELECT source.* FROM source WHERE pred
)
