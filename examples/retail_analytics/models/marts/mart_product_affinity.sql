-- Product affinity via self-join (products frequently bought together)
SELECT
    a.product_id AS product_a,
    b.product_id AS product_b,
    pa.category AS category_a,
    pb.category AS category_b,
    COUNT(DISTINCT a.order_id) AS co_occurrence_count
FROM smelt.models.staging.stg_order_items AS a
INNER JOIN smelt.models.staging.stg_order_items AS b
    ON a.order_id = b.order_id AND a.product_id < b.product_id
INNER JOIN smelt.models.staging.stg_products AS pa ON a.product_id = pa.product_id
INNER JOIN smelt.models.staging.stg_products AS pb ON b.product_id = pb.product_id
GROUP BY a.product_id, b.product_id, pa.category, pb.category
HAVING COUNT(DISTINCT a.order_id) >= 2

