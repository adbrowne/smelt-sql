use crate::config::{BackendType, Config, Materialization, Target};
use crate::discovery::ModelFile;
use crate::errors::{extract_snippet, text_range_to_line_col, CliError};
use anyhow::Result;
use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_dialect::{wrap_with_type_casts, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;
use smelt_types::DataType;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub name: String,
    pub sql: String,
    pub materialization: Materialization,
}

fn dialect_for_backend(backend_type: BackendType) -> (SqlDialect, BackendCapabilities) {
    match backend_type {
        BackendType::DuckDB => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        BackendType::Spark => (SqlDialect::SparkSQL, BackendCapabilities::spark()),
    }
}

/// Resolve all smelt.ref() and smelt.source() calls in arbitrary SQL text by replacing
/// them with qualified table names.
pub fn resolve_refs_in_sql(sql: &str, schema: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let ctx = PrintContext {
        dialect: &SqlDialect::DuckDB,
        capabilities: &BackendCapabilities::duckdb(),
        schema,
        ephemeral_models: std::collections::HashSet::new(),
        cross_engine_refs: std::collections::HashMap::new(),
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

pub struct SqlCompiler {
    config: Config,
    dialect: SqlDialect,
    capabilities: BackendCapabilities,
    /// Cross-engine refs: model_name -> parquet read expression.
    /// Set externally before compilation when cross-engine references exist.
    cross_engine_refs: HashMap<String, String>,
}

impl SqlCompiler {
    pub fn new(config: Config, target: &Target) -> Self {
        let (dialect, capabilities) = dialect_for_backend(target.backend_type());
        Self {
            config,
            dialect,
            capabilities,
            cross_engine_refs: HashMap::new(),
        }
    }

    /// Set cross-engine ref mappings (model_name -> parquet read expression).
    pub fn set_cross_engine_refs(&mut self, refs: HashMap<String, String>) {
        self.cross_engine_refs = refs;
    }

    /// Compile a model's SQL by replacing smelt.ref() calls with table references
    pub fn compile(&self, model: &ModelFile, schema: &str) -> Result<CompiledModel> {
        // ERROR if any named parameters detected
        for ref_info in &model.refs {
            if ref_info.has_named_params {
                let (line, col) = text_range_to_line_col(&model.content, ref_info.range);
                let snippet = extract_snippet(&model.content, ref_info.range, 0);

                return Err(CliError::NamedParametersNotSupported {
                    model: model.name.clone(),
                    file: model.path.clone(),
                    line,
                    col,
                    snippet,
                }
                .into());
            }
        }

        // Strip frontmatter to avoid parse errors from YAML metadata
        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let parse = smelt_parser::parse(&clean_content);
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: std::collections::HashSet::new(),
            cross_engine_refs: self.cross_engine_refs.clone(),
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Type-conforming cast insertion: wrap SELECT columns with CASTs so
        // backend output types match smelt's type inference exactly.
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Wrap SELECT columns with CASTs based on type inference.
    ///
    /// Returns the original SQL unchanged if type inference can't extract
    /// column names/types (e.g. models referencing other models via smelt.ref()
    /// where upstream schemas aren't yet available).
    fn apply_type_casts(&self, sql: &str) -> String {
        let parse = smelt_parser::parse(sql);
        let file = match File::cast(parse.syntax()) {
            Some(f) => f,
            None => return sql.to_string(),
        };
        let select_stmt = match file.select_stmt() {
            Some(s) => s,
            None => return sql.to_string(),
        };

        let ctx = TypeContext::new();
        let column_types = infer_select_column_types(&select_stmt, &ctx);

        let select_list = match select_stmt.select_list() {
            Some(sl) => sl,
            None => return sql.to_string(),
        };
        let items: Vec<_> = select_list.items().collect();

        // Only apply casts if we have concrete types for at least one column
        let has_concrete = column_types
            .iter()
            .any(|tc| !matches!(tc.data_type, DataType::Unknown | DataType::Null));
        if !has_concrete {
            return sql.to_string();
        }

        let col_names: Vec<String> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                item.alias().unwrap_or_else(|| {
                    // Fallback: infer name from expression (e.g. bare column ref "user_id")
                    item.expression()
                        .and_then(|e| e.infer_name())
                        .unwrap_or_else(|| format!("_col{}", i + 1))
                })
            })
            .collect();
        let col_name_refs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
        let col_type_refs: Vec<DataType> =
            column_types.iter().map(|tc| tc.data_type.clone()).collect();

        wrap_with_type_casts(sql, &col_name_refs, &col_type_refs)
    }

    /// Compile a model with custom SQL (e.g., for transformed queries).
    /// This is used for incremental processing where the SQL has been transformed.
    pub fn compile_with_sql(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
    ) -> Result<CompiledModel> {
        let parse = smelt_parser::parse(sql);
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: std::collections::HashSet::new(),
            cross_engine_refs: self.cross_engine_refs.clone(),
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Build an `EphemeralResolver` using this compiler's dialect/capabilities.
    pub fn build_ephemeral_resolver(
        &self,
        ephemeral_models: &[(String, String)],
        schema: &str,
    ) -> EphemeralResolver {
        EphemeralResolver::new(ephemeral_models, &self.dialect, &self.capabilities, schema)
    }

    /// Like `compile_with_sql`, but also inlines referenced ephemeral models as CTEs.
    pub fn compile_with_sql_and_ephemerals(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
        resolver: &EphemeralResolver,
    ) -> Result<CompiledModel> {
        let ephemeral_refs: HashSet<&str> = resolver
            .ephemeral_names
            .iter()
            .map(|s| s.as_str())
            .collect();

        let parse = smelt_parser::parse(sql);
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: self.cross_engine_refs.clone(),
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references
        let referenced: Vec<&str> = model
            .refs
            .iter()
            .filter(|r| resolver.ephemeral_names.contains(&r.model_name))
            .map(|r| r.model_name.as_str())
            .collect();

        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let cte_list = resolver.get_cte_list(&referenced);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: final_sql,
            materialization,
        })
    }
}

/// Resolved ephemeral models ready for CTE inlining.
///
/// Holds the compiled SQL for each ephemeral model, with refs to other
/// ephemeral models already resolved as `__smelt_{name}` CTE names.
/// Internal CTEs of ephemeral models are hoisted and namespaced.
#[derive(Debug)]
pub struct EphemeralResolver {
    /// Ephemeral model names in topological order (dependencies first).
    pub order: Vec<String>,
    /// model_name -> list of (cte_alias, cte_body) pairs.
    /// For a simple ephemeral model, this is `[("__smelt_model", "SELECT ...")]`.
    /// For one with internal CTEs, the internal CTEs come first:
    /// `[("__smelt_model__cleaned", "SELECT ..."), ("__smelt_model", "SELECT ... FROM __smelt_model__cleaned")]`.
    cte_fragments: HashMap<String, Vec<(String, String)>>,
    /// Set of ephemeral model names (for quick lookup).
    pub ephemeral_names: HashSet<String>,
}

impl EphemeralResolver {
    /// Create an empty resolver (no ephemeral models).
    pub fn empty() -> Self {
        Self {
            order: Vec::new(),
            cte_fragments: HashMap::new(),
            ephemeral_names: HashSet::new(),
        }
    }

    /// Build an EphemeralResolver from a set of ephemeral models.
    ///
    /// Models must be provided in topological order (dependencies first).
    pub fn new(
        ephemeral_models: &[(String, String)], // (name, raw_sql) in topological order
        dialect: &SqlDialect,
        capabilities: &BackendCapabilities,
        schema: &str,
    ) -> Self {
        let ephemeral_names: HashSet<String> =
            ephemeral_models.iter().map(|(n, _)| n.clone()).collect();

        let mut cte_fragments: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut order = Vec::new();

        for (model_name, raw_sql) in ephemeral_models {
            order.push(model_name.clone());
            let fragments = Self::compile_ephemeral(
                model_name,
                raw_sql,
                &ephemeral_names,
                dialect,
                capabilities,
                schema,
            );
            cte_fragments.insert(model_name.clone(), fragments);
        }

        Self {
            order,
            cte_fragments,
            ephemeral_names,
        }
    }

    /// Compile a single ephemeral model's SQL into CTE fragments.
    ///
    /// For a model without internal CTEs: produces `[("__smelt_model", "SELECT ...")]`.
    /// For a model with internal CTEs: hoists them with namespaced names:
    /// `[("__smelt_model__cte1", "..."), ("__smelt_model", "SELECT FROM __smelt_model__cte1")]`.
    fn compile_ephemeral(
        model_name: &str,
        raw_sql: &str,
        ephemeral_names: &HashSet<String>,
        dialect: &SqlDialect,
        capabilities: &BackendCapabilities,
        schema: &str,
    ) -> Vec<(String, String)> {
        let ephemeral_refs: HashSet<&str> = ephemeral_names.iter().map(|s| s.as_str()).collect();
        let clean_sql = smelt_parser::strip_frontmatter(raw_sql);
        let parse = smelt_parser::parse(&clean_sql);

        // Compile with ephemeral refs resolved to __smelt_ names
        let ctx = PrintContext {
            dialect,
            capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: std::collections::HashMap::new(),
        };
        let compiled = smelt_dialect::print(&parse.syntax(), &ctx);

        // Check for internal CTEs by parsing the compiled output
        let file = File::cast(parse.syntax());
        let select_stmt = file.as_ref().and_then(|f| f.select_stmt());
        let has_with = select_stmt.as_ref().and_then(|s| s.with_clause()).is_some();

        if !has_with {
            // Simple case — no internal CTEs
            let alias = format!("__smelt_{}", model_name);
            return vec![(alias, compiled)];
        }

        // Has internal CTEs — extract CTE names, namespace them, and hoist
        let internal_cte_names: Vec<String> = select_stmt
            .as_ref()
            .and_then(|s| s.with_clause())
            .map(|w| w.ctes().filter_map(|c| c.name()).collect())
            .unwrap_or_default();

        // Build rename map: cte_name -> __smelt_model__cte_name
        let mut rename_map: Vec<(String, String)> = Vec::new();
        for cte_name in &internal_cte_names {
            let namespaced = format!("__smelt_{}__{}", model_name, cte_name);
            rename_map.push((cte_name.clone(), namespaced));
        }

        // Apply renames to the full compiled SQL
        let mut renamed = compiled;
        for (old_name, new_name) in &rename_map {
            renamed = rename_table_references(&renamed, old_name, new_name);
        }

        // Now parse the renamed SQL to extract individual CTEs and main body
        let parts = extract_cte_parts(&renamed);

        let mut fragments: Vec<(String, String)> = Vec::new();
        for (cte_name, cte_body) in &parts.ctes {
            fragments.push((cte_name.clone(), cte_body.clone()));
        }
        let alias = format!("__smelt_{}", model_name);
        fragments.push((alias, parts.main_body));

        fragments
    }

    /// Get the flattened CTE list for a model that references ephemeral models.
    ///
    /// Returns (cte_alias, cte_body) pairs in correct topological order,
    /// deduplicated (each ephemeral appears once even if referenced multiple times).
    pub fn get_cte_list(&self, referenced_ephemerals: &[&str]) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        // Walk in topological order, only including referenced ephemerals
        // and their transitive dependencies
        let needed: HashSet<&str> = self.collect_transitive_deps(referenced_ephemerals);

        for model_name in &self.order {
            if needed.contains(model_name.as_str()) && seen.insert(model_name.clone()) {
                if let Some(fragments) = self.cte_fragments.get(model_name) {
                    result.extend(fragments.iter().cloned());
                }
            }
        }

        result
    }

    /// Collect transitive ephemeral dependencies.
    fn collect_transitive_deps<'a>(&'a self, roots: &[&'a str]) -> HashSet<&'a str> {
        let mut needed: HashSet<&str> = HashSet::new();
        let mut queue: Vec<&str> = roots.to_vec();

        while let Some(name) = queue.pop() {
            if !self.ephemeral_names.contains(name) || !needed.insert(name) {
                continue;
            }
            // Check if this ephemeral model references other ephemerals
            // by looking at its CTE fragments for __smelt_ prefixed references
            if let Some(fragments) = self.cte_fragments.get(name) {
                for (_, body) in fragments {
                    for other in &self.ephemeral_names {
                        let cte_ref = format!("__smelt_{}", other);
                        if body.contains(&cte_ref) {
                            queue.push(other.as_str());
                        }
                    }
                }
            }
        }

        needed
    }
}

/// Prepend ephemeral CTEs to a compiled SQL string.
///
/// Handles merging with existing WITH clauses in the model's SQL.
pub fn prepend_ephemeral_ctes(sql: &str, cte_list: &[(String, String)]) -> String {
    if cte_list.is_empty() {
        return sql.to_string();
    }

    let mut cte_parts: Vec<String> = Vec::new();
    for (alias, body) in cte_list {
        let trimmed = body.trim();
        // Wrap in parens if not already wrapped
        if trimmed.starts_with('(') {
            cte_parts.push(format!("{} AS {}", alias, trimmed));
        } else {
            cte_parts.push(format!("{} AS (\n{}\n)", alias, trimmed));
        }
    }

    let trimmed_sql = sql.trim_start();

    // Check if the model already has a WITH clause
    let upper = trimmed_sql.to_uppercase();
    if upper.starts_with("WITH ") {
        // Merge: strip "WITH " from the model's SQL and prepend our CTEs
        let rest = &trimmed_sql[5..]; // Skip "WITH "
                                      // Check for RECURSIVE
        let rest_upper = rest.trim_start().to_uppercase();
        if rest_upper.starts_with("RECURSIVE ") {
            // User has WITH RECURSIVE — our non-recursive CTEs go before
            let after_recursive = &rest.trim_start()[10..]; // Skip "RECURSIVE "
            format!(
                "WITH {}, RECURSIVE {}",
                cte_parts.join(", "),
                after_recursive
            )
        } else {
            format!("WITH {}, {}", cte_parts.join(", "), rest)
        }
    } else {
        format!("WITH {}\n{}", cte_parts.join(", "), trimmed_sql)
    }
}

/// Parsed CTE parts from a SQL string.
struct CteParts {
    /// Individual CTEs: (name, body_sql)
    ctes: Vec<(String, String)>,
    /// The main SELECT after all CTEs
    main_body: String,
}

/// Extract CTE definitions and main body from a SQL string that starts with WITH.
///
/// Returns individual (cte_name, cte_body) pairs and the remaining SELECT.
fn extract_cte_parts(sql: &str) -> CteParts {
    let trimmed = sql.trim_start();
    let upper = trimmed.to_uppercase();
    if !upper.starts_with("WITH ") {
        return CteParts {
            ctes: vec![],
            main_body: sql.to_string(),
        };
    }

    // Skip "WITH " (and optional "RECURSIVE ")
    let mut pos = 5; // Skip "WITH "
    let rest_upper = trimmed[pos..].trim_start().to_uppercase();
    if rest_upper.starts_with("RECURSIVE ") {
        pos += trimmed[pos..]
            .find("RECURSIVE")
            .expect("starts_with check above guarantees RECURSIVE is present")
            + 10;
    }

    let bytes = trimmed.as_bytes();
    let mut ctes = Vec::new();

    loop {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Read CTE name (identifier)
        let name_start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
            pos += 1;
        }
        let cte_name = trimmed[name_start..pos].to_string();

        // Skip whitespace and "AS"
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip optional column list in parens before AS
        if pos < bytes.len() && bytes[pos] == b'(' {
            // This might be a column list or the CTE body — peek ahead for AS
            let paren_start = pos;
            let mut depth = 1;
            let mut pp = pos + 1;
            while pp < bytes.len() && depth > 0 {
                match bytes[pp] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'\'' => {
                        pp += 1;
                        while pp < bytes.len() && bytes[pp] != b'\'' {
                            pp += 1;
                        }
                    }
                    _ => {}
                }
                pp += 1;
            }
            // Check if AS follows this paren group
            let after = trimmed[pp..].trim_start().to_uppercase();
            if after.starts_with("AS") {
                // This was a column list, skip it
                pos = pp;
            } else {
                // No AS after parens — this shouldn't happen in valid SQL
                pos = paren_start;
            }
        }

        // Skip "AS" keyword
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if trimmed[pos..].to_uppercase().starts_with("AS") {
            pos += 2;
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Read CTE body (in parens)
        if pos < bytes.len() && bytes[pos] == b'(' {
            let body_start = pos + 1;
            let mut depth = 1;
            pos += 1;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'\'' => {
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != b'\'' {
                            pos += 1;
                        }
                    }
                    _ => {}
                }
                pos += 1;
            }
            let body_end = pos - 1; // Exclude closing paren
            let body = trimmed[body_start..body_end].trim().to_string();
            ctes.push((cte_name, body));
        }

        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Check for comma (more CTEs) or end of WITH clause
        if pos < bytes.len() && bytes[pos] == b',' {
            pos += 1; // Skip comma, continue to next CTE
        } else {
            break; // No more CTEs — rest is the main body
        }
    }

    let main_body = trimmed[pos..].trim().to_string();
    CteParts { ctes, main_body }
}

/// Rename table references in SQL text.
/// Simple string replacement — replaces word-boundary occurrences of `old_name` with `new_name`.
fn rename_table_references(sql: &str, old_name: &str, new_name: &str) -> String {
    // Use word-boundary-aware replacement to avoid replacing substrings
    let mut result = String::new();
    let mut remaining = sql;

    while let Some(pos) = remaining.find(old_name) {
        // Check that it's a word boundary (not part of a larger identifier)
        let before_ok = pos == 0
            || !remaining.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && remaining.as_bytes()[pos - 1] != b'_';
        let after_pos = pos + old_name.len();
        let after_ok = after_pos >= remaining.len()
            || (!remaining.as_bytes()[after_pos].is_ascii_alphanumeric()
                && remaining.as_bytes()[after_pos] != b'_');

        if before_ok && after_ok {
            result.push_str(&remaining[..pos]);
            result.push_str(new_name);
            remaining = &remaining[after_pos..];
        } else {
            result.push_str(&remaining[..after_pos]);
            remaining = &remaining[after_pos..];
        }
    }

    result.push_str(remaining);
    result
}

impl SqlCompiler {
    /// Compile a model with ephemeral CTE inlining.
    ///
    /// Like `compile()`, but also inlines any referenced ephemeral models as CTEs
    /// with `__smelt_` namespaced aliases.
    pub fn compile_with_ephemerals(
        &self,
        model: &ModelFile,
        schema: &str,
        resolver: &EphemeralResolver,
    ) -> Result<CompiledModel> {
        // Check for named params (same as compile)
        for ref_info in &model.refs {
            if ref_info.has_named_params {
                let (line, col) = text_range_to_line_col(&model.content, ref_info.range);
                let snippet = extract_snippet(&model.content, ref_info.range, 0);
                return Err(CliError::NamedParametersNotSupported {
                    model: model.name.clone(),
                    file: model.path.clone(),
                    line,
                    col,
                    snippet,
                }
                .into());
            }
        }

        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let parse = smelt_parser::parse(&clean_content);

        // Build ephemeral set for the printer
        let ephemeral_refs: HashSet<&str> = resolver
            .ephemeral_names
            .iter()
            .map(|s| s.as_str())
            .collect();

        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: self.cross_engine_refs.clone(),
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references
        let referenced: Vec<&str> = model
            .refs
            .iter()
            .filter(|r| resolver.ephemeral_names.contains(&r.model_name))
            .map(|r| r.model_name.as_str())
            .collect();

        // Prepend ephemeral CTEs if any
        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let cte_list = resolver.get_cte_list(&referenced);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
            sql: final_sql,
            materialization,
        })
    }
}

/// Registry of SQL compilers, one per target.
///
/// Each target may have a different dialect (DuckDB vs Spark), so we need
/// one compiler per target to emit correct SQL.
pub struct CompilerRegistry {
    compilers: HashMap<String, SqlCompiler>,
}

impl CompilerRegistry {
    /// Create compilers for all targets in the set.
    pub fn new(config: &Config, targets: &HashMap<String, Target>) -> Self {
        let mut compilers = HashMap::new();
        for (name, target) in targets {
            compilers.insert(name.clone(), SqlCompiler::new(config.clone(), target));
        }
        Self { compilers }
    }

    /// Get the compiler for a target name.
    pub fn get(&self, target_name: &str) -> &SqlCompiler {
        &self.compilers[target_name]
    }

    /// Set cross-engine ref mappings for a specific target's compiler.
    pub fn set_cross_engine_refs(&mut self, target_name: &str, refs: HashMap<String, String>) {
        if let Some(compiler) = self.compilers.get_mut(target_name) {
            compiler.set_cross_engine_refs(refs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelConfig, Target};
    use crate::discovery::RefInfo;
    use std::collections::HashMap;

    fn make_test_target() -> Target {
        Target {
            target_type: "duckdb".to_string(),
            database: Some("test.duckdb".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
        }
    }

    /// Helper function to parse SQL and extract refs with real TextRange values
    fn extract_refs_from_sql(sql: &str) -> Vec<RefInfo> {
        let parse = smelt_parser::parse(sql);
        if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
            file.refs()
                .filter_map(|ref_call| {
                    Some(RefInfo {
                        model_name: ref_call.model_name()?,
                        has_named_params: ref_call.named_params().count() > 0,
                        range: ref_call.range(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn make_test_config() -> Config {
        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
                warehouse: None,
                format: None,
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    #[test]
    fn test_simple_ref_replacement() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.ref('raw_events')
GROUP BY user_id
"#;

        let model = ModelFile {
            name: "user_stats".to_string(),
            path: "models/user_stats.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.raw_events"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT a.user_id, b.session_id
FROM smelt.ref('model_a') a
JOIN smelt.ref('model_b') b ON a.id = b.id
"#;

        let model = ModelFile {
            name: "combined".to_string(),
            path: "models/combined.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a a"));
        assert!(compiled.sql.contains("JOIN main.model_b b"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_named_params_error() {
        let sql = r#"
SELECT user_id
FROM smelt.ref('raw_events', filter => event_type = 'page_view')
"#;

        let model = ModelFile {
            name: "filtered".to_string(),
            path: "models/filtered.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let result = compiler.compile(&model, "main");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("named parameters"));
        assert!(err_msg.contains("not yet supported"));
    }

    #[test]
    fn test_materialization_from_config() {
        let model = ModelFile {
            name: "test_model".to_string(),
            path: "models/test_model.sql".into(),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: None,
                tags: Vec::new(),
                target: None,
            },
        );

        let compiler = SqlCompiler::new(config, &make_test_target());
        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(matches!(compiled.materialization, Materialization::Table));
    }

    #[test]
    fn test_ref_with_double_quotes() {
        let sql = r#"SELECT * FROM smelt.ref("model_a")"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_ref_with_whitespace() {
        let sql = r#"SELECT * FROM smelt.ref( 'model_a' )"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_multiple_refs_same_model() {
        let sql = r#"
SELECT a.id, b.id
FROM smelt.ref('model_a') a
JOIN smelt.ref('model_a') b ON a.parent_id = b.id
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Both instances should be replaced
        assert_eq!(compiled.sql.matches("main.model_a").count(), 2);
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    #[test]
    fn test_refs_preserve_formatting() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as count
FROM smelt.ref('events')
WHERE event_type = 'click'
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Verify formatting is preserved (newlines, indentation)
        assert!(compiled.sql.contains("SELECT\n    user_id,"));
        assert!(compiled.sql.contains("FROM main.events"));
        assert!(compiled.sql.contains("WHERE event_type = 'click'"));
        assert!(!compiled.sql.contains("smelt.ref"));
    }

    // ===== Ephemeral model tests =====

    #[test]
    fn test_ephemeral_simple_cte_inlining() {
        // Ephemeral model: staging_users
        let ephemeral_sql = "SELECT id, name FROM raw_users WHERE active = true";

        // Downstream model references the ephemeral
        let sql = "SELECT * FROM smelt.ref('staging_users')";
        let model = ModelFile {
            name: "final_users".to_string(),
            path: "models/final_users.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging_users".to_string(), ephemeral_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        assert!(compiled.sql.contains("__smelt_staging_users"));
        assert!(compiled.sql.contains("WITH"));
        assert!(!compiled.sql.contains("smelt.ref"));
        assert!(!compiled.sql.contains("main.staging_users"));
    }

    #[test]
    fn test_ephemeral_transitive_deps() {
        // C (ephemeral) -> B (ephemeral) -> A (table)
        let c_sql = "SELECT * FROM raw_data";
        let b_sql = "SELECT * FROM smelt.ref('c')";

        let sql = "SELECT * FROM smelt.ref('b')";
        let model = ModelFile {
            name: "a".to_string(),
            path: "models/a.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[
                ("c".to_string(), c_sql.to_string()),
                ("b".to_string(), b_sql.to_string()),
            ],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Both C and B should be in the CTE list
        assert!(compiled.sql.contains("__smelt_c"));
        assert!(compiled.sql.contains("__smelt_b"));
        // C should come before B (topological order)
        let c_pos = compiled.sql.find("__smelt_c").unwrap();
        let b_pos = compiled.sql.find("__smelt_b").unwrap();
        assert!(c_pos < b_pos, "C should appear before B in CTEs");
    }

    #[test]
    fn test_ephemeral_mixed_refs() {
        // staging (ephemeral), regular_model (table)
        let staging_sql = "SELECT * FROM raw_data";

        let sql = "SELECT * FROM smelt.ref('staging') JOIN smelt.ref('regular_model') ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging".to_string(), staging_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Ephemeral → CTE name, non-ephemeral → schema.table
        assert!(compiled.sql.contains("__smelt_staging"));
        assert!(compiled.sql.contains("main.regular_model"));
    }

    #[test]
    fn test_ephemeral_with_existing_with_clause() {
        let staging_sql = "SELECT * FROM raw_data";

        let sql =
            "WITH my_cte AS (SELECT 1 as x) SELECT * FROM smelt.ref('staging') JOIN my_cte ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging".to_string(), staging_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Should have a single WITH clause with both CTEs
        let with_count = compiled.sql.matches("WITH ").count();
        assert_eq!(with_count, 1, "Should have exactly one WITH clause");
        assert!(compiled.sql.contains("__smelt_staging"));
        assert!(compiled.sql.contains("my_cte"));
    }

    #[test]
    fn test_prepend_ephemeral_ctes_no_existing_with() {
        let cte_list = vec![
            ("__smelt_a".to_string(), "SELECT 1 as x".to_string()),
            (
                "__smelt_b".to_string(),
                "SELECT * FROM __smelt_a".to_string(),
            ),
        ];
        let sql = "SELECT * FROM __smelt_b";
        let result = prepend_ephemeral_ctes(sql, &cte_list);

        assert!(result.starts_with("WITH"));
        assert!(result.contains("__smelt_a AS"));
        assert!(result.contains("__smelt_b AS"));
        assert!(result.contains("SELECT * FROM __smelt_b"));
    }

    #[test]
    fn test_prepend_ephemeral_ctes_with_existing_with() {
        let cte_list = vec![("__smelt_staging".to_string(), "SELECT 1 as x".to_string())];
        let sql = "WITH my_cte AS (SELECT 2 as y) SELECT * FROM __smelt_staging JOIN my_cte";
        let result = prepend_ephemeral_ctes(sql, &cte_list);

        let with_count = result.matches("WITH ").count();
        assert_eq!(with_count, 1, "Should merge into single WITH");
        assert!(result.contains("__smelt_staging AS"));
        assert!(result.contains("my_cte AS"));
    }

    #[test]
    fn test_extract_cte_parts() {
        let sql = "WITH a AS (SELECT 1), b AS (SELECT 2 FROM a) SELECT * FROM b";
        let parts = extract_cte_parts(sql);

        assert_eq!(parts.ctes.len(), 2);
        assert_eq!(parts.ctes[0].0, "a");
        assert_eq!(parts.ctes[0].1, "SELECT 1");
        assert_eq!(parts.ctes[1].0, "b");
        assert_eq!(parts.ctes[1].1, "SELECT 2 FROM a");
        assert_eq!(parts.main_body, "SELECT * FROM b");
    }

    #[test]
    fn test_rename_table_references() {
        let sql = "SELECT * FROM cleaned WHERE cleaned.id > 0";
        let result = rename_table_references(sql, "cleaned", "__smelt_model__cleaned");
        assert!(result.contains("__smelt_model__cleaned"));
        assert!(!result.contains(" cleaned"));
    }

    #[test]
    fn test_case_expression_with_alias_no_question_marks() {
        let sql = "SELECT CASE WHEN x > 0 THEN 'high' ELSE 'low' END AS label FROM t";

        let model = ModelFile {
            name: "case_test".to_string(),
            path: "models/case_test.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Should NOT contain question marks in the output
        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE expression should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("AS ?"),
            "CASE expression should not produce ... AS ?: {}",
            compiled.sql
        );
        // Should contain the alias 'label'
        assert!(
            compiled.sql.contains("label"),
            "Should preserve the 'label' alias: {}",
            compiled.sql
        );
    }

    #[test]
    fn test_case_expression_without_alias_no_question_marks() {
        // CASE without explicit alias — should produce a valid name, not '?'
        let sql = "SELECT x, CASE WHEN x > 0 THEN 'high' ELSE 'low' END FROM t";

        let model = ModelFile {
            name: "case_test2".to_string(),
            path: "models/case_test2.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE without alias should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("AS ?"),
            "CASE without alias should not produce ... AS ?: {}",
            compiled.sql
        );
    }

    #[test]
    fn test_case_in_aggregate_no_question_marks() {
        // COUNT(CASE WHEN ... THEN 1 END) — common funnel pattern
        let sql = "SELECT COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) AS purchases FROM t GROUP BY x";

        let model = ModelFile {
            name: "case_agg_test".to_string(),
            path: "models/case_agg_test.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE in aggregate should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("purchases"),
            "Should preserve the 'purchases' alias: {}",
            compiled.sql
        );
    }
}
