---
deterministic: true
provenance: { order_id: [orders.order_id], total: [orders.total], customer_id: [orders.customer_id], customer_name: [dim_customer.customer_name], customer_tier: [dim_customer.customer_tier] }
joins:
  - table: dim_customer
    on: orders.customer_id = dim_customer.customer_id
---
smelt.define enriched_order(orders: TableExpr<{order_id: BigInt, customer_id: Text, total: Numeric}>) -> TableExpr AS (
  SELECT
    orders.order_id,
    orders.customer_id,
    orders.total,
    dim_customer.customer_name AS customer_name,
    dim_customer.customer_tier AS customer_tier
  FROM orders
  LEFT JOIN smelt.models.dim_customer AS dim_customer
    ON orders.customer_id = dim_customer.customer_id
)

