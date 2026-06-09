-- This fixture triggers UnknownStructFieldType: the struct field `b` has an
-- unrecognised type name `Bogus`.
smelt.define fn_struct_bad_field(t: Expr<Struct<{a: Integer, b: Bogus}>>) -> Expr<Integer> AS (
  t.a
)
