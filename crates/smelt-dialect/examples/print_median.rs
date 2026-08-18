use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};

fn main() {
    let sql = "SELECT g, MEDIAN(val) AS med_val FROM fixture GROUP BY g";
    let caps = BackendCapabilities::bigquery();
    let ctx = PrintContext {
        dialect: &SqlDialect::BigQuery,
        capabilities: &caps,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
    };
    println!("{}", print(&smelt_parser::parse(sql).syntax(), &ctx));
}
