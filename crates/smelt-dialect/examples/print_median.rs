use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;

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
        restructure_plans: &[],
    };
    let out = print(&smelt_parser::parse(sql).syntax(), &ctx);
    if std::env::args().any(|a| a == "--roundtrip") {
        // Does the emitted SQL parse back cleanly, aliases intact? The compiler's
        // type-cast wrapper re-parses the printed SQL to name its columns.
        let probe = std::env::args().nth(2).unwrap_or(out.clone());
        let parse = smelt_parser::parse(&probe);
        println!("parse errors: {:?}", parse.errors);
        let file = File::cast(parse.syntax()).expect("file");
        if let Some(select) = file.select_stmt() {
            for (i, item) in select.select_list().expect("list").items().enumerate() {
                println!(
                    "item {i}: alias={:?} inferred={:?}",
                    item.alias(),
                    item.expression().and_then(|e| e.infer_name())
                );
            }
        }
    } else {
        println!("{out}");
    }
}
