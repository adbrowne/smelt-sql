/// Linear subtyping rank of an expression-typed AST node (Phase 14, §16 #24).
///
/// Every typed node synthesised by the checker carries one of these alongside
/// its [`crate::DataType`]. The ordering `Scalar < Agg < Window` captures SQL's
/// linear "where can this expression appear" rule:
///
/// * `Scalar` — a plain expression (literal, column, arithmetic, scalar
///   function). Acceptable in every splice point.
/// * `Agg` — an aggregate call (`SUM(x)`, `COUNT(*)`, …). Acceptable in
///   `SELECT`, `HAVING`, `ORDER BY`, but not in `WHERE` / `GROUP BY` / `ON`.
/// * `Window` — an aggregate or window function with an `OVER (...)` clause
///   (`ROW_NUMBER() OVER (…)`, `SUM(x) OVER (…)`). Acceptable only in `SELECT`
///   and `QUALIFY`; rejected in `WHERE`, `GROUP BY`, `ON`, etc.
///
/// The check at every splice point is `subkind_of(found, expected)` — O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExprKind {
    /// Plain scalar expression — acceptable in every splice point.
    Scalar,
    /// Aggregate call — `SUM(x)`, `COUNT(*)`, etc.
    Agg,
    /// Aggregate / window call carrying an `OVER (…)` clause.
    Window,
}

impl ExprKind {
    /// Linear rank: `Scalar` = 0, `Agg` = 1, `Window` = 2.
    fn rank(self) -> u8 {
        match self {
            ExprKind::Scalar => 0,
            ExprKind::Agg => 1,
            ExprKind::Window => 2,
        }
    }
}

/// Linear subkind check (§16 #24).
///
/// Returns `true` iff `found` may appear in a context that expects `expected`.
/// The chain is `Scalar <= Agg <= Window`, so a context that accepts `Window`
/// accepts everything; a context that accepts `Scalar` rejects both `Agg`
/// and `Window`.
pub fn subkind_of(found: ExprKind, expected: ExprKind) -> bool {
    found.rank() <= expected.rank()
}

/// Compute the kind ceiling of a list of items (§16 #24, `SelectItems<K>`).
///
/// Returns the maximum kind in the slice. An empty slice is by convention
/// `Scalar` — this matches the empty-default for an empty `SelectItems<K>`
/// value (which only arises from error recovery; well-formed SELECT lists
/// have at least one item).
pub fn kind_ceiling(items: &[ExprKind]) -> ExprKind {
    let mut max = ExprKind::Scalar;
    for &k in items {
        if k.rank() > max.rank() {
            max = k;
        }
    }
    max
}
