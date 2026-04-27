-- Phase 4 (smelt-functions) negative fixture: a parameter annotated with an
-- unsupported sort keyword. Before Phase 13 this fixture used `TableExpr<T>`;
-- Phase 13 made `TableExpr` a first-class (parser-accepted) sort head, so
-- the fixture now uses `TableExprWeirdo<T>` — still an unrecognised sort
-- keyword, which `smelt-types::signatures::parse_smelt_type` rejects with
-- `SmeltTypeParseError::UnsupportedSort`; `smelt-db` surfaces that as
-- `DiagnosticCode::InvalidFunctionTypeRef`. The message substring
-- `TableExpr` still matches since the unknown sort name starts with it.
smelt.define bad(x: TableExprWeirdo<T>) AS (x)
