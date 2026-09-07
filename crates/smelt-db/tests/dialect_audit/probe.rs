//! Probes derived from the registry, not authored against it.
//!
//! For each entry: the parameters' `TypeConstraint`s select fixture columns,
//! `SyntaxForm` decides the spelling, and `ExprKind` decides the query shape.
//! An entry the rules cannot spell is reported by name — never silently
//! dropped, which is how a registry-blind harness quietly stops covering
//! things.

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_dialect::{
    plan_restructure, print, settle_emissions, BackendCapabilities, PrintContext, SqlDialect,
};
use smelt_parser::ast::File;
pub use smelt_types::signatures::Position;
use smelt_types::{
    BuiltinRegistry, CallFacts, ConditionalArm, DataType, DialectId, Emission, ExprKind,
    OperandClass, SigParam, Signature, SyntaxForm, TypeConstraint, TypedColumn,
};
use std::collections::{HashMap, HashSet};

use crate::fixture;
use crate::overrides;

/// The alias suffix for a probe at `position`.
///
/// `Position::Any` is a lookup wildcard, never an actual probe position — a
/// probe never carries it, so this is unreachable in practice.
fn suffix(position: Position) -> &'static str {
    match position {
        Position::Any => unreachable!("Position::Any is a lookup wildcard, never probed"),
        Position::Scalar => "scalar",
        Position::Aggregate => "agg",
        Position::WholePartitionWindow => "wpwin",
        Position::Window => "win",
    }
}

#[derive(Debug, Clone)]
pub struct Probe {
    /// Canonical registry name.
    pub name: &'static str,
    pub position: Position,
    /// The expression in smelt SQL, before dialect lowering.
    pub expr: String,
    /// Deterministic alias: `p_<sanitised name>_<position>`, with an
    /// `_a{k}` suffix when `arm` is set.
    pub alias: String,
    /// When set, this probe runs the schema leg only, for the recorded reason.
    pub schema_only: Option<&'static str>,
    /// The `Emission::Conditional` arm index this probe was derived to
    /// reach, or `None` for an ordinary (non-conditional) probe.
    pub arm: Option<usize>,
    /// The call-site facts this probe's own arguments imply, for
    /// [`Signature::settle_at`] — the arm-derivation's own resolved classes
    /// for a conditional-arm probe; for an ordinary probe, each bare-column
    /// argument's real fixture class (`OperandClass::Unresolved` for an
    /// override's own non-column expression). This must agree with what
    /// `print_for` actually settles the printed call to — for a
    /// `Conditional` entry it is exactly what decides whether the probe is
    /// exempt from execution, so a blind guess here would check the
    /// exemption against a different call than the one that runs.
    pub facts: CallFacts,
}

impl Probe {
    /// The full smelt-SQL statement for this probe, before dialect lowering
    /// and before the fixture CTE is prefixed.
    ///
    /// The four probe shapes are the registry's four call positions exactly
    /// (`docs/specs/multi_backend.md` §"Cross-engine emission audit") — the
    /// audit maintains no axis of its own. `WholePartitionWindow`'s `OVER`
    /// clause carries `PARTITION BY g` with no `ORDER BY` and no frame,
    /// which is whole-partition by construction (§"Emission is scoped to
    /// call position"). `Window`'s own `ORDER BY` is `rid`, not a data
    /// column: a data column carries NULLs, and engines disagree on where
    /// NULLs sort, which would make the frame — and so the result —
    /// engine-dependent for reasons that have nothing to do with emission.
    pub fn statement(&self) -> String {
        match self.position {
            Position::Any => unreachable!("Position::Any is a lookup wildcard, never probed"),
            Position::Scalar => format!(
                "SELECT {} AS {} FROM fixture ORDER BY rid",
                self.expr, self.alias
            ),
            Position::Aggregate => format!(
                "SELECT g, {} AS {} FROM fixture GROUP BY g ORDER BY g",
                self.expr, self.alias
            ),
            Position::WholePartitionWindow => format!(
                "SELECT {} OVER (PARTITION BY g) AS {} FROM fixture ORDER BY rid",
                self.expr, self.alias
            ),
            Position::Window => format!(
                "SELECT {} OVER (PARTITION BY g ORDER BY rid) AS {} FROM fixture ORDER BY rid",
                self.expr, self.alias
            ),
        }
    }
}

/// Why an entry yields no probe, for the totality gate's failure message.
///
/// One variant: an entry is either probed or it is a gap the gate names. There
/// is deliberately no "skipped" verdict — a *nondeterministic* entry is still
/// probed on the schema leg and carries `Probe::schema_only`, so nothing drops
/// out of the enumeration entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotProbed {
    /// The signature's params/form give no type-correct spelling and no
    /// override exists.
    Underivable { detail: String },
    /// An `Emission::Conditional` arm no operand-class assignment reaches —
    /// shadowed by an earlier arm's broader guard. Named by index, never
    /// silently dropped from the arm enumeration.
    UnreachableArm { index: usize, detail: String },
}

/// The fixture column that satisfies `constraint`.
fn column_for(constraint: &TypeConstraint) -> Option<&'static str> {
    use smelt_types::DataType as D;
    Some(match constraint {
        TypeConstraint::Concrete(D::Text) | TypeConstraint::Concrete(D::Varchar { .. }) => "s_text",
        TypeConstraint::Concrete(D::Char { .. }) => "s_text",
        TypeConstraint::Concrete(D::SmallInt) | TypeConstraint::Concrete(D::Integer) => "n_int",
        TypeConstraint::Concrete(D::BigInt) => "n_bigint",
        TypeConstraint::Concrete(D::Float) | TypeConstraint::Concrete(D::Double) => "n_double",
        TypeConstraint::Concrete(D::Decimal { .. }) => "n_dec",
        TypeConstraint::Concrete(D::Boolean) => "b_bool",
        TypeConstraint::Concrete(D::Date) => "d_date",
        TypeConstraint::Concrete(D::Timestamp { .. }) => "ts_ts",
        TypeConstraint::Concrete(D::Array(_)) => "arr_int",
        // `Numeric` picks the widest numeric column so an integer-only entry
        // still type-checks after implicit widening, and `Ordered`/`Any` pick a
        // BIGINT: every entry accepting them accepts an integer.
        TypeConstraint::Numeric => "n_double",
        TypeConstraint::Ordered | TypeConstraint::Any => "n_bigint",
        // Time, Interval, Blob, Json, Struct, Map, Null, Unknown: the fixture
        // has no column for these families, so an entry demanding one needs an
        // override rather than a wrong-typed argument.
        TypeConstraint::Concrete(_) => return None,
    })
}

/// The fixture column — or the bare `NULL` literal for [`OperandClass::Unresolved`]
/// — that models a call-site argument of `class`.
///
/// A separate mapping from [`column_for`]: that one answers "what satisfies
/// this declared parameter type", this one answers "what does an arm guard
/// on this class need at the call site". Merging them would perturb the
/// existing (non-conditional) probe set and pull gaps that belong to a later
/// phase into this one. No typed fixture column can classify as
/// `Unresolved` — a NULL-bearing column still has a declared type — so that
/// class is probed with a bare `NULL` literal instead
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
pub fn arg_for_class(class: OperandClass) -> &'static str {
    match class {
        OperandClass::Integral => "n_bigint",
        OperandClass::Decimal => "n_dec",
        OperandClass::Floating => "n_double",
        OperandClass::String => "s_text",
        OperandClass::Boolean => "b_bool",
        OperandClass::Temporal => "ts_ts",
        OperandClass::Interval => "iv_interval",
        OperandClass::Composite => "arr_int",
        OperandClass::Binary => "bin_blob",
        OperandClass::Unresolved => "NULL",
    }
}

/// Every `OperandClass`, searched exhaustively when deriving an arm probe.
const ALL_OPERAND_CLASSES: &[OperandClass] = &[
    OperandClass::Integral,
    OperandClass::Decimal,
    OperandClass::Floating,
    OperandClass::String,
    OperandClass::Boolean,
    OperandClass::Temporal,
    OperandClass::Interval,
    OperandClass::Composite,
    OperandClass::Binary,
    OperandClass::Unresolved,
];

/// The cartesian product of `classes` taken `n` at a time. Small and
/// test-only, so clarity wins over avoiding the `Vec<Vec<_>>` allocation.
fn cartesian(classes: &[OperandClass], n: usize) -> Vec<Vec<OperandClass>> {
    if n == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for rest in cartesian(classes, n - 1) {
        for class in classes {
            let mut v = rest.clone();
            v.push(*class);
            out.push(v);
        }
    }
    out
}

/// Find an operand-class assignment of length `arity` that makes
/// `sig.settle_at(dialect, position, _)` resolve to `arms[target].verdict` —
/// proof that arm `target` is actually *reached* by the ordered-arm walk,
/// never merely asserted from its position in the list. The indices
/// `arms[target]` guards on are fixed to their required class; every other
/// index is searched exhaustively over every [`OperandClass`] (bounded: real
/// conditional arities are small). An earlier arm with a broader guard that
/// always wins first makes this search exhaustively fail, which is exactly
/// the "unreachable arm" finding.
fn find_assignment(
    sig: &Signature,
    dialect: DialectId,
    position: Position,
    arms: &'static [ConditionalArm],
    target: usize,
    arity: usize,
) -> Option<Vec<OperandClass>> {
    let arm = &arms[target];
    let fixed: HashMap<usize, OperandClass> = arm.classes.iter().copied().collect();
    let free: Vec<usize> = (0..arity).filter(|i| !fixed.contains_key(i)).collect();

    for combo in cartesian(ALL_OPERAND_CLASSES, free.len()) {
        let mut classes = vec![OperandClass::Unresolved; arity];
        for (idx, class) in &fixed {
            classes[*idx] = *class;
        }
        for (free_idx, class) in free.iter().zip(&combo) {
            classes[*free_idx] = *class;
        }
        let facts = CallFacts::new(classes.clone());
        if sig.settle_at(dialect, position, &facts) == arm.verdict {
            return Some(classes);
        }
    }
    None
}

/// The call expression for a synthetic arm probe: fixture args chosen per
/// operand class, spelled the same way an ordinary probe is spelled — an
/// override's fixed spelling first, else the derived shape.
fn expr_for_classes(name: &str, sig: &Signature, classes: &[OperandClass]) -> Option<String> {
    let args: Vec<String> = classes
        .iter()
        .map(|c| arg_for_class(*c).to_string())
        .collect();
    let ov = overrides::find(name);
    match ov.and_then(|o| o.spelling) {
        Some(template) => Some(apply_template(template, &args)),
        None => spell(name, sig.syntax_form, &args),
    }
}

/// Every arm of every `Emission::Conditional` entry `sig` carries, one probe
/// per distinct arm guard across dialects — deduplicated by the arm list's
/// own value, since the same static arms table is routinely shared by
/// several dialects — proven reachable by [`find_assignment`] rather than
/// merely read off the arm's position in the list. An arm no assignment
/// reaches is named by index, never silently dropped
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
pub fn conditional_arm_probes(name: &'static str, sig: &Signature) -> (Vec<Probe>, Vec<NotProbed>) {
    let mut probes = Vec::new();
    let mut failures = Vec::new();
    let mut seen_arm_sets: Vec<&'static [ConditionalArm]> = Vec::new();

    for (dialect, _table_position, emission) in sig.emission {
        let Emission::Conditional(arms) = emission else {
            continue;
        };
        if seen_arm_sets.contains(arms) {
            continue;
        }
        seen_arm_sets.push(arms);

        // The emission table's own position key is routinely `Position::Any`
        // (a lookup wildcard, never a position a real call occupies — see
        // `suffix`); the position a probe is actually printed and executed at
        // is `sig.kind`'s concrete position(s), the same ones `positions`
        // derives for an ordinary probe. `Signature::settle_at` resolves a
        // concrete position back to the `Any` row via its own fallback, so
        // probing at the concrete position exercises exactly the arm the
        // real compile path would settle on.
        for position in positions(sig) {
            for (index, arm) in arms.iter().enumerate() {
                // An arity-guarded arm has exactly one arity to try. An
                // arity-less arm (including the mandatory trailing
                // `otherwise`) matches any arity, so it may be shadowed by a
                // sibling's explicit-arity guard at the signature's own
                // fixed arity — the arity a non-variadic signature's `arity:
                // None` arm would naively be tried at. Sweep a small range
                // instead of asserting only that one, so an arity-less arm
                // reachable at a *different* arity (the common shape: two
                // explicit-arity arms plus a catch-all `otherwise` for every
                // other arity) is still proven, not falsely reported
                // unreachable.
                // An arity-less arm's natural candidate is `sig.params.len()`
                // (the same default an ordinary probe's `columns_for_param`
                // uses — see its docstring). That is sufficient for a
                // class-guarded arm distinguished from its siblings by
                // operand type alone (`TRUNC`, `TO_JSON`). A trailing
                // `otherwise` guarding on arity too (`LOG`'s two-argument
                // form) may be shadowed at that default by an earlier arm's
                // explicit-arity guard, so the search extends a couple of
                // arities past the default — never further, so it cannot
                // wander into an arity the signature itself has no real
                // support for. An arity too small to hold every index the
                // arm's own class guards name is never a candidate at all.
                let default_arity = sig.params.len();
                let min_arity = arm
                    .classes
                    .iter()
                    .map(|(idx, _)| idx + 1)
                    .max()
                    .unwrap_or(0)
                    .max(default_arity);
                let candidate_arities: Vec<usize> = match arm.arity {
                    Some(n) => vec![n],
                    None => (min_arity..=min_arity + 2).collect(),
                };
                let found = candidate_arities.into_iter().find_map(|arity| {
                    find_assignment(sig, *dialect, position, arms, index, arity)
                        .map(|classes| (arity, classes))
                });
                match found {
                    Some((_, classes)) => match expr_for_classes(name, sig, &classes) {
                        Some(expr) => probes.push(Probe {
                            name,
                            position,
                            expr,
                            alias: format!(
                                "p_{}_{}_{}_a{index}",
                                sanitise(name),
                                dialect.slug(),
                                suffix(position)
                            ),
                            schema_only: None,
                            arm: Some(index),
                            facts: CallFacts::new(classes),
                        }),
                        None => failures.push(NotProbed::Underivable {
                            detail: format!(
                                "{name} arm {index}: classes {classes:?} have no derivable \
                                 spelling"
                            ),
                        }),
                    },
                    None => failures.push(NotProbed::UnreachableArm {
                        index,
                        detail: format!(
                            "{name} arm {index} on {}/{:?}: no operand-class assignment makes it \
                             the first matching arm — shadowed by an earlier arm's broader guard",
                            dialect.slug(),
                            position
                        ),
                    }),
                }
            }
        }
    }

    (probes, failures)
}

/// Resolve one parameter slot to fixture column expressions.
///
/// A `Variadic` slot expands to exactly **one** copy. Much of the registry
/// spells "arity not yet modelled" as `Variadic(Any)`, so expanding to two
/// would hand `ACOS` and `YEAR` a second argument they do not take. One copy is
/// the minimum arity; the genuinely-two-argument entries (`POW`, `CORR`, `LOG`)
/// carry an `args` override.
fn columns_for_param(sig: &Signature, param: &SigParam) -> Option<Vec<&'static str>> {
    match param {
        SigParam::Concrete(c) => Some(vec![column_for(c)?]),
        SigParam::Var(name) => {
            let tp = sig.type_params.iter().find(|t| &t.name == name)?;
            Some(vec![column_for(&tp.constraint)?])
        }
        SigParam::Variadic(inner) => columns_for_param(sig, inner),
    }
}

/// Spell `name` with `args` under `form`.
fn spell(name: &str, form: SyntaxForm, args: &[String]) -> Option<String> {
    match form {
        SyntaxForm::Call => Some(format!("{name}({})", args.join(", "))),
        SyntaxForm::Infix => {
            if args.len() != 2 {
                return None;
            }
            Some(format!("{} {name} {}", args[0], args[1]))
        }
        SyntaxForm::Postfix => {
            if args.len() != 1 {
                return None;
            }
            Some(format!("{} {name}", args[0]))
        }
        // A table function is not a scalar call position; the fixture already
        // carries the array column these unnest.
        SyntaxForm::TableFn => Some(format!("{name}(arr_int)")),
        // No uniform shape by definition — `overrides.rs` must supply one.
        SyntaxForm::Special => None,
    }
}

/// Substitute `{0}`, `{1}`, … in a spelling template.
fn apply_template(template: &str, args: &[String]) -> String {
    let mut out = template.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// Alias-safe form of a registry name: operators have no identifier spelling.
fn sanitise(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    // An all-operator name (`//`, `^`) sanitises to underscores, which are a
    // legal but indistinguishable alias; append the code points so `^` and `**`
    // do not collide.
    if !name.chars().any(|c| c.is_ascii_alphanumeric()) {
        out.push('_');
        for c in name.chars() {
            out.push_str(&format!("{:x}", c as u32));
        }
    }
    out
}

/// The positions an entry is probed in.
///
/// An aggregate is probed in **all three** of its positions — aggregate and
/// both window positions: `MEDIAN` proves the lowering differs between all
/// three, and probing fewer would have missed the BigQuery aggregate form,
/// or the DuckDB/Spark whole-partition-window restructure, entirely. The
/// probe positions are the registry's four call positions exactly (`Any` is
/// a lookup wildcard, never a position a call occupies), so this function
/// maintains no axis of its own.
fn positions(sig: &Signature) -> Vec<Position> {
    match sig.kind {
        ExprKind::Scalar => vec![Position::Scalar],
        ExprKind::Agg => vec![
            Position::Aggregate,
            Position::WholePartitionWindow,
            Position::Window,
        ],
        ExprKind::Window => vec![Position::Window],
    }
}

/// Every probe `sig` implies, or the reason it yields none.
pub fn probe_or_reason(sig: &Signature) -> Result<Vec<Probe>, NotProbed> {
    let name: &'static str = BuiltinRegistry::canonical_name(&sig.name).unwrap_or("");
    if name.is_empty() {
        return Err(NotProbed::Underivable {
            detail: "name does not resolve back through the registry".into(),
        });
    }
    let ov = overrides::find(name);

    let args: Vec<String> = match ov.and_then(|o| o.args) {
        Some(fixed) => fixed.iter().map(|s| s.to_string()).collect(),
        None => {
            let mut acc = Vec::new();
            for param in &sig.params {
                match columns_for_param(sig, param) {
                    Some(cols) => acc.extend(cols.into_iter().map(str::to_string)),
                    None => {
                        return Err(NotProbed::Underivable {
                            detail: format!(
                                "parameter {param:?} has no fixture column; add an `args` \
                                 override naming a meaningful expression"
                            ),
                        })
                    }
                }
            }
            acc
        }
    };

    let expr = match ov.and_then(|o| o.spelling) {
        Some(template) => apply_template(template, &args),
        None => spell(name, sig.syntax_form, &args).ok_or_else(|| NotProbed::Underivable {
            detail: format!(
                "{:?} with {} argument(s) has no derivable spelling; add a `spelling` override",
                sig.syntax_form,
                args.len()
            ),
        })?,
    };

    let schema_only = ov.and_then(|o| o.schema_only);
    // For a `Conditional` entry, `facts` is not a "do not affect the verdict"
    // placeholder the way it is for every other entry — it is the very input
    // `is_declared_unsupported` settles against to decide whether this probe
    // is exempt from execution. Leaving it blind (`unresolved`) would answer
    // that question against a *different* call than the one actually
    // printed, which derives its arm from the real fixture column types via
    // `print_for`'s own `settle_emissions` — so a bare column argument's
    // class is looked up the same way here, and only a non-bare argument
    // (an override's own expression) falls back to `Unresolved`.
    let column_types: HashMap<&str, DataType> = fixture::column_types().into_iter().collect();
    let facts = CallFacts::new(
        args.iter()
            .map(|a| {
                column_types
                    .get(a.as_str())
                    .map(OperandClass::of)
                    .unwrap_or(OperandClass::Unresolved)
            })
            .collect(),
    );
    Ok(positions(sig)
        .into_iter()
        .map(|position| Probe {
            name,
            position,
            expr: expr.clone(),
            alias: format!("p_{}_{}", sanitise(name), suffix(position)),
            schema_only,
            arm: None,
            facts: facts.clone(),
        })
        .collect())
}

/// Every probe the registry implies: one ordinary probe per entry, plus one
/// probe per reachable `Emission::Conditional` arm. An arm the mechanism
/// cannot reach is `every_conditional_arm_is_covered_by_a_probe`'s finding to
/// report, not this function's to swallow.
pub fn derive_probes() -> Vec<Probe> {
    let mut names: Vec<&'static str> = BuiltinRegistry::names().collect();
    names.sort_unstable();
    let mut probes = Vec::new();
    for name in names {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        if let Ok(base) = probe_or_reason(sig) {
            probes.extend(base);
        }
        let (arm_probes, _unreachable) = conditional_arm_probes(name, sig);
        probes.extend(arm_probes);
    }
    probes
}

/// Parse one smelt-SQL statement, plan any statement-level restructure it
/// needs, print it for `dialect`, and prefix the dialect's fixture CTE.
///
/// The single place a probe becomes engine SQL — both legs and every
/// hand-written regression test go through it, so no test can accidentally
/// compare a differently-printed query. Mirrors the compile path's
/// `print_checked_for` (`crates/smelt-runtime/src/compile.rs`): planning
/// happens against the source CST, before printing, never against printed
/// SQL. A call whose position the registry declares `Unsupported` — or a
/// query block an admissibility rule refuses — has nothing to plan; such a
/// probe is only ever reached here when the caller already knows to expect
/// a refusal (a declared-unsupported or ledger-registered pair), so printing
/// it verbatim on a best-effort basis, exactly as the printer does for an
/// unplanned `Unsupported`/`Restructure` call, is what the caller wants.
pub fn print_for(dialect: DialectId, smelt_sql: &str) -> String {
    let sql_dialect = match dialect {
        DialectId::DuckDb => SqlDialect::DuckDB,
        DialectId::SparkSql => SqlDialect::SparkSQL,
        DialectId::BigQuery => SqlDialect::BigQuery,
    };
    let capabilities = match dialect {
        DialectId::DuckDb => BackendCapabilities::duckdb(),
        DialectId::SparkSql => BackendCapabilities::spark(),
        DialectId::BigQuery => BackendCapabilities::bigquery(),
    };
    let parsed = smelt_parser::parse(smelt_sql);
    let root = parsed.syntax();
    let plans = plan_restructure(&root, sql_dialect).unwrap_or_default();
    // A probe's argument is always a bare fixture column reference (never a
    // compound expression) — `fixture::column_types()` is the same table the
    // type leg infers against, so looking an argument node's own text up in
    // it settles a `Conditional` entry's class-guarded arms exactly like the
    // real compile path does with a live `TypeContext`. Without this, every
    // arm settles blind (`CallFacts::unresolved`) and a class-guarded arm's
    // probe prints its *un*lowered smelt SQL verbatim — which is exactly the
    // engine-rejected spelling the arm exists to avoid printing.
    let column_types: HashMap<&str, smelt_types::DataType> =
        fixture::column_types().into_iter().collect();
    let settled = settle_emissions(&root, sql_dialect, |node| {
        column_types.get(node.text().to_string().trim()).cloned()
    });
    let ctx = PrintContext {
        dialect: &sql_dialect,
        capabilities: &capabilities,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &plans,
        settled_emissions: &settled,
    };
    combine_fixture_and_printed(&fixture::fixture_cte(dialect), &print(&root, &ctx))
}

/// Join the fixture's own `WITH fixture AS (…) ` prefix to the printed
/// statement.
///
/// A statement-level restructure appends its synthesised CTE to the
/// statement's *own* `WITH` list (`docs/specs/multi_backend.md`
/// §"Statement-level lowering"), so a printed probe with no author-written
/// `WITH` clause of its own gets a brand-new leading `WITH __smelt_r0 AS
/// (…)` from the printer. Naively concatenating that after the fixture's own
/// `WITH fixture AS (…) ` would emit two `WITH` keywords back to back — a
/// syntax error, not a semantic one, so it is worth guarding explicitly
/// rather than leaving it to surface as a confusing engine refusal. The
/// fixture is the closest thing a probe has to an author-written CTE, so the
/// fix is the same rule: fold the printed statement's `WITH` list into the
/// fixture's.
fn combine_fixture_and_printed(fixture_cte: &str, printed: &str) -> String {
    match printed.strip_prefix("WITH ") {
        Some(rest) => format!("{}, {rest}", fixture_cte.trim_end()),
        None => format!("{fixture_cte}{printed}"),
    }
}

/// smelt's own inferred type for each column of `smelt_sql`, as
/// `(alias, DataType)` pairs in select-list order.
///
/// Inference runs over the probe's *source* SQL — before any dialect lowering —
/// which is the same rule the compile path follows
/// (`multi_backend.md` §"Output-schema type conformance"). The fixture is
/// declared to the `TypeContext` as a CTE rather than re-parsed out of the
/// generated `VALUES` text, so a change to the fixture's rendering cannot
/// silently change what inference is asked about.
pub fn infer_types(smelt_sql: &str) -> Vec<(String, DataType)> {
    let parse = smelt_parser::parse(smelt_sql);
    let Some(file) = File::cast(parse.syntax()) else {
        return Vec::new();
    };
    let Some(select_stmt) = file.select_stmt() else {
        return Vec::new();
    };

    let mut ctx = TypeContext::new();
    for (name, data_type) in fixture::column_types() {
        ctx.add_cte_column("fixture", name, TypedColumn::nullable(data_type));
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    let Some(select_list) = select_stmt.select_list() else {
        return Vec::new();
    };
    select_list
        .items()
        .zip(column_types.iter())
        .map(|item| {
            (
                item.0.alias().unwrap_or_else(|| "?".to_string()),
                item.1.data_type.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::{SettledEmission, TypeExpr};

    fn two_arg_signature() -> Signature {
        Signature::new(
            "TEST_TWO_ARG",
            vec![],
            vec![
                SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
                SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        )
    }

    /// Test 1: every `OperandClass` except `Unresolved` resolves to a
    /// fixture column whose `column_types()` entry classifies back to that
    /// same class; `Unresolved` resolves to the `NULL` literal.
    #[test]
    fn the_fixture_has_a_column_for_every_operand_class() {
        let types: HashMap<&str, DataType> = fixture::column_types().into_iter().collect();
        for class in [
            OperandClass::Integral,
            OperandClass::Decimal,
            OperandClass::Floating,
            OperandClass::String,
            OperandClass::Boolean,
            OperandClass::Temporal,
            OperandClass::Interval,
            OperandClass::Composite,
            OperandClass::Binary,
        ] {
            let col = arg_for_class(class);
            let dt = types
                .get(col)
                .unwrap_or_else(|| panic!("{class:?}: column {col} not in the fixture"));
            assert_eq!(
                OperandClass::of(dt),
                class,
                "{class:?}: column {col} classifies as {:?}, not {class:?}",
                OperandClass::of(dt)
            );
        }
        assert_eq!(arg_for_class(OperandClass::Unresolved), "NULL");
    }

    /// Test 3: a synthetic two-argument conditional yields two scalar probes
    /// with distinct aliases.
    #[test]
    fn a_conditional_entry_is_probed_once_per_arm() {
        const ARMS: &[ConditionalArm] = &[
            ConditionalArm {
                arity: None,
                classes: &[(0, OperandClass::Integral), (1, OperandClass::Integral)],
                verdict: SettledEmission::Native,
            },
            ConditionalArm {
                arity: None,
                classes: &[],
                verdict: SettledEmission::Unsupported {
                    reason: "otherwise",
                },
            },
        ];
        let sig = two_arg_signature().with_emission(&[(
            DialectId::DuckDb,
            Position::Scalar,
            Emission::Conditional(ARMS),
        )]);

        let (probes, failures) = conditional_arm_probes("TEST_TWO_ARG", &sig);
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(probes.len(), 2, "{probes:#?}");
        assert!(probes.iter().all(|p| p.position == Position::Scalar));
        let mut aliases: Vec<&str> = probes.iter().map(|p| p.alias.as_str()).collect();
        let total = aliases.len();
        aliases.sort_unstable();
        aliases.dedup();
        assert_eq!(
            aliases.len(),
            total,
            "expected distinct aliases: {probes:#?}"
        );
    }

    /// Test 4: the arguments chosen for arm `k` produce `CallFacts` that
    /// `settle_at` resolves to arm `k`'s verdict, for every arm of the
    /// synthetic entry.
    #[test]
    fn an_arm_probe_selects_the_arm_it_was_derived_for() {
        const ARMS: &[ConditionalArm] = &[
            ConditionalArm {
                arity: None,
                classes: &[(0, OperandClass::Integral), (1, OperandClass::Integral)],
                verdict: SettledEmission::Native,
            },
            ConditionalArm {
                arity: None,
                classes: &[(0, OperandClass::String)],
                verdict: SettledEmission::Rename("STR_ARM"),
            },
            ConditionalArm {
                arity: None,
                classes: &[],
                verdict: SettledEmission::Unsupported {
                    reason: "otherwise",
                },
            },
        ];
        let sig = two_arg_signature().with_emission(&[(
            DialectId::DuckDb,
            Position::Scalar,
            Emission::Conditional(ARMS),
        )]);

        for (index, arm) in ARMS.iter().enumerate() {
            let arity = arm.arity.unwrap_or(sig.params.len());
            let classes = find_assignment(
                &sig,
                DialectId::DuckDb,
                Position::Scalar,
                ARMS,
                index,
                arity,
            )
            .unwrap_or_else(|| panic!("arm {index} unreachable"));
            let facts = CallFacts::new(classes);
            assert_eq!(
                sig.settle_at(DialectId::DuckDb, Position::Scalar, &facts),
                arm.verdict,
                "arm {index}"
            );
        }
    }

    /// Test 5: an arm no assignment reaches is named by index, rather than
    /// yielding one fewer probe silently.
    #[test]
    fn an_unreachable_arm_is_named_never_skipped() {
        const ARMS: &[ConditionalArm] = &[
            ConditionalArm {
                arity: None,
                classes: &[(0, OperandClass::Integral)],
                verdict: SettledEmission::Native,
            },
            // Shadowed: arm 0 already matches whenever argument 0 is
            // Integral, regardless of argument 1, so this arm's narrower
            // guard is never reached.
            ConditionalArm {
                arity: None,
                classes: &[(0, OperandClass::Integral), (1, OperandClass::String)],
                verdict: SettledEmission::Rename("SHADOWED"),
            },
            ConditionalArm {
                arity: None,
                classes: &[],
                verdict: SettledEmission::Unsupported {
                    reason: "otherwise",
                },
            },
        ];
        let sig = two_arg_signature().with_emission(&[(
            DialectId::DuckDb,
            Position::Scalar,
            Emission::Conditional(ARMS),
        )]);

        let (probes, failures) = conditional_arm_probes("TEST_TWO_ARG", &sig);
        assert_eq!(probes.len(), 2, "arms 0 and 2 are reachable: {probes:#?}");
        assert_eq!(failures.len(), 1, "{failures:?}");
        match &failures[0] {
            NotProbed::UnreachableArm { index, .. } => assert_eq!(*index, 1),
            other => panic!("expected UnreachableArm, got {other:?}"),
        }
    }
}
