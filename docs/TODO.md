# TODO

## Cross-Model Type Inference

- [ ] **Salsa cycle recovery for circular refs** — Currently, if model A refs model B which refs model A, Salsa will panic with a cycle error. Add `salsa::cycle` recovery attributes to `resolved_model_schema`, `typed_model_schema`, and `type_context` queries to return empty/default schemas gracefully and produce a diagnostic.

- [ ] **Cross-model type mismatch diagnostics** — When a downstream model uses a column in a type-incompatible way (e.g., `SUM(col)` where upstream infers `col` as VARCHAR), produce a warning diagnostic in the LSP. Compare `model_input_constraints` against actual upstream `typed_model_schema`.

- [ ] **Multi-model property tests** — Extend `type_property_tests.rs` to generate two-model chains (model_A with typed CTE columns, model_B refs model_A) and verify types match DuckDB output. Requires setting up Salsa Database with multiple models in the property test.
