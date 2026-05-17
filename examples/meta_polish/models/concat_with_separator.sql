-- Demonstrates parameterised reducer concat_with(sep).
-- reduce(['alpha', 'beta', 'gamma'], concat_with(' OR ')) folds left over the
-- Text list, joining elements with the given separator.
-- No source table: check_type_diagnostics early-returns, so only the
-- meta-language checks in check_file_diagnostics run.
SELECT reduce(['alpha', 'beta', 'gamma'], concat_with(' OR '))
