-- Phase C worked example: coalesce all numeric columns of a model, replacing
-- NULL with 0 via COALESCE. Mirrors research §5.2.
--
-- smelt.columns_of(t) materialises the column list of `t` at expansion time.
-- filter keeps only the numeric columns (c.is_numeric = true).
-- map rewrites each numeric column to COALESCE(c.name, 0).
--
-- Note: the `AS c.name` alias from research §5.2 is a Phase D feature
-- (meta-Text-as-identifier in alias position). This fixture exercises the
-- Phase C surface: smelt.columns_of, filter, and map with c.is_numeric.
--
-- The function is skipped by the definition-time body checker (TableExpr
-- params are deferred to call-site expansion). No definition-time diagnostics
-- fire for this file.
smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (
    smelt.columns_of(t)
      |> filter(fn c => c.is_numeric)
      |> map(fn c => COALESCE(c.name, 0))
)
