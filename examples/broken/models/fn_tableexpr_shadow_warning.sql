-- Phase 15 fixture: a function parameter named `user_id` shadows a
-- column of the same name in the caller-supplied `TableExpr` schema
-- (§16 #1). The warning anchors at the parameter declaration, not the
-- body usage. Body still typechecks clean — the parameter resolves
-- first, with FROM-scope columns shadowed.
--
-- Companion `fn_tableexpr_shadow_warning_other.sql` supplies a caller
-- model whose schema includes `user_id`.

smelt.define shadow_demo(user_id: Expr<Text>, source: TableExpr) AS (
  SELECT user_id FROM source
)

SELECT *
FROM smelt.models.shadow_demo(
  'abc',
  smelt.models.fn_tableexpr_shadow_warning_other
) AS s
