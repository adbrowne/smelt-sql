-- Intentional error: numeric literal + string literal across type families.
-- The compiler must not synthesise an implicit cast across families (spec §1).
-- Emits: TypeMismatch at the `+` operator span.
SELECT 42 + '3'
