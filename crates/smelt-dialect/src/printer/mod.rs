//! Dialect-aware CST printer.
//!
//! Walks a Rowan CST and emits SQL text, performing dialect-specific rewrites:
//! - `smelt.<path>` → resolved table name (via `SmeltPathRef`/`SmeltPathCall`)
//! - QUALIFY → subquery rewrite (when `!caps.supports_qualify`)
//! - ARRAY[1,2,3] → ARRAY(1,2,3) (when `!caps.supports_array_literal`)
//! - DATE '2024-01-01' → DATE('2024-01-01') (when `!caps.supports_date_literal`)
//! - expr::type → CAST(expr AS type) (when `!caps.supports_double_colon_cast`)
//!
//! The default behavior is verbatim: tokens (including whitespace and comments)
//! are emitted exactly as they appear in the source. This guarantees an identity
//! property for DuckDB with no refs/sources.

use smelt_parser::ast::{SmeltAsStructCall, SmeltPathCall, SmeltPathRef};
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::FunctionCall;

use std::collections::{HashMap, HashSet};

use crate::restructure::RestructurePlan;
use crate::{BackendCapabilities, SqlDialect};
use smelt_types::SettledEmission;

mod pipe;
mod pipe_stages;
mod registry_emit;
mod restructure_emit;
mod rewrites;
mod smelt_path;

#[cfg(test)]
mod tests;

use self::registry_emit::emit_registered_function;
use self::registry_emit::emit_registered_operator;
pub use self::registry_emit::print_template;
use self::restructure_emit::active_substitution_for;
use self::rewrites::print_array_rewrite;
use self::rewrites::print_cast_rewrite;
use self::rewrites::print_children;
use self::smelt_path::expand_smelt_path_call_star;

use self::pipe::print_pipe_rewrite;
use self::pipe::reexpand_call_body;
use self::restructure_emit::print_restructured_select;
use self::restructure_emit::restructure_plan_select_stmt;
use self::rewrites::print_select_with_qualify_rewrite;
use self::rewrites::print_strip_trailing_commas;
use self::smelt_path::smelt_path_call_has_explicit_alias;

/// Emitter closure type for `smelt.as_struct(alias [EXCEPT cols])`.
/// Called with `(alias, except_columns)` → emitted SQL, or `None` to pass through.
pub type AsStructEmitter<'a> = Box<dyn Fn(&str, &[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.fn.*` calls.
/// Called with `(fn_name, positional_arg_sqls, named_args)` → expanded SQL, or `None`.
pub type SmeltFnExpander<'a> =
    Box<dyn Fn(&str, Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Resolver closure type for `smelt.<path>` value references (`SMELT_PATH_REF`).
/// Called with path segments (everything after `smelt`); returns backend SQL or `None` to
/// emit verbatim.
pub type SmeltPathRefResolver<'a> = Box<dyn Fn(&[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
/// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` → expanded SQL, or `None`
/// to emit verbatim.
pub type SmeltPathCallExpander<'a> =
    Box<dyn Fn(&[String], Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Context for dialect-aware printing.
pub struct PrintContext<'a> {
    pub dialect: &'a SqlDialect,
    pub capabilities: &'a BackendCapabilities,
    pub schema: &'a str,
    /// Model names that are ephemeral — refs to these emit `__smelt_{name}` instead of `schema.name`.
    pub ephemeral_models: HashSet<&'a str>,
    /// Cross-engine refs: model_name -> `read_parquet('{path}/**/*.parquet', hive_partitioning=true)`.
    /// When a ref is in this map, the parquet expression is emitted instead of `schema.model`.
    pub cross_engine_refs: HashMap<String, String>,
    /// Emitter for `smelt.as_struct(alias [EXCEPT cols])`.
    ///
    /// Called with `(alias, except_columns)` and returns the backend-specific SQL string.
    /// `None` = pass through verbatim (backward compat for tests / contexts without schema info).
    pub smelt_as_struct: Option<AsStructEmitter<'a>>,
    /// Expander for `smelt.fn.*` calls.
    ///
    /// Called with `(fn_name, positional_arg_sqls, named_args)` and returns the expanded SQL.
    /// `None` = pass through verbatim (backward compat for tests / contexts without function info).
    pub smelt_fn: Option<SmeltFnExpander<'a>>,
    /// Resolver for `smelt.<path>` value references (`SMELT_PATH_REF`).
    ///
    /// Called with path segments (everything after `smelt`) and returns the backend SQL string.
    /// `None` = pass through verbatim (backward compat for callers that have not configured
    /// path resolution).
    pub smelt_path_ref: Option<SmeltPathRefResolver<'a>>,
    /// Expander for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
    ///
    /// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` and returns the
    /// expanded SQL. `None` = pass through verbatim.
    pub smelt_path_call: Option<SmeltPathCallExpander<'a>>,
    /// Statement-level restructures already planned for this tree
    /// (`docs/specs/multi_backend.md` §"Statement-level lowering"), typically
    /// `restructure::plan`'s output for the same source `SyntaxNode` about to
    /// be printed. A `SELECT_STMT` node matching one of these plans is
    /// emitted as the synthesised-CTE form instead of verbatim; every other
    /// node is unaffected. Empty when nothing on this tree needs
    /// restructuring.
    pub restructure_plans: &'a [RestructurePlan],
    /// Pre-settled operand-conditional verdicts, keyed by the call/operator
    /// node's own `TextRange` (`docs/specs/multi_backend.md`
    /// §"Operand-conditional verdicts"). Populated by the crate's settlement
    /// walk over the same source tree, before printing starts. The printer
    /// looks a node's verdict up here rather than resolving one itself — it
    /// holds no type context and cannot. Empty when the caller has no type
    /// context (e.g. `resolve_refs_in_sql`); a lookup miss falls back to an
    /// arity-only settlement.
    pub settled_emissions: &'a [(smelt_parser::TextRange, SettledEmission)],
}

/// Print a CST node as dialect-specific SQL.
pub fn print(node: &SyntaxNode, ctx: &PrintContext) -> String {
    let mut out = String::new();
    print_node(node, ctx, &mut out);
    out
}

pub(crate) fn print_node(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    // Statement-level restructure substitution (`with_active_substitutions`):
    // checked before the per-kind dispatch below so a node matched anywhere
    // in the tree — nested inside arithmetic, another call's argument list,
    // … — is substituted in place, with everything else printed unchanged
    // by the ordinary recursive dispatch. See "Statement-level restructure
    // emission" further down this file.
    if let Some(replacement) = active_substitution_for(node) {
        out.push_str(&replacement);
        return;
    }
    match node.kind() {
        SyntaxKind::SMELT_AS_STRUCT_CALL => {
            if let Some(ref emitter) = ctx.smelt_as_struct {
                if let Some(call) = SmeltAsStructCall::cast(node.clone()) {
                    if let Some(alias) = call.alias() {
                        let except = call.except_columns();
                        if let Some(sql) = emitter(&alias, &except) {
                            out.push_str(&sql);
                            return;
                        }
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_REF => {
            if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
                let segs = path_ref.segments();

                // Try explicit resolver first.
                let resolved = ctx
                    .smelt_path_ref
                    .as_ref()
                    .and_then(|resolver| resolver(&segs));

                // Fall back to built-in resolution based on the path namespace:
                //   smelt.models.<name>   → schema.name  (or cross-engine / ephemeral)
                //   smelt.sources.<src>.<tbl>  → src.tbl
                let resolved = resolved.or_else(|| match segs.as_slice() {
                    [ns, name] if ns == "models" => {
                        if ctx.ephemeral_models.contains(name.as_str()) {
                            Some(format!("__smelt_{}", name))
                        } else if let Some(parquet_expr) = ctx.cross_engine_refs.get(name.as_str())
                        {
                            Some(parquet_expr.clone())
                        } else {
                            Some(format!("{}.{}", ctx.schema, name))
                        }
                    }
                    [ns, src, tbl] if ns == "sources" => Some(format!("{}.{}", src, tbl)),
                    _ => None,
                });

                if let Some(sql) = resolved {
                    out.push_str(&sql);
                    // Re-emit trailing trivia (whitespace/comments) captured
                    // inside this node by the parser's look-ahead skip_trivia
                    // call. These are direct-child tokens after the SMELT_PATH
                    // sub-node (e.g. the space before an `AS` alias).
                    for child in node.children_with_tokens() {
                        if let SyntaxElement::Token(t) = child {
                            if t.kind().is_trivia() {
                                out.push_str(t.text());
                            }
                        }
                    }
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_CALL => {
            if let (Some(ref expander), Some(path_call)) =
                (&ctx.smelt_path_call, SmeltPathCall::cast(node.clone()))
            {
                let segs = path_call.segments();
                let positional: Vec<String> = path_call
                    .arg_list()
                    .map(|al| {
                        al.positional_args()
                            .into_iter()
                            .map(|arg| {
                                let mut s = String::new();
                                print_node(arg.syntax(), ctx, &mut s);
                                s
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut named: Vec<(String, String)> = path_call
                    .arg_list()
                    .map(|al| {
                        al.named_params()
                            .filter_map(|np| {
                                let name = np.name()?;
                                let expr = np.value_expr()?;
                                let mut s = String::new();
                                print_node(expr.syntax(), ctx, &mut s);
                                Some((name, s))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Merge block `PASSING <name> AS (<body>)` fragment bindings into
                // the named-argument set. A PASSING clause binds parameter
                // `<name>` to `<body>` exactly as a `<name> => <body>` inline
                // named argument would — the type-checker rejects supplying the
                // same parameter both ways, so there is no collision to resolve.
                // The body is printed through `ctx` so any nested smelt
                // constructs inside the fragment are expanded too.
                for clause in path_call.passing_clauses() {
                    if let Some(name) = clause.name() {
                        let body = if let Some(expr) = clause.body_expr() {
                            let mut s = String::new();
                            print_node(expr.syntax(), ctx, &mut s);
                            s
                        } else if let Some(text) = clause.body_text() {
                            text
                        } else {
                            continue;
                        };
                        named.push((name, body));
                    }
                }
                if let Some(expanded) = expander(&segs, positional, named) {
                    // Detect FROM-position: the SMELT_PATH_CALL node's parent
                    // must be TABLE_REF, and TABLE_REF's parent must be
                    // FROM_CLAUSE. FROM-position only — JOIN/LATERAL positions
                    // are not handled here.
                    let in_from_position = node
                        .parent()
                        .filter(|p| p.kind() == SyntaxKind::TABLE_REF)
                        .and_then(|table_ref| table_ref.parent())
                        .map(|gp| gp.kind() == SyntaxKind::FROM_CLAUSE)
                        .unwrap_or(false);

                    // Detect user-supplied alias: scan TABLE_REF children that
                    // come after this SMELT_PATH_CALL node.  An alias is present
                    // when there is an AS_KW or a bare IDENT following the call.
                    let has_explicit_alias = in_from_position && {
                        node.parent()
                            .filter(|p| p.kind() == SyntaxKind::TABLE_REF)
                            .map(|table_ref| smelt_path_call_has_explicit_alias(&table_ref, node))
                            .unwrap_or(false)
                    };

                    if in_from_position && !has_explicit_alias {
                        // Synthesise `(<expanded>) AS __smelt_t<start_offset>`.
                        // Using the source byte offset of the call makes the
                        // alias stable per call site with no shared mutable
                        // state — valid across all models in a project because
                        // each model is printed independently.
                        let offset = u32::from(node.text_range().start());
                        let body = reexpand_call_body(&expanded, ctx);
                        let body_trimmed = body.trim_end();
                        out.push('(');
                        out.push_str(body_trimmed);
                        out.push_str(&format!(") AS __smelt_t{offset}"));
                    } else {
                        out.push_str(&reexpand_call_body(&expanded, ctx));
                    }
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_CALL_STAR => {
            // `smelt.<path>(args).*` — lower to per-field projections when
            // the expander provides a brace-struct literal body; fall back to
            // verbatim otherwise.
            if let Some(expanded) = expand_smelt_path_call_star(node, ctx) {
                out.push_str(&expanded);
                return;
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::FUNCTION_CALL => {
            if let Some(fc) = FunctionCall::cast(node.clone()) {
                if let Some(name) = fc.name() {
                    if emit_registered_function(node, &fc, &name, ctx, out) {
                        return;
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::BINARY_EXPR => {
            if emit_registered_operator(node, ctx, out) {
                return;
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::PIPE_QUERY if !ctx.capabilities.supports_pipe_syntax => {
            print_pipe_rewrite(node, ctx, out);
        }
        SyntaxKind::SELECT_STMT => {
            if let Some(plan) = ctx
                .restructure_plans
                .iter()
                .find(|p| restructure_plan_select_stmt(p) == node)
            {
                print_restructured_select(plan, ctx, out);
            } else if !ctx.capabilities.supports_qualify {
                print_select_with_qualify_rewrite(node, ctx, out);
            } else {
                print_children(node, ctx, out);
            }
        }
        SyntaxKind::ARRAY_LITERAL if !ctx.capabilities.supports_array_literal => {
            print_array_rewrite(node, ctx, out);
        }
        SyntaxKind::SELECT_LIST | SyntaxKind::GROUP_BY_CLAUSE
            if !ctx.capabilities.supports_trailing_commas =>
        {
            print_strip_trailing_commas(node, ctx, out);
        }
        SyntaxKind::CAST_EXPR if !ctx.capabilities.supports_double_colon_cast => {
            print_cast_rewrite(node, ctx, out);
        }
        // Top-level smelt DSL declarations that are not SQL: suppress them so
        // they never reach the backend engine.  `SMELT_DEFINE` and
        // `SMELT_EXTERN` carry function bodies / extern signatures that the
        // compiler inlines via the function-expander closure above; they must
        // not be emitted verbatim.  `SMELT_RECORD_DECL` carries type
        // declarations used by the analyzer but invisible to the engine.
        SyntaxKind::SMELT_DEFINE
        | SyntaxKind::SMELT_EXTERN
        | SyntaxKind::SMELT_RECORD_DECL
        | SyntaxKind::SMELT_TEST => {
            // Emit nothing — drop the declaration from the compiled SQL.
        }
        _ => {
            print_children(node, ctx, out);
        }
    }
}
