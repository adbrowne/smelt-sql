//! Shared check execution helper used by both `smelt check` (CLI) and the
//! build seam in `execute_project` (runtime). Extracts the compile+execute+count
//! logic so neither consumer duplicates it.
//!
//! Run-pipeline parity rule: this module lives in `smelt-runtime`; `smelt-cli`'s
//! `commands/check.rs` calls `run_single_check` directly rather than
//! re-implementing the execute path.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use arrow::array::RecordBatch;
use serde::{Deserialize, Serialize};

use smelt_backend::Backend;
use smelt_core::{discovery::ModelFile, metadata::CheckSeverity};

use crate::compile::{EphemeralResolver, SqlCompiler};

// ── Status & outcome types ────────────────────────────────────────────────────

/// Result of executing a single `smelt.check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    /// Zero failing rows.
    Pass,
    /// Non-zero failing rows and severity is `error`.
    Fail,
    /// Non-zero failing rows and severity is `warn`.
    Warn,
    /// A referenced model has not been built (relation absent from target).
    TargetNotBuilt,
}

/// Complete result record for one check execution.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub name: String,
    pub severity: CheckSeverity,
    pub status: CheckStatus,
    pub row_count: usize,
    /// Up to 5 violating rows (empty on Pass / TargetNotBuilt).
    pub sample: Vec<BTreeMap<String, String>>,
    /// Human-readable message (error text, TargetNotBuilt description, etc.).
    pub message: Option<String>,
    /// Compiled failing-rows SQL, when the check compiled successfully.
    /// `None` when the check never reached compilation (no declaration, no body,
    /// target not built, or a compile error). Surfaced by `smelt check --verbose`.
    pub sql: Option<String>,
}

// ── Main entry point ──────────────────────────────────────────────────────────

/// Compile and execute one `smelt.check`, returning a `CheckOutcome`.
///
/// `check_model` must contain a `smelt.check <name> AS (<select>)` declaration.
/// The SELECT is compiled through the sanctioned `compiler.compile_with_sql_and_ephemerals`
/// path (run-pipeline parity rule) and executed against `backend`.
///
/// A referenced model that is not yet built (relation absent from the target)
/// always returns `TargetNotBuilt` — this is a loud failure regardless of
/// `severity` (fail-loud discipline).
pub async fn run_single_check(
    compiler: &SqlCompiler,
    backend: &dyn Backend,
    schema: &str,
    check_model: &ModelFile,
    severity: CheckSeverity,
    ephemeral_names: &HashSet<String>,
    resolver: &EphemeralResolver,
) -> Result<CheckOutcome> {
    // Extract all data from the AST synchronously before any await.
    // SyntaxNode is !Send, so it must not be held across await points.
    let (check_name, body_select_text) = {
        let clean_body = smelt_parser::strip_frontmatter(&check_model.content);
        let parse = smelt_parser::parse(&clean_body);
        let ast_file_opt = smelt_parser::ast::File::cast(parse.syntax());

        let check = match ast_file_opt.as_ref().and_then(|f| f.checks().next()) {
            Some(c) => c,
            None => {
                let name = check_model.name.clone();
                return Ok(CheckOutcome {
                    name: name.clone(),
                    severity,
                    status: CheckStatus::Fail,
                    row_count: 0,
                    sample: vec![],
                    message: Some(format!("no smelt.check declaration found in file '{name}'")),
                    sql: None,
                });
            }
        };

        let name = check.name().unwrap_or_else(|| check_model.name.clone());
        let body = match check.body_select() {
            Some(s) => s.syntax().text().to_string(),
            None => {
                return Ok(CheckOutcome {
                    name,
                    severity,
                    status: CheckStatus::Fail,
                    row_count: 0,
                    sample: vec![],
                    message: Some("check has no SELECT body".to_string()),
                    sql: None,
                });
            }
        };
        (name, body)
        // `check`, `parse`, `ast_file_opt` are dropped here — !Send types gone
    };

    // ── CheckTargetNotBuilt pre-check ─────────────────────────────────────────
    for ref_info in &check_model.refs {
        let segs = ref_info.smelt_ref.to_path();
        if segs.is_empty() {
            continue;
        }
        if segs[0] == "sources" || segs[0] == "functions" {
            continue;
        }
        let relation_name = segs.join("_");
        if ephemeral_names.contains(&relation_name) {
            continue;
        }
        match backend.table_exists(schema, &relation_name).await {
            Ok(true) => {}
            Ok(false) => {
                return Ok(CheckOutcome {
                    name: check_name,
                    severity,
                    status: CheckStatus::TargetNotBuilt,
                    row_count: 0,
                    sample: vec![],
                    message: Some(format!(
                        "CheckTargetNotBuilt: model '{}' referenced by check has not been built \
                         in the target schema '{schema}'",
                        segs.join(".")
                    )),
                    sql: None,
                });
            }
            Err(e) => {
                return Ok(CheckOutcome {
                    name: check_name,
                    severity,
                    status: CheckStatus::TargetNotBuilt,
                    row_count: 0,
                    sample: vec![],
                    message: Some(format!(
                        "CheckTargetNotBuilt: error verifying '{}' in schema '{schema}': {e}",
                        relation_name
                    )),
                    sql: None,
                });
            }
        }
    }

    // ── Compile through the sanctioned CompilerRegistry path ──────────────────
    let compiled = match compiler.compile_with_sql_and_ephemerals(
        check_model,
        schema,
        &body_select_text,
        resolver,
    ) {
        Ok(c) => c,
        Err(e) => {
            return Ok(CheckOutcome {
                name: check_name,
                severity,
                status: CheckStatus::Fail,
                row_count: 0,
                sample: vec![],
                message: Some(format!("compilation error: {e}")),
                sql: None,
            });
        }
    };

    // ── Execute the failing-rows query ────────────────────────────────────────
    let batches = match backend.execute_sql(&compiled.sql).await {
        Ok(b) => b,
        Err(e) => {
            return Ok(CheckOutcome {
                name: check_name,
                severity,
                status: CheckStatus::Fail,
                row_count: 0,
                sample: vec![],
                message: Some(format!("execution error: {e}")),
                sql: Some(compiled.sql.clone()),
            });
        }
    };

    let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();

    if row_count == 0 {
        return Ok(CheckOutcome {
            name: check_name,
            severity,
            status: CheckStatus::Pass,
            row_count: 0,
            sample: vec![],
            message: None,
            sql: Some(compiled.sql.clone()),
        });
    }

    let sample = batches_to_rows(&batches);
    let sample_capped: Vec<_> = sample.into_iter().take(5).collect();

    let status = match severity {
        CheckSeverity::Error => CheckStatus::Fail,
        CheckSeverity::Warn => CheckStatus::Warn,
    };

    Ok(CheckOutcome {
        name: check_name,
        severity,
        status,
        row_count,
        sample: sample_capped,
        message: Some(format!("{row_count} violating row(s)")),
        sql: Some(compiled.sql.clone()),
    })
}

// ── Row extraction ────────────────────────────────────────────────────────────

/// Convert Arrow `RecordBatch`es into string-map rows (one entry per cell).
/// Used for violation samples. Mirrors `smelt_cli::test_runner::batches_to_rows`.
pub fn batches_to_rows(batches: &[RecordBatch]) -> Vec<BTreeMap<String, String>> {
    let mut rows = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        for row_idx in 0..batch.num_rows() {
            let mut row = BTreeMap::new();
            for (col_idx, field) in schema.fields().iter().enumerate() {
                let col = batch.column(col_idx);
                let value = arrow::util::display::array_value_to_string(col, row_idx)
                    .unwrap_or_else(|_| "ERROR".to_string());
                row.insert(field.name().clone(), value);
            }
            rows.push(row);
        }
    }
    rows
}
