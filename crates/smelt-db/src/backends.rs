//! Phase 11 (smelt-functions) — per-function backend inference.
//!
//! A function's *backend set* is the set of backends on which the
//! function's body can be executed. It may be declared in the per-decl
//! frontmatter (`backends: [...]`) or — for externs — implied by the
//! backend-namespace sugar (`smelt.extern duckdb.foo(...)`). Absent an
//! explicit declaration, the set is *inferred* by walking the body:
//!
//!   * A bare SQL function call whose name is `<backend>.<foo>` narrows
//!     the caller's set to `[<backend>]`.
//!   * A `smelt.fn.<name>(...)` call intersects the callee's backend
//!     set into the caller's running set.
//!   * Everything else leaves the set unchanged.
//!
//! The caller's final set is then checked against the declared set
//! under the §16 #23 **narrow-only** rule: `declared ⊆ inferred`. A
//! declared set broader than the inferred set emits
//! `DiagnosticCode::BackendsWideningNotAllowed`.
//!
//! Pure-function rule: this module is free of Salsa. Callers in
//! `smelt-db/src/lib.rs` wire it into a tracked query.

use smelt_parser::ast::{Expr, FunctionCall, SmeltFnCall};
use smelt_parser::syntax_kind::SyntaxKind;
use smelt_types::signatures::{BackendSet, FunctionSig, SigOrigin};

/// Walk a body expression and infer which backends it can target.
///
/// `sig_lookup` resolves a `smelt.fn.<name>` call site to the callee's
/// [`FunctionSig`]. Callees whose own backend set is inferrable via
/// [`effective_backends`] contribute their set to the intersection; the
/// caller (`function_backends_query`) resolves circularity by looking
/// up *declared* sets on upstream signatures, not walking into nested
/// bodies.
pub fn infer_body_backends<F>(body: &Expr, sig_lookup: &F) -> BackendSet
where
    F: Fn(&str) -> Option<FunctionSig>,
{
    let mut set = BackendSet::All;
    walk_expr_for_backends(body.syntax(), &mut set, sig_lookup);
    set
}

fn walk_expr_for_backends<F>(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    set: &mut BackendSet,
    sig_lookup: &F,
) where
    F: Fn(&str) -> Option<FunctionSig>,
{
    // Recurse children, checking each for a narrowing contribution.
    for child in node.descendants() {
        match child.kind() {
            SyntaxKind::FUNCTION_CALL => {
                if let Some(fc) = FunctionCall::cast(child.clone()) {
                    if let Some(namespace) = fc.namespace() {
                        // `namespace.foo(...)`. If namespace is `smelt`
                        // this is a ref/source/metric call — skip. If
                        // namespace looks like a backend name
                        // (lowercase identifier), narrow.
                        let ns_lower = namespace.to_ascii_lowercase();
                        if ns_lower == "smelt" {
                            continue;
                        }
                        if is_plausible_backend_name(&ns_lower) {
                            let backend = BackendSet::from_names([ns_lower]);
                            *set = set.intersect(&backend);
                        }
                    }
                }
            }
            SyntaxKind::SMELT_FN_CALL => {
                if let Some(call) = SmeltFnCall::cast(child.clone()) {
                    let segments = call.call_path().map(|p| p.segments()).unwrap_or_default();
                    if let Some(name) = segments.last() {
                        if let Some(callee_sig) = sig_lookup(name) {
                            let callee = effective_backends(&callee_sig);
                            *set = set.intersect(&callee);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The "effective" backend set for a signature — its declared set when
/// present, else `All`. Used as the callee contribution when intersecting
/// into a caller's inferred set. Deliberately does not recurse into the
/// callee's body: that would loop on circular references and the
/// callee's own `function_backends` query handles inference for us when
/// it runs.
pub fn effective_backends(sig: &FunctionSig) -> BackendSet {
    sig.declared_backends.clone().unwrap_or(BackendSet::All)
}

/// Heuristic: does this namespace look like a backend identifier?
///
/// We recognise the v1 targets: `duckdb`, `spark`, `databricks`.
/// Extending this list is the only place new backends need to be
/// registered for the Phase 11 inference rule.
pub fn is_plausible_backend_name(name: &str) -> bool {
    matches!(name, "duckdb" | "spark" | "databricks")
}

/// Apply the narrow-only rule: `declared ⊆ inferred` ⇒ `declared`,
/// otherwise the declared set is a widening and we return `Err` carrying
/// a human-readable message that the caller surfaces as
/// [`crate::DiagnosticCode::BackendsWideningNotAllowed`].
///
/// Defaults:
///   * No declared set → return the inferred set.
///   * No body to walk → declared set wins (externs).
pub fn apply_narrow_rule(
    declared: Option<&BackendSet>,
    inferred: &BackendSet,
) -> Result<BackendSet, String> {
    let Some(declared) = declared else {
        return Ok(inferred.clone());
    };
    if declared.is_subset_of(inferred) {
        Ok(declared.clone())
    } else {
        Err(format!(
            "declared backends {} widen the body's inferred backends {}",
            declared.render(),
            inferred.render()
        ))
    }
}

/// Convenience: compute the final backend set for `sig` given its
/// (optionally walked) inferred set. For `SigOrigin::Extern` with no
/// body we skip inference and use the declared set verbatim (or `All`
/// when none is declared).
pub fn resolve_backends(
    sig: &FunctionSig,
    inferred: Option<BackendSet>,
) -> Result<BackendSet, String> {
    match sig.origin {
        SigOrigin::Extern => Ok(sig.declared_backends.clone().unwrap_or(BackendSet::All)),
        SigOrigin::Define => {
            let inferred = inferred.unwrap_or(BackendSet::All);
            apply_narrow_rule(sig.declared_backends.as_ref(), &inferred)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_rule_defaults_to_inferred() {
        let result = apply_narrow_rule(None, &BackendSet::All).unwrap();
        assert_eq!(result, BackendSet::All);
    }

    #[test]
    fn narrow_rule_accepts_subset() {
        let declared = BackendSet::from_names(["duckdb"]);
        let inferred = BackendSet::from_names(["duckdb", "spark"]);
        let result = apply_narrow_rule(Some(&declared), &inferred).unwrap();
        assert_eq!(result, declared);
    }

    #[test]
    fn narrow_rule_rejects_widening() {
        let declared = BackendSet::from_names(["duckdb", "spark"]);
        let inferred = BackendSet::from_names(["duckdb"]);
        let err = apply_narrow_rule(Some(&declared), &inferred).unwrap_err();
        assert!(err.contains("widen"), "{err}");
    }

    #[test]
    fn narrow_rule_declared_all_over_inferred_only_is_widening() {
        let declared = BackendSet::All;
        let inferred = BackendSet::from_names(["duckdb"]);
        let err = apply_narrow_rule(Some(&declared), &inferred).unwrap_err();
        assert!(err.contains("widen"), "{err}");
    }

    #[test]
    fn plausible_backend_names() {
        assert!(is_plausible_backend_name("duckdb"));
        assert!(is_plausible_backend_name("spark"));
        assert!(!is_plausible_backend_name("smelt"));
        assert!(!is_plausible_backend_name("lower"));
    }
}
