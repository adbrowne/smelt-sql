WITH seed AS (SELECT 10 AS amount)
SELECT * FROM smelt.functions.pick(seed) PASSING pred AS (amount > 5)
