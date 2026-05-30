WITH seed AS (SELECT 1 AS id)
SELECT * FROM smelt.functions.cyc(seed)
