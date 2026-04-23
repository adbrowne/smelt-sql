-- Phase 10 broken fixture: declaring a `smelt.extern` whose name matches a
-- built-in (LOWER) collides with the canonical registry. The workspace
-- function checker emits an `ExternCollidesWithBuiltin` diagnostic anchored
-- at the extern's name span.
smelt.extern LOWER(s: Expr<Text>) -> Expr<Text>
