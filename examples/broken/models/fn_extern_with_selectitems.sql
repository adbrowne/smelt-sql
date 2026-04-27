-- Phase 52 broken fixture: `smelt.extern` with a SelectItems parameter.
-- Must emit ExternFragmentParamUnsupported — fragment sorts require PASSING
-- clauses which smelt.extern does not support (§16 #18).
smelt.extern fn_bad_extern(items: SelectItems<Agg>) -> TableExpr
