//! `smelt explain <model>` over a real fixture with **model-to-model** edges
//! (`incremental_models.md` §"Upstream model edges"): a maintained model's ref
//! to another maintained model derives a creation-trigger cell clocked by the
//! upstream's own `timeseries:` declaration.
#![allow(dead_code, unused_imports)]

mod support;

mod creation_cells;
mod json_output;
mod observed_delta;
mod projection;
mod pushdown_and_locality;
mod relation_contract;
mod row_identity;
mod sql_and_grain;
mod write_pin_explain_surface;
mod write_variant_explain_surface;
