-- Phase 10 companion fixture: pairs with `fn_extern_duplicate_other.sql` to
-- produce the duplicate diagnostic. This file is alphabetically earlier so
-- it "wins" — no diagnostic fires here; the companion carries the error.
smelt.extern extern_twice(x: Expr<Integer>) -> Expr<Integer>
