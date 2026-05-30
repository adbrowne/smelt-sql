WITH seed AS (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue, 'west' AS region)
SELECT revenue, region FROM smelt.functions.passall(seed)
