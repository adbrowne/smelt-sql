use super::*;
use crate::DataType;

/// Format a [`SmeltType`] as a concise hover string (Phase 18).
///
/// Used by the LSP hover handler to display parameter types in
/// `smelt.define` signatures. Examples:
///   - `Expr<Integer>` → `"Expr<Integer>"`
///   - `Expr<Numeric>` → `"Expr<Numeric>"`
///   - `TableExpr` → `"TableExpr"`
///   - `TableExpr<{revenue: Numeric, cost: Numeric}>` → `"TableExpr<{revenue: Numeric, cost: Numeric}>"`
///   - With named tail `..r` → `"TableExpr<{revenue: Numeric, ..r}>"`
pub fn format_smelt_type_hover(ty: &SmeltType) -> String {
    match ty {
        SmeltType::Expr(tc) => format!("Expr<{}>", format_type_constraint_hover(tc)),
        SmeltType::List(inner) => format!("List<{}>", format_smelt_type_hover(inner)),
        SmeltType::Lambda(params, body_ty) => {
            let body_str = format_smelt_type_hover(body_ty);
            if params.len() == 1 {
                format!(
                    "Lambda<{}, {}>",
                    format_smelt_type_hover(&params[0]),
                    body_str
                )
            } else {
                let params_str = params
                    .iter()
                    .map(format_smelt_type_hover)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Lambda<({params_str}), {body_str}>")
            }
        }
        SmeltType::TableExpr(None) => "TableExpr".to_string(),
        SmeltType::TableExpr(Some(req)) => {
            let mut s = String::from("TableExpr<{");
            for (i, (col, col_req, _not_null)) in req.required.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(col);
                s.push_str(": ");
                s.push_str(&col_req.render());
            }
            match &req.tail {
                RowTail::None => {}
                RowTail::Anon => {
                    if !req.required.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                }
                RowTail::Named(name) => {
                    if !req.required.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                    s.push_str(name);
                }
            }
            s.push_str("}>");
            s
        }
        SmeltType::SelectItems { kind, context } => {
            let kind_str = match kind {
                ExprKind::Scalar => "Scalar",
                ExprKind::Agg => "Agg",
                ExprKind::Window => "Window",
            };
            if let Some(ctx) = context {
                format!("SelectItems<{}, {}>", kind_str, ctx.name())
            } else {
                format!("SelectItems<{}>", kind_str)
            }
        }
        SmeltType::Struct { fields, tail } => {
            let mut s = String::from("Expr<Struct<{");
            for (i, (name, dt)) in fields.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(name);
                s.push_str(": ");
                s.push_str(&dt.to_string());
            }
            match tail {
                StructRowTail::None => {}
                StructRowTail::Anon => {
                    if !fields.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                }
                StructRowTail::Named(name) => {
                    if !fields.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str("..");
                    s.push_str(name);
                }
            }
            s.push_str("}>");
            s.push('>');
            s
        }
        SmeltType::Unknown => "Unknown".to_string(),
        SmeltType::ColumnRef => "ColumnRef".to_string(),
        SmeltType::ModelRef => "ModelRef".to_string(),
        SmeltType::SourceRef => "SourceRef".to_string(),
        SmeltType::ModelDef => "ModelDef".to_string(),
        SmeltType::Record { fields, name } => {
            if let Some(n) = name {
                n.clone()
            } else {
                let field_str: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", format_smelt_type_hover(v)))
                    .collect();
                format!("Record<{{{}}}>", field_str.join(", "))
            }
        }
        SmeltType::Map { key, value } => {
            format!(
                "Map<{}, {}>",
                format_smelt_type_hover(key),
                format_smelt_type_hover(value)
            )
        }
    }
}

fn format_type_constraint_hover(tc: &TypeConstraint) -> String {
    match tc {
        TypeConstraint::Concrete(dt) => dt.to_sql().to_string(),
        TypeConstraint::Numeric => "Numeric".to_string(),
        TypeConstraint::Ordered => "Ordered".to_string(),
        TypeConstraint::Any => "Any".to_string(),
    }
}

/// A cache-key-safe wrapper around a list of parameter (name, DataType) bindings.
/// Used as a Salsa cache key for per-(callee, arg-types) expansion caching (Phase 26+).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DataTypeHash(pub Vec<(String, DataType)>);

impl DataTypeHash {
    pub fn new(bindings: Vec<(String, DataType)>) -> Self {
        Self(bindings)
    }
}
