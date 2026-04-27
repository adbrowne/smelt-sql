-- Phase 10 broken fixture: two `smelt.extern`s with the same name across
-- sibling files (companion is `fn_extern_duplicate.sql`). The workspace
-- function checker emits a `DuplicateFunctionDefinition` diagnostic on
-- the alphabetically-later file — this one.
smelt.extern extern_twice(y: Expr<Integer>) -> Expr<Integer>
