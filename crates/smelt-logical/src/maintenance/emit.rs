//! Physical maintenance SQL emission — v0 tracer bullet.
//!
//! One emitter per [`Technique`](super::Technique), following the
//! physical-maintenance notation of
//! `docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`:
//! the partition predicate is carried on **both** the scan and the write
//! target wherever the op is region-scoped — a predicate stated only on one
//! side is a logical bound the storage layer cannot use
//! (`01-framework.md` §5).
//!
//! Emission is pure string construction over a caller-supplied SELECT body
//! (the model SQL with source refs resolved to physical table names); clamp
//! *injection into* the body is the runtime transformer's job
//! (`smelt-runtime/src/transformer.rs`) and is deliberately not duplicated
//! here.

/// A half-open region `[start, end)` on the output partition column; values
/// are SQL literals (already quoted where needed).
#[derive(Debug, Clone)]
pub struct Region {
    pub start: String,
    pub end: String,
}

impl Region {
    fn predicate(&self, qualifier: Option<&str>, column: &str) -> String {
        let col = match qualifier {
            Some(q) => format!("{q}.{column}"),
            None => column.to_string(),
        };
        format!(
            "{col} >= {start} AND {col} < {end}",
            start = self.start,
            end = self.end
        )
    }
}

/// Recompute-a-region (bottom-right): `DELETE` exactly the write window,
/// `INSERT` its recompute. The same predicate bounds both statements — the
/// DELETE range must equal exactly what the INSERT writes.
pub fn emit_delete_insert(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
) -> Vec<String> {
    let pred = region.predicate(None, partition_col);
    vec![
        format!("DELETE FROM {table} WHERE {pred}"),
        format!("INSERT INTO {table} SELECT * FROM ({body}) WHERE {pred}"),
    ]
}

/// Column-scoped re-derivation (bottom-left): a keyed `MERGE` writing only
/// `columns`, leaving skeleton and siblings in place. `region` scopes both
/// the source scan and the merge target when the op is partition-bounded;
/// `None` is the declared full-scan case (K8 `allow_full_scan`).
pub fn emit_column_scoped_merge(
    table: &str,
    key: &[String],
    columns: &[String],
    source_select: &str,
    partition_col: Option<&str>,
    region: Option<&Region>,
) -> Vec<String> {
    let mut using = format!("SELECT * FROM ({source_select})");
    let mut on: Vec<String> = key.iter().map(|k| format!("t.{k} = s.{k}")).collect();
    if let (Some(p), Some(r)) = (partition_col, region) {
        // Partition predicate on the scan side...
        using.push_str(&format!(" WHERE {}", r.predicate(None, p)));
        // ...and on the write target, so the engine prunes both.
        on.push(r.predicate(Some("t"), p));
    }
    let sets = columns
        .iter()
        .map(|c| format!("{c} = s.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![format!(
        "MERGE INTO {table} t USING ({using}) s ON {on} \
         WHEN MATCHED THEN UPDATE SET {sets}",
        on = on.join(" AND ")
    )]
}

/// In-place field backfill (top-left with an empty input delta): `UPDATE`
/// the stored region from its own columns; no upstream read at all.
pub fn emit_in_place_update(
    table: &str,
    assignments: &[(String, String)],
    partition_col: &str,
    region: &Region,
) -> Vec<String> {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![format!(
        "UPDATE {table} SET {sets} WHERE {}",
        region.predicate(None, partition_col)
    )]
}

/// Fold-a-delta into keyed end-state (top-left): `MERGE` the delta aggregate
/// into stored key state, combining additively on matched keys and inserting
/// unseen keys. `delta_select` computes the model's aggregate over exactly
/// the delta (the never-fold-twice obligation is the ledger's, not this
/// statement's).
pub fn emit_keyed_fold(
    table: &str,
    key: &[String],
    add_columns: &[String],
    all_columns: &[String],
    delta_select: &str,
) -> Vec<String> {
    let on = key
        .iter()
        .map(|k| format!("t.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sets = add_columns
        .iter()
        .map(|c| format!("{c} = t.{c} + s.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let cols = all_columns.join(", ");
    let vals = all_columns
        .iter()
        .map(|c| format!("s.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![format!(
        "MERGE INTO {table} t USING ({delta_select}) s ON {on} \
         WHEN MATCHED THEN UPDATE SET {sets} \
         WHEN NOT MATCHED THEN INSERT ({cols}) VALUES ({vals})"
    )]
}
