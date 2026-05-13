//! Per-feature Salsa queries.
//!
//! Each submodule corresponds to one phase in the query graph:
//!
//! - `parse` — parse / model / refs / sources extraction
//! - `project` — `sources.yml`, `smelt.yml` vars, paths/seeds/sources, plus
//!   workspace-level `all_models` / `models_with_tag` / `sources_with_tag`
//! - `functions` — per-file signature / body / `resolve_function`
//! - `function_diagnostics` — duplicate names, body checks, frontmatter,
//!   backends widening, call-graph cycle, `smelt.fn.*` call-site checks
//! - `schema` — `model_schema`, `type_context`, `typed_model_schema`,
//!   `resolved_model_schema`, `columns_of_for_table_expr`,
//!   `model_input_constraints`, `model_function_type`, plus the pure
//!   `RefSchemaProvider` trait and `StaticRefSchemaProvider`
//! - `check_types` — per-model column-type diagnostics
//! - `loader` — `smelt.config.load_yaml` / `load_json` / `load_toml` resolution

pub mod check_types;
pub mod function_diagnostics;
pub mod functions;
pub mod loader;
pub mod parse;
pub mod project;
pub mod schema;
