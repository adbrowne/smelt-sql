use super::*;
use crate::{DataType, DialectId};
use std::collections::HashMap;

/// One type parameter in a signature generic list (§16 #14).
///
/// Generics live on built-in signatures only; `smelt.define` stays
/// monomorphic in v1. The `constraint` narrows what concrete types may
/// bind: `TypeConstraint::Numeric` triggers the promotion-chain branch of
/// unification, everything else requires exact equality across positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParam {
    /// The variable's name as written in the signature (e.g. `"T"`).
    pub name: String,
    /// Narrowing constraint; `TypeConstraint::Any` means "no constraint".
    pub constraint: TypeConstraint,
}

/// A single parameter slot in a signature (§16 #14 + #15).
///
/// `Concrete(c)` demands a fixed `TypeConstraint` (usually
/// `Concrete(DataType::…)`). `Var(name)` refers to one of the signature's
/// [`TypeParam`]s by name. `Variadic(inner)` is only legal in the trailing
/// slot — enforced by [`Signature::new`]; see
/// [`SignatureBuildError::NonTrailingVariadic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigParam {
    /// A concrete constraint the argument must satisfy (e.g.
    /// `Concrete(Concrete(Text))` for a scalar Text parameter).
    Concrete(TypeConstraint),
    /// A reference to a generic type parameter by name.
    Var(String),
    /// Trailing zero-or-more parameter of the inner shape. The inner
    /// `SigParam` is itself a `Concrete(...)` or `Var(...)` (never a
    /// nested `Variadic`).
    Variadic(Box<SigParam>),
}

/// A signature's return type — same vocabulary as [`SigParam`] without
/// [`SigParam::Variadic`] (SQL functions return a single column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    /// A fixed `TypeConstraint` for the return type.
    Concrete(TypeConstraint),
    /// The return type is bound to a generic type variable of this name.
    Var(String),
}

/// Error produced at registry-construction time when a [`Signature`] is
/// shaped in a way the inference routine can't handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureBuildError {
    /// A non-trailing [`SigParam::Variadic`] appeared in the parameter list.
    NonTrailingVariadic { name: String, position: usize },
    /// A variadic contained another variadic — not expressible in v1.
    NestedVariadic { name: String },
    /// A [`TypeExpr::Var`] or [`SigParam::Var`] referenced a name that
    /// isn't declared in `type_params`.
    UndeclaredTypeVar { name: String, var_name: String },
}

impl std::fmt::Display for SignatureBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureBuildError::NonTrailingVariadic { name, position } => write!(
                f,
                "signature `{name}` declares a variadic at position {position} but variadics must be the final parameter"
            ),
            SignatureBuildError::NestedVariadic { name } => write!(
                f,
                "signature `{name}` nests a variadic inside a variadic"
            ),
            SignatureBuildError::UndeclaredTypeVar { name, var_name } => write!(
                f,
                "signature `{name}` references undeclared type variable `{var_name}`"
            ),
        }
    }
}

impl std::error::Error for SignatureBuildError {}

/// Polymorphic signature of a SQL built-in in the canonical registry.
///
/// Phase 7 was monomorphic (`params: Vec<TypeConstraint>`, same for return).
/// Phase 8 extends to full generics + trailing variadic per §16 #14/#15:
///
/// * [`Signature::type_params`] — generic list (empty for monomorphic
///   entries).
/// * [`Signature::params`] — positional [`SigParam`]s; at most one trailing
///   [`SigParam::Variadic`].
/// * [`Signature::return_type`] — concrete [`TypeConstraint`] or a reference
///   to one of the type parameters.
///
/// Construct entries via [`Signature::new`] so the well-formedness checks
/// run once at registry initialisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Canonical (upper-cased) function name.
    pub name: String,
    /// Declared generic type parameters, in declaration order. Empty when
    /// the signature is monomorphic.
    pub type_params: Vec<TypeParam>,
    /// Positional parameters, in declaration order. At most one
    /// [`SigParam::Variadic`], which must be last.
    pub params: Vec<SigParam>,
    /// Return type.
    pub return_type: TypeExpr,
    /// Canonical return type per §16 #9's widening chain. `Some(dt)` when
    /// the registry declares a canonical type that differs from what a
    /// given backend natively returns (e.g. `SUM(INTEGER)` canonical is
    /// `BigInt` even though DuckDB returns `HUGEINT`). `None` means
    /// "derive from [`Signature::return_type`] at call time" — the common
    /// case for monomorphic signatures.
    ///
    /// emitter when `needs_cast_for` returns `true`.
    pub canonical_return: Option<DataType>,
    /// Per-backend native return-type overrides. Keyed on [`DialectId`].
    /// An entry here means "this backend natively returns a type that differs
    /// from [`Self::canonical_return`]" — Step 7+ emits a CAST at emit-time
    /// to preserve the canonical type.
    ///
    /// Phase 12: recording-only. `HashMap::default()` on entries that
    /// need no override (the canonical type is also the native type on
    /// every backend).
    pub engine_native: HashMap<DialectId, DataType>,
    /// Default [`ExprKind`] for a call to this signature when no `OVER (…)`
    /// clause is present (Phase 14, §16 #24).
    ///
    /// Aggregates (`SUM`, `AVG`, `MIN`, `MAX`, `COUNT`) seed `Agg`. Window
    /// functions that are *only* meaningful with an `OVER (…)` clause
    /// (`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`) seed `Window`.
    /// Everything else seeds `Scalar`.
    ///
    /// The type checker overrides this at the call site: an aggregate call
    /// with an attached `OVER (…)` clause is treated as `Window` regardless
    /// of the seeded kind (the canonical SQL dual-mode behaviour).
    pub kind: ExprKind,
    /// Dialect-specific alternate names that resolve to this same entry
    /// (e.g. `IFNULL`'s `aliases` includes `NVL`; `JSON_OBJECT`'s includes
    /// `JSON_BUILD_OBJECT`). Empty for entries with no dialect alias.
    ///
    /// This is the single authoritative row per canonical function: an
    /// alias is a name, not a duplicated signature. [`BuiltinRegistry::resolve`]
    /// and [`BuiltinRegistry::canonical_name`] check this table (via the
    /// derived alias index) after a direct canonical-name match fails.
    pub aliases: &'static [&'static str],
    /// Nullability-propagation policy for this signature's result, layered
    /// on top of the generic "always nullable" default a registry-resolved
    /// call otherwise gets. See [`NullabilityPropagation`].
    pub nullability: NullabilityPropagation,
    /// How this built-in is spelled at a call site (Phase 2, #171).
    ///
    /// Defaults to [`SyntaxForm::Call`]. Non-`Call` entries are operators,
    /// predicates, table functions, and dedicated-syntax forms that have
    /// registry entries for hover/completion but are not part of the
    /// callable-function surface.
    pub syntax_form: SyntaxForm,
    /// Per-`(dialect, position)` emission verdicts for this entry.
    ///
    /// `&[]` means every dialect and position is `Native`. Use
    /// [`Self::with_emission`] to populate and [`Self::emission_at`] to
    /// query. Populated only for entries whose printer treatment differs
    /// from plain name-pass-through on at least one `(dialect, position)`
    /// pair. A verdict stated with [`Position::Any`] applies to every
    /// position that has no more specific entry of its own.
    pub emission: &'static [(DialectId, Position, Emission)],
}

/// Nullability-propagation policy for a registry-resolved call's result,
/// consulted by the type-inference layer (`smelt-db`'s
/// `registry_result_nullable`) alongside the generic per-function default.
///
/// Registry data, not a hand-matched special case — per the function-registry
/// single-ownership rule (architecture.md §Constraints #14), a function's
/// nullability behaviour is declared once here rather than duplicated as a
/// name-matched arm in `smelt-db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NullabilityPropagation {
    /// No propagation tag — the result is nullable regardless of argument
    /// nullability or query shape. The default for every signature that
    /// doesn't opt into a more precise rule.
    #[default]
    None,
    /// Extremal-aggregate rule (`MIN`/`MAX`): a NOT NULL argument produces a
    /// NOT NULL result, but **only** under a `GROUP BY` — every group is
    /// guaranteed at least one row, so the fold can't collapse to NULL. An
    /// aggregate over possibly-empty ungrouped input stays nullable; that's
    /// a soundness boundary (an empty `SELECT MIN(x) FROM t` yields one NULL
    /// row), not a limitation the tag can lift.
    GroupedExtremal,
}

impl Signature {
    /// Build a validated signature. Panics with a readable message when
    /// the signature violates a structural invariant — registry seeds are
    /// static data, so any error here is a programmer bug caught at first
    /// call (via [`std::sync::LazyLock`]).
    pub fn new(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<SigParam>,
        return_type: TypeExpr,
    ) -> Self {
        Self::try_new(name, type_params, params, return_type).expect("malformed built-in signature")
    }

    /// Non-panicking variant for tests / future `smelt.extern` use.
    pub fn try_new(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<SigParam>,
        return_type: TypeExpr,
    ) -> Result<Self, SignatureBuildError> {
        // Variadic must be trailing only.
        for (idx, p) in params.iter().enumerate() {
            if let SigParam::Variadic(inner) = p {
                if idx != params.len() - 1 {
                    return Err(SignatureBuildError::NonTrailingVariadic {
                        name: name.to_string(),
                        position: idx + 1,
                    });
                }
                if matches!(**inner, SigParam::Variadic(_)) {
                    return Err(SignatureBuildError::NestedVariadic {
                        name: name.to_string(),
                    });
                }
            }
        }
        // Every type-var reference must be declared.
        let declared: std::collections::HashSet<&str> =
            type_params.iter().map(|tp| tp.name.as_str()).collect();
        for p in &params {
            check_param_vars(name, p, &declared)?;
        }
        if let TypeExpr::Var(var_name) = &return_type {
            if !declared.contains(var_name.as_str()) {
                return Err(SignatureBuildError::UndeclaredTypeVar {
                    name: name.to_string(),
                    var_name: var_name.clone(),
                });
            }
        }
        Ok(Self {
            name: name.to_string(),
            type_params,
            params,
            return_type,
            canonical_return: None,
            engine_native: HashMap::new(),
            kind: ExprKind::Scalar,
            aliases: &[],
            nullability: NullabilityPropagation::None,
            syntax_form: SyntaxForm::Call,
            emission: &[],
        })
    }

    /// Attach an [`ExprKind`] to this signature (Phase 14).
    ///
    /// Builder-style — used by the registry seed to mark aggregates and
    /// window functions. Defaults to [`ExprKind::Scalar`] if never called.
    pub fn with_kind(mut self, kind: ExprKind) -> Self {
        self.kind = kind;
        self
    }

    /// Attach dialect-specific alias names to this signature (Function-registry
    /// single ownership, architecture.md §Constraints #14).
    ///
    /// Builder-style — used by the registry seed to register alternate
    /// spellings (e.g. `NVL` for `IFNULL`) that resolve to this same entry
    /// without duplicating its signature.
    pub fn with_aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// Attach a canonical return type to this signature (Phase 12).
    ///
    /// Builder-style — intended for static registry initialisation. The
    /// canonical type is compared to each `engine_native` entry to decide
    /// whether a CAST should be emitted on that backend.
    pub fn with_canonical_return(mut self, dt: DataType) -> Self {
        self.canonical_return = Some(dt);
        self
    }

    /// Declare a per-backend native return-type override (Phase 12).
    ///
    /// Calling this multiple times with different dialects builds up the full
    /// override table.
    pub fn with_engine_native(mut self, dialect: DialectId, dt: DataType) -> Self {
        self.engine_native.insert(dialect, dt);
        self
    }

    /// Attach a [`NullabilityPropagation`] tag to this signature.
    ///
    /// Builder-style — used by the registry seed to opt a function into a
    /// precise nullability rule (e.g. `MIN`/`MAX`'s grouped-extremal rule)
    /// instead of the generic "always nullable" default.
    pub fn with_nullability(mut self, rule: NullabilityPropagation) -> Self {
        self.nullability = rule;
        self
    }

    /// Set the [`SyntaxForm`] for this signature (Phase 2, #171).
    ///
    /// Builder-style — used by the registry seed to mark operators, predicates,
    /// table functions, and dedicated-syntax forms. Defaults to
    /// [`SyntaxForm::Call`] if never called.
    pub fn with_syntax_form(mut self, form: SyntaxForm) -> Self {
        self.syntax_form = form;
        self
    }

    /// Attach a per-`(dialect, position)` emission table to this signature.
    ///
    /// Builder-style — used by the registry seed to declare how each
    /// `(dialect, position)` pair must spell or rewrite this entry. A pair
    /// with no entry, direct or via [`Position::Any`], is implicitly
    /// [`Emission::Native`].
    pub fn with_emission(mut self, table: &'static [(DialectId, Position, Emission)]) -> Self {
        self.emission = table;
        self
    }

    /// The emission verdict for `dialect` at `position`.
    ///
    /// Lookup consults the exact `(dialect, position)` pair first, then
    /// `(dialect, Position::Any)`, and stops — there is deliberately no
    /// fallback *between* positions, and in particular none between the two
    /// window positions (`WholePartitionWindow` and `Window`), because a
    /// caller that decided a call's actual position is the only one
    /// entitled to fall back to `Any`; a lookup that fell from one concrete
    /// position to another would answer a different question than the one
    /// asked. A pair with no entry at all is `Native`.
    pub fn emission_at(&self, dialect: DialectId, position: Position) -> Emission {
        self.emission
            .iter()
            .find(|(d, p, _)| *d == dialect && *p == position)
            .or_else(|| {
                self.emission
                    .iter()
                    .find(|(d, p, _)| *d == dialect && *p == Position::Any)
            })
            .map(|(_, _, e)| *e)
            .unwrap_or(Emission::Native)
    }

    /// The settled emission verdict for `dialect` at `position`, given the
    /// call's own [`CallFacts`] (`docs/specs/multi_backend.md`
    /// §"Operand-conditional verdicts").
    ///
    /// Delegates to [`Self::emission_at`] and, for a `Conditional` entry,
    /// resolves the first arm whose guard `facts` satisfies — registry
    /// validation ([`validate_conditional`]) guarantees a trailing
    /// `otherwise` arm always matches, so this never falls through. Every
    /// other `Emission` variant maps to its [`SettledEmission`] counterpart
    /// unchanged. This is the only place a `Conditional` entry is resolved;
    /// the printer never calls [`Self::emission_at`] for this purpose and
    /// never sees an unresolved `Conditional`.
    pub fn settle_at(
        &self,
        dialect: DialectId,
        position: Position,
        facts: &CallFacts,
    ) -> SettledEmission {
        match self.emission_at(dialect, position) {
            Emission::Native => SettledEmission::Native,
            Emission::Rename(name) => SettledEmission::Rename(name),
            Emission::Template(template) => SettledEmission::Template(template),
            Emission::Rewrite(id) => SettledEmission::Rewrite(id),
            Emission::Restructure(id) => SettledEmission::Restructure(id),
            Emission::Unsupported { reason } => SettledEmission::Unsupported { reason },
            Emission::Conditional(arms) => arms
                .iter()
                .find(|arm| arm.matches(facts))
                .map(|arm| arm.verdict)
                .expect(
                    "registry validation guarantees a Conditional entry ends in an otherwise \
                     arm that always matches",
                ),
        }
    }

    /// Does the signature require a CAST back to the canonical return
    /// type when executed on `dialect`? (§16 #9 / Phase 12, recording
    /// only — Step 7+ consumes this.)
    ///
    /// Returns `false` when no canonical type is declared (the common
    /// case — the signature's own [`Self::return_type`] is already
    /// canonical) or when the dialect's native type equals the canonical
    /// type. Returns `true` when the dialect is listed in
    /// [`Self::engine_native`] with a type that differs from
    /// [`Self::canonical_return`].
    pub fn needs_cast_for(&self, dialect: DialectId) -> bool {
        let Some(canonical) = &self.canonical_return else {
            return false;
        };
        match self.engine_native.get(&dialect) {
            Some(native) => native != canonical,
            None => false,
        }
    }

    /// Look up a declared type parameter by name.
    pub fn type_param(&self, var_name: &str) -> Option<&TypeParam> {
        self.type_params.iter().find(|tp| tp.name == var_name)
    }
}

fn check_param_vars(
    sig_name: &str,
    p: &SigParam,
    declared: &std::collections::HashSet<&str>,
) -> Result<(), SignatureBuildError> {
    match p {
        SigParam::Concrete(_) => Ok(()),
        SigParam::Var(v) => {
            if declared.contains(v.as_str()) {
                Ok(())
            } else {
                Err(SignatureBuildError::UndeclaredTypeVar {
                    name: sig_name.to_string(),
                    var_name: v.clone(),
                })
            }
        }
        SigParam::Variadic(inner) => check_param_vars(sig_name, inner, declared),
    }
}
