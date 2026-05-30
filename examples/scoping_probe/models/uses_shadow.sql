WITH seed AS (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue, 'west' AS region)
SELECT * FROM smelt.functions.filter_rev(seed, revenue => 50)
