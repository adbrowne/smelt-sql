//! Completion-context detection for the LSP.

/// Completion context types
#[derive(Debug)]
pub enum CompletionContext {
    InsideRef,               // Cursor inside ref('|')
    InsideSource,            // Cursor inside source('|')
    ColumnName,              // Cursor in a position where column name is expected
    QualifiedColumn(String), // Cursor after alias. (e.g., "t." for table alias t)
    FromClause,              // Cursor in FROM/JOIN position (offer CTE names)
    /// Phase 2c: cursor positioned after a `smelt.` prefix (path form), e.g.
    /// `FROM smelt.|` or `FROM smelt.models.|`. Completion should return all
    /// workspace entities as `smelt.<segments>` labels.
    SmeltPath,
    /// Phase 48: cursor inside the body of a `PASSING <name> AS (|)` clause
    /// attached to a `smelt.fn.<callee>(...)` call. Carries the parameter
    /// name and the trailing call-path segment so the completion list can
    /// be filtered by the callee's signature.
    InPassingBody {
        callee: String,
        passing_name: String,
    },
    None,
}

/// Determine what kind of completion to provide based on cursor position
pub fn determine_completion_context(text: &str, offset: usize) -> CompletionContext {
    // Look backward from cursor to determine context
    let before_cursor = &text[..offset.min(text.len())];

    // Phase 48: detect cursor sitting inside the body of a
    // `PASSING <name> AS (|)` clause. Heuristic: walk backwards from the
    // cursor for an unmatched `(` whose preceding tokens form
    // `PASSING <ident> AS`. The callee name is the last segment of the
    // most recent `smelt.fn.<...>` call before the PASSING.
    if let Some(ctx) = detect_passing_body(before_cursor) {
        return ctx;
    }

    // Phase 2c: detect cursor after a `smelt.` path prefix. This must be
    // checked before the legacy `ref(` / `source(` checks so that
    // `smelt.ref(` still falls through to InsideRef.
    // Pattern: text ends with `smelt.` or `smelt.<word>.` (possibly with
    // partial segment at cursor).
    {
        // Find the last word boundary: scan back from cursor for valid path chars
        // (alphanumeric, _, .) until we hit whitespace or other delimiter.
        let trimmed = before_cursor
            .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        let suffix = &before_cursor[trimmed.len()..];
        // A smelt path starts with `smelt.` and contains only word chars and dots.
        if suffix.starts_with("smelt.")
            && suffix[6..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            // Make sure this is NOT `smelt.ref(` or `smelt.source(` (legacy forms
            // get their own context below).
            let rest = &suffix[6..];
            let is_legacy = rest.starts_with("ref(")
                || rest.starts_with("ref('")
                || rest.starts_with("source(")
                || rest.starts_with("source('")
                || rest.starts_with("fn.");
            if !is_legacy {
                return CompletionContext::SmeltPath;
            }
        }
    }

    // Check if we're inside source('')
    // Simple heuristic: look for source(' before cursor and no closing )
    if let Some(source_start) = before_cursor.rfind("source(") {
        let after_source = &before_cursor[source_start..];
        // Check if we're inside the quotes
        let quote_count = after_source
            .chars()
            .filter(|&c| c == '\'' || c == '"')
            .count();
        if quote_count == 1 && !after_source.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideSource;
        }
    }

    // Check if we're inside ref('')
    // Simple heuristic: look for ref(' before cursor and no closing )
    if let Some(ref_start) = before_cursor.rfind("ref(") {
        let after_ref = &before_cursor[ref_start..];
        // Check if we're inside the quotes
        let quote_count = after_ref.chars().filter(|&c| c == '\'' || c == '"').count();
        if quote_count == 1 && !after_ref.contains(')') {
            // Odd number of quotes means we're inside a string, and no closing paren yet
            return CompletionContext::InsideRef;
        }
    }

    // Check if we're after alias. (e.g., "t." for qualified column completion)
    // Look for pattern: identifier followed by dot at or just before cursor
    if let Some(alias) = extract_alias_before_dot(before_cursor) {
        return CompletionContext::QualifiedColumn(alias);
    }

    // Check if we're in a column context (after SELECT, comma in SELECT list)
    let before_trimmed = before_cursor.trim_end();

    // Look for SELECT keyword
    if let Some(select_pos) = before_trimmed.rfind("SELECT") {
        let after_select = &before_trimmed[select_pos..];
        // Make sure we haven't hit FROM yet
        if !after_select.contains("FROM") {
            // We're in the SELECT list
            return CompletionContext::ColumnName;
        }
    }

    // Check if we're in a FROM/JOIN position (after FROM or JOIN keyword)
    // Look for the last FROM or JOIN keyword and check we're in table-ref position
    let upper = before_trimmed.to_uppercase();
    if is_in_from_position(&upper) {
        return CompletionContext::FromClause;
    }

    CompletionContext::None
}

/// Check if cursor is in a FROM/JOIN table reference position
pub(crate) fn is_in_from_position(upper_text: &str) -> bool {
    // Find the last occurrence of FROM or JOIN keywords
    let from_pos = upper_text.rfind("FROM");
    let join_pos = upper_text.rfind("JOIN");

    let keyword_end = match (from_pos, join_pos) {
        (Some(f), Some(j)) => {
            if f > j {
                Some(f + 4) // "FROM" is 4 chars
            } else {
                Some(j + 4) // "JOIN" is 4 chars
            }
        }
        (Some(f), None) => Some(f + 4),
        (None, Some(j)) => Some(j + 4),
        (None, None) => None,
    };

    let keyword_end = match keyword_end {
        Some(e) => e,
        None => return false,
    };

    // Text after the keyword
    let after_keyword = &upper_text[keyword_end..];

    // We're in FROM position if:
    // 1. Nothing after keyword (just whitespace) - typing the first table ref
    // 2. Or after a comma (additional table ref in comma-separated list)
    // But NOT if we've already entered a complete expression (have ON, WHERE, etc.)
    let trimmed = after_keyword.trim();
    if trimmed.is_empty() {
        return true;
    }

    // If we see clause keywords after the FROM/JOIN, we've moved past table position
    let terminating_keywords = [
        "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "UNION", "ON", "USING", "INNER", "LEFT",
        "RIGHT", "FULL", "CROSS", "SELECT",
    ];
    for kw in &terminating_keywords {
        if trimmed.contains(kw) {
            return false;
        }
    }

    // If the text after keyword is just whitespace or a partial identifier, we're in position
    // Check: no complete table expression yet (no whitespace-separated tokens beyond one)
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    // If 0 tokens (just spaces) or 1 partial token being typed - we're in FROM position
    tokens.len() <= 1
}

/// Phase 48: heuristically detect whether the cursor sits inside the body
/// of a `PASSING <name> AS (|)` clause attached to a `smelt.fn.<callee>(...)`
/// call.
///
/// The heuristic walks backwards from the cursor:
/// 1. Find the nearest unmatched `(`. The cursor lies inside whatever
///    parenthesised expression that opener belongs to.
/// 2. Just before that `(` (allowing whitespace), look for the literal
///    `AS`. Before that, an identifier (the parameter name). Before that,
///    the keyword `PASSING`.
/// 3. Before the `PASSING`, the most recent `smelt.fn.<...>(...)` call
///    determines the callee name (last dot-segment of the call path).
///
/// Returns `None` for non-PASSING-body cursors so the rest of the
/// dispatcher takes over.
fn detect_passing_body(before_cursor: &str) -> Option<CompletionContext> {
    // Step 1: find the nearest unmatched open-paren walking right-to-left.
    let mut depth = 0i32;
    let mut open_paren: Option<usize> = None;
    for (i, ch) in before_cursor.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    open_paren = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let open_paren = open_paren?;

    // Step 2: directly before the `(` we should see `AS` (case-insensitive,
    // possibly with surrounding whitespace).
    let pre = before_cursor[..open_paren].trim_end();
    let as_end = pre.len();
    if !pre.to_ascii_uppercase().ends_with("AS") {
        return None;
    }
    let after_as = &pre[..as_end - 2];
    let after_as_trimmed = after_as.trim_end();

    // Step 3: extract the identifier before AS — the PASSING name.
    let mut name_end = after_as_trimmed.len();
    let mut name_start = name_end;
    for (i, ch) in after_as_trimmed.char_indices().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            name_start = i;
        } else {
            name_end = name_start;
            break;
        }
        // If we walk to the very start, name_end stays at full length.
    }
    if name_start == after_as_trimmed.len() {
        return None;
    }
    let passing_name = &after_as_trimmed[name_start..name_end.max(name_start + 1)];
    if passing_name.is_empty() {
        return None;
    }

    // Step 4: before the parameter name, the keyword `PASSING`.
    let pre_name = after_as_trimmed[..name_start].trim_end();
    if !pre_name.to_ascii_uppercase().ends_with("PASSING") {
        return None;
    }

    // Step 5: extract the callee name — last `smelt.functions.<...>` call before
    // the `PASSING`. We look for the most recent `smelt.functions.` literal in
    // `before_cursor` and take the dotted-identifier that follows.
    // Phase 5b: `smelt.fn.*` is removed; only `smelt.functions.*` is valid.
    let smelt_fn = before_cursor.rfind("smelt.functions.")?;
    let after = &before_cursor[smelt_fn + "smelt.functions.".len()..];
    let mut last_segment_end = 0usize;
    for (i, ch) in after.char_indices() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            last_segment_end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    let dotted = &after[..last_segment_end];
    let callee = dotted.split('.').next_back()?.to_string();
    if callee.is_empty() {
        return None;
    }

    Some(CompletionContext::InPassingBody {
        callee,
        passing_name: passing_name.to_string(),
    })
}

/// Extract the alias/identifier before a dot at the end of the text
/// Returns Some(alias) if text ends with "identifier." or "identifier.partial"
fn extract_alias_before_dot(text: &str) -> Option<String> {
    // Find the last dot
    let dot_pos = text.rfind('.')?;

    // Check what's after the dot - should be empty or partial identifier
    let after_dot = &text[dot_pos + 1..];
    if !after_dot.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    // Find the identifier before the dot
    let before_dot = &text[..dot_pos];
    let before_dot_trimmed = before_dot.trim_end();

    // Walk backward to find the start of the identifier
    let mut ident_start = before_dot_trimmed.len();
    for (i, c) in before_dot_trimmed.char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            ident_start = i;
        } else {
            break;
        }
    }

    let alias = &before_dot_trimmed[ident_start..];

    // Must be a valid identifier (not empty, starts with letter or underscore)
    if alias.is_empty() {
        return None;
    }
    let first_char = alias.chars().next()?;
    if !first_char.is_alphabetic() && first_char != '_' {
        return None;
    }

    // Avoid triggering on smelt.source() or smelt.ref() - these have dot but aren't aliases
    // Check if the identifier is "smelt" and followed by source or ref
    if alias.eq_ignore_ascii_case("smelt") {
        let after_dot_lower = after_dot.to_lowercase();
        if after_dot_lower.starts_with("source") || after_dot_lower.starts_with("ref") {
            return None;
        }
    }

    Some(alias.to_string())
}

/// Target of a table alias in FROM clause
#[derive(Debug, Clone)]
pub(crate) enum AliasTarget {
    Source {
        source_name: String,
        table_name: String,
    },
    Model {
        model_name: String,
    },
}

/// Extract alias mappings from a SELECT statement's FROM clause
pub(crate) fn extract_from_aliases(
    select_stmt: &smelt_parser::ast::SelectStmt,
    db: &smelt_db::Database,
) -> std::collections::HashMap<String, AliasTarget> {
    let mut aliases = std::collections::HashMap::new();

    if let Some(from_clause) = select_stmt.from_clause() {
        // Process main table refs in FROM clause
        for table_ref in from_clause.table_refs() {
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segments = path_ref.segments();
                add_path_ref_alias(&table_ref, &segments, &mut aliases);
            }
        }

        // Process JOINed table refs
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if let Some(path_ref) = table_ref.smelt_path_ref() {
                    let segments = path_ref.segments();
                    add_path_ref_alias(&table_ref, &segments, &mut aliases);
                }
            }
        }
    }

    // Note: db parameter reserved for future use (e.g., resolving model schemas)
    let _ = db;

    aliases
}

/// Insert an alias entry for a `smelt.<path>` table ref based on its segments.
fn add_path_ref_alias(
    table_ref: &smelt_parser::ast::TableRef,
    segments: &[String],
    aliases: &mut std::collections::HashMap<String, AliasTarget>,
) {
    match segments.first().map(|s| s.as_str()) {
        Some("models") => {
            if let Some(model_name) = segments.get(1).cloned() {
                let alias_name = table_ref.alias().unwrap_or_else(|| model_name.clone());
                aliases.insert(alias_name, AliasTarget::Model { model_name });
            }
        }
        Some("sources") => {
            if let (Some(source_name), Some(table_name)) =
                (segments.get(1).cloned(), segments.get(2).cloned())
            {
                let alias_name = table_ref.alias().unwrap_or_else(|| table_name.clone());
                aliases.insert(
                    alias_name,
                    AliasTarget::Source {
                        source_name,
                        table_name,
                    },
                );
            }
        }
        _ => {}
    }
}
