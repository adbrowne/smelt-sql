-- Staged store dimension
SELECT
    store_id,
    COALESCE(store_type, 'unknown') AS store_type,
    region,
    country,
    CASE
        WHEN store_size = 'Flagship' THEN 'XL'
        WHEN store_size = 'Large' THEN 'L'
        WHEN store_size = 'Medium' THEN 'M'
        WHEN store_size = 'Small' THEN 'S'
        ELSE 'Unknown'
    END AS store_size_code
FROM smelt.source('raw.stores')
