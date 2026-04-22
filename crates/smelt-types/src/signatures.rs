//! Function signature indexing for `smelt.define` declarations.
//!
//! This module defines the data shape used by the Salsa-backed function
//! registry (`smelt-db::functions_in_file`, `function_signature`, etc.) and
//! the pure extraction function that produces it from a parsed AST.
//!
//! Pure-function rule (CLAUDE.md): everything here is dependency-free
//! w.r.t. Salsa. Callers in `smelt-db` are responsible for wiring these
//! extractors into tracked queries.
//!
//! Phase 3 scope (per `docs/plans/20260422-smelt-functions-steps-1-2.md`):
//!   - Raw `type_ref_text` only; structured `Expr<T>` parsing is Phase 4.
//!   - Tier classification based on annotation completeness.
//!   - `smelt.extern` is NOT handled here — that arrives in Phase 10.

use smelt_parser::ast::{File as AstFile, Param as AstParam, Range, SmeltDefine};
use smelt_parser::offset_to_position;

/// Description of a single parameter in a `smelt.define`.
///
/// `type_ref_text` is the raw source text of the `TypeRef` node (e.g.
/// `"Expr<Numeric>"`) or `None` when the parameter is unannotated. Phase 4
/// will introduce a parsed `SmeltType` alongside this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The parameter's declared name.
    pub name: String,
    /// Raw text of the declared type, or `None` if unannotated.
    pub type_ref_text: Option<String>,
    /// `true` when the parameter has a default value.
    pub has_default: bool,
}

/// Tier of a function, derived from annotation completeness.
///
/// - `Tier::Three`: every parameter annotated AND return type annotated.
/// - `Tier::Two`: every parameter annotated, return type missing.
/// - `Tier::One`: at least one parameter unannotated (return-type state
///   irrelevant for this tier — unannotated inputs force Tier 1).
///
/// Behavioural differences between tiers (bidirectional checking, param
/// coercion, etc.) land in Steps 5+.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    One,
    Two,
    Three,
}

/// Signature of a single `smelt.define` declaration.
///
/// The `name_range` field points at the `DEFINE_NAME` token in the source
/// file; diagnostic-emitting code (duplicate-definition detection, etc.)
/// uses it as the anchor for error spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSig {
    /// The function's declared name.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<ParamSpec>,
    /// Raw text of the declared return type, or `None` if no `-> Type` clause.
    pub return_type_text: Option<String>,
    /// Tier, derived from annotation completeness at parse time.
    pub tier: Tier,
    /// Line/column range of the function-name identifier, for diagnostics.
    pub name_range: Range,
}

/// Extract a `ParamSpec` from an AST `Param`.
///
/// Pure: does not consult Salsa. Takes only the AST node.
fn extract_param_spec(param: &AstParam) -> ParamSpec {
    ParamSpec {
        name: param.name().unwrap_or_default(),
        type_ref_text: param.type_ref().map(|t| t.text()),
        has_default: param.default_value().is_some(),
    }
}

fn compute_tier(params: &[ParamSpec], return_type_text: Option<&str>) -> Tier {
    let all_params_typed = params.iter().all(|p| p.type_ref_text.is_some());
    if !all_params_typed {
        return Tier::One;
    }
    if return_type_text.is_some() {
        Tier::Three
    } else {
        Tier::Two
    }
}

/// Convert a `SmeltDefine` AST node into a `FunctionSig`.
///
/// Returns `None` if the declaration is missing a name (error recovery
/// produced a fragment without an identifier). File text is required so
/// the `name_range` can be rendered as a line/column range — `Range` is
/// the same type returned by other `smelt-db` queries.
pub fn extract_signature(define: &SmeltDefine, text: &str) -> Option<FunctionSig> {
    let name = define.name()?;
    let name_text_range = define.name_range()?;
    let name_range = Range {
        start: offset_to_position(text, usize::from(name_text_range.start())),
        end: offset_to_position(text, usize::from(name_text_range.end())),
    };

    let params: Vec<ParamSpec> = define
        .param_list()
        .map(|pl| pl.params().map(|p| extract_param_spec(&p)).collect())
        .unwrap_or_default();

    let return_type_text = define.return_type().map(|t| t.text());
    let tier = compute_tier(&params, return_type_text.as_deref());

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        tier,
        name_range,
    })
}

/// Extract all function signatures from a parsed file.
///
/// Pure function: takes an AST + source text and returns a freshly
/// allocated vector of signatures in declaration order. Callers in
/// `smelt-db` wrap this in a Salsa tracked query.
pub fn extract_function_signatures(file: &AstFile, text: &str) -> Vec<FunctionSig> {
    file.defines()
        .filter_map(|d| extract_signature(&d, text))
        .collect()
}

/// Extract the signature for a single named define, if present.
///
/// Returns `None` when no declaration with the given name exists. If
/// multiple declarations share the name (which is a diagnostic), the
/// first in declaration order wins.
pub fn extract_function_signature_by_name(
    file: &AstFile,
    text: &str,
    name: &str,
) -> Option<FunctionSig> {
    file.defines()
        .filter_map(|d| extract_signature(&d, text))
        .find(|sig| sig.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::{parse, File as AstFile};

    fn parse_file(text: &str) -> (AstFile, String) {
        let clean = smelt_parser::strip_frontmatter(text);
        let parse = parse(&clean);
        let ast = AstFile::cast(parse.syntax()).expect("FILE node");
        (ast, clean)
    }

    #[test]
    fn extracts_minimal_signature() {
        let (file, text) = parse_file("smelt.define foo(x) AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params.len(), 1);
        assert_eq!(sigs[0].params[0].name, "x");
        assert!(sigs[0].params[0].type_ref_text.is_none());
        assert!(sigs[0].return_type_text.is_none());
        assert_eq!(sigs[0].tier, Tier::One);
    }

    #[test]
    fn tier_two_when_params_annotated_return_missing() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].tier, Tier::Two);
    }

    #[test]
    fn tier_three_when_fully_annotated() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs[0].tier, Tier::Three);
        let ret = sigs[0].return_type_text.as_deref().unwrap();
        assert!(
            ret.contains("Expr<Integer>"),
            "expected return text to contain Expr<Integer>, got {ret:?}"
        );
    }

    #[test]
    fn default_value_flagged() {
        let (file, text) = parse_file("smelt.define f(x: Expr<Integer> = 0) AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert!(sigs[0].params[0].has_default);
    }

    #[test]
    fn lookup_by_name() {
        let (file, text) = parse_file("smelt.define a(x) AS (x)\nsmelt.define b(y) AS (y)\n");
        let sig = extract_function_signature_by_name(&file, &text, "b").unwrap();
        assert_eq!(sig.name, "b");
        assert!(extract_function_signature_by_name(&file, &text, "nope").is_none());
    }
}
