-- Phase 4 (smelt-functions) negative fixture: a parameter annotated with an
-- unsupported sort (`TableExpr<T>`). The parser accepts the flat type-ref
-- tokens, but structured parsing in `smelt-types::signatures::parse_smelt_type`
-- rejects non-`Expr` sorts with `SmeltTypeParseError::UnsupportedSort`,
-- which `smelt-db` surfaces as `DiagnosticCode::InvalidFunctionTypeRef`.
smelt.define bad(x: TableExpr<T>) AS (x)
