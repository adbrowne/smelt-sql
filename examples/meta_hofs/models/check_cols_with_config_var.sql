-- Happy path: use smelt.config.var to read compile-time values from smelt.yml.
-- smelt.config.var('region') resolves to 'us-west-2'.
-- smelt.config.var('min_revenue') resolves to '100'.
-- smelt.config.var('flag') resolves to 'true'.
-- No source table: check_type_diagnostics early-returns, avoiding unrelated
-- UndeclaredColumn checks. Only meta-language checks in check_file_diagnostics run.
SELECT
    smelt.config.var('region'),
    smelt.config.var('min_revenue'),
    smelt.config.var('flag')
