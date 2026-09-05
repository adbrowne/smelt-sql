//! Probes derived from the registry, not authored against it.
//!
//! For each entry: the parameters' `TypeConstraint`s select fixture columns,
//! `SyntaxForm` decides the spelling, and `ExprKind` decides the query shape.
//! An entry the rules cannot spell is reported by name — never silently
//! dropped, which is how a registry-blind harness quietly stops covering
//! things.

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_dialect::{plan_restructure, print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;
pub use smelt_types::signatures::Position;
use smelt_types::{
    BuiltinRegistry, DataType, DialectId, ExprKind, SigParam, Signature, SyntaxForm,
    TypeConstraint, TypedColumn,
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
    /// Deterministic alias: `p_<sanitised name>_<position>`.
    pub alias: String,
    /// When set, this probe runs the schema leg only, for the recorded reason.
    pub schema_only: Option<&'static str>,
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
    Ok(positions(sig)
        .into_iter()
        .map(|position| Probe {
            name,
            position,
            expr: expr.clone(),
            alias: format!("p_{}_{}", sanitise(name), suffix(position)),
            schema_only,
        })
        .collect())
}

/// Every probe the registry implies.
pub fn derive_probes() -> Vec<Probe> {
    let mut names: Vec<&'static str> = BuiltinRegistry::names().collect();
    names.sort_unstable();
    names
        .into_iter()
        .filter_map(|name| BuiltinRegistry::resolve(name))
        .filter_map(|sig| probe_or_reason(sig).ok())
        .flatten()
        .collect()
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
        settled_emissions: &[],
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
