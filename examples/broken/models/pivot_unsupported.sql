-- PIVOT is not supported: output schema cannot be determined at compile time
SELECT *
FROM (SELECT department, quarter, revenue FROM smelt.models.quarterly_revenue)
PIVOT (SUM(revenue) FOR quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))
