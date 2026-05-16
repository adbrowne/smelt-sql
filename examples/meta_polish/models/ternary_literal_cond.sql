-- Demonstrates meta-world ternary with a compile-time Boolean condition.
-- `if true then 'strict' else 'permissive'` resolves to 'strict' at compile
-- time (short-circuit: the else branch is type-checked but not evaluated).
-- No source table: check_type_diagnostics early-returns, so only the
-- meta-language checks in check_file_diagnostics run.
SELECT if true then 'strict' else 'permissive'
