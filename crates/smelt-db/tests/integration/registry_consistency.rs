//! Function-registry single-ownership gates (Phase 8).
//!
//! These tests enforce the "Function-registry single ownership" invariant
//! (docs/specs/architecture.md): a built-in SQL function's recognition,
//! classification (aggregate/window/scalar), and — for migrated functions —
//! typing all derive from `BuiltinRegistry`. A name that lives in one list
//! but not the others must fail here rather than degrade silently.

use smelt_db::type_inference::{infer_select_column_types, registry_migrated_names, TypeContext};
use smelt_parser::ast::File;
use smelt_types::signatures::{BuiltinRegistry, ExprKind, SyntaxForm};
use smelt_types::{DataType, FunctionCategory, SqlFunction, TypedColumn};

/// Expected `ExprKind` classification for a `SqlFunction`, derived from its
/// category. This is the oracle for registry-vs-enum classification parity.
fn expected_kind(f: SqlFunction) -> ExprKind {
    match f.category() {
        FunctionCategory::Aggregate => ExprKind::Agg,
        FunctionCategory::WindowRanking
        | FunctionCategory::WindowDistribution
        | FunctionCategory::WindowNavigation => ExprKind::Window,
        _ => ExprKind::Scalar,
    }
}

#[test]
fn every_recognized_function_is_registry_backed() {
    let mut missing_from_registry: Vec<String> = Vec::new();
    let mut kind_mismatch: Vec<String> = Vec::new();

    // Direction 1: every canonical name `SqlFunction` recognises resolves in
    // the registry, and its classification agrees.
    for f in SqlFunction::all() {
        match BuiltinRegistry::resolve(f.name()) {
            None => missing_from_registry.push(f.name().to_string()),
            Some(sig) => {
                let want = expected_kind(f);
                if sig.kind != want {
                    kind_mismatch.push(format!(
                        "{}: enum says {:?}, registry says {:?}",
                        f.name(),
                        want,
                        sig.kind
                    ));
                }
            }
        }
    }

    // Direction 2: every non-operator registry entry is a recognised function.
    let mut missing_from_enum: Vec<String> = Vec::new();
    for name in BuiltinRegistry::names() {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        // Dedicated-syntax entries (operators, CAST, interval add/sub, table
        // functions) are exempt from the callable-function surface. The
        // exemption is registry data, not a hand-written list: a new operator
        // entry is exempt automatically, and an entry that stops being one
        // re-enters the gate.
        if sig.syntax_form != SyntaxForm::Call {
            continue;
        }
        if SqlFunction::from_name(name).is_none() {
            missing_from_enum.push(name.to_string());
        }
    }

    missing_from_registry.sort();
    missing_from_enum.sort();
    kind_mismatch.sort();

    assert!(
        missing_from_registry.is_empty()
            && missing_from_enum.is_empty()
            && kind_mismatch.is_empty(),
        "function-registry drift detected:\n\
         - recognised by SqlFunction but MISSING from BuiltinRegistry ({}): {:?}\n\
         - in BuiltinRegistry but NOT a recognised function ({}): {:?}\n\
         - classification (kind) mismatches ({}): {:?}",
        missing_from_registry.len(),
        missing_from_registry,
        missing_from_enum.len(),
        missing_from_enum,
        kind_mismatch.len(),
        kind_mismatch,
    );
}

#[test]
fn every_alias_is_registry_backed() {
    // Function-registry single ownership (architecture.md §Constraints #14):
    // dialect aliases (NVL, GET_JSON_OBJECT, ...) must be recognized,
    // classified, and typed entirely through `BuiltinRegistry` — never a
    // second alias-only mapping living outside it. This is the direction
    // `every_recognized_function_is_registry_backed` cannot see: that test
    // only walks canonical `SqlFunction` names, so an alias resolved solely
    // by a hand-written match in `SqlFunction::from_name` (with no
    // registry-side `aliases` entry) would pass it silently.
    let mut alias_missing_from_sqlfunction: Vec<String> = Vec::new();
    let mut alias_kind_mismatch: Vec<String> = Vec::new();
    let mut alias_resolves_to_wrong_canonical: Vec<String> = Vec::new();

    for (alias, canonical) in BuiltinRegistry::aliases() {
        let Some(canonical_sig) = BuiltinRegistry::resolve(canonical) else {
            // Structural invariant of the registry itself; not expected to
            // fire, but names it rather than panicking obscurely.
            alias_missing_from_sqlfunction.push(format!(
                "{alias}: registry alias points at unregistered canonical name {canonical}"
            ));
            continue;
        };

        match SqlFunction::from_name(alias) {
            None => alias_missing_from_sqlfunction.push(alias.to_string()),
            Some(f) => {
                if f.name() != canonical {
                    alias_resolves_to_wrong_canonical.push(format!(
                        "{alias}: SqlFunction::from_name resolves to {}, registry says {canonical}",
                        f.name()
                    ));
                }
                let want = expected_kind(f);
                if canonical_sig.kind != want {
                    alias_kind_mismatch.push(format!(
                        "{alias} (-> {canonical}): enum says {:?}, registry says {:?}",
                        want, canonical_sig.kind
                    ));
                }
            }
        }
    }

    alias_missing_from_sqlfunction.sort();
    alias_kind_mismatch.sort();
    alias_resolves_to_wrong_canonical.sort();

    assert!(
        alias_missing_from_sqlfunction.is_empty()
            && alias_kind_mismatch.is_empty()
            && alias_resolves_to_wrong_canonical.is_empty(),
        "alias-registry drift detected:\n\
         - registry alias NOT recognized by SqlFunction::from_name ({}): {:?}\n\
         - alias resolves to the wrong canonical function ({}): {:?}\n\
         - classification (kind) mismatches ({}): {:?}",
        alias_missing_from_sqlfunction.len(),
        alias_missing_from_sqlfunction,
        alias_resolves_to_wrong_canonical.len(),
        alias_resolves_to_wrong_canonical,
        alias_kind_mismatch.len(),
        alias_kind_mismatch,
    );
}

#[test]
fn legacy_match_ratchet() {
    // The number of recognised functions whose primary typing path is still
    // the hand-written `match` in `function_call.rs` (i.e. NOT registry-first
    // via `try_registry_inference`). Shrink-only: the checked-in baseline is
    // an upper bound that migration drives down. Raising it requires editing
    // `.claude/registry-migration-baseline.txt` (reviewer-visible).
    let migrated: std::collections::HashSet<&str> =
        registry_migrated_names().iter().copied().collect();
    let legacy_typed: Vec<&str> = SqlFunction::all()
        .map(|f| f.name())
        .filter(|n| !migrated.contains(n))
        .collect();
    let count = legacy_typed.len();

    let baseline: usize = include_str!("../../../../.claude/registry-migration-baseline.txt")
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .and_then(|l| l.trim().parse().ok())
        .expect("registry-migration-baseline.txt must contain a count");

    assert!(
        count <= baseline,
        "legacy-match ratchet regressed: {count} functions still typed by the \
         hand-written match (baseline {baseline}). Migrate more into \
         BuiltinRegistry or, with reviewer sign-off, raise the baseline.\n\
         Still-legacy functions: {legacy_typed:?}"
    );
    if count < baseline {
        eprintln!(
            "note: registry-migration ratchet can shrink to {count} \
             (baseline {baseline}); update .claude/registry-migration-baseline.txt"
        );
    }
}

#[test]
fn unrecognized_function_still_warns() {
    // The consolidation must not make an unknown name panic or pass silently:
    // it still infers `Unknown(Dynamic)` (which the lib-level checker turns
    // into an `UnrecognizedFunction` warning through the known-unknowns path).
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "upstream",
        "x",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        },
    );
    let sql = "SELECT DEFINITELY_NOT_A_FUNCTION(x) AS r FROM upstream";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    let types = infer_select_column_types(&select, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::unknown_dynamic());
}

#[test]
fn to_seconds_and_md5_registered() {
    let md5 = BuiltinRegistry::resolve("MD5").expect("MD5 present");
    assert_eq!(md5.kind, ExprKind::Scalar);
    let to_seconds = BuiltinRegistry::resolve("TO_SECONDS").expect("TO_SECONDS present");
    assert_eq!(to_seconds.kind, ExprKind::Scalar);

    assert_eq!(SqlFunction::from_name("MD5"), Some(SqlFunction::Md5));
    assert_eq!(
        SqlFunction::from_name("TO_SECONDS"),
        Some(SqlFunction::ToSeconds)
    );
}
