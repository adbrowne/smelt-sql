-- Union all per-cohort emitted models into a single table.
--
-- References the three models emitted by cohorts.gen.sql:
-- smelt.cohorts.us_west, smelt.cohorts.us_east, smelt.cohorts.eu
-- These are tagged 'cohort' and are visible here after the W3 emission pass.
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_west
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_east
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.eu
