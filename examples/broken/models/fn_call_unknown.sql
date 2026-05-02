-- Phase 6 broken fixture: `smelt.functions.does_not_exist` is not declared in the
-- workspace. The call site must emit exactly one `UnknownSmeltFn` diagnostic,
-- anchored at the call-path span.
SELECT smelt.functions.does_not_exist(1) AS r
