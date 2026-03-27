//! Test compiler: extracts CTEs and compiles test SQL.

use smelt_parser::ast::File as AstFile;

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
fn contains_word(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
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

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
}
