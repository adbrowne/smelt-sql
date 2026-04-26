-- Phase 53 broken fixture (cross-file extern same-name collision — file B).
-- Alphabetically later: DuplicateFunctionDefinition fires here because
-- cross_file_extern is already declared in fn_extern_collide_cross_file_a.sql.
---
backends: [spark]
---
smelt.extern cross_file_extern(x: Expr<Integer>) -> Expr<Integer>
