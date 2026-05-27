-- This function declares a return type with an unrecognized type name in a
-- struct field position. `Bogus` is not a known DataType, so this annotation
-- must emit `InvalidFunctionTypeRef` at the declaration.
smelt.define make_event() -> Expr<Struct<{a: Integer, b: Bogus}>> AS (1)
