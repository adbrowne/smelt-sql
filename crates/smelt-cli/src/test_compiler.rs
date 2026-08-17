//! Test compiler: extracts CTEs and compiles test SQL.

use std::collections::BTreeMap;

use smelt_parser::ast::{File as AstFile, RecordLiteral};
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

/// Check if a string looks like a decimal number: contains `.`, no exponent, parseable as f64.
/// These strings are cast to `DECIMAL` (not `VARCHAR`) in generated SQL (D-44).
///
/// Examples: "300.00" → true, "3.14e2" → false, "42" → false.
pub fn is_decimal_string(s: &str) -> bool {
    if !s.contains('.') || s.contains('e') || s.contains('E') {
        return false;
    }
    s.parse::<f64>().is_ok()
}

/// Count the number of digits after the decimal point in a decimal string.
///
/// Used to determine the CAST scale so DuckDB preserves trailing zeros (e.g.
/// `'100.50'` → scale 2 → `DECIMAL(18, 2)` → Arrow shows `"100.50"` not `"100.500"`).
fn decimal_string_scale(s: &str) -> usize {
    s.find('.').map(|pos| s.len() - pos - 1).unwrap_or(0)
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
            } else if is_decimal_string(s) {
                // D-44: decimal-shaped strings cast to DECIMAL(18, scale), not VARCHAR,
                // so that SUM/AVG etc. accept them and trailing zeros are preserved
                // (e.g. '100.50' → DECIMAL(18,2) → Arrow "100.50", not "100.500").
                let scale = decimal_string_scale(s);
                format!(
                    "CAST('{}' AS DECIMAL(18, {}))",
                    s.replace('\'', "''"),
                    scale
                )
            } else {
                format!("'{}'", s.replace('\'', "''"))
            }
        }
        _ => "NULL".to_string(),
    }
}

/// Convert YAML rows to a SQL CTE definition wrapping an inline row-set
/// derived table — [`smelt_core::build_row_set_table`], the single
/// dialect-aware owner, picks `VALUES (…)` or the portable `SELECT … UNION
/// ALL SELECT …` rewrite GoogleSQL requires.
///
/// `smelt test` always executes the compiled SQL against an embedded
/// DuckDB regardless of the project's configured backend target, so every
/// caller here passes `BackendType::DuckDB`; the dialect parameter exists
/// so this function does not format its own row-set constructor and drift
/// from the shared owner if `smelt test` ever gains a target-aware mode.
///
/// Example output (DuckDB):
/// ```sql
/// mock_data AS (SELECT * FROM (VALUES (1, 100.0, '2024-01-01'::DATE)) AS t(user_id, amount, created_at))
/// ```
pub fn yaml_rows_to_sql(
    dialect: smelt_core::BackendType,
    name: &str,
    rows: &[BTreeMap<String, serde_yaml::Value>],
) -> String {
    if rows.is_empty() {
        let row_set = smelt_core::build_row_set_table(
            dialect,
            "t",
            &["__empty"],
            &[vec!["NULL".to_string()]],
        );
        return format!("{} AS (SELECT * FROM {} WHERE 1=0)", name, row_set);
    }

    // Derive columns from first row (BTreeMap gives alphabetical order)
    let columns: Vec<&String> = rows[0].keys().collect();

    let value_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|col| {
                    row.get(*col)
                        .map(yaml_value_to_sql)
                        .unwrap_or_else(|| "NULL".to_string())
                })
                .collect()
        })
        .collect();

    let col_names: Vec<&str> = columns.iter().map(|c| c.as_str()).collect();
    let row_set = smelt_core::build_row_set_table(dialect, "t", &col_names, &value_rows);
    format!("{} AS (SELECT * FROM {})", name, row_set)
}

// ── AST → row bridge (Phase 5) ─────────────────────────────────────────────

/// Convert a `RecordLiteral` from the `smelt.test` AST to a row map that the
/// existing `yaml_rows_to_sql()` / `yaml_value_to_sql()` coercion table accepts.
///
/// Each `RecordField` value expression is parsed from its raw source text:
/// - Integer literal  `42`          → `Value::Number(i64)`
/// - Float literal    `3.14`        → `Value::Number(f64)`
/// - Decimal string   `'300.00'`    → `Value::String("300.00")`  (is_decimal_string)
/// - Date string      `'2024-01-01'`→ `Value::String("2024-01-01")` (is_date_string)
/// - Other strings    `'hello'`     → `Value::String("hello")`
/// - Boolean          `true/false`  → `Value::Bool`
/// - Null             `null`        → `Value::Null`
///
/// Omitted fields (property-test rows) are absent from the returned map; the
/// caller's property loop generates random values for them.
pub fn record_literal_to_yaml_row(lit: &RecordLiteral) -> BTreeMap<String, serde_yaml::Value> {
    let mut row = BTreeMap::new();
    for field in lit.fields() {
        let name = match field.name() {
            Some(n) => n,
            None => continue,
        };
        let value_text = match field.value_expr() {
            Some(expr) => expr.syntax().text().to_string(),
            None => continue,
        };
        let yaml_value = ast_value_text_to_yaml(value_text.trim());
        row.insert(name, yaml_value);
    }
    row
}

/// Convert raw expression text from a `RecordField` value_expr to a
/// `serde_yaml::Value`.  The text is the verbatim source text of the
/// expression — we recognise only the literal forms that are valid in a
/// `smelt.test` record literal.
fn ast_value_text_to_yaml(text: &str) -> serde_yaml::Value {
    // NULL
    if text.eq_ignore_ascii_case("null") {
        return serde_yaml::Value::Null;
    }
    // Boolean
    if text.eq_ignore_ascii_case("true") {
        return serde_yaml::Value::Bool(true);
    }
    if text.eq_ignore_ascii_case("false") {
        return serde_yaml::Value::Bool(false);
    }
    // String literal: starts and ends with single quote.
    // The inner content is used as-is (escaped '' → ' by unescaping).
    if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2 {
        let inner = &text[1..text.len() - 1];
        let unescaped = inner.replace("''", "'");
        return serde_yaml::Value::String(unescaped);
    }
    // Integer: no decimal point, parseable as i64
    if !text.contains('.') {
        if let Ok(n) = text.parse::<i64>() {
            return serde_yaml::Value::Number(serde_yaml::Number::from(n));
        }
    }
    // Float: has decimal point, parseable as f64
    if let Ok(f) = text.parse::<f64>() {
        // Use serde_yaml::Number::from(f64) — note this may lose precision
        // for very large decimals, but yaml_value_to_sql's `is_decimal_string`
        // already handles numeric strings passed as Value::String so users
        // should quote decimal values that need exact precision.
        return serde_yaml::Value::Number(serde_yaml::Number::from(f));
    }
    // Fallback: treat as NULL (covers parse failures and unsupported forms)
    serde_yaml::Value::Null
}

// ──────────────────────────────────────────────────────────────────────────────

/// Find external `smelt.<path>` refs in a SQL body, replace them with
/// underscore-joined CTE names, and return (replaced_sql, [(cte_name, dot_key)]).
fn find_and_replace_smelt_path_refs(body_sql: &str) -> (String, Vec<(String, String)>) {
    let parse = smelt_parser::parse(body_sql);
    let file = match AstFile::cast(parse.syntax()) {
        Some(f) => f,
        None => return (body_sql.to_string(), vec![]),
    };

    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let mut refs: Vec<(String, String)> = Vec::new();

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
        if !refs.iter().any(|(n, _)| n == &cte_name) {
            refs.push((cte_name.clone(), dot_key));
        }
        let range = path_ref.text_range();
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        replacements.push((start, end, cte_name));
    }

    replacements.sort_by_key(|r| std::cmp::Reverse(r.0));
    let mut result = body_sql.to_string();
    for (start, end, name) in replacements {
        result.replace_range(start..end, &name);
    }

    (result, refs)
}

/// Collect the transitive internal CTE chain reachable from `target` through
/// internal CTE dependencies. Returns names in topological order (dependencies
/// before their dependents, `target` last).
fn collect_transitive_chain(ctes: &[CteInfo], target: &str) -> Vec<String> {
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    let mut bfs_order = Vec::new();

    queue.push_back(target.to_string());
    visited.insert(target.to_string());

    while let Some(name) = queue.pop_front() {
        bfs_order.push(name.clone());
        if let Some(cte) = ctes.iter().find(|c| c.name == name) {
            for dep in &cte.dependencies {
                if !visited.contains(dep) {
                    visited.insert(dep.clone());
                    queue.push_back(dep.clone());
                }
            }
        }
    }

    // BFS visits [target, direct_deps, transitive_deps...].
    // Reverse for topological order: dependencies first, target last.
    bfs_order.reverse();
    bfs_order
}

/// Scan `body_sql` for the first `smelt.<path>#<cte>` reference.
///
/// Returns `Some((model_segments, cte_name))` when the body contains a CTE-ref
/// suffix, signalling a CTE-level `smelt.test`; returns `None` for a
/// full-query test (no `#` suffix on any path ref).
pub fn find_cte_ref_in_body(body_sql: &str) -> Option<(Vec<String>, String)> {
    let parse = smelt_parser::parse(body_sql);
    let file = AstFile::cast(parse.syntax())?;
    for node in file.syntax().descendants() {
        if let Some(path_ref) = smelt_parser::ast::SmeltPathRef::cast(node) {
            if let Some(cte_name) = path_ref.cte_name() {
                return Some((path_ref.segments(), cte_name));
            }
        }
    }
    None
}

/// Collect the subject model leaf names from all `smelt.test` declarations in a file.
///
/// Used by `smelt test --select` to determine which regular models a new-syntax test
/// file targets so the file can be included or excluded by the selector.
///
/// For each `smelt.test` declaration, the assertion body SELECT is scanned for
/// `smelt.<path>` refs (including `smelt.<model>#<cte>` refs); the leaf segment
/// of each path (the last segment, i.e. the model name) is collected as the
/// subject model name.  Duplicate leaf names are deduplicated.
pub fn new_syntax_test_subject_model_leaves(content: &str) -> Vec<String> {
    let clean = smelt_parser::strip_frontmatter(content);
    let parse = smelt_parser::parse(&clean);
    let file = match AstFile::cast(parse.syntax()) {
        Some(f) => f,
        None => return vec![],
    };
    let mut leaves = Vec::new();
    for smelt_test in file.tests() {
        let body_select = match smelt_test.body_select() {
            Some(s) => s.syntax().text().to_string(),
            None => continue,
        };
        let body_parse = smelt_parser::parse(&body_select);
        let body_file = match AstFile::cast(body_parse.syntax()) {
            Some(f) => f,
            None => continue,
        };
        for node in body_file.syntax().descendants() {
            if let Some(path_ref) = smelt_parser::ast::SmeltPathRef::cast(node) {
                let segments = path_ref.segments();
                if let Some(leaf) = segments.last() {
                    if !leaves.contains(leaf) {
                        leaves.push(leaf.clone());
                    }
                }
            }
        }
    }
    leaves
}

/// Find the plain (non-`#cte`) `smelt.<path>` model references in a test body
/// SELECT, returning each ref's address segments and its byte range within
/// `body_sql`. Refs that carry a `#<cte>` suffix are excluded (those go through
/// the CTE-level path). Used by the whole-query test path to inline the model
/// under test.
pub fn find_plain_model_refs_in_body(body_sql: &str) -> Vec<(Vec<String>, (usize, usize))> {
    let parse = smelt_parser::parse(body_sql);
    let file = match AstFile::cast(parse.syntax()) {
        Some(f) => f,
        None => return vec![],
    };
    let mut refs = Vec::new();
    for node in file.syntax().descendants() {
        if let Some(path_ref) = smelt_parser::ast::SmeltPathRef::cast(node) {
            if path_ref.cte_name().is_some() {
                continue;
            }
            let range = path_ref.text_range();
            let start: usize = range.start().into();
            let end: usize = range.end().into();
            refs.push((path_ref.segments(), (start, end)));
        }
    }
    refs
}

/// Typed error returned when whole-query test inlining detects a structural
/// problem with a `smelt.<path>` ref in the test body.
///
/// Carries a diagnostic **code** (for terminal rendering as `error[CODE]:`),
/// a human-readable message, the path address of the offending ref (for
/// display), and the ref's byte range within `body_select` (for file-level
/// anchoring by the caller).
#[derive(Debug)]
pub struct TestInliningError {
    /// Diagnostic code, e.g. `"AmbiguousTestModel"` or `"NonStandaloneTestModel"`.
    pub code: &'static str,
    /// Human-readable description of the problem.
    pub message: String,
    /// Dotted address of the offending ref (e.g. `"users"`, `"a.users"`).
    pub ref_path: String,
    /// Byte range of the offending ref within `body_select` (start, end).
    pub body_ref_range: (usize, usize),
}

impl std::fmt::Display for TestInliningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TestInliningError {}

/// Detect whether a model body contains a non-standalone construct — a
/// per-model element that cannot be compiled in an isolated context without a
/// real build. Currently detects:
///   * `smelt.config.var(…)` — compile-time config variable that requires the
///     workspace `vars:` map to be resolved. Without a build context the call
///     is left verbatim and the backend rejects it.
///
/// Used by `inline_unmocked_model_refs` to fail loud with `NonStandaloneTestModel`
/// instead of letting a raw backend error surface.
fn contains_non_standalone_construct(body: &str) -> bool {
    // Match the qualified call marker, not a bare `config.var` substring, so a
    // mention inside a string literal or comment doesn't misclassify the model.
    body.contains("smelt.config.var")
}

/// Resolve a `smelt.<path>` ref's address segments to the body of the project
/// model it names, if any. A fully-qualified ref matches a canonical address in
/// `canonical_bodies`. A single-segment ref (`smelt.users`) matches by leaf name
/// via `leaf_to_canonicals`; if that leaf is the name of two or more distinct
/// models, the reference is **ambiguous** — returns `Err(candidates)` carrying
/// the list of matching canonical addresses so the caller can emit
/// `AmbiguousTestModel`. A ref that names no project model returns `Ok(None)`
/// (it is a source/seed/extern, left for the mock pass).
fn resolve_model_body<'a>(
    segments: &[String],
    canonical_bodies: &'a BTreeMap<String, String>,
    leaf_to_canonicals: &BTreeMap<String, Vec<String>>,
) -> Result<Option<&'a String>, Vec<String>> {
    let dot_key = segments.join(".");
    if let Some(body) = canonical_bodies.get(&dot_key) {
        return Ok(Some(body));
    }
    if segments.len() == 1 {
        if let Some(canonicals) = leaf_to_canonicals.get(&dot_key) {
            match canonicals.as_slice() {
                [only] => return Ok(canonical_bodies.get(only)),
                many => {
                    return Err(many.to_vec());
                }
            }
        }
    }
    Ok(None)
}

/// Recursively inline `smelt.<path>` model references in `sql` that are NOT
/// directly mocked via `inputs`. A ref whose dotted address is a key in `inputs`
/// is left in place (it is substituted with mock rows by the final mock pass);
/// a ref that resolves to a project model (see [`resolve_model_body`]) is
/// replaced by its parenthesised body, whose own refs are inlined in turn. A ref
/// that names no project model (a source/seed/extern) is left for the mock pass.
///
/// Returns `Err(TestInliningError)` when an ambiguous single-segment ref or a
/// non-standalone upstream is encountered; the error carries the diagnostic
/// code, message, and the offending ref's byte range within `sql` for anchoring.
fn inline_unmocked_model_refs(
    sql: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    canonical_bodies: &BTreeMap<String, String>,
    leaf_to_canonicals: &BTreeMap<String, Vec<String>>,
    depth: usize,
) -> Result<String, TestInliningError> {
    if depth > 32 {
        return Err(TestInliningError {
            code: "TestInliningDepthExceeded",
            message: "smelt.test model inlining exceeded depth 32 — possible dependency cycle"
                .to_string(),
            ref_path: String::new(),
            body_ref_range: (0, 0),
        });
    }
    let refs = find_plain_model_refs_in_body(sql);
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    for (segments, (start, end)) in refs {
        let dot_key = segments.join(".");
        // Directly mocked → leave for the mock pass.
        if inputs.contains_key(&dot_key) {
            continue;
        }
        match resolve_model_body(&segments, canonical_bodies, leaf_to_canonicals) {
            Err(candidates) => {
                // Ambiguous single-segment ref: multiple models share this leaf name.
                return Err(TestInliningError {
                    code: "AmbiguousTestModel",
                    message: format!(
                        "'smelt.{}' matches multiple models ({}); \
                         reference it by its full dotted address",
                        dot_key,
                        candidates.join(", ")
                    ),
                    ref_path: dot_key,
                    body_ref_range: (start, end),
                });
            }
            Ok(None) => {
                // Not a project model (source/seed/extern) → leave for the mock pass.
            }
            Ok(Some(body)) => {
                // Before inlining, detect non-standalone constructs in the upstream body.
                if contains_non_standalone_construct(body) {
                    return Err(TestInliningError {
                        code: "NonStandaloneTestModel",
                        message: format!(
                            "'smelt.{}' uses smelt.config.var (per-model config vars \
                             that require a build context) and cannot compile standalone. \
                             Mock this dependency at its boundary with: \
                             PASSING {} AS (...)",
                            dot_key, dot_key
                        ),
                        ref_path: dot_key,
                        body_ref_range: (start, end),
                    });
                }
                let inner = inline_unmocked_model_refs(
                    body,
                    inputs,
                    canonical_bodies,
                    leaf_to_canonicals,
                    depth + 1,
                )
                .map_err(|mut e| {
                    // A transitive error carries a range relative to the
                    // upstream body it was found in. Re-anchor it to this
                    // ref at every unwind level so the final range is always
                    // relative to the original `body_select` (the test file).
                    e.message = format!("{} (reached via 'smelt.{}')", e.message, dot_key);
                    e.body_ref_range = (start, end);
                    e
                })?;
                replacements.push((start, end, format!("(\n{}\n)", inner)));
            }
        }
    }
    // Apply right-to-left so earlier byte offsets stay valid.
    replacements.sort_by_key(|r| std::cmp::Reverse(r.0));
    let mut result = sql.to_string();
    for (start, end, rep) in replacements {
        result.replace_range(start..end, &rep);
    }
    Ok(result)
}

/// Compile a whole-query (non-`#cte`) test. The assertion `body_select` may:
///   * be self-contained (no `smelt.<path>` refs), or
///   * read its dependencies directly (`FROM smelt.users` mocked by `PASSING users`), or
///   * reference a model under test (`FROM smelt.gold.x`), whose own upstream deps
///     are mocked via `PASSING`.
///
/// Every `smelt.<path>` ref that is NOT directly provided in `inputs` and that
/// resolves to a project model is inlined recursively, so the assertion runs
/// against the real model output and the model's upstream deps become the
/// mockable `PASSING` inputs (testing.md §Execution model — "inlining the body
/// of every model it references"). The remaining refs (those in `inputs`, plus
/// sources/seeds) are then substituted with mock CTEs, with `smelt.functions.*`
/// expanded when `fn_bodies` is provided.
///
/// `canonical_bodies` maps every regular model's canonical dotted address to its
/// frontmatter-stripped body; `leaf_to_canonicals` maps each model's leaf name
/// to the canonical addresses that share it (so a single-segment ref to an
/// ambiguous leaf fails loud rather than resolving arbitrarily).
pub fn compile_whole_query_test(
    body_select: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    canonical_bodies: &BTreeMap<String, String>,
    leaf_to_canonicals: &BTreeMap<String, Vec<String>>,
    fn_bodies: Option<&FnBodyMap>,
) -> Result<String, TestInliningError> {
    let inlined =
        inline_unmocked_model_refs(body_select, inputs, canonical_bodies, leaf_to_canonicals, 0)?;
    compile_whole_model_test_inner(&inlined, inputs, None, fn_bodies).map_err(|msg| {
        TestInliningError {
            code: "TestCompilationError",
            message: msg,
            ref_path: String::new(),
            body_ref_range: (0, 0),
        }
    })
}

/// Compile a test that targets a specific CTE within a model.
///
/// Spec §"CTE-level tests" (D-45): the mock boundary is the model's external
/// `smelt.<path>` dependencies reachable from the target CTE's transitive
/// internal chain — NOT the internal CTEs themselves. Internal CTEs run
/// as-written; only the external `smelt.<path>` refs are substituted with mock
/// data from `inputs` (dot-key lookup, D-42).
///
/// Returns a standalone SQL query:
/// `WITH <mock_external_ctes>, <internal_chain_ctes> SELECT * FROM <target_cte>`
pub fn compile_cte_test(
    model_sql: &str,
    target_cte: &str,
    inputs: &BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    _sql_body: Option<&str>,
) -> Result<String, String> {
    let ctes = extract_ctes(model_sql);

    if !ctes.iter().any(|c| c.name == target_cte) {
        return Err(format!(
            "UnknownTestCte: CTE '{}' not found in model",
            target_cte
        ));
    }

    // Collect the transitive internal chain in topological order (deps first).
    let chain = collect_transitive_chain(&ctes, target_cte);

    // For each CTE in the chain, replace external smelt.<path> refs with
    // _-joined names and collect all discovered external refs.
    let mut external_refs: Vec<(String, String)> = Vec::new(); // (cte_name, dot_key)
    let mut chain_ctes: Vec<(String, String)> = Vec::new(); // (name, replaced_body)

    for cte_name in &chain {
        let cte = match ctes.iter().find(|c| &c.name == cte_name) {
            Some(c) => c,
            None => {
                return Err(format!(
                    "internal error: CTE '{}' not found in model",
                    cte_name
                ))
            }
        };
        let (replaced_body, refs) = find_and_replace_smelt_path_refs(&cte.body);
        for (cn, dk) in refs {
            if !external_refs.iter().any(|(n, _)| n == &cn) {
                external_refs.push((cn, dk));
            }
        }
        chain_ctes.push((cte_name.clone(), replaced_body));
    }

    // D-43: reject any inputs key that isn't a reachable external dep.
    let valid_dot_keys: std::collections::HashSet<&str> =
        external_refs.iter().map(|(_, dk)| dk.as_str()).collect();
    let mut unknown_keys: Vec<&str> = inputs
        .keys()
        .filter(|k| !valid_dot_keys.contains(k.as_str()))
        .map(|k| k.as_str())
        .collect();
    if !unknown_keys.is_empty() {
        unknown_keys.sort();
        let mut actual: Vec<&str> = valid_dot_keys.into_iter().collect();
        actual.sort();
        return Err(format!(
            "UnknownTestInput: {} — not a reachable external dependency of CTE '{}'. Reachable deps: [{}]",
            unknown_keys
                .iter()
                .map(|k| format!("'{}'", k))
                .collect::<Vec<_>>()
                .join(", "),
            target_cte,
            actual.join(", ")
        ));
    }

    // Build mock CTEs for external refs using dot-key lookup (D-42).
    // External refs not in `inputs` get an empty CTE (zero rows).
    let mut mock_cte_parts: Vec<String> = Vec::new();
    for (cte_name, dot_key) in &external_refs {
        let rows = inputs.get(dot_key).map(|v| v.as_slice()).unwrap_or(&[]);
        mock_cte_parts.push(yaml_rows_to_sql(
            smelt_core::BackendType::DuckDB,
            cte_name,
            rows,
        ));
    }

    // Internal chain CTEs (all except the target) go in the WITH clause.
    // The target CTE's replaced body is the final SELECT.
    let chain_cte_parts: Vec<String> = chain_ctes
        .iter()
        .filter(|(name, _)| name.as_str() != target_cte)
        .map(|(name, body)| format!("{} AS ({})", name, body))
        .collect();

    let target_body = chain_ctes
        .iter()
        .find(|(name, _)| name.as_str() == target_cte)
        .map(|(_, body)| body.clone())
        .ok_or_else(|| {
            format!(
                "internal error: target CTE '{}' missing from chain",
                target_cte
            )
        })?;

    let all_cte_parts: Vec<String> = mock_cte_parts.into_iter().chain(chain_cte_parts).collect();

    if all_cte_parts.is_empty() {
        Ok(target_body)
    } else {
        Ok(format!(
            "WITH {}\n{}",
            all_cte_parts.join(",\n"),
            target_body
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

    // D-43: reject any inputs key that doesn't match an actual dep of this model.
    // Every key in `inputs` must be a dot-separated path of a real smelt.<path> ref.
    let valid_dot_keys: std::collections::HashSet<&str> =
        cte_name_to_dot_key.values().map(|s| s.as_str()).collect();
    let mut unknown_keys: Vec<&str> = inputs
        .keys()
        .filter(|k| !valid_dot_keys.contains(k.as_str()))
        .map(|k| k.as_str())
        .collect();
    if !unknown_keys.is_empty() {
        unknown_keys.sort();
        let mut actual: Vec<&str> = valid_dot_keys.into_iter().collect();
        actual.sort();
        return Err(format!(
            "UnknownTestInput: {} — not a dependency of this model. Actual deps: [{}]",
            unknown_keys
                .iter()
                .map(|k| format!("'{}'", k))
                .collect::<Vec<_>>()
                .join(", "),
            actual.join(", ")
        ));
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
        mock_cte_parts.push(yaml_rows_to_sql(
            smelt_core::BackendType::DuckDB,
            ref_name,
            rows,
        ));
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
    fn test_is_decimal_string() {
        // True: contains '.', no exponent, parseable
        assert!(is_decimal_string("300.00"));
        assert!(is_decimal_string("1.0000001"));
        assert!(is_decimal_string("0.5"));
        assert!(is_decimal_string("-9.99"));
        // False: no decimal point
        assert!(!is_decimal_string("42"));
        assert!(!is_decimal_string("0"));
        // False: scientific notation
        assert!(!is_decimal_string("3.14e2"));
        assert!(!is_decimal_string("1E10"));
        // False: not a number
        assert!(!is_decimal_string("hello"));
        assert!(!is_decimal_string("2024-01-01"));
        // False: date-shaped (handled by is_date_string, but also no '.' so not decimal)
        assert!(!is_decimal_string("2024-01-01"));
    }

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
        let result = yaml_rows_to_sql(smelt_core::BackendType::DuckDB, "mock_data", &[row]);
        assert!(result.contains("mock_data AS"));
        assert!(result.contains("VALUES"));
        assert!(result.contains("t(amount, user_id)"));
    }

    #[test]
    fn test_yaml_rows_to_sql_empty() {
        let result = yaml_rows_to_sql(smelt_core::BackendType::DuckDB, "empty", &[]);
        assert!(result.contains("WHERE 1=0"));
    }

    /// The rejection test at the `test_compiler` call site: for
    /// `BackendType::BigQuery` the emitted mock-data CTE is never a `FROM
    /// (VALUES …)` table-value constructor. Asserts the emitted SQL is
    /// exactly what `smelt_core::build_row_set_table` renders for the same
    /// rows, so a future hand-rolled (but still VALUES-free) rewrite here
    /// would fail rather than pass vacuously. Uses two rows — a single row
    /// never exercises the `UNION ALL` join between rows, and an equality
    /// check would hold trivially for almost any rewrite.
    #[test]
    fn test_yaml_rows_to_sql_bigquery_is_not_a_values_constructor() {
        let mut row1 = BTreeMap::new();
        row1.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(1)),
        );
        let mut row2 = BTreeMap::new();
        row2.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(2)),
        );
        let result = yaml_rows_to_sql(
            smelt_core::BackendType::BigQuery,
            "mock_data",
            &[row1, row2],
        );
        assert!(
            !result.contains("VALUES"),
            "BigQuery has no table-value constructor, got: {result}"
        );
        let expected_row_set = smelt_core::build_row_set_table(
            smelt_core::BackendType::BigQuery,
            "t",
            &["user_id"],
            &[vec!["1".to_string()], vec!["2".to_string()]],
        );
        let expected = format!("mock_data AS (SELECT * FROM {expected_row_set})");
        assert_eq!(
            result, expected,
            "expected the mock-data CTE to route through smelt_core::build_row_set_table verbatim"
        );
    }

    #[test]
    fn test_yaml_rows_to_sql_date_detection() {
        let mut row = BTreeMap::new();
        row.insert(
            "day".to_string(),
            serde_yaml::Value::String("2024-01-01".to_string()),
        );
        let result = yaml_rows_to_sql(smelt_core::BackendType::DuckDB, "dates", &[row]);
        assert!(result.contains("::DATE"));
    }

    #[test]
    fn test_compile_cte_test_basic() {
        // D-45: targeting 'daily' includes internal dep 'cleaned' in the chain.
        // 'cleaned' has no smelt.<path> refs, so no external mock CTE is needed.
        // The result must have 'cleaned AS (...)' in the WITH clause (runs as-written)
        // and 'daily's body as the final SELECT (not wrapped in a CTE).
        let model_sql = r#"
WITH cleaned AS (
    SELECT user_id, amount FROM raw_orders WHERE status = 'completed'
),
daily AS (
    SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily
"#;
        let inputs = BTreeMap::new(); // no smelt.<path> refs in this model

        let result = compile_cte_test(model_sql, "daily", &inputs, None).unwrap();
        // 'cleaned' runs as-written — it's in the WITH clause
        assert!(result.contains("cleaned AS"));
        assert!(result.contains("SUM(amount)"));
        // 'daily' is the target — its body is the final SELECT, not a CTE
        assert!(!result.contains("daily AS"));
    }

    #[test]
    fn test_compile_cte_test_internal_deps_run_as_written() {
        // D-45: internal CTE 'a' is NOT in inputs and is NOT a smelt.<path> dep,
        // so it runs as-written in the WITH clause. This is the new expected behavior:
        // no error when an internal CTE dep is absent from inputs.
        let model_sql = r#"
WITH a AS (SELECT 1 as x),
b AS (SELECT x FROM a)
SELECT * FROM b
"#;
        let inputs = BTreeMap::new();
        let result = compile_cte_test(model_sql, "b", &inputs, None).unwrap();
        // 'a' runs as-written in the WITH clause
        assert!(result.contains("a AS ("));
        // 'b' is the target — its body is the final SELECT, not a CTE
        assert!(!result.contains("b AS ("));
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

    #[test]
    fn test_compile_whole_query_test_inlines_model_and_mocks_its_deps() {
        // The assertion query references a model under test via smelt.<path>.
        // The model itself reads a multi-segment upstream dep. The whole-query
        // compiler must inline the model (so its upstream dep is mockable) and
        // mock that dep from `inputs` keyed by the dotted address.
        let body_select = "SELECT user_id, revenue FROM smelt.marts.customer_revenue";
        let mut canonical_bodies = BTreeMap::new();
        canonical_bodies.insert(
            "marts.customer_revenue".to_string(),
            "SELECT user_id, amount AS revenue FROM smelt.silver.orders".to_string(),
        );
        let mut leaf_to_canonicals = BTreeMap::new();
        leaf_to_canonicals.insert(
            "customer_revenue".to_string(),
            vec!["marts.customer_revenue".to_string()],
        );

        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "user_id".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(7)),
        );
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(250)),
        );
        // The dep key is the model's upstream, NOT the model referenced in body.
        inputs.insert("silver.orders".to_string(), vec![row]);

        let compiled = compile_whole_query_test(
            body_select,
            &inputs,
            &canonical_bodies,
            &leaf_to_canonicals,
            None,
        )
        .unwrap();

        // The model's upstream dep is mocked with the provided rows.
        assert!(
            compiled.contains("silver_orders AS") && compiled.contains("250"),
            "model's upstream dep must be mocked from inputs; got:\n{compiled}"
        );
        // The outer assertion projection is preserved over the inlined subquery.
        assert!(
            compiled.contains("revenue FROM ("),
            "assertion projection must wrap the inlined model as a subquery; got:\n{compiled}"
        );
        // The original smelt.<path> ref to the model is gone (inlined).
        assert!(
            !compiled.contains("smelt.marts.customer_revenue"),
            "model ref must be replaced by the inlined subquery; got:\n{compiled}"
        );
    }

    #[test]
    fn test_compile_whole_query_test_self_contained_and_direct_mock() {
        let no_models: BTreeMap<String, String> = BTreeMap::new();
        let no_leaves: BTreeMap<String, Vec<String>> = BTreeMap::new();

        // (a) Self-contained body with no smelt.<path> ref → returned as-is.
        let self_contained = "SELECT (1.0 + 2.0) AS val";
        let no_inputs = BTreeMap::new();
        let compiled =
            compile_whole_query_test(self_contained, &no_inputs, &no_models, &no_leaves, None)
                .unwrap();
        assert!(
            compiled.contains("SELECT (1.0 + 2.0) AS val"),
            "self-contained body must pass through; got:\n{compiled}"
        );

        // (b) Body reads a dep directly; PASSING mocks it in place (no inlining,
        // even though no model body is supplied for it).
        let direct = "SELECT SUM(amount) AS total FROM smelt.silver.orders";
        let mut inputs = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert(
            "amount".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(100)),
        );
        inputs.insert("silver.orders".to_string(), vec![row]);
        let compiled =
            compile_whole_query_test(direct, &inputs, &no_models, &no_leaves, None).unwrap();
        assert!(
            compiled.contains("silver_orders AS") && compiled.contains("100"),
            "directly-mocked dep must be substituted with rows; got:\n{compiled}"
        );
    }

    #[test]
    fn test_compile_whole_query_test_rejects_unknown_dep() {
        // A PASSING dep that is not reached by the (inlined) query is an error.
        let body_select = "SELECT user_id FROM smelt.marts.customer_revenue";
        let mut canonical_bodies = BTreeMap::new();
        canonical_bodies.insert(
            "marts.customer_revenue".to_string(),
            "SELECT user_id FROM smelt.silver.orders".to_string(),
        );
        let no_leaves: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        inputs.insert("silver.not_a_dep".to_string(), vec![]);
        let err =
            compile_whole_query_test(body_select, &inputs, &canonical_bodies, &no_leaves, None)
                .unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("UnknownTestInput") && err_str.contains("silver.not_a_dep"),
            "expected UnknownTestInput for a non-dependency; got: {err_str}"
        );
    }

    #[test]
    fn test_compile_whole_query_test_ambiguous_leaf_ref_fails_loud() {
        // A single-segment ref to a leaf name shared by two models must NOT
        // silently resolve to one — it is rejected (fail-loud discipline).
        let body_select = "SELECT id FROM smelt.users";
        let mut canonical_bodies = BTreeMap::new();
        canonical_bodies.insert(
            "staging.users".to_string(),
            "SELECT id FROM smelt.raw.users".to_string(),
        );
        canonical_bodies.insert(
            "marts.users".to_string(),
            "SELECT id FROM smelt.raw.users".to_string(),
        );
        let mut leaf_to_canonicals = BTreeMap::new();
        leaf_to_canonicals.insert(
            "users".to_string(),
            vec!["marts.users".to_string(), "staging.users".to_string()],
        );
        let inputs = BTreeMap::new();
        let err = compile_whole_query_test(
            body_select,
            &inputs,
            &canonical_bodies,
            &leaf_to_canonicals,
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, "AmbiguousTestModel");
        let err_str = err.to_string();
        assert!(
            err_str.contains("AmbiguousTestModel") && err_str.contains("smelt.users"),
            "ambiguous single-segment ref must fail loud; got: {err_str}"
        );
    }

    #[test]
    fn test_transitive_inlining_error_anchors_at_test_body_ref() {
        // The test body refs marts.top; marts.top refs staging.base, which is
        // non-standalone (config.var). The error surfaces at depth 1, but the
        // range it carries must be relative to the ORIGINAL body_select — i.e.
        // the `smelt.marts.top` ref the user wrote — not to marts.top's body.
        let body_select = "SELECT id FROM smelt.marts.top";
        let mut canonical_bodies = BTreeMap::new();
        // NB: the upstream ref sits at a DIFFERENT byte offset in marts.top's
        // body than the test-body ref does in `body_select`, so this test
        // discriminates a correctly re-anchored range from a leaked child range.
        canonical_bodies.insert(
            "marts.top".to_string(),
            "SELECT id, extra_col FROM smelt.staging.base".to_string(),
        );
        canonical_bodies.insert(
            "staging.base".to_string(),
            "SELECT smelt.config.var('tenant') AS id".to_string(),
        );
        let no_leaves: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let inputs = BTreeMap::new();
        let err =
            compile_whole_query_test(body_select, &inputs, &canonical_bodies, &no_leaves, None)
                .unwrap_err();
        assert_eq!(err.code, "NonStandaloneTestModel");
        // The offending upstream is named in the message…
        assert!(
            err.message.contains("staging.base"),
            "message must name the non-standalone upstream; got: {}",
            err.message
        );
        // …but the anchor is the ref in the test body.
        let expected_start = body_select.find("smelt.marts.top").unwrap();
        assert_eq!(
            err.body_ref_range.0, expected_start,
            "range must anchor at the test-body ref, not inside the upstream body; got {:?}",
            err.body_ref_range
        );
    }

    #[test]
    fn test_non_standalone_marker_requires_smelt_prefix() {
        // A bare `config.var` mention (e.g. in a comment or string literal)
        // must not misclassify the model as non-standalone.
        let body_select = "SELECT note FROM smelt.marts.notes";
        let mut canonical_bodies = BTreeMap::new();
        canonical_bodies.insert(
            "marts.notes".to_string(),
            "SELECT 'see config.var docs' AS note".to_string(),
        );
        let no_leaves: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let inputs = BTreeMap::new();
        let compiled =
            compile_whole_query_test(body_select, &inputs, &canonical_bodies, &no_leaves, None)
                .unwrap();
        assert!(
            compiled.contains("'see config.var docs'"),
            "a bare config.var mention must inline normally; got:\n{compiled}"
        );
    }
}
