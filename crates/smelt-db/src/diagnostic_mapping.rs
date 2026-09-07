//! Pure diagnostic helpers: structural checks and code mappings that take
//! syntax / plain data and return diagnostics, with no database access
//! (`architecture.md` §"Salsa purity rule (analysis)").

use crate::*;

/// Pure structural check: walk all `SMELT_PATH_REF` nodes in `syntax`
/// that carry a `#`-suffix `CTE_SEGMENT` child.  For each such node, check
/// whether any ancestor is a `SMELT_TEST` node.  If NOT, emit a
/// `CteRefOutsideTest` diagnostic anchored at the `#` token.
///
/// This is a Salsa-purity-compliant analysis function (no DB access).  The
/// thin Salsa wrapper in `check_file_diagnostics` calls it after gathering the
/// parse input.
pub(crate) fn cte_ref_outside_test_diagnostics(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltPathRef;
    use smelt_parser::SyntaxKind::{SMELT_PATH_REF, SMELT_TEST};

    let mut diags = Vec::new();
    for node in syntax.descendants().filter(|n| n.kind() == SMELT_PATH_REF) {
        if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
            if let Some(hash_range) = path_ref.hash_range() {
                // Emit unless there is a SMELT_TEST ancestor.
                let inside_test = node.ancestors().any(|a| a.kind() == SMELT_TEST);
                if !inside_test {
                    diags.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "CTE references using `#` are only valid inside a `smelt.test` body; \
                                  remove the `#<cte>` suffix or move this reference inside a `smelt.test` declaration"
                            .to_string(),
                        range: hash_range,
                        code: Some(DiagnosticCode::CteRefOutsideTest),
                        data: None,
                    });
                }
            }
        }
    }
    diags
}

/// Lowercase display of a `Granularity` for diagnostic messages (matches the
/// wire/frontmatter spelling, e.g. `granularity: day`).
pub(crate) fn granularity_lower(g: smelt_core::Granularity) -> &'static str {
    use smelt_core::Granularity as G;
    match g {
        G::Hour => "hour",
        G::Day => "day",
        G::Week => "week",
        G::Month => "month",
        G::Quarter => "quarter",
        G::Year => "year",
    }
}

/// Map a planner-rule diagnostic code onto smelt-db's diagnostic-code
/// catalogue. The 1:1 mapping is the seam the Diagnostic-parity rule relies on
/// (`architecture.md` §"Planner scope").
pub(crate) fn rule_diagnostic_code(code: smelt_logical::RuleDiagnosticCode) -> DiagnosticCode {
    use smelt_logical::RuleDiagnosticCode as R;
    match code {
        R::KeyedRequiresGroupBy => DiagnosticCode::KeyedRequiresGroupBy,
        R::KeyedUnknownCombiner => DiagnosticCode::KeyedUnknownCombiner,
        R::KeyedGroupByContainsPartitionColumn => {
            DiagnosticCode::KeyedGroupByContainsPartitionColumn
        }
        R::KeyedForbidsWindowFunctions => DiagnosticCode::KeyedForbidsWindowFunctions,
        R::KeyedForbidsNondeterministic => DiagnosticCode::KeyedForbidsNondeterministic,
        R::KeyedSnapshotPostureUnsupported => DiagnosticCode::KeyedSnapshotPostureUnsupported,
        R::KeyedSnapshotSourceUnsupportedColumn => {
            DiagnosticCode::KeyedSnapshotSourceUnsupportedColumn
        }
        R::KeyedMultipleDrivingSources => DiagnosticCode::KeyedMultipleDrivingSources,
        R::KeyedSqlNotParseable => DiagnosticCode::KeyedSqlNotParseable,
        R::KeyedOnceWriteUnproven => DiagnosticCode::KeyedOnceWriteUnproven,
        R::KeyedStateColumnCollision => DiagnosticCode::KeyedStateColumnCollision,
        R::PartitionGrainNotSafe => DiagnosticCode::PartitionGrainNotSafe,
        R::EventTimeColumnNotVisibleAtOuterSelect => {
            DiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect
        }
        R::PartitionGrainForbidsMetrics => DiagnosticCode::PartitionGrainForbidsMetrics,
    }
}

/// Remap a parse error message to a more specific diagnostic code when the
/// error originated from the pipe-stage parser.
///
/// The pipe-stage parser emits errors via `Parser::error()`, which stores them
/// as parse errors with the message text. This function inspects the message to
/// promote those errors to their proper diagnostic codes so consumers can
/// distinguish pipe-specific errors from generic syntax errors.
///
/// Mapping rules:
/// - `"pipe operator '<kw>' is not supported — …"` → `PipeOperatorUnsupported`
/// - `"unknown pipe operator '<kw>'"` → `PipeUnknownOperator`
/// - `"malformed '<kw>' pipe stage"` → `PipeStageMalformed`
/// - `"unexpected content after model body"` → `TrailingTopLevelContent`
/// - anything else → `ParseError` (unchanged)
pub(crate) fn remap_pipe_parse_error_code(message: &str) -> DiagnosticCode {
    if message.starts_with("pipe operator '") && message.contains("is not supported") {
        DiagnosticCode::PipeOperatorUnsupported
    } else if message.starts_with("unknown pipe operator '") {
        DiagnosticCode::PipeUnknownOperator
    } else if message.starts_with("malformed '") && message.contains("pipe stage") {
        DiagnosticCode::PipeStageMalformed
    } else if message == "unexpected content after model body" {
        DiagnosticCode::TrailingTopLevelContent
    } else {
        DiagnosticCode::ParseError
    }
}
