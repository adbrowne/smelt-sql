/// Typed AST wrappers over Rowan CST
use crate::syntax_kind::SyntaxNode;
use crate::SyntaxKind::*;

/// Trims a node's raw source text for verbatim printing, but keeps a
/// trailing line comment's terminating newline intact.
///
/// A `--` line comment only ends at `\n`/EOF (`lexer::consume_comment`); a
/// `/* */` block comment is self-terminating. Blindly trimming trailing
/// whitespace strips that newline whenever the span's last token is a line
/// comment, so whatever the printer concatenates next (a keyword, a
/// separator) is silently swallowed into the comment on re-parse — see the
/// `round_trip` fuzz regression this guards (`SELECT x --c\nHAVING ...`
/// printed without the newline merges `HAVING ...` into the comment).
pub(crate) fn trim_source_text(node: &SyntaxNode) -> String {
    let trimmed = node.text().to_string().trim().to_string();
    let ends_in_open_line_comment = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() != WHITESPACE)
        .last()
        .is_some_and(|t| t.kind() == COMMENT && t.text().starts_with("--"));
    if ends_in_open_line_comment {
        format!("{trimmed}\n")
    } else {
        trimmed
    }
}

/// Root file node
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File(SyntaxNode);

impl File {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FILE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }

    /// The top-level `PipeQuery` node, if the file body is a FROM-first pipe query.
    pub fn pipe_query(&self) -> Option<PipeQuery> {
        self.0.children().find_map(PipeQuery::cast)
    }

    /// The top-level `VALUES_CLAUSE` node, if the file body is a bare
    /// `VALUES (…), (…)` statement (no `SELECT` wrapper).
    pub fn values_clause(&self) -> Option<ValuesClause> {
        self.0.children().find_map(ValuesClause::cast)
    }

    /// Whether the file has a valid top-level query body (SELECT_STMT or PIPE_QUERY).
    pub fn has_query_body(&self) -> bool {
        self.select_stmt().is_some() || self.pipe_query().is_some()
    }

    /// Iterate over top-level `smelt.define` declarations in this file.
    pub fn defines(&self) -> impl Iterator<Item = SmeltDefine> + '_ {
        self.0.children().filter_map(SmeltDefine::cast)
    }

    /// Iterate over top-level `smelt.extern` declarations in this file.
    pub fn externs(&self) -> impl Iterator<Item = SmeltExtern> + '_ {
        self.0.children().filter_map(SmeltExtern::cast)
    }

    /// Iterate over top-level `smelt.test` declarations in this file.
    pub fn tests(&self) -> impl Iterator<Item = SmeltTest> + '_ {
        self.0.children().filter_map(SmeltTest::cast)
    }

    /// Iterate over top-level `smelt.check` declarations in this file.
    pub fn checks(&self) -> impl Iterator<Item = SmeltCheck> + '_ {
        self.0.children().filter_map(SmeltCheck::cast)
    }
}

mod clauses;
mod declarations;
mod expr;
mod meta;
mod select;
#[cfg(test)]
mod tests;

pub use clauses::*;
pub use declarations::*;
pub use expr::*;
pub use meta::*;
pub use select::*;
