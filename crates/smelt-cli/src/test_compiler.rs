//! Test compiler: extracts CTEs and compiles test SQL.

use std::collections::BTreeMap;

use smelt_parser::ast::File as AstFile;
use smelt_runtime::{substitute_params_with_named, FnBodyMap};

/// Information about a CTE extracted from a SQL model.
#[derive(Debug, Clone)]
pub struct CteInfo {
    /// CTE name
    pub name: String,
    /// The SQL body of the CTE (without the surrounding parens)
    pub body: String,
    /// Names of other CTEs that this CTE depends on
    pub dependencies: Vec<String>,
}

/// Extract CTEs from a SQL string, including their dependencies.
///
/// Dependencies are detected by finding table references in each CTE body
/// that match other CTE names in the same WITH clause.
pub fn extract_ctes(sql: &str) -> Vec<CteInfo> {
    let clean = smelt_parser::strip_frontmatter(sql);
    let parse = smelt_parser::parse(&clean);
    let file = match AstFile::cast(parse.syntax()) {
        Some(f) => f,
        None => return vec![],
    };

    let select = match file.select_stmt() {
        Some(s) => s,
        None => return vec![],
    };

    let with_clause = match select.with_clause() {
        Some(w) => w,
        None => return vec![],
    };

    // First pass: collect all CTE names and bodies
    let mut cte_names: Vec<String> = Vec::new();
    let mut cte_bodies: Vec<String> = Vec::new();

    for cte in with_clause.ctes() {
        let name = match cte.name() {
            Some(n) => n,
            None => continue,
        };
        // Get the CTE body text from the subquery's select statement
        let body = match cte.query() {
            Some(subquery) => match subquery.select_stmt() {
                Some(select) => select.to_string(),
                None => continue,
            },
            None => continue,
        };
        cte_names.push(name);
        cte_bodies.push(body);
    }

    // Second pass: detect dependencies between CTEs
    let mut result = Vec::new();
    for (i, name) in cte_names.iter().enumerate() {
        let body = &cte_bodies[i];
        let body_upper = body.to_uppercase();

        let mut deps = Vec::new();
        for (j, other_name) in cte_names.iter().enumerate() {
            if i == j {
                continue;
            }
            // Check if this CTE references the other CTE name
            // Use word-boundary check to avoid false positives
            let other_upper = other_name.to_uppercase();
            if contains_word(&body_upper, &other_upper) {
                deps.push(other_name.clone());
            }
        }

        result.push(CteInfo {
            name: name.clone(),
            body: body.clone(),
            dependencies: deps,
        });
    }

    result
}

/// Check if `haystack` contains `word` as a whole word (not as a substring of a larger identifier).
/// Skips SQL string literals to avoid false positives (e.g., `WHERE status = 'events'`).
fn contains_word(haystack: &str, word: &str) -> bool {
    let stripped = strip_string_literals(haystack);
    let bytes = stripped.as_bytes();
    let word_bytes = word.as_bytes();
    let word_len = word_bytes.len();

    for i in 0..=bytes.len().saturating_sub(word_len) {
        if &bytes[i..i + word_len] == word_bytes {
            // Check word boundary before
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            // Check word boundary after
            let after_ok = i + word_len >= bytes.len() || !is_ident_char(bytes[i + word_len]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// Replace SQL string literals with spaces to avoid false matches in word search.
fn strip_string_literals(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            result.push(' ');
            i += 1;
            // Skip until closing quote (handle escaped quotes '')
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2; // Skip escaped quote
                    } else {
                        i += 1; // Closing quote
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Check if a string matches the YYYY-MM-DD date pattern.
fn is_date_string(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

/// Check if a string matches the `YYYY-MM-DD HH:MM:SS` timestamp pattern
/// (with an optional fractional-seconds suffix).  These strings are cast to
/// `TIMESTAMP` rather than `VARCHAR` so that functions like `epoch_us()` can
/// consume them directly in inline tests.
fn is_timestamp_string(s: &str) -> bool {
    // Minimum form: "YYYY-MM-DD HH:MM:SS" = 19 chars; separator is space or 'T'
    if s.len() < 19 {
        return false;
    }
    let bytes = s.as_bytes();
    // Date part: YYYY-MM-DD
    if !is_date_string(&s[..10]) {
        return false;
    }
    // Separator must be ' ' or 'T'
    if bytes[10] != b' ' && bytes[10] != b'T' {
        return false;
    }
    // Time part: HH:MM:SS
    bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[11..13].iter().all(|b| b.is_ascii_digit())
        && bytes[14..16].iter().all(|b| b.is_ascii_digit())
        && bytes[17..19].iter().all(|b| b.is_ascii_digit())
}

/// Convert a serde_yaml::Value to a SQL literal.
fn yaml_value_to_sql(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::Number(n) => {
            if n.is_i64() {
                format!("{}", n.as_i64().expect("checked is_i64() above"))
            } else {
                format!(
                    "{}",
                    n.as_f64().expect("serde_yaml Number is either i64 or f64")
                )
            }
        }
        serde_yaml::Value::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        serde_yaml::Value::Null => "NULL".to_string(),
        serde_yaml::Value::String(s) => {
            if is_date_string(s) {
                format!("'{}'::DATE", s)
            } else if is_timestamp_string(s) {
                format!("'{}'::TIMESTAMP", s)
            } else {
                format!("'{}'", s.replace('\'', "''"))
            }
        }
        _ => "NULL".to_string(),
    }
}

/// Convert YAML rows to a SQL CTE definition using VALUES.
///
/// Example output:
/// ```sql
/// mock_data AS (SELECT * FROM (VALUES (1, 100.0, '2024-01-01'::DATE)) AS t(user_id, amount, created_at))
/// ```
pub fn yaml_rows_to_sql(name: &str, rows: &[BTreeMap<String, serde_yaml::Value>]) -> String {
    if rows.is_empty() {
        return format!(
            "{} AS (SELECT * FROM (VALUES (NULL)) AS t(__empty) WHERE 1=0)",
            name
        );
    }

    // Derive columns from first row (BTreeMap gives alphabetical order)
    let columns: Vec<&String> = rows[0].keys().collect();

    let value_rows: Vec<String> = rows
        .iter()
        .map(|row| {
            let vals: Vec<String> = columns
                .iter()
                .map(|col| {
                    row.get(*col)
                        .map(yaml_value_to_sql)
                        .unwrap_or_else(|| "NULL".to_string())
                })
                .collect();
            format!("({})", vals.join(", "))
        })
        .collect();

    let col_names: Vec<&str> = columns.iter().map(|c| c.as_str()).collect();
    format!(
        "{} AS (SELECT * FROM (VALUES {}) AS t({}))",
        name,
        value_rows.join(", "),
        col_names.join(", ")
    )
}

/// Compile a test that targets a specific CTE within a model.
///
/// Extracts the target CTE's body, mocks its dependencies using `inputs`,
/// and returns a standalone SQL query.
pub fn compile_cte_test(
    model_sql: &str,
    target_cte: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    sql_body: Option<&str>,
) -> Result<String, String> {
    let ctes = extract_ctes(model_sql);

    // Find target CTE
    let target = ctes
        .iter()
        .find(|c| c.name == target_cte)
        .ok_or_else(|| format!("CTE '{}' not found in model", target_cte))?;

    // Build mock CTEs for dependencies
    let mut mock_cte_parts: Vec<String> = Vec::new();

    for dep in &target.dependencies {
        if let Some(rows) = inputs.get(dep) {
            mock_cte_parts.push(yaml_rows_to_sql(dep, rows));
        } else if let Some(body) = sql_body {
            // Check if sql_body defines this CTE
            let body_ctes = extract_ctes(&format!("WITH {} SELECT 1", body));
            if let Some(found) = body_ctes.iter().find(|c| c.name == *dep) {
                mock_cte_parts.push(format!("{} AS ({})", dep, found.body));
            } else {
                return Err(format!(
                    "Dependency '{}' of CTE '{}' is not mocked in inputs",
                    dep, target_cte
                ));
            }
        } else {
            return Err(format!(
                "Dependency '{}' of CTE '{}' is not mocked in inputs",
                dep, target_cte
            ));
        }
    }

    // Assemble: WITH <mock CTEs> <target body as main SELECT>
    if mock_cte_parts.is_empty() {
        Ok(target.body.clone())
    } else {
        Ok(format!(
            "WITH {}\n{}",
            mock_cte_parts.join(",\n"),
            target.body
        ))
    }
}

/// Expand `smelt.functions.*` path-call nodes in `sql` by substituting their
/// declared bodies using `fn_bodies`.  Returns the SQL with all expandable
/// calls replaced.
///
/// This is a text-level expansion applied AFTER `smelt.ref()` path-refs have
/// already been replaced, so named-arg values (like `source => silver_events_parsed`)
/// already contain the substituted CTE name.
///
/// Calls to unknown functions (not in `fn_bodies`) are left verbatim.
fn expand_fn_calls_in_sql(sql: &str, fn_bodies: &FnBodyMap) -> String {
    let parse = smelt_parser::parse(sql);
    let file = match AstFile::cast(parse.syntax()) {
        Some(f) => f,
        None => return sql.to_string(),
    };

    // Collect SmeltPathCall replacements sorted descending by start offset so
    // we can apply them right-to-left without shifting earlier offsets.
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();

    for call in file
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
    {
        let segs = call.segments();
        // Only expand smelt.functions.* calls.
        if segs.first().map(|s| s.as_str()) != Some("functions") {
            continue;
        }
        let fn_name = match segs.get(1) {
            Some(n) => n.clone(),
            None => continue,
        };
        let (params, body_sql) = match fn_bodies.get(&fn_name) {
            Some(entry) => entry,
            None => continue,
        };

        // Extract positional and named args as text from the already-substituted SQL.
        let positional: Vec<String> = call
            .arg_list()
            .map(|al| {
                al.positional_args()
                    .into_iter()
                    .map(|arg| {
                        let r = arg.syntax().text_range();
                        let s: usize = r.start().into();
                        let e: usize = r.end().into();
                        sql[s..e].to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let named: Vec<(String, String)> = call
            .arg_list()
            .map(|al| {
                al.named_params()
                    .filter_map(|np| {
                        let name = np.name()?;
                        let expr = np.value_expr()?;
                        let r = expr.syntax().text_range();
                        let s: usize = r.start().into();
                        let e: usize = r.end().into();
                        Some((name, sql[s..e].to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let expanded = substitute_params_with_named(body_sql, params, &positional, &named);
        let range = call.text_range();
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        replacements.push((start, end, expanded));
    }

    // Apply replacements right-to-left to preserve offsets.
    replacements.sort_by_key(|r| std::cmp::Reverse(r.0));
    let mut result = sql.to_string();
    for (start, end, replacement) in replacements {
        result.replace_range(start..end, &replacement);
    }
    result
}

/// Compile a test for a whole model by mocking smelt.ref() calls.
///
/// Replaces each `smelt.models.name` with the bare CTE name and prepends
/// mock CTE definitions as a WITH clause.
///
/// When `fn_bodies` is provided, `smelt.functions.*` call nodes are also
/// expanded inline using named-argument substitution.
pub fn compile_whole_model_test(
    model_sql: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    sql_body: Option<&str>,
) -> Result<String, String> {
    compile_whole_model_test_inner(model_sql, inputs, sql_body, None)
}

/// Like [`compile_whole_model_test`] but also expands `smelt.functions.*` call
/// nodes using the provided function body map.
pub fn compile_whole_model_test_with_fns(
    model_sql: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    sql_body: Option<&str>,
    fn_bodies: &FnBodyMap,
) -> Result<String, String> {
    compile_whole_model_test_inner(model_sql, inputs, sql_body, Some(fn_bodies))
}

fn compile_whole_model_test_inner(
    model_sql: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    sql_body: Option<&str>,
    fn_bodies: Option<&FnBodyMap>,
) -> Result<String, String> {
    let clean = smelt_parser::strip_frontmatter(model_sql);
    // Apply compile-time meta-language expansion (HOFs, reduce, pipe,
    // ternary, config vars) before codegen — mirroring SqlCompiler::compile.
    // Without this, `reduce([…], and_all)` reaches DuckDB verbatim and fails
    // with "Referenced column 'and_all' was not found because the FROM clause
    // is missing" (config vars left verbatim: absent vars are no-ops).
    let empty_vars = std::collections::BTreeMap::new();
    let clean = smelt_runtime::meta_eval::expand_in_model_meta(
        &clean,
        &smelt_runtime::meta_eval::MetaEvalContext::vars_only(&empty_vars),
    );
    let parse = smelt_parser::parse(&clean);
    let file = AstFile::cast(parse.syntax()).ok_or("Failed to parse model SQL")?;

    // Collect all smelt.<path> refs and their text ranges (in reverse order for replacement).
    // The CTE name is the path segments joined by "_" (e.g. smelt.users → "users",
    // smelt.staging.orders → "staging_orders").
    // The public `inputs` key uses dot-separation (D-42: "silver.orders", not "silver_orders").
    let mut ref_replacements: Vec<(usize, usize, String)> = Vec::new();
    // Map CTE name → dot-separated inputs key for the mock-CTE lookup below.
    let mut cte_name_to_dot_key: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for path_ref in file
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathRef::cast)
    {
        let segments = path_ref.segments();
        if segments.is_empty() {
            continue;
        }
        let cte_name = segments.join("_");
        let dot_key = segments.join(".");
        cte_name_to_dot_key
            .entry(cte_name.clone())
            .or_insert(dot_key);
        let range = path_ref.text_range();
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        ref_replacements.push((start, end, cte_name));
    }

    // Sort by start position descending so replacements don't shift offsets
    ref_replacements.sort_by_key(|r| std::cmp::Reverse(r.0));

    // Replace smelt.<path> refs with bare CTE names in the SQL
    let mut result_sql = clean.clone();
    let mut ref_names: Vec<String> = Vec::new();
    for (start, end, name) in &ref_replacements {
        result_sql.replace_range(*start..*end, name);
        if !ref_names.contains(name) {
            ref_names.push(name.clone());
        }
    }
    ref_names.sort();

    // Expand smelt.functions.* calls if a body map was provided.  This runs
    // AFTER path-ref substitution so named-arg values like
    // `source => silver_events_parsed` already reference the CTE name.
    if let Some(bodies) = fn_bodies {
        result_sql = expand_fn_calls_in_sql(&result_sql, bodies);
    }

    // Build mock CTEs — every smelt.<path> ref in the model gets a CTE.
    // Refs present in `inputs` (keyed by dot-separated address, D-42) get the
    // provided rows; unlisted refs get an empty CTE (zero rows) per spec
    // Semantics §Whole-model tests.
    let mut mock_cte_parts: Vec<String> = Vec::new();
    for ref_name in &ref_names {
        let dot_key = cte_name_to_dot_key
            .get(ref_name)
            .map(String::as_str)
            .unwrap_or(ref_name.as_str());
        let rows = inputs.get(dot_key).map(|v| v.as_slice()).unwrap_or(&[]);
        mock_cte_parts.push(yaml_rows_to_sql(ref_name, rows));
    }

    // Add sql_body CTEs if provided
    if let Some(body) = sql_body {
        let body_ctes = extract_ctes(&format!("WITH {} SELECT 1", body));
        for cte in body_ctes {
            if !mock_cte_parts.iter().any(|p| p.starts_with(&cte.name)) {
                mock_cte_parts.push(format!("{} AS ({})", cte.name, cte.body));
            }
        }
    }

    // Prepend WITH clause.
    //
    // If the model SQL already contains a WITH clause (common for multi-CTE
    // models), inject the mock CTEs inside the existing WITH rather than
    // prepending a second WITH keyword, which is invalid SQL.
    //
    // Models often have a leading block of SQL comments before the WITH keyword,
    // so we scan for the first occurrence of " WITH " (case-insensitive) rather
    // than checking whether the SQL starts with "WITH".
    //
    // e.g. model SQL (comments elided):
    //   WITH lagged AS (...) SELECT ... FROM lagged
    // becomes:
    //   WITH silver_events_parsed AS (...),
    //   lagged AS (...) SELECT ... FROM lagged
    let trimmed = result_sql.trim();
    if mock_cte_parts.is_empty() {
        Ok(trimmed.to_string())
    } else {
        let mock_sql = mock_cte_parts.join(",\n");
        if let Some(with_pos) = find_leading_with(trimmed) {
            // Inject mock CTEs right after the existing WITH keyword.
            let (prefix, after_with) = trimmed.split_at(with_pos + "WITH".len());
            Ok(format!(
                "{} {},\n{}",
                prefix,
                mock_sql,
                after_with.trim_start()
            ))
        } else {
            Ok(format!("WITH {}\n{}", mock_sql, trimmed))
        }
    }
}

/// Find the byte position of the first top-level `WITH` keyword in `sql`.
///
/// Returns `Some(pos)` if the SQL's non-comment, non-whitespace content begins
/// with `WITH`, and `None` otherwise.  Only leading single-line (`--`) and
/// block (`/* */`) comments are skipped; the function stops as soon as it
/// encounters anything other than whitespace or a comment prefix.
fn find_leading_with(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    loop {
        // Skip whitespace
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            return None;
        }

        // Skip single-line comment: -- ... \n
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Skip block comment: /* ... */
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // skip closing */
            continue;
        }

        // Check for WITH keyword (case-insensitive), followed by whitespace.
        if i + 4 < len
            && bytes[i..i + 4].eq_ignore_ascii_case(b"WITH")
            && bytes[i + 4].is_ascii_whitespace()
        {
            return Some(i);
        }

        return None;
    }
}

/// Compile a column-level test into a SQL query.
/// Returns (test_display_name, sql) where the SQL returns failing rows (0 rows = pass).
pub fn compile_column_test(
    schema: &str,
    table: &str,
    column: &str,
    test: &smelt_core::metadata::ColumnTest,
) -> Result<(String, String), String> {
    match test {
        smelt_core::metadata::ColumnTest::Simple(name) => match name.as_str() {
            "not_null" => Ok((
                format!("{}.{}.not_null", table, column),
                format!(
                    "SELECT \"{}\" FROM \"{}\".\"{}\" WHERE \"{}\" IS NULL LIMIT 1",
                    column, schema, table, column
                ),
            )),
            "unique" => Ok((
                format!("{}.{}.unique", table, column),
                format!(
                    "SELECT \"{col}\", COUNT(*) AS cnt FROM \"{schema}\".\"{table}\" GROUP BY \"{col}\" HAVING COUNT(*) > 1 LIMIT 1",
                    col = column, schema = schema, table = table
                ),
            )),
            other => Err(format!("Unknown column test: '{}'", other)),
        },
        smelt_core::metadata::ColumnTest::Parameterized(params) => {
            // Reject entries with multiple constraint keys (e.g., {min: 0, max: 100})
            // since the else-if chain would silently ignore all but the first.
            // Each constraint should be its own entry: [{min: 0}, {max: 100}]
            let constraint_keys: Vec<_> = params
                .keys()
                .filter(|k| {
                    matches!(k.as_str(), "min" | "max" | "accepted_values")
                })
                .collect();
            if constraint_keys.len() > 1 {
                return Err(format!(
                    "Column test has multiple constraints {:?} in a single entry. \
                     Use separate entries instead: e.g., [{{min: 0}}, {{max: 100}}]",
                    constraint_keys
                ));
            }

            if let Some(values) = params.get("accepted_values") {
                let values_list = match values {
                    serde_yaml::Value::Sequence(seq) => seq
                        .iter()
                        .map(|v| match v {
                            serde_yaml::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                            serde_yaml::Value::Number(n) => n.to_string(),
                            serde_yaml::Value::Bool(b) => b.to_string(),
                            _ => format!("'{}'", v.as_str().unwrap_or("")),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => return Err("accepted_values must be a list".to_string()),
                };
                Ok((
                    format!("{}.{}.accepted_values", table, column),
                    format!(
                        "SELECT \"{col}\" FROM \"{schema}\".\"{table}\" WHERE \"{col}\" NOT IN ({values}) AND \"{col}\" IS NOT NULL LIMIT 1",
                        col = column, schema = schema, table = table, values = values_list
                    ),
                ))
            } else if let Some(min_val) = params.get("min") {
                let min_str = match min_val {
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::String(s) => format!("'{}'", s),
                    _ => return Err("min value must be a number or string".to_string()),
                };
                Ok((
                    format!("{}.{}.min", table, column),
                    format!(
                        "SELECT \"{col}\" FROM \"{schema}\".\"{table}\" WHERE \"{col}\" < {min} LIMIT 1",
                        col = column, schema = schema, table = table, min = min_str
                    ),
                ))
            } else if let Some(max_val) = params.get("max") {
                let max_str = match max_val {
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::String(s) => format!("'{}'", s),
                    _ => return Err("max value must be a number or string".to_string()),
                };
                Ok((
                    format!("{}.{}.max", table, column),
                    format!(
                        "SELECT \"{col}\" FROM \"{schema}\".\"{table}\" WHERE \"{col}\" > {max} LIMIT 1",
                        col = column, schema = schema, table = table, max = max_str
                    ),
                ))
            } else {
                Err(format!(
                    "Unknown parameterized test: {:?}",
                    params.keys().collect::<Vec<_>>()
                ))
            }
        }
    }
}

/// Validate the `expect` list of a test config. Returns an error string when
/// `expect` is empty — spec Constraint-3 requires at least one expected row.
pub fn validate_test_expect(expect: &[BTreeMap<String, serde_yaml::Value>]) -> Option<String> {
    if expect.is_empty() {
        Some("test has no 'expect' rows — 'expect' is required (spec Constraint-3)".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ctes_basic() {
        let sql = r#"
WITH cleaned AS (
    SELECT user_id, amount FROM raw_orders WHERE status = 'completed'
),
daily AS (
    SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily
"#;
        let ctes = extract_ctes(sql);
        assert_eq!(ctes.len(), 2);
        assert_eq!(ctes[0].name, "cleaned");
        assert!(ctes[0].dependencies.is_empty());
        assert_eq!(ctes[1].name, "daily");
        assert_eq!(ctes[1].dependencies, vec!["cleaned"]);
    }

    #[test]
    fn test_extract_ctes_no_with() {
        let sql = "SELECT * FROM users";
        let ctes = extract_ctes(sql);
        assert!(ctes.is_empty());
    }

    #[test]
    fn test_extract_ctes_chain() {
        let sql = r#"
WITH a AS (SELECT 1 as x),
b AS (SELECT x FROM a),
c AS (SELECT x FROM b)
SELECT * FROM c
"#;
        let ctes = extract_ctes(sql);
        assert_eq!(ctes.len(), 3);
        assert!(ctes[0].dependencies.is_empty());
        assert_eq!(ctes[1].dependencies, vec!["a"]);
        assert_eq!(ctes[2].dependencies, vec!["b"]);
    }

    #[test]
    fn test_contains_word() {
        assert!(contains_word("FROM CLEANED GROUP BY", "CLEANED"));
        assert!(!contains_word("FROM CLEANED_V2 GROUP BY", "CLEANED"));
        assert!(contains_word("CLEANED", "CLEANED"));
    }

    #[test]
    fn test_contains_word_skips_string_literals() {
        // Should NOT match CTE name inside a string literal
        assert!(!contains_word(
            "SELECT * FROM RAW WHERE STATUS = 'EVENTS'",
            "EVENTS"
        ));
        // Should still match actual table reference
        assert!(contains_word(
            "SELECT * FROM EVENTS WHERE STATUS = 'ACTIVE'",
            "EVENTS"
        ));
        // Escaped quotes should be handled
        assert!(!contains_word(
            "SELECT * FROM RAW WHERE NAME = 'IT''S EVENTS'",
            "EVENTS"
        ));
    }

    #[test]
    fn test_extract_ctes_with_frontmatter() {
        let sql = r#"---
name: my_model
materialization: table
---
WITH step1 AS (
    SELECT 1 as x
)
SELECT * FROM step1
"#;
        let ctes = extract_ctes(sql);
        assert_eq!(ctes.len(), 1);
        assert_eq!(ctes[0].name, "step1");
    }

    #[test]
    fn test_yaml_rows_to_sql_basic() {
        let mut row = BTreeMap::new();
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        row.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        let result = yaml_rows_to_sql("mock_data", &[row]);
        assert!(result.contains("mock_data AS"));
        assert!(result.contains("VALUES"));
        assert!(result.contains("t(amount, user_id)"));
    }

    #[test]
    fn test_yaml_rows_to_sql_empty() {
        let result = yaml_rows_to_sql("empty", &[]);
        assert!(result.contains("WHERE 1=0"));
    }

    #[test]
    fn test_yaml_rows_to_sql_date_detection() {
        let mut row = BTreeMap::new();
        row.insert(
            "day".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        let result = yaml_rows_to_sql("dates", &[row]);
        assert!(result.contains("::DATE"));
    }

    #[test]
    fn test_compile_cte_test_basic() {
        let model_sql = r#"
WITH cleaned AS (
    SELECT user_id, amount FROM raw_orders WHERE status = 'completed'
),
daily AS (
    SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily
"#;
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        row.insert(
            "created_at".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        inputs.insert("cleaned".to_string(), vec![row]);

        let result = compile_cte_test(model_sql, "daily", &inputs, None).unwrap();
        assert!(result.contains("cleaned AS"));
        assert!(result.contains("SUM(amount)"));
        // The target CTE body should be the main SELECT, not wrapped in a CTE
        assert!(!result.contains("daily AS"));
    }

    #[test]
    fn test_compile_cte_test_missing_dependency() {
        let model_sql = r#"
WITH a AS (SELECT 1 as x),
b AS (SELECT x FROM a)
SELECT * FROM b
"#;
        let inputs = BTreeMap::new(); // No mock for 'a'
        let result = compile_cte_test(model_sql, "b", &inputs, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_whole_model_test() {
        let model_sql = r#"
SELECT order_date, COUNT(*) AS order_count
FROM smelt.raw_orders
GROUP BY order_date
"#;
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "order_date".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        row.insert(
            "order_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        inputs.insert("raw_orders".to_string(), vec![row]);

        let result = compile_whole_model_test(model_sql, &inputs, None).unwrap();
        assert!(result.contains("raw_orders AS"));
        assert!(result.contains("order_count"));
        // smelt path refs should be replaced with bare model names
        assert!(!result.contains("smelt.raw_orders"));
    }

    #[test]
    fn test_compile_column_test_not_null() {
        use smelt_core::metadata::ColumnTest;
        let test = ColumnTest::Simple("not_null".to_string());
        let (name, sql) = compile_column_test("main", "orders", "order_id", &test).unwrap();
        assert_eq!(name, "orders.order_id.not_null");
        assert!(sql.contains("IS NULL"));
        assert!(sql.contains("\"main\".\"orders\""));
    }

    #[test]
    fn test_compile_column_test_unique() {
        use smelt_core::metadata::ColumnTest;
        let test = ColumnTest::Simple("unique".to_string());
        let (name, sql) = compile_column_test("main", "orders", "order_id", &test).unwrap();
        assert_eq!(name, "orders.order_id.unique");
        assert!(sql.contains("HAVING COUNT(*)"));
    }

    #[test]
    fn test_compile_column_test_accepted_values() {
        use smelt_core::metadata::ColumnTest;
        let mut params = BTreeMap::new();
        params.insert(
            "accepted_values".to_string(),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("a".to_string()),
                serde_yaml::Value::String("b".to_string()),
            ]),
        );
        let test = ColumnTest::Parameterized(params);
        let (name, sql) = compile_column_test("main", "orders", "status", &test).unwrap();
        assert_eq!(name, "orders.status.accepted_values");
        assert!(sql.contains("NOT IN"));
        assert!(sql.contains("'a'"));
    }

    #[test]
    fn test_compile_column_test_min() {
        use smelt_core::metadata::ColumnTest;
        let mut params = BTreeMap::new();
        params.insert(
            "min".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0)),
        );
        let test = ColumnTest::Parameterized(params);
        let (name, sql) = compile_column_test("main", "orders", "amount", &test).unwrap();
        assert_eq!(name, "orders.amount.min");
        assert!(sql.contains("< 0"));
    }

    #[test]
    fn test_compile_whole_model_test_unlisted_dep_gets_empty_cte() {
        // BUG-041: dependencies not in `inputs` should become empty CTEs (zero rows),
        // not be silently omitted (which produces invalid SQL when DuckDB executes it).
        let model_sql = r#"
SELECT u.user_id, COUNT(o.order_id) AS order_count
FROM smelt.users u
LEFT JOIN smelt.orders o ON u.user_id = o.user_id
GROUP BY u.user_id
"#;
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        inputs.insert("users".to_string(), vec![row]);
        // orders is intentionally NOT in inputs — it should get an empty CTE
        let result = compile_whole_model_test(model_sql, &inputs, None).unwrap();
        assert!(
            result.contains("orders AS"),
            "unlisted dep 'orders' must be mocked as an empty CTE; got:\n{result}"
        );
        assert!(
            result.contains("WHERE 1=0"),
            "empty CTE must use WHERE 1=0; got:\n{result}"
        );
        assert!(
            result.contains("users AS"),
            "listed dep 'users' must be present; got:\n{result}"
        );
    }

    #[test]
    fn test_compile_whole_model_reduce_hof_expanded() {
        // Regression: reduce([true,false,true], and_all) must be expanded to
        // (true) AND (false) AND (true) before execution. Without
        // expand_in_model_meta the reducer name reaches DuckDB verbatim and
        // fails with "Referenced column 'and_all' was not found".
        let model_sql = "SELECT reduce([true, false, true], and_all) AS all_true";
        let inputs = BTreeMap::new();
        let result = compile_whole_model_test(model_sql, &inputs, None).unwrap();
        assert!(
            result.contains("AND"),
            "reduce(and_all) must be expanded to AND fold; got:\n{result}"
        );
        assert!(
            !result.contains("and_all"),
            "and_all reducer name must not appear in final SQL; got:\n{result}"
        );
    }

    #[test]
    fn test_validate_test_expect_empty_is_error() {
        // BUG-043: spec Constraint-3 — `expect` is required.
        // An empty expect list should be caught before test execution.
        let empty: Vec<BTreeMap<String, serde_yaml::Value>> = vec![];
        assert!(
            validate_test_expect(&empty).is_some(),
            "empty expect list must be flagged as an error"
        );
    }

    #[test]
    fn test_validate_test_expect_non_empty_is_ok() {
        let mut row = BTreeMap::new();
        row.insert(
            "x".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        assert!(
            validate_test_expect(&[row]).is_none(),
            "non-empty expect list must be valid"
        );
    }

    #[test]
    fn test_compile_whole_model_test_dot_key_inputs() {
        // D-42: `inputs` keys must use dot-separated bare address paths
        // (e.g. "silver.orders"), not underscore-joined CTE names ("silver_orders").
        let model_sql = "SELECT SUM(amount) AS total FROM smelt.silver.orders";
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        // Key uses dot-separation (the public API).
        inputs.insert("silver.orders".to_string(), vec![row]);
        let result = compile_whole_model_test(model_sql, &inputs, None).unwrap();
        // The CTE name in generated SQL uses underscore (valid SQL identifier).
        assert!(
            result.contains("silver_orders AS"),
            "CTE name must use underscore form; got:\n{result}"
        );
        // The row data must appear — this proves the dot-key lookup found the rows.
        assert!(
            result.contains("100"),
            "rows from inputs must be injected under the dot-key; got:\n{result}"
        );
        // Must not be an empty CTE (WHERE 1=0 signals empty mock).
        assert!(
            !result.contains("WHERE 1=0"),
            "dot-key lookup must not produce an empty CTE; got:\n{result}"
        );
    }
}
