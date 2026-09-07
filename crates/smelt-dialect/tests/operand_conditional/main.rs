//! Compile-path settlement of operand-conditional verdicts
//! (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
//!
//! Phase 7 populated the first production `Conditional` entries — `LOG`,
//! `TRUNC`, `TO_JSON`, `//` per class — on Spark. These tests exercise
//! `settle_emissions`'s walk mechanics — position/arity read off the source
//! CST, operand class read through the caller's `type_of` callback, and the
//! result matching a direct `Signature::settle_at` call — against `//`, and
//! (below) the first-argument-class arms of `TRUNC`/`TO_JSON` and the
//! non-`Conditional` `DAYOFWEEK` template. The arm-selection logic itself
//! (first match wins, arity guards, class guards, the `otherwise` fallback)
//! is proven against synthetic signatures in
//! `crates/smelt-types/tests/registry_coverage.rs`.
//!
//! Split into [`settlement`] (the `settle_emissions`/`settled_verdict_for`
//! mechanics and the `TRUNC`/`TO_JSON` operand-class arms) and
//! [`spark_templates`] (printed-output assertions for the Spark
//! `Emission::Template` rows).

mod settlement;
mod spark_templates;

pub(crate) fn print_with(
    sql: &str,
    dialect: &smelt_dialect::SqlDialect,
    caps: &smelt_dialect::BackendCapabilities,
) -> String {
    use std::collections::{HashMap, HashSet};

    use smelt_dialect::{print, PrintContext};

    let parsed = smelt_parser::parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}
