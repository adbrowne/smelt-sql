//! Per-model schema queries and `TypeContext` construction.
//!
//! Most of this file is **pure**: `build_type_context`, the
//! `RefSchemaProvider` trait, and `StaticRefSchemaProvider`. Only a handful
//! of `#[salsa::tracked]` wrappers (`model_schema`, `available_columns`,
//! `type_context`, `typed_model_schema`, `resolved_model_schema`,
//! `columns_of_for_table_expr`, `model_input_constraints`,
//! `model_function_type`) thread Salsa lookups into the pure helpers.

use std::collections::HashMap;
use std::sync::Arc;

use rowan::TextRange;
use smelt_parser::{self, ast::SmeltPathRef, File as AstFile, TableRef};
use smelt_types::{DataType, TypedColumn};

use crate::function_body_check::{self, infer_tableexpr_return_schema};
use crate::queries::functions::{file_signature_inputs, resolve_function};
use crate::queries::parse::parse_file;
use crate::queries::project::{project_seeds, project_sources, sources_config};
use crate::schema::{self, Column, ColumnSource, InputConstraint, ModelSchema, ResolvedSchema};
use crate::type_inference::{self, infer_cte_columns, infer_select_column_types, TypeContext};
use crate::{find_project, resolve_ref, SourceFile, Workspace};

use smelt_core::{SourceInfo, SourcesConfig};

// ============================================================================
// Schema queries
// ============================================================================

#[salsa::tracked]
pub fn model_schema(db: &dyn salsa::Database, file: SourceFile) -> Arc<ModelSchema> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return Arc::new(ModelSchema::empty()),
    };

    let from_refs: Vec<String> = if let Some(from_clause) = select_stmt.from_clause() {
        from_clause
            .table_refs()
            .filter_map(|table_ref| {
                // Path-form: smelt.models.foo → leaf segment "foo"
                table_ref
                    .smelt_path_ref()
                    .and_then(|pr| pr.segments().last().cloned())
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut columns = Vec::new();
    let mut row_extensions = Vec::new();

    for item in select_list.items() {
        if item.is_wildcard() {
            for ref_name in &from_refs {
                row_extensions.push(schema::RowExtension {
                    ref_name: ref_name.clone(),
                    excluded_columns: vec![],
                    range: item.range(),
                });
            }
            continue;
        }

        let name = match item.column_name() {
            Some(n) => n,
            None => continue,
        };

        let alias = item.alias();
        let expression = item.expression().map(|e| e.text()).unwrap_or_default();

        let source = if let Some(expr) = item.expression() {
            if expr.as_function_call().is_some() {
                ColumnSource::Computed
            } else if let Some(col_ref) = expr.as_column_ref() {
                let column_name = col_ref.name().to_string();
                if from_refs.len() == 1 {
                    ColumnSource::FromModel {
                        model_name: from_refs[0].clone(),
                        column_name,
                    }
                } else if from_refs.is_empty() {
                    ColumnSource::ExternalTable {
                        table_name: col_ref.qualifier().unwrap_or("unknown").to_string(),
                    }
                } else {
                    ColumnSource::Unknown
                }
            } else {
                ColumnSource::Computed
            }
        } else {
            ColumnSource::Unknown
        };

        columns.push(Column {
            name,
            alias,
            source,
            expression,
            range: item.range(),
            data_type: None,
        });
    }

    if !row_extensions.is_empty() {
        let explicit_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        for ext in &mut row_extensions {
            ext.excluded_columns = explicit_names.clone();
        }
    }

    Arc::new(ModelSchema {
        columns,
        row_extensions,
        input_constraints: vec![],
    })
}

#[salsa::tracked]
pub fn available_columns(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<Column>> {
    let schema = model_schema(db, file);
    let mut available = schema.columns.clone();

    // Walk FROM/JOIN clause SmeltPathRef nodes for smelt.models.* refs and
    // include upstream model columns (used by LSP column completion).
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let project = find_project(db, workspace, &file.project_root(db).clone());

    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(from_clause) = select_stmt.from_clause() {
                for table_ref in from_clause.table_refs() {
                    if let Some(path_ref) = table_ref.smelt_path_ref() {
                        let segs = path_ref.segments();
                        if segs.first().map(|s| s.as_str()) == Some("models") {
                            if let Some(model_name) = segs.last().cloned() {
                                if let Some(upstream) =
                                    project.and_then(|p| resolve_ref(db, workspace, p, model_name))
                                {
                                    let upstream_schema = model_schema(db, upstream);
                                    for col in upstream_schema.columns.iter() {
                                        available.push(col.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Arc::new(available)
}

// ============================================================================
// Type checking queries (with cycle recovery via cycle_initial)
// ============================================================================

fn typed_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ModelSchema> {
    Arc::new(ModelSchema::empty())
}

fn type_context_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<TypeContext> {
    Arc::new(TypeContext::new())
}

fn resolved_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ResolvedSchema> {
    Arc::new(ResolvedSchema {
        columns: vec![],
        is_fully_resolved: true,
        unresolved_extensions: vec![],
    })
}

/// Provider for upstream `smelt.ref()` schema lookups, used by the pure
/// `build_type_context` function.
///
/// The Salsa version uses [`SalsaRefSchemaProvider`] (delegates to the new
/// 0.26-API free functions `resolve_ref` + `resolved_model_schema`).
/// The CLI batch compiler uses [`StaticRefSchemaProvider`], which is fully
/// pure and takes pre-computed maps.
pub trait RefSchemaProvider {
    /// Returns the typed columns for the model named `model_name`, if known.
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>>;
    /// Returns the typed columns for the seed named `seed_name`, if known.
    /// Seeds and model refs are looked up separately because the type-context
    /// loop wants to distinguish them (CSV files don't participate in
    /// SELECT * schema resolution, etc.).
    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>>;
    /// Returns the typed columns for the `smelt.functions.<name>(...)` call in
    /// FROM position, by resolving the function's `TableExpr` return schema.
    /// The default implementation returns `None`; `SalsaRefSchemaProvider`
    /// overrides this with a full Salsa-backed resolution.
    fn smelt_path_call_columns(
        &self,
        _call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        None
    }
}

/// `RefSchemaProvider` impl that delegates to the Salsa database. Used by the
/// `type_context()` Salsa query so the LSP keeps benefiting from
/// incremental recomputation.
pub struct SalsaRefSchemaProvider<'a> {
    db: &'a dyn salsa::Database,
    workspace: Workspace,
    /// Project the resolver is scoped to. See
    /// `docs/specs/architecture.md` → "Project isolation rule".
    /// `None` only on legacy code paths that pre-date the rule; new callers
    /// should always supply a project. When `None`, function resolution
    /// returns `None` (no cross-project signature leak).
    project: Option<crate::ProjectInput>,
    /// Cycle guard for `resolve_smelt_path_call_schema`. Holds the set of
    /// function names currently being resolved on the call stack. When a
    /// callee is already in this set, the resolver short-circuits (returns
    /// `None`) instead of recursing infinitely. Uses `RefCell` so the guard
    /// can be updated through the shared `&self` reference that closures
    /// capture.
    visiting: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl<'a> SalsaRefSchemaProvider<'a> {
    /// Construct a project-scoped provider. Use `new_for_file` or
    /// `new_for_project` for convenience; this raw form is for callers
    /// that already hold a `ProjectInput`.
    pub fn new(
        db: &'a dyn salsa::Database,
        workspace: Workspace,
        project: Option<crate::ProjectInput>,
    ) -> Self {
        Self {
            db,
            workspace,
            project,
            visiting: std::cell::RefCell::new(std::collections::HashSet::new()),
        }
    }

    /// Construct a provider scoped to the project containing `file`. The
    /// typical entry point — the file under analysis tells us the project.
    pub fn new_for_file(
        db: &'a dyn salsa::Database,
        workspace: Workspace,
        file: SourceFile,
    ) -> Self {
        let project_root = file.project_root(db).clone();
        let project = crate::find_project(db, workspace, &project_root);
        Self::new(db, workspace, project)
    }
}

impl SalsaRefSchemaProvider<'_> {
    /// Phase 45: resolve a `TableRef` (FROM/JOIN entry) to the columns it
    /// contributes. Used by [`function_body_check::register_join_alias_schemas`]
    /// from the `infer_tableexpr_return_schema` path so that `<alias>.*`
    /// projections from joined tables expand correctly.
    pub fn resolve_table_ref_schema(
        &self,
        table_ref: &smelt_parser::ast::TableRef,
    ) -> Option<Vec<(String, TypedColumn)>> {
        // smelt.functions.<name>(...) in FROM/JOIN position — resolve schema.
        if let Some(path_call) = table_ref.smelt_path_call() {
            return self.resolve_smelt_path_call_schema(&path_call);
        }

        // smelt.<path> value-form in FROM/JOIN position.
        if let Some(path_ref) = table_ref.smelt_path_ref() {
            let segs = path_ref.segments();
            let model_name = segs.last().cloned().unwrap_or_default();
            let seed_key = segs.join("_");
            return self
                .resolved_columns(&model_name)
                .or_else(|| self.seed_columns(&seed_key));
        }

        None
    }

    /// Resolve a `smelt.functions.<name>(...)` path-call in FROM position
    /// to its inferred output schema.
    ///
    /// Flow:
    ///   1. Resolve the path's tail segment to a workspace `FunctionSig`.
    ///   2. If the signature's return type isn't `TableExpr`, return `None`.
    ///   3. Re-parse the callee's file and find the body `SelectStmt`.
    ///   4. Build a body ctx by resolving each `TableExpr` argument to its
    ///      schema, seeding via `add_tableexpr_param`. Other `Expr<T>` params
    ///      are seeded to `Unknown`.
    ///   5. Call `infer_tableexpr_return_schema` on the body and return cols.
    fn resolve_smelt_path_call_schema(
        &self,
        call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        let segments = call.segments();
        let name = segments.last()?.clone();

        // Cycle guard: if we are already resolving `name` further up the call
        // stack (mutual or direct recursion), short-circuit rather than
        // recursing infinitely. The workspace emits `FunctionCallCycle` for
        // such code; this guard prevents a stack overflow when the resolver
        // runs on in-progress (invalid) code where the diagnostic path hasn't
        // fired yet. We return `None` (unresolved / opaque columns) — callers
        // treat this identically to an unknown function call.
        {
            let mut vis = self.visiting.borrow_mut();
            if vis.contains(&name) {
                return None;
            }
            vis.insert(name.clone());
        }

        // Project isolation rule: only consider signatures declared in the
        // same project as the file whose schema we're computing.
        let result = self.resolve_smelt_path_call_schema_inner(call, &name);
        self.visiting.borrow_mut().remove(&name);
        result
    }

    fn resolve_smelt_path_call_schema_inner(
        &self,
        call: &smelt_parser::ast::SmeltPathCall,
        name: &str,
    ) -> Option<Vec<(String, TypedColumn)>> {
        use smelt_parser::ast::Expr as AstExpr;
        use smelt_types::signatures::SmeltType;

        let project = self.project?;
        let sig_arc = resolve_function(self.db, self.workspace, project, name.to_string())?;
        let sig: &smelt_types::signatures::FunctionSig = sig_arc.as_ref();

        // Only `TableExpr`-returning functions contribute a FROM schema.
        match &sig.return_type {
            Some(Ok(SmeltType::TableExpr(_))) => {}
            _ => return None,
        }

        // Find the callee's file + body SelectStmt.
        let files: Vec<SourceFile> = self.workspace.files(self.db).to_vec();
        let mut body_select: Option<smelt_parser::ast::SelectStmt> = None;
        for f in &files {
            let sigs = file_signature_inputs(self.db, *f);
            if !sigs.iter().any(|s| s.name == sig.name) {
                continue;
            }
            let f_parse = parse_file(self.db, *f);
            let f_syntax = f_parse.syntax();
            if let Some(ast) = AstFile::cast(f_syntax) {
                for define in ast.defines() {
                    if define.name().as_deref() == Some(&sig.name) {
                        if let Some(body) = define.body() {
                            if let Some(stmt) = body.select_stmt() {
                                body_select = Some(stmt);
                                break;
                            }
                        }
                    }
                }
            }
            if body_select.is_some() {
                break;
            }
        }
        let body_select = body_select?;

        // Bind args to params by position / name.
        let arg_list = call.arg_list();
        let positional: Vec<AstExpr> = arg_list
            .as_ref()
            .map(|al| al.positional_args())
            .unwrap_or_default();
        let named: Vec<smelt_parser::ast::NamedParam> = arg_list
            .as_ref()
            .map(|al| al.named_params().collect())
            .unwrap_or_default();

        let mut bindings: std::collections::HashMap<String, AstExpr> =
            std::collections::HashMap::new();
        for (i, arg) in positional.iter().enumerate() {
            if let Some(p) = sig.params.get(i) {
                bindings.insert(p.name.clone(), arg.clone());
            }
        }
        for np in &named {
            if let (Some(nm), Some(value)) = (np.name(), np.value_expr()) {
                bindings.insert(nm, value);
            }
        }

        // Seed the body ctx with every parameter's caller schema / type.
        let mut body_ctx = TypeContext::new();
        for param in &sig.params {
            if param.name.is_empty() {
                continue;
            }
            if matches!(&param.type_ref, Some(Ok(SmeltType::TableExpr(_)))) {
                // Resolve the TableExpr argument to its column schema.
                if let Some(arg_expr) = bindings.get(&param.name) {
                    // If the argument is itself a smelt.functions.* call, resolve
                    // it recursively to get the inner call's output schema.
                    if let Some(nested) = arg_expr.as_smelt_path_call() {
                        if let Some(cols) = self.resolve_smelt_path_call_schema(&nested) {
                            body_ctx.add_tableexpr_param(&param.name, &cols);
                        }
                        continue;
                    }
                    // Walk the arg expression for a smelt.<path> ref.
                    let mut seeded = false;
                    for node in arg_expr.syntax().descendants() {
                        if node.kind() == smelt_parser::SyntaxKind::SMELT_PATH_REF {
                            if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
                                let segs = path_ref.segments();
                                let model_name = segs.last().cloned().unwrap_or_default();
                                let seed_key = segs.join("_");
                                if let Some(cols) = self
                                    .resolved_columns(&model_name)
                                    .or_else(|| self.seed_columns(&seed_key))
                                {
                                    body_ctx.add_tableexpr_param(&param.name, &cols);
                                    seeded = true;
                                    break;
                                }
                            }
                        }
                    }

                    // If the argument is a bare identifier that names a CTE or
                    // derived table in the CALLER's WITH clause, resolve that
                    // CTE's column schema and seed the body context with it.
                    // This handles the pattern:
                    //   WITH x AS (SELECT … AS revenue, … AS cost)
                    //   SELECT col FROM smelt.functions.f(x)
                    if !seeded {
                        if let Some(cte_name) = arg_expr
                            .as_column_ref()
                            .filter(|cr| cr.qualifier().is_none())
                            .map(|cr| cr.name().to_string())
                        {
                            if let Some(cols) = cte_columns_from_caller_select(&cte_name, call) {
                                body_ctx.add_tableexpr_param(&param.name, &cols);
                            }
                        }
                    }
                }
            } else {
                // Expr<T> param — bind to Unknown for schema inference.
                let dt = match &param.type_ref {
                    Some(Ok(SmeltType::Expr(
                        smelt_types::signatures::TypeConstraint::Concrete(dt),
                    ))) => dt.clone(),
                    _ => DataType::Unknown,
                };
                body_ctx.add_function_param(&param.name, TypedColumn::nullable(dt));
            }
        }

        // Seed workspace signatures so nested function calls in the
        // body infer their return types.
        let mut wsp_files = files.clone();
        wsp_files.sort_by(|a, b| a.path(self.db).cmp(b.path(self.db)));
        for f in &wsp_files {
            let sigs = file_signature_inputs(self.db, *f);
            for s in sigs.iter() {
                body_ctx.add_function_signature(&s.name, s.clone());
            }
        }

        // Extract CTE schemas from the body's WITH clause so that
        // `infer_tableexpr_return_schema` can resolve bare column references
        // from CTE-derived rows. Cycle diagnostics are discarded — they're
        // surfaced separately by `cte_cycle_diagnostics_for_file`.
        //
        // Pass a resolver so that CTE bodies of the form
        // `SELECT * FROM smelt.functions.<name>(args)` have their nested
        // call's output schema resolved and seeded into the CTE context.
        // Without this, `SELECT *` produces a synthetic `col1: Unknown` and
        // any column the inner call adds (e.g. `session_id` from `sessionize`)
        // remains unresolved in the outer body (§Semantics rule 4).
        let path_call_resolver =
            |call: &smelt_parser::ast::SmeltPathCall| -> Option<Vec<(String, TypedColumn)>> {
                self.resolve_smelt_path_call_schema(call)
            };
        let (body_ctx_with_ctes, _cycle_diags) =
            function_body_check::extract_function_body_cte_schemas(
                &body_select,
                &body_ctx,
                "",
                Some(&path_call_resolver),
            );
        let mut body_ctx = body_ctx_with_ctes;

        // Seed JOIN-aliased schemas so that `infer_tableexpr_return_schema`
        // can expand `<alias>.*` projections from joined tables.
        let join_lookup =
            |table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
                self.resolve_table_ref_schema(table_ref)
            };
        function_body_check::register_join_alias_schemas(
            &mut body_ctx,
            &body_select,
            sig,
            &join_lookup,
        );

        let schema = infer_tableexpr_return_schema(&body_select, &body_ctx)?;
        // Project to (name, TypedColumn) pairs.
        let cols: Vec<(String, TypedColumn)> = schema
            .columns
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    }),
                )
            })
            .collect();
        Some(cols)
    }
}

impl RefSchemaProvider for SalsaRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        // Project isolation rule: only consider models declared in the same
        // project as the file whose schema we're computing. Without this
        // filter, a same-named model in another project leaks its column
        // types in. See lib.rs::resolve_ref and the standing CI gate in
        // crates/smelt-lsp/tests/example_workspaces.rs.
        let project = self.project?;
        let upstream = resolve_ref(self.db, self.workspace, project, model_name.to_string())?;
        let resolved = resolved_model_schema(self.db, self.workspace, upstream);
        Some(
            resolved
                .columns
                .iter()
                .map(|col| {
                    let typed_col = col.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    });
                    (col.name.clone(), typed_col)
                })
                .collect(),
        )
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        // Project isolation rule: only consider seeds declared in the same
        // project as the file whose schema we're computing.
        let project = self.project?;
        for seed in project_seeds(self.db, project).iter() {
            if seed.address_segments.join("_") == seed_name {
                return Some(
                    seed.columns
                        .iter()
                        .map(|(name, dt)| {
                            (
                                name.clone(),
                                TypedColumn {
                                    data_type: dt.clone(),
                                    nullable: true,
                                },
                            )
                        })
                        .collect(),
                );
            }
        }
        None
    }

    fn smelt_path_call_columns(
        &self,
        call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        self.resolve_smelt_path_call_schema(call)
    }
}

/// Fully pure `RefSchemaProvider` for batch compilation (CLI, planner). Holds
/// pre-computed maps of model and seed schemas so it can answer lookups
/// without touching Salsa.
pub struct StaticRefSchemaProvider<'a> {
    pub models: &'a HashMap<String, Vec<(String, TypedColumn)>>,
    pub seeds: &'a HashMap<String, Vec<(String, TypedColumn)>>,
}

impl RefSchemaProvider for StaticRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.models.get(model_name).cloned()
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.seeds.get(seed_name).cloned()
    }
}

/// Pure (Salsa-free) builder for a `TypeContext` from a parsed AST and the
/// surrounding source/seed/model schemas.
///
/// This is the canonical builder; the `type_context()` Salsa query is a thin
/// wrapper that gathers Salsa inputs (sources_config, parsed file, upstream
/// schemas) and delegates here. The CLI batch compiler uses
/// `StaticRefSchemaProvider` to call this directly without a `Database`.
///
/// See CLAUDE.md "Pure Function Rule" for why this matters.
pub fn build_type_context(
    file: &AstFile,
    sources_config: &SourcesConfig,
    refs: &dyn RefSchemaProvider,
) -> TypeContext {
    let mut ctx = TypeContext::new();

    // Source columns from sources.yml.
    for source in &sources_config.sources {
        for table in &source.tables {
            for col in &table.columns {
                let data_type = col.data_type.clone().unwrap_or(DataType::Unknown);
                ctx.add_source_column(
                    &source.name,
                    &table.name,
                    &col.name,
                    TypedColumn {
                        data_type,
                        nullable: true,
                    },
                );
            }
        }
    }

    if let Some(select_stmt) = file.select_stmt() {
        // Process WITH clause CTEs first (CTEs shadow outer scope).
        if let Some(with_clause) = select_stmt.with_clause() {
            for cte in with_clause.ctes() {
                if let Some(cte_name) = cte.name() {
                    // For recursive CTEs with explicit column list, bootstrap
                    // with Unknown types so the recursive reference can find
                    // the columns.
                    if with_clause.is_recursive() {
                        for col_name in cte.column_names() {
                            ctx.add_cte_column(
                                &cte_name,
                                &col_name,
                                TypedColumn {
                                    data_type: DataType::Unknown,
                                    nullable: true,
                                },
                            );
                        }
                    }

                    if let Some(cte_select) = cte.query().and_then(|q| q.select_stmt()) {
                        process_from_clause_pure(&cte_select, refs, &mut ctx);
                    }

                    let columns = infer_cte_columns(&cte, &ctx);
                    for (col_name, typed_col) in &columns {
                        ctx.add_cte_column(&cte_name, col_name, typed_col.clone());
                    }

                    ctx.add_alias(&cte_name, &cte_name);

                    // If the CTE body is a wildcard SELECT from a
                    // smelt.functions.* call that couldn't be resolved,
                    // mark the CTE opaque so outer column references
                    // don't cascade false-positive UndeclaredColumn
                    // diagnostics.
                    //
                    // Note: we cannot rely on `columns.is_empty()` here
                    // because `infer_cte_columns` always returns at least
                    // one entry for `SELECT *` (a synthetic "col1" with
                    // Unknown type). Instead, check directly whether the
                    // FROM source is a SMELT_PATH_CALL that didn't resolve.
                    {
                        let should_mark_opaque = cte
                            .query()
                            .and_then(|q| q.select_stmt())
                            .map(|s| {
                                let has_wildcard = s
                                    .select_list()
                                    .map(|sl| sl.items().any(|item| item.is_wildcard()))
                                    .unwrap_or(false);
                                if !has_wildcard {
                                    return false;
                                }
                                // The FROM clause has a smelt path call
                                // that couldn't be resolved.
                                s.from_clause()
                                    .and_then(|fc| {
                                        fc.table_refs().find_map(|tr| tr.smelt_path_call())
                                    })
                                    .map(|path_call| {
                                        refs.smelt_path_call_columns(&path_call).is_none()
                                    })
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false);
                        if should_mark_opaque {
                            ctx.mark_cte_opaque(&cte_name);
                        }
                    }
                }
            }
        }

        process_from_clause_pure(&select_stmt, refs, &mut ctx);
    }

    ctx
}

fn process_from_clause_pure(
    select_stmt: &smelt_parser::ast::SelectStmt,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            process_table_ref_pure(&table_ref, refs, ctx);
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                process_table_ref_pure(&table_ref, refs, ctx);
            }
        }
    }
}

fn process_table_ref_pure(
    table_ref: &TableRef,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    // `smelt.<path>` call-form references (SMELT_PATH_CALL) in the FROM
    // clause. Ask the provider for the inferred TableExpr return schema.
    // If the provider resolves it, seed all columns under the alias binding
    // so the caller's SELECT can validate projections (e.g. UndeclaredColumn
    // for columns not in the function's return schema). If not resolved,
    // fall back to an opaque alias registration.
    if let Some(path_call) = table_ref.smelt_path_call() {
        let segments = path_call.segments();
        let fn_name = segments.last().cloned().unwrap_or_default();
        let bind_to = table_ref.alias().unwrap_or_else(|| fn_name.clone());
        if let Some(cols) = refs.smelt_path_call_columns(&path_call) {
            for (col_name, typed_col) in &cols {
                ctx.add_model_column(&bind_to, col_name, typed_col.clone());
            }
            if !bind_to.is_empty() {
                ctx.add_alias(&bind_to, &bind_to);
            }
        } else if !bind_to.is_empty() {
            // Opaque fallback: alias is registered but no column types.
            ctx.add_alias(&bind_to, &bind_to);
        }
        // Don't fall through to the generic identifier path.
        return;
    }

    // `smelt.<path>` value-form references (SMELT_PATH_REF) in the
    // FROM clause. The path tuple is used to determine the entity name for
    // schema lookup: the full segments joined with "_" are used as the seed
    // key, while the last segment is used for model lookup.
    if let Some(path_ref) = table_ref.smelt_path_ref() {
        let segments = path_ref.segments();
        let model_name = segments.last().cloned().unwrap_or_default();
        let seed_key = segments.join("_");
        // Try seed first, then model.
        if let Some((entity_name, cols)) = refs
            .seed_columns(&seed_key)
            .map(|c| (seed_key.clone(), c))
            .or_else(|| {
                refs.resolved_columns(&model_name)
                    .map(|c| (model_name.clone(), c))
            })
        {
            for (col_name, typed_col) in &cols {
                ctx.add_model_column(&entity_name, col_name, typed_col.clone());
            }
            let bind_to = table_ref.alias().unwrap_or_else(|| entity_name.clone());
            ctx.add_alias(&bind_to, &entity_name);
        } else if segments.first().map(|s| s.as_str()) == Some("sources") {
            // smelt.sources.<source_name>.<table_name> path refs. The source
            // columns were already seeded into `ctx` by `build_type_context`
            // (via `add_source_column`). We just need to register the alias
            // so that `lookup_identifier` can resolve `alias → entity_name`
            // and correctly validate qualified refs like `alias.col_name`
            // as well as bare alias references (e.g. `smelt.functions.f(e)`
            // where `e` is an alias for a source table).
            let bind_to = table_ref.alias().unwrap_or_else(|| model_name.clone());
            ctx.add_alias(&bind_to, &model_name);
        }
        return;
    }

    // CTE references with aliases (e.g. "FROM daily_totals dt")
    // OR bare upstream MODEL/seed references (e.g. "FROM main.stg_orders AS o"
    // — produced by the dialect printer after `smelt.models.stg_orders` is
    // resolved). Without this branch, the alias `o` is never bound and
    // `o.line_revenue` resolves to Unknown, which silently narrows
    // `SUM(o.line_revenue)` to BIGINT in `_smelt_typed`. See B8.
    if table_ref.function_call().is_none() && table_ref.subquery().is_none() {
        if let Some(raw_name) = table_ref.identifier() {
            // Strip an optional leading schema qualifier (`schema.table`).
            // The dialect printer emits `<schema>.<model_name>`; we want the
            // last segment to look up against the schema provider.
            let table_name = bare_table_name(table_ref).unwrap_or(raw_name.clone());

            if ctx.is_cte(&table_name) {
                if let Some(explicit_alias) = table_ref.alias() {
                    ctx.add_alias(&explicit_alias, &table_name);
                }
            } else if let Some(cols) = refs
                .resolved_columns(&table_name)
                .or_else(|| refs.seed_columns(&table_name))
            {
                for (col_name, typed_col) in cols {
                    ctx.add_model_column(&table_name, &col_name, typed_col);
                }
                // Bind the alias (or the table name itself, so qualified
                // refs like `stg_orders.col` also resolve).
                let bind_to = table_ref.alias().unwrap_or_else(|| table_name.clone());
                ctx.add_alias(&bind_to, &table_name);
            }
        }
    }

    // Subqueries / LATERAL subqueries
    if let Some(subquery) = table_ref.subquery() {
        if let Some(alias) = table_ref.alias() {
            if let Some(select_stmt) = subquery.select_stmt() {
                if let Some(select_list) = select_stmt.select_list() {
                    let mut subquery_ctx = ctx.clone();
                    process_from_clause_pure(&select_stmt, refs, &mut subquery_ctx);

                    let column_types = infer_select_column_types(&select_stmt, &subquery_ctx);

                    for (i, item) in select_list.items().enumerate() {
                        let col_name = if let Some(item_alias) = item.alias() {
                            item_alias
                        } else if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                col_ref.name().to_string()
                            } else {
                                format!("col{}", i + 1)
                            }
                        } else {
                            format!("col{}", i + 1)
                        };

                        let typed_col = column_types.get(i).cloned().unwrap_or(TypedColumn {
                            data_type: DataType::Unknown,
                            nullable: true,
                        });

                        ctx.add_cte_column(&alias, &col_name, typed_col);
                    }

                    ctx.add_alias(&alias, &alias);
                }
            }
        }
    }
}

/// Extract the table-name segment from a bare `TableRef` like `schema.table`,
/// stripping an optional schema qualifier. Used by `process_table_ref_pure`
/// to look up upstream MODEL/seed schemas after the dialect printer has
/// resolved `smelt.models.foo` to `<schema>.foo`.
///
/// Returns `None` for function calls and subqueries (those have their own
/// handling paths).
fn bare_table_name(table_ref: &TableRef) -> Option<String> {
    use smelt_parser::SyntaxKind::{AS_KW, DOT, IDENT};

    if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
        return None;
    }

    // Walk tokens and collect the IDENTs that come BEFORE any AS keyword.
    // The last such IDENT (after any DOT segments) is the table name.
    let mut idents: Vec<String> = Vec::new();
    let mut last_was_dot = false;
    let mut started = false;
    for tok in table_ref
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
    {
        match tok.kind() {
            AS_KW => break,
            IDENT => {
                if !started || last_was_dot {
                    idents.push(tok.text().to_string());
                } else {
                    // Implicit alias (no AS keyword): bail out, take what
                    // we have so far.
                    break;
                }
                started = true;
                last_was_dot = false;
            }
            DOT => {
                last_was_dot = true;
            }
            _ => {}
        }
    }

    idents.last().cloned()
}

/// Pure function: populate a `TypeContext` with column type information from
/// Phase 6 per-entity `SourceInfo` records.
///
/// The source's identity in the TypeContext is `(schema, table)` where:
///   schema = `address_segments[address_segments.len() - 2]` (e.g. "raw")
///   table  = `address_segments[address_segments.len() - 1]` (e.g. "users")
///
/// This mirrors how `smelt.sources.raw.users` is resolved: the last two
/// segments of the path are the schema and table.
pub fn add_source_info_to_type_context(sources: &[SourceInfo], ctx: &mut TypeContext) {
    for source in sources {
        let segs = &source.address_segments;
        if segs.len() < 2 {
            continue; // degenerate address — skip
        }
        let schema_name = &segs[segs.len() - 2];
        let table_name = &segs[segs.len() - 1];
        for col in &source.columns {
            ctx.add_source_column(
                schema_name,
                table_name,
                &col.name,
                TypedColumn {
                    data_type: col.data_type.clone(),
                    nullable: col.nullable,
                },
            );
        }
    }
}

#[salsa::tracked(cycle_initial = type_context_initial)]
pub fn type_context(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<TypeContext> {
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    // Phase 6: per-entity sources take precedence when present; fall back to
    // the legacy aggregate `sources.yml` for projects not yet migrated.
    let per_entity_sources: Arc<Vec<SourceInfo>> = project
        .map(|p| project_sources(db, p))
        .unwrap_or_else(|| Arc::new(Vec::new()));

    let legacy_sources: Arc<SourcesConfig> = if per_entity_sources.is_empty() {
        project
            .map(|p| sources_config(db, p))
            .unwrap_or_else(|| Arc::new(SourcesConfig::default()))
    } else {
        Arc::new(SourcesConfig::default())
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(TypeContext::new()),
    };

    let provider = SalsaRefSchemaProvider::new(db, workspace, project);
    let mut ctx = build_type_context(&ast, &legacy_sources, &provider);

    // Phase 6: add per-entity source columns to the TypeContext.
    // Source address_segments like ["sources", "raw", "users"] → schema="raw", table="users".
    add_source_info_to_type_context(&per_entity_sources, &mut ctx);

    // Seed the workspace's `smelt.define` signatures so path-call type
    // inference can resolve declared return types when a SELECT projects a
    // `smelt.functions.*` call. Kept pure — we only hand the signature data to
    // the `TypeContext`; analysis logic doesn't call back into Salsa.
    let mut wsp_files: Vec<SourceFile> = workspace.files(db).to_vec();
    wsp_files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in &wsp_files {
        let sigs = file_signature_inputs(db, *f);
        for sig in sigs.iter() {
            ctx.add_function_signature(&sig.name, sig.clone());
        }
    }

    Arc::new(ctx)
}

/// Pure helper: given a bare CTE name and the `SmeltPathCall` node that is
/// using it as a `TableExpr` argument, find the CTE in the CALLER's `WITH`
/// clause and return its resolved column schema.
///
/// This handles the pattern:
///   WITH x AS (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue, …)
///   SELECT col FROM smelt.functions.f(x)
///
/// Steps:
///   1. Walk up ancestors of `call` to find the nearest `SELECT_STMT`.
///   2. Find the CTE named `cte_name` in the WITH clause.
///   3. Process any preceding CTEs first (so forward references in the target
///      CTE can resolve), then call `infer_cte_columns` on the target CTE.
///
/// Pure — no Salsa access.
fn cte_columns_from_caller_select(
    cte_name: &str,
    call: &smelt_parser::ast::SmeltPathCall,
) -> Option<Vec<(String, TypedColumn)>> {
    use smelt_parser::ast::{Cte, SelectStmt, WithClause};
    use smelt_parser::SyntaxKind::SELECT_STMT;

    // Walk up to find the nearest SELECT_STMT ancestor (the caller's select).
    let caller_select = call
        .syntax()
        .ancestors()
        .find(|n| n.kind() == SELECT_STMT)
        .and_then(SelectStmt::cast)?;

    let with_clause: WithClause = caller_select.with_clause()?;

    // Collect all CTEs in order; process them in order so that each CTE can
    // reference preceding ones.
    let all_ctes: Vec<Cte> = with_clause.ctes().collect();

    // Build a TypeContext by processing preceding CTEs in order, then the
    // target CTE.
    let mut ctx = TypeContext::new();
    for cte in &all_ctes {
        let name = match cte.name() {
            Some(n) => n,
            None => continue,
        };
        let cols = infer_cte_columns(cte, &ctx);
        for (col_name, typed_col) in &cols {
            ctx.add_cte_column(&name, col_name, typed_col.clone());
        }
        ctx.add_alias(&name, &name);
        if name == cte_name {
            // Found the target CTE — return its columns.
            return Some(cols);
        }
    }

    // CTE name not found in the WITH clause.
    None
}

/// Walk a syntax node for the first `SMELT_PATH_CALL_STAR` child and return the
/// `SmeltPathCall` it wraps.
///
/// The SELECT-item structure for `smelt.functions.<f>(args).*` is:
///   `SELECT_ITEM → EXPRESSION → SMELT_PATH_CALL_STAR → SMELT_PATH_CALL`
///
/// Pure — no Salsa access.
fn find_inner_path_call_of_star(
    expr_node: &smelt_parser::syntax_kind::SyntaxNode,
) -> Option<smelt_parser::ast::SmeltPathCall> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::{SMELT_PATH_CALL, SMELT_PATH_CALL_STAR};

    // expr_node is the EXPRESSION node. Look for a SMELT_PATH_CALL_STAR child.
    for child in expr_node.children() {
        if child.kind() == SMELT_PATH_CALL_STAR {
            // The SMELT_PATH_CALL_STAR contains the inner SMELT_PATH_CALL.
            for inner in child.children() {
                if inner.kind() == SMELT_PATH_CALL {
                    return SmeltPathCall::cast(inner);
                }
            }
        }
    }
    None
}

/// Pure helper: scan a SELECT statement's items for `smelt.functions.<f>(args).*`
/// (`SMELT_PATH_CALL_STAR`) spread expressions and expand the function's struct
/// return fields into `Column` entries.
///
/// This implements §"Struct returns and `.*` spread" from
/// `docs/specs/function_schema_inference.md`: the schema layer must expand
/// `<name>(args).*` into the declared struct fields (in declared order) rather
/// than recording zero columns for the spread.
///
/// `infer_smelt_path_call_type` handles row-tail (`Struct<{…, ..r}>`) binding
/// from the call-site `TypeContext` — the extras are already folded into the
/// returned `DataType::Struct`.
///
/// Pure — no Salsa access; obeys the smelt-db pure-function rule.
fn collect_struct_spread_columns(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
) -> Vec<Column> {
    use crate::type_inference::function_call::infer_smelt_path_call_type;
    use smelt_types::signatures::{SmeltType, StructRowTail};

    let Some(select_list) = select_stmt.select_list() else {
        return vec![];
    };

    let mut cols = Vec::new();

    for item in select_list.items() {
        // Wildcard items (`*` or `table.*`) are handled via row_extensions.
        if item.is_wildcard() {
            continue;
        }
        // Regular named columns are already captured in the base schema.
        // SMELT_PATH_CALL_STAR items have no resolvable column name because
        // `infer_name()` cannot produce a name for the `.*` construct.
        if item.column_name().is_some() {
            continue;
        }

        // Find a SMELT_PATH_CALL_STAR inside the expression node.
        let Some(expr) = item.expression() else {
            continue;
        };
        let Some(inner_call) = find_inner_path_call_of_star(expr.syntax()) else {
            continue;
        };

        // Guard: only expand CLOSED structs at the schema layer.
        //
        // A row-tail (`Struct<{…, ..r}>`) return means the codegen expander
        // (`expand_smelt_path_call_star`) falls back to verbatim SQL when a
        // SPREAD_ITEM is present in the function body. Expanding at the schema
        // layer while codegen falls back to verbatim would violate invariant 2
        // (schema-layer/codegen agreement). Until codegen and schema expansion
        // are unified for row-tail structs, skip expansion and contribute zero
        // columns — matching the pre-existing verbatim-fallback behaviour.
        //
        // Closed-struct check: consult the function signature in the TypeContext.
        // `infer_smelt_path_call_type` has already folded row-tail extras into a
        // concrete `DataType::Struct`, so we cannot distinguish closed from tail
        // from the return value alone — we must read the signature.
        let fn_name = inner_call.segments().last().cloned().unwrap_or_default();
        let is_closed_struct = if let Some(sig) = ctx.lookup_function_signature(&fn_name) {
            matches!(
                &sig.return_type,
                Some(Ok(SmeltType::Struct {
                    tail: StructRowTail::None,
                    ..
                }))
            )
        } else {
            // Signature not found — cannot confirm closed; skip expansion.
            false
        };
        if !is_closed_struct {
            continue;
        }

        // Resolve the call's return type. For a closed `Struct<{f1: T1, …, fN: TN}>`,
        // `infer_smelt_path_call_type` returns a concrete `DataType::Struct`.
        let Some(typed_col) = infer_smelt_path_call_type(&inner_call, ctx) else {
            continue;
        };

        if let DataType::Struct(fields) = typed_col.data_type {
            for (field_name, field_type) in fields {
                cols.push(Column {
                    name: field_name,
                    alias: None,
                    source: ColumnSource::Computed,
                    expression: String::new(),
                    range: item.range(),
                    data_type: Some(TypedColumn {
                        data_type: field_type,
                        nullable: true,
                    }),
                });
            }
        }
        // Non-struct or unresolved return → no columns contributed (no panic).
    }

    cols
}

#[salsa::tracked(cycle_initial = typed_model_schema_initial)]
pub fn typed_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ModelSchema> {
    let base_schema = model_schema(db, file);
    let ctx = type_context(db, workspace, file);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return base_schema,
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return base_schema,
    };

    let inferred_types = infer_select_column_types(&select_stmt, &ctx);

    let mut typed_columns = Vec::new();
    for (i, col) in base_schema.columns.iter().enumerate() {
        let mut col = col.clone();
        if let Some(typed_col) = inferred_types.get(i) {
            col.data_type = Some(typed_col.clone());
        }
        typed_columns.push(col);
    }

    // Append columns contributed by `smelt.functions.<f>(args).*` struct spreads.
    // These are not in `base_schema.columns` (they were skipped by `model_schema`
    // because the `.*` item has no single resolvable column name), so we detect
    // them separately and append their struct fields in declared order.
    let spread_cols = collect_struct_spread_columns(&select_stmt, &ctx);
    typed_columns.extend(spread_cols);

    Arc::new(ModelSchema {
        columns: typed_columns,
        row_extensions: base_schema.row_extensions.clone(),
        input_constraints: base_schema.input_constraints.clone(),
    })
}

#[salsa::tracked(cycle_initial = resolved_model_schema_initial)]
pub fn resolved_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ResolvedSchema> {
    let typed_schema = typed_model_schema(db, workspace, file);

    if typed_schema.row_extensions.is_empty() {
        return Arc::new(ResolvedSchema {
            columns: typed_schema.columns.clone(),
            is_fully_resolved: true,
            unresolved_extensions: vec![],
        });
    }

    let mut columns = Vec::new();
    let mut unresolved_extensions = Vec::new();
    let mut is_fully_resolved = true;

    let project = find_project(db, workspace, &file.project_root(db).clone());

    for ext in &typed_schema.row_extensions {
        if let Some(upstream) =
            project.and_then(|p| resolve_ref(db, workspace, p, ext.ref_name.clone()))
        {
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            for col in &upstream_resolved.columns {
                if !ext.excluded_columns.contains(&col.name) {
                    columns.push(col.clone());
                }
            }
            if !upstream_resolved.is_fully_resolved {
                is_fully_resolved = false;
                for upstream_ext in &upstream_resolved.unresolved_extensions {
                    unresolved_extensions.push(upstream_ext.clone());
                }
            }
        } else {
            is_fully_resolved = false;
            unresolved_extensions.push(ext.clone());
        }
    }

    for col in &typed_schema.columns {
        columns.push(col.clone());
    }

    Arc::new(ResolvedSchema {
        columns,
        is_fully_resolved,
        unresolved_extensions,
    })
}

/// Phase C (meta-language) — Salsa-cached query that materialises the concrete
/// `Vec<ColumnRefValue>` for a given model path at expansion time.
///
/// Takes a `model_name` (the leaf segment of a `smelt.<path>` reference, e.g.
/// `"orders"`) and resolves it via [`resolve_ref`] + [`resolved_model_schema`] to
/// produce a [`smelt_types::ColumnRefValue`] per column, preserving the source
/// schema's declared column order.
///
/// Returns `Ok(columns)` when the schema resolves to a concrete (non-empty)
/// column list, or `Err(())` when:
/// - the model is not found in the workspace (`resolve_ref` returns `None`), or
/// - the resolved schema has no columns and is not fully resolved (upstream
///   `Unknown`).
///
/// The error token is `()` — callers emit `ColumnsOfUnresolvableSchema` and
/// drop the surrounding splice on `Err`.
///
/// This query obeys the `smelt-db` pure-function rule: the analysis (building
/// `ColumnRefValue`s from `Column`s) is pure; the Salsa wrapper only wires up
/// the inputs.
#[salsa::tracked]
pub fn columns_of_for_table_expr(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: crate::ProjectInput,
    model_name: String,
) -> Result<Arc<Vec<smelt_types::ColumnRefValue>>, ()> {
    // Resolve the model name to a SourceFile via the existing resolution machinery.
    let file = match resolve_ref(db, workspace, project, model_name.clone()) {
        Some(f) => f,
        None => return Err(()),
    };

    // Use the typed + row-extended schema so callers see the full column list
    // (including wildcard-expanded columns from upstream models).
    let schema = resolved_model_schema(db, workspace, file);

    // If the schema is unresolvable (no columns and not fully resolved), signal
    // an unknown schema so the caller can emit ColumnsOfUnresolvableSchema.
    if schema.columns.is_empty() && !schema.is_fully_resolved {
        return Err(());
    }

    // Project each Column into a ColumnRefValue.
    let columns: Vec<smelt_types::ColumnRefValue> = columns_to_column_ref_values(&schema.columns);

    Ok(Arc::new(columns))
}

/// Pure helper: convert a slice of [`schema::Column`]s into
/// [`smelt_types::ColumnRefValue`]s in declaration order.
///
/// This function is deliberately separated from the Salsa query body so that
/// the conversion logic is independently testable without a database.
pub fn columns_to_column_ref_values(
    columns: &[schema::Column],
) -> Vec<smelt_types::ColumnRefValue> {
    columns
        .iter()
        .map(|col| {
            let data_type = col.data_type.as_ref().map(|tc| tc.data_type.clone());
            let is_numeric = data_type
                .as_ref()
                .map(|dt| dt.is_numeric())
                .unwrap_or(false);
            smelt_types::ColumnRefValue {
                name: col.name.clone(),
                data_type,
                is_numeric,
                source_span: Some(col.range),
            }
        })
        .collect()
}

#[salsa::tracked]
pub fn model_input_constraints(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<InputConstraint>> {
    use schema::{ColumnConstraint, InputConstraint};

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ctx = type_context(db, workspace, file);

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(vec![]),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(vec![]),
    };

    let mut alias_to_ref: HashMap<String, String> = HashMap::new();
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            // smelt.<path> table references in FROM position.
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segs = path_ref.segments();
                if let Some(entity_name) = segs.last().cloned() {
                    if !entity_name.is_empty() {
                        alias_to_ref.insert(entity_name.clone(), entity_name.clone());
                        if let Some(alias) = table_ref.alias() {
                            alias_to_ref.insert(alias, entity_name);
                        }
                    }
                }
            }
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                // smelt.<path> table references in JOIN position.
                if let Some(path_ref) = table_ref.smelt_path_ref() {
                    let segs = path_ref.segments();
                    if let Some(entity_name) = segs.last().cloned() {
                        if !entity_name.is_empty() {
                            alias_to_ref.insert(entity_name.clone(), entity_name.clone());
                            if let Some(alias) = table_ref.alias() {
                                alias_to_ref.insert(alias, entity_name);
                            }
                        }
                    }
                }
            }
        }
    }

    if alias_to_ref.is_empty() {
        return Arc::new(vec![]);
    }

    let mut constraints_map: HashMap<String, HashMap<String, ColumnConstraint>> = HashMap::new();

    let mut record_constraint =
        |ref_name: &str, col_name: &str, expected_type: Option<TypedColumn>, range: TextRange| {
            let entry = constraints_map
                .entry(ref_name.to_string())
                .or_default()
                .entry(col_name.to_string())
                .or_insert_with(|| ColumnConstraint {
                    expected_type: None,
                    usage_sites: vec![],
                });
            if entry.expected_type.is_none() {
                entry.expected_type = expected_type;
            }
            entry.usage_sites.push(range);
        };

    {
        let mut visitor = |qualifier: Option<&str>,
                           col_name: &str,
                           type_hint: Option<&TypedColumn>,
                           range: TextRange| {
            if col_name == "*" {
                return;
            }
            let inferred_type = type_hint.cloned();
            if let Some(q) = qualifier {
                let resolved = ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
                if let Some(ref_name) = alias_to_ref.get(&resolved) {
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(Some(q), col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            } else {
                let unique_refs: std::collections::HashSet<&String> =
                    alias_to_ref.values().collect();
                if unique_refs.len() == 1 {
                    let ref_name = alias_to_ref
                        .values()
                        .next()
                        .expect("unique_refs.len() == 1 guarantees at least one value");
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(None, col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            }
        };

        type_inference::walk_select_columns_with_visitor(&select_stmt, &ctx, None, &mut visitor);
    }

    let constraints: Vec<InputConstraint> = constraints_map
        .into_iter()
        .map(|(ref_name, required_columns)| InputConstraint {
            ref_name,
            required_columns,
        })
        .collect();

    Arc::new(constraints)
}

#[salsa::tracked]
pub fn model_function_type(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<schema::ModelFunctionType> {
    use schema::{FunctionInput, FunctionOutput, TypedField};

    let path = file.path(db);
    let model_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let input_constraints = model_input_constraints(db, workspace, file);

    let mut inputs: Vec<FunctionInput> = input_constraints
        .iter()
        .map(|ic| {
            let mut columns: Vec<TypedField> = ic
                .required_columns
                .iter()
                .map(|(col_name, constraint)| TypedField {
                    name: col_name.clone(),
                    constraint: constraint.expected_type.clone(),
                })
                .collect();
            columns.sort_by(|a, b| a.name.cmp(&b.name));
            FunctionInput {
                ref_name: ic.ref_name.clone(),
                columns,
            }
        })
        .collect();

    inputs.sort_by(|a, b| a.ref_name.cmp(&b.ref_name));

    let typed_schema = typed_model_schema(db, workspace, file);

    let outputs: Vec<FunctionOutput> = typed_schema
        .columns
        .iter()
        .filter(|col| col.name != "*")
        .map(|col| FunctionOutput {
            name: col.name.clone(),
            data_type: col.data_type.clone(),
        })
        .collect();

    let has_wildcard_output = !typed_schema.row_extensions.is_empty();

    Arc::new(schema::ModelFunctionType {
        model_name,
        inputs,
        outputs,
        has_wildcard_output,
    })
}
