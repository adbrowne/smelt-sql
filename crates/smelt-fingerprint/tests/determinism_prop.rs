//! Determinism property test — the soundness gate for the `deterministic` flag.
//!
//! Two invariants, over a generated mix of pure and non-deterministic queries:
//!
//! 1. **Detection (regression guard).** Every non-deterministic construct the
//!    generator injects — a deny-listed built-in (`random()`, `now()`, `uuid()`,
//!    bare `current_timestamp`) or a row-slicing tail clause (`LIMIT`/`OFFSET`)
//!    — must move the model to `deterministic == false`. This guards that the
//!    deny-list keeps covering the vocabulary.
//!
//! 2. **Reproducibility (the load-bearing claim).** Anything the detector reports
//!    `deterministic` must actually be reproducible: built twice, in two
//!    independent DuckDB instances, it yields the same relation. A failure here
//!    means the detector called a non-deterministic model deterministic — the
//!    exact false positive that would let the reuse layer point a new environment
//!    at a stale materialisation. This is the determinism analogue of the
//!    equivalence soundness gate in `soundness_prop.rs`.
//!
//! The two checks are complementary: (1) pins the constructs we know about, (2)
//! catches non-determinism we *failed* to anticipate (an unflagged construct
//! makes a query present as deterministic, and the two builds then disagree).
//!
//! Honour `PROPTEST_CASES` for deeper local runs.

mod oracle;
use oracle::{relations_equal, DuckDbRelationOracle};
use proptest::prelude::*;
use smelt_fingerprint::output_fingerprint_from_sql;

const COLS: [&str; 3] = ["a", "b", "c"];
/// Three rows over (a INT, b INT, c DOUBLE), distinct so a slice has effect.
const SEED_BODY: &str =
    "SELECT 1 AS a, 2 AS b, 1.5 AS c UNION ALL SELECT 4, 0, 2.5 UNION ALL SELECT 7, 3, 9.0";

/// A projection expression. Pure variants are a function of the inputs; the rest
/// are inline non-determinism with no function property to read.
#[derive(Debug, Clone, Copy)]
enum Atom {
    Col(usize),
    AddOne(usize),
    Abs(usize),
    // --- non-deterministic ---
    Random,
    Now,
    Uuid,
    CurrentTs,
}

impl Atom {
    fn is_nondet(self) -> bool {
        matches!(
            self,
            Atom::Random | Atom::Now | Atom::Uuid | Atom::CurrentTs
        )
    }

    fn expr(self) -> String {
        match self {
            Atom::Col(i) => COLS[i].to_string(),
            Atom::AddOne(i) => format!("{} + 1", COLS[i]),
            Atom::Abs(i) => format!("abs({})", COLS[i]),
            Atom::Random => "random()".to_string(),
            Atom::Now => "now()".to_string(),
            Atom::Uuid => "uuid()".to_string(),
            Atom::CurrentTs => "current_timestamp".to_string(),
        }
    }
}

/// Whether to append a row-slicing tail clause (deliberately with no ORDER BY —
/// these are non-deterministic and the detector must flag them).
#[derive(Debug, Clone, Copy)]
enum Slice {
    None,
    Limit(u32),
    LimitOffset(u32, u32),
}

#[derive(Debug, Clone)]
struct Query {
    proj: Vec<Atom>,
    slice: Slice,
}

impl Query {
    fn injects_nondet(&self) -> bool {
        self.proj.iter().any(|a| a.is_nondet()) || !matches!(self.slice, Slice::None)
    }

    fn to_sql(&self) -> String {
        let cols: Vec<String> = self
            .proj
            .iter()
            .enumerate()
            .map(|(i, a)| format!("{} AS p{i}", a.expr()))
            .collect();
        let tail = match self.slice {
            Slice::None => String::new(),
            Slice::Limit(n) => format!(" LIMIT {n}"),
            Slice::LimitOffset(n, o) => format!(" LIMIT {n} OFFSET {o}"),
        };
        format!(
            "SELECT {cols} FROM ({SEED_BODY}) AS t{tail}",
            cols = cols.join(", "),
        )
    }
}

/// An aggregate over a single column. Order-*insensitive* ones are pure
/// functions of the input multiset; order-*sensitive* ones depend on a row order
/// a relation does not fix (and smelt has no aggregate-`ORDER BY` to pin it).
#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)] // variants mirror real SQL function names
enum Agg {
    // order-insensitive
    Sum,
    Count,
    Min,
    Max,
    Avg,
    // order-sensitive
    ArrayAgg,
    List,
    StringAgg,
    GroupConcat,
    ListAgg,
    AnyValue,
    Arbitrary,
}

impl Agg {
    fn is_order_sensitive(self) -> bool {
        matches!(
            self,
            Agg::ArrayAgg
                | Agg::List
                | Agg::StringAgg
                | Agg::GroupConcat
                | Agg::ListAgg
                | Agg::AnyValue
                | Agg::Arbitrary
        )
    }

    fn expr(self, col: &str) -> String {
        match self {
            Agg::Sum => format!("sum({col})"),
            Agg::Count => format!("count({col})"),
            Agg::Min => format!("min({col})"),
            Agg::Max => format!("max({col})"),
            Agg::Avg => format!("avg({col})"),
            Agg::ArrayAgg => format!("array_agg({col})"),
            Agg::List => format!("list({col})"),
            Agg::StringAgg => format!("string_agg(CAST({col} AS VARCHAR), ',')"),
            Agg::GroupConcat => format!("group_concat({col})"),
            Agg::ListAgg => format!("listagg(CAST({col} AS VARCHAR), ',')"),
            Agg::AnyValue => format!("any_value({col})"),
            Agg::Arbitrary => format!("arbitrary({col})"),
        }
    }
}

fn agg_strategy() -> impl Strategy<Value = Agg> {
    prop_oneof![
        Just(Agg::Sum),
        Just(Agg::Count),
        Just(Agg::Min),
        Just(Agg::Max),
        Just(Agg::Avg),
        Just(Agg::ArrayAgg),
        Just(Agg::List),
        Just(Agg::StringAgg),
        Just(Agg::GroupConcat),
        Just(Agg::ListAgg),
        Just(Agg::AnyValue),
        Just(Agg::Arbitrary),
    ]
}

fn atom_strategy() -> impl Strategy<Value = Atom> {
    prop_oneof![
        (0usize..3).prop_map(Atom::Col),
        (0usize..3).prop_map(Atom::AddOne),
        (0usize..3).prop_map(Atom::Abs),
        Just(Atom::Random),
        Just(Atom::Now),
        Just(Atom::Uuid),
        Just(Atom::CurrentTs),
    ]
}

fn slice_strategy() -> impl Strategy<Value = Slice> {
    prop_oneof![
        Just(Slice::None),
        (1u32..3).prop_map(Slice::Limit),
        (1u32..3, 0u32..2).prop_map(|(n, o)| Slice::LimitOffset(n, o)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    #[test]
    fn determinism_flag_is_sound(
        proj in prop::collection::vec(atom_strategy(), 1..=3),
        slice in slice_strategy(),
    ) {
        let q = Query { proj, slice };
        let sql = q.to_sql();
        let r = output_fingerprint_from_sql(&sql, &[])
            .unwrap_or_else(|| panic!("did not parse: {sql}"));

        // (1) Detection: every injected non-deterministic construct must flag.
        if q.injects_nondet() {
            prop_assert!(
                !r.deterministic,
                "DETECTION GAP: injected non-determinism but flag says deterministic\n  sql: {}\n  reasons: {:?}",
                sql,
                r.non_determinism,
            );
        }

        // (2) Reproducibility: a deterministic verdict must hold against DuckDB.
        // Build the model twice in independent instances and require identical
        // relations. (Only deterministic queries reach here; non-deterministic
        // ones legitimately differ run-to-run and are never reuse-matched.)
        if r.deterministic {
            let r1 = DuckDbRelationOracle::new().run(&sql)
                .unwrap_or_else(|e| panic!("run 1 failed ({e}): {sql}"));
            let r2 = DuckDbRelationOracle::new().run(&sql)
                .unwrap_or_else(|e| panic!("run 2 failed ({e}): {sql}"));
            prop_assert!(
                relations_equal(&r1, &r2).is_ok(),
                "UNSOUND: flagged deterministic but two builds differ\n  sql: {}\n  diff: {:?}",
                sql,
                relations_equal(&r1, &r2),
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Aggregate determinism: an order-sensitive aggregate must flag, and an
    /// order-insensitive one must both pass the flag and reproduce across two
    /// independent DuckDB builds. (Aggregates need their own generator because a
    /// projection cannot mix an aggregate with a bare column without a GROUP BY.)
    #[test]
    fn aggregate_determinism_is_sound(
        agg in agg_strategy(),
        col in 0usize..3,
    ) {
        let sql = format!("SELECT {} AS s FROM ({SEED_BODY}) AS t", agg.expr(COLS[col]));
        let r = output_fingerprint_from_sql(&sql, &[])
            .unwrap_or_else(|| panic!("did not parse: {sql}"));

        if agg.is_order_sensitive() {
            prop_assert!(
                !r.deterministic,
                "DETECTION GAP: order-sensitive aggregate flagged deterministic\n  sql: {}\n  reasons: {:?}",
                sql,
                r.non_determinism,
            );
        } else {
            prop_assert!(
                r.deterministic,
                "FALSE POSITIVE: order-insensitive aggregate flagged non-deterministic\n  sql: {}\n  reasons: {:?}",
                sql,
                r.non_determinism,
            );
            let r1 = DuckDbRelationOracle::new().run(&sql)
                .unwrap_or_else(|e| panic!("run 1 failed ({e}): {sql}"));
            let r2 = DuckDbRelationOracle::new().run(&sql)
                .unwrap_or_else(|e| panic!("run 2 failed ({e}): {sql}"));
            prop_assert!(
                relations_equal(&r1, &r2).is_ok(),
                "UNSOUND: flagged deterministic but two builds differ\n  sql: {}\n  diff: {:?}",
                sql,
                relations_equal(&r1, &r2),
            );
        }
    }
}
