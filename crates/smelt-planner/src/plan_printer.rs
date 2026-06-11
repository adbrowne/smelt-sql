//! Deterministic textual rendering of a logical plan tree.
//!
//! [`format_plan`] is invoked by `smelt build --show-plan` to display the
//! result of running the Phase 32–34 logical-plan rules. The output shape
//! is deliberately stable: callers grep on it in tests, so any field order
//! changes here are observable as test failures.

use std::fmt::Write;

use crate::logical::{LogicalNode, Plan};

/// Render `plan` as a deterministic indented tree.
///
/// Output is line-oriented; each node occupies one line plus zero or more
/// child lines indented two spaces. No `HashMap`, `Instant`, or pointer
/// addresses contribute to the output, so two identical plans always print
/// byte-for-byte identical strings.
pub fn format_plan(plan: &Plan) -> String {
    let mut out = String::new();
    render(plan, 0, &mut out);
    out
}

fn render(node: &Plan, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match node.as_ref() {
        LogicalNode::Select {
            projections,
            from,
            filter,
        } => {
            // infallible: String::write_str never returns Err
            let _ = writeln!(out, "{pad}Select projections={projections:?}");
            if let Some(f) = filter {
                let _ = writeln!(out, "{pad}  filter:");
                render(f, indent + 2, out);
            }
            if let Some(src) = from {
                let _ = writeln!(out, "{pad}  from:");
                render(src, indent + 2, out);
            }
        }
        LogicalNode::FunctionCall {
            fn_id,
            args,
            transparent,
            provenance,
            properties,
            pushed_filter,
            body,
        } => {
            let _ = writeln!(
                out,
                "{pad}FunctionCall fn_id={fn_id:?} transparent={transparent} \
                 deterministic={det} idempotent={idem} append_only={ao} \
                 needs_cast={cast} provenance={prov} pushed_filter={pf} args={argc} body={body_present}",
                det = properties.deterministic,
                idem = properties.idempotent,
                ao = properties.append_only,
                cast = properties.needs_cast,
                prov = format_provenance(provenance),
                pf = if pushed_filter.is_some() {
                    "Some(_)"
                } else {
                    "None"
                },
                argc = args.len(),
                body_present = if body.is_some() { "Some(_)" } else { "None" },
            );
            for arg in args {
                render(arg, indent + 1, out);
            }
        }
        LogicalNode::ExpandedCall {
            fn_id,
            provenance,
            properties,
            pushed_filter,
            body,
        } => {
            let _ = writeln!(
                out,
                "{pad}ExpandedCall fn_id={fn_id:?} \
                 deterministic={det} idempotent={idem} append_only={ao} \
                 needs_cast={cast} provenance={prov} pushed_filter={pf}",
                det = properties.deterministic,
                idem = properties.idempotent,
                ao = properties.append_only,
                cast = properties.needs_cast,
                prov = format_provenance(provenance),
                pf = if pushed_filter.is_some() {
                    "Some(_)"
                } else {
                    "None"
                },
            );
            if let Some(b) = body {
                let _ = writeln!(out, "{pad}  body:");
                render(b, indent + 2, out);
            }
        }
        LogicalNode::Tagged { tag, inner } => {
            let _ = writeln!(out, "{pad}Tagged tag={tag:?}");
            render(inner, indent + 1, out);
        }
        LogicalNode::SpliceList(items) => {
            let _ = writeln!(out, "{pad}SpliceList len={}", items.len());
            for it in items {
                render(it, indent + 1, out);
            }
        }
        LogicalNode::Raw { sql_text } => {
            let _ = writeln!(out, "{pad}Raw sql_text={sql_text:?}");
        }
        LogicalNode::TableRef { name } => {
            let _ = writeln!(out, "{pad}TableRef name={name:?}");
        }
        LogicalNode::Literal(dt) => {
            let _ = writeln!(out, "{pad}Literal {dt:?}");
        }
        LogicalNode::Cast { inner, target_type } => {
            let _ = writeln!(out, "{pad}Cast target_type={target_type:?}");
            render(inner, indent + 1, out);
        }
        LogicalNode::LeftJoin {
            lhs,
            rhs,
            join_columns,
            cardinality,
            output_columns,
        } => {
            let _ = writeln!(
                out,
                "{pad}LeftJoin join_columns={join_columns:?} \
                 cardinality={cardinality:?} output_columns={output_columns:?}"
            );
            let _ = writeln!(out, "{pad}  lhs:");
            render(lhs, indent + 2, out);
            let _ = writeln!(out, "{pad}  rhs:");
            render(rhs, indent + 2, out);
        }
    }
}

fn format_provenance(p: &crate::logical::Provenance) -> String {
    match p {
        crate::logical::Provenance::Unknown => "Unknown".to_string(),
        crate::logical::Provenance::Declared(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(col, srcs)| format!("{col}=[{}]", srcs.join(", ")))
                .collect();
            format!("Declared({{{}}})", parts.join(", "))
        }
    }
}
