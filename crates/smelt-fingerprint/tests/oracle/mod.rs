//! DuckDB relation oracle (test-only).
//!
//! Executes SQL against an in-memory DuckDB and captures the full result as a
//! [`Relation`] — a multiset of rows keyed by column name. [`relations_equal`]
//! is the soundness check the fingerprint is proven against: two queries are
//! equal as relations iff they have the same column-name set and the same
//! multiset of rows (columns matched by **name**, not position).
//!
//! Comparing by name is what makes a projection reorder (`SELECT a, b` vs
//! `SELECT b, a`) a genuine relation equivalence — which is exactly the
//! equivalence the canonicaliser is allowed to recognise.
//!
//! Extends the schema-only pattern in
//! `crates/smelt-db/tests/prop_helpers/duckdb_oracle.rs` to also materialise
//! rows.

#![allow(dead_code)] // shared across multiple test binaries; not all use every item

use duckdb::types::Value;
use duckdb::Connection;
use std::collections::BTreeMap;

/// A normalised SQL cell value with a total order, so rows can be sorted and
/// compared as a multiset. Numeric widths collapse to a single space (all
/// integers → `i128`, all floats → canonical bits) so width-only differences do
/// not spuriously break equality; everything exotic (decimal, timestamp, list,
/// struct, …) is captured by its stable `Debug` rendering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanonCell {
    Null,
    Bool(bool),
    Int(i128),
    /// f64 bit pattern (with `-0.0` normalised to `+0.0`).
    Float(u64),
    Text(String),
    /// Fallback for types we do not model structurally; stable `Debug` text.
    Other(String),
}

impl CanonCell {
    fn from_value(v: Value) -> Self {
        match v {
            Value::Null => CanonCell::Null,
            Value::Boolean(b) => CanonCell::Bool(b),
            Value::TinyInt(i) => CanonCell::Int(i as i128),
            Value::SmallInt(i) => CanonCell::Int(i as i128),
            Value::Int(i) => CanonCell::Int(i as i128),
            Value::BigInt(i) => CanonCell::Int(i as i128),
            Value::HugeInt(i) => CanonCell::Int(i),
            Value::UTinyInt(i) => CanonCell::Int(i as i128),
            Value::USmallInt(i) => CanonCell::Int(i as i128),
            Value::UInt(i) => CanonCell::Int(i as i128),
            Value::UBigInt(i) => CanonCell::Int(i as i128),
            Value::Float(f) => CanonCell::Float(norm_bits(f as f64)),
            Value::Double(f) => CanonCell::Float(norm_bits(f)),
            Value::Text(s) => CanonCell::Text(s),
            other => CanonCell::Other(format!("{other:?}")),
        }
    }
}

fn norm_bits(f: f64) -> u64 {
    if f == 0.0 {
        0
    } else {
        f.to_bits()
    }
}

/// A fully materialised query result.
#[derive(Debug, Clone)]
pub struct Relation {
    /// Column names in the order DuckDB returned them.
    pub columns: Vec<String>,
    /// Each row keyed by column name (so column order is irrelevant on compare).
    pub rows: Vec<BTreeMap<String, CanonCell>>,
}

/// In-memory DuckDB, mirroring `DuckDbOracle::new()`.
pub struct DuckDbRelationOracle {
    conn: Connection,
}

impl Default for DuckDbRelationOracle {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDbRelationOracle {
    pub fn new() -> Self {
        Self {
            conn: Connection::open_in_memory().expect("open in-memory DuckDB"),
        }
    }

    /// Execute `sql`, returning every row as a [`Relation`].
    ///
    /// DuckDB only exposes column metadata once the statement has executed, so
    /// the column names are read from the first row's statement rather than
    /// from the freshly-prepared statement.
    pub fn run(&self, sql: &str) -> Result<Relation, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare: {e}"))?;
        let mut rows_iter = stmt.query([]).map_err(|e| format!("query: {e}"))?;

        let mut columns: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        while let Some(row) = rows_iter.next().map_err(|e| format!("row: {e}"))? {
            if columns.is_empty() {
                columns = AsRef::<duckdb::Statement>::as_ref(row).column_names();
            }
            let mut map = BTreeMap::new();
            for (i, name) in columns.iter().enumerate() {
                let value: Value = row.get(i).map_err(|e| format!("get col {i}: {e}"))?;
                map.insert(name.clone(), CanonCell::from_value(value));
            }
            rows.push(map);
        }
        Ok(Relation { columns, rows })
    }
}

/// Relation equality: same set of column names, and the same multiset of rows
/// (rows matched by column name, duplicate counts respected).
///
/// Returns `Ok(())` on equality, or a human-readable explanation of the first
/// difference found.
pub fn relations_equal(a: &Relation, b: &Relation) -> Result<(), String> {
    let mut a_cols: Vec<&String> = a.columns.iter().collect();
    let mut b_cols: Vec<&String> = b.columns.iter().collect();
    a_cols.sort();
    b_cols.sort();
    if a_cols != b_cols {
        return Err(format!("column name sets differ: {a_cols:?} vs {b_cols:?}"));
    }

    if a.rows.len() != b.rows.len() {
        return Err(format!(
            "row counts differ: {} vs {}",
            a.rows.len(),
            b.rows.len()
        ));
    }

    let mut a_sorted = a.rows.clone();
    let mut b_sorted = b.rows.clone();
    a_sorted.sort();
    b_sorted.sort();

    for (i, (ra, rb)) in a_sorted.iter().zip(b_sorted.iter()).enumerate() {
        if ra != rb {
            return Err(format!(
                "row multiset differs at sorted index {i}: {ra:?} vs {rb:?}"
            ));
        }
    }
    Ok(())
}
