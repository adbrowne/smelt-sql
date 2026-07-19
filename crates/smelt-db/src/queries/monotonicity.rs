//! Nullability gate for the pure event-time monotonicity trace.
//!
//! `smelt_logical::trace_event_time` is a pure structural classifier with no
//! visibility into column nullability (it lives below `smelt-db`). A
//! `Traceable` verdict whose leaf source column can be `NULL` is unsound to
//! push down: a full refresh keeps NULL-event-time rows, but a pushed window
//! filter silently drops them (docs/specs/model_properties.md §"Event-time
//! monotonicity trace" and the column-nullability-gate row; audit §2.5/P3 in
//! column form).
//!
//! This module is the "thin Salsa wrapper composes inputs, calls the pure
//! function" pattern (architecture.md §"Salsa purity rule"): [`gate_nullable_leaf`]
//! is pure and independently testable; [`trace_event_time_checked`] is the
//! query-shaped wrapper that resolves the leaf column's nullability from
//! smelt-db's inferred schema and calls it.

use smelt_logical::{BoundContext, EventTimeTrace, NotTraceableKind};

use crate::queries::project::project_sources;
use crate::queries::schema::typed_model_schema;
use crate::{resolve_ref_path, RefKind, Workspace};

/// Downgrade a `Traceable` verdict to `NotTraceable` when the traced leaf
/// source column is nullable, or when its nullability cannot be resolved.
/// Fail-closed (Constraint 12): a downgrade only ever narrows eligibility,
/// so it is always sound; keeping `Traceable` on unknown nullability would
/// not be. `StaticSeed` / `NotTraceable` verdicts pass through unchanged —
/// the pure primitive already claims nothing for them.
pub fn gate_nullable_leaf(trace: EventTimeTrace, leaf_nullable: Option<bool>) -> EventTimeTrace {
    let (source, source_column) = match &trace {
        EventTimeTrace::Traceable {
            source,
            source_column,
            ..
        } => (source.clone(), source_column.clone()),
        EventTimeTrace::StaticSeed { .. } | EventTimeTrace::NotTraceable { .. } => return trace,
    };
    match leaf_nullable {
        Some(false) => trace,
        Some(true) => EventTimeTrace::NotTraceable {
            reason: format!("event-time leaf column {source}.{source_column} is nullable"),
            kind: NotTraceableKind::Disproven,
        },
        None => EventTimeTrace::NotTraceable {
            reason: format!(
                "event-time leaf column {source}.{source_column} nullability could not be resolved"
            ),
            kind: NotTraceableKind::Disproven,
        },
    }
}

/// Resolve `column_name`'s `nullable` flag on the model/source addressed by
/// `source_ref` (a `BoundContext` source name, e.g. `"silver.events_parsed"`).
/// Declared `.yml` sources (`RefKind::Source`) read the `nullable:` flag
/// declared on the column directly; SQL models read smelt-db's inferred
/// `ModelSchema`. Returns `None` — "unresolvable" — when the ref can't be
/// resolved, the address doesn't match a known source/model, or the column
/// isn't in its schema / has no inferred type. Callers must treat `None` as
/// fail-closed, not as "assume non-null".
fn resolve_leaf_nullability(
    db: &dyn salsa::Database,
    workspace: Workspace,
    source_ref: &str,
    column_name: &str,
) -> Option<bool> {
    let segments: Vec<String> = source_ref
        .strip_prefix("smelt.")
        .unwrap_or(source_ref)
        .split('.')
        .map(|s| s.to_string())
        .collect();
    let resolved = resolve_ref_path(db, workspace, segments)?;
    match resolved.kind {
        RefKind::Source => workspace.projects(db).iter().copied().find_map(|project| {
            project_sources(db, project)
                .iter()
                .find(|s| s.address_segments == resolved.path)
                .and_then(|s| s.columns.iter().find(|c| c.name == column_name))
                .map(|c| c.nullable)
        }),
        _ => {
            let file = resolved.source_file?;
            let schema = typed_model_schema(db, workspace, file);
            let col = schema.find_column(column_name)?;
            col.data_type.as_ref().map(|t| t.nullable)
        }
    }
}

/// Trace `event_time_expr`'s monotonicity, then narrow the verdict by the
/// nullability of its traced leaf column. Thin composition of the pure
/// `smelt_logical::trace_event_time` classifier with smelt-db's schema
/// resolution (Salsa purity rule: no analysis logic duplicated here).
pub fn trace_event_time_checked(
    db: &dyn salsa::Database,
    workspace: Workspace,
    event_time_expr: &smelt_parser::Expr,
    ctx: &BoundContext,
) -> EventTimeTrace {
    let trace = smelt_logical::trace_event_time(event_time_expr, ctx);
    let leaf_nullable = match &trace {
        EventTimeTrace::Traceable {
            source,
            source_column,
            ..
        } => resolve_leaf_nullability(db, workspace, source, source_column),
        EventTimeTrace::StaticSeed { .. } | EventTimeTrace::NotTraceable { .. } => {
            return trace;
        }
    };
    gate_nullable_leaf(trace, leaf_nullable)
}
