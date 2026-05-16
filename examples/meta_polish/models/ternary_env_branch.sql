-- Demonstrates meta-world ternary driven by a compile-time variable.
-- `smelt.config.var('env')` resolves at compile time from the workspace
-- `vars:` block. With `env: dev`, the engine sees only `SELECT 'permissive'`.
-- Switching `vars.env` to `'prod'` flips the result without re-editing the
-- model. The unreached branch is type-checked but not evaluated.
-- No source table: check_type_diagnostics early-returns, so only the
-- meta-language checks in check_file_diagnostics run.
SELECT if smelt.config.var('env') = 'prod' then 'strict' else 'permissive'
