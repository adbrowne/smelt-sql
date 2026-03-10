-- Staged order items with price conversion and return handling
SELECT
    order_item_id,
    order_id,
    product_id,
    CAST(quantity AS INTEGER) AS quantity,
    CAST(unit_price_cents AS FLOAT) / 100.0 AS unit_price,
    COALESCE(return_flag, false) AS is_returned,
    CASE
        WHEN return_flag = true THEN COALESCE(return_reason, 'Unspecified')
        WHEN return_flag = false THEN 'N/A'
    END AS return_reason,
    CAST(item_date AS DATE) AS item_date
FROM smelt.source('raw.order_items')
