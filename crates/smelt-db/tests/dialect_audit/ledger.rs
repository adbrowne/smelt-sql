//! Accepted `(entry, dialect)` divergences.
//!
//! A pair absent from this table must pass both legs. A pair present in it is a
//! recorded, reviewer-visible exception — never a silent skip.
//!
//! The table is **two-sided**, like `.claude/hardening-baseline.txt`: an
//! unregistered mismatch fails loudly, and so does an unreachable row. A row
//! naming an entry the registry no longer has, or a pair the harness never
//! probes, is an error telling you to delete it.

use smelt_types::DialectId;

/// Why a pair is exempt from passing both legs.
///
/// One variant today. Two others were designed and deliberately not written
/// yet, because neither has a real instance to record:
///
/// - **`Divergent`** — an accepted, permanent semantic difference (Spark's
///   integer-division semantics and the like). DuckDB is the reference engine,
///   so it cannot diverge from itself; the variant lands with the first
///   cross-engine sweep that finds one.
/// - **`SchemaOnly`** — nondeterministic entries. That verdict already lives in
///   `overrides.rs` as `Override::schema_only`, attached to the probe rather
///   than to a `(name, dialect)` pair, which is the right home: `NOW()` is
///   nondeterministic on every engine, not on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A lowering smelt owes, with a tracking issue. Does not fail, but the
    /// count ratchets down only (`.claude/dialect-gaps-baseline.txt`).
    Gap {
        issue: &'static str,
        detail: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct LedgerRow {
    pub name: &'static str,
    pub dialect: DialectId,
    pub verdict: Verdict,
}

const fn gap(name: &'static str, dialect: DialectId, detail: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

/// Every accepted `(entry, dialect)` divergence.
pub fn dialect_divergences() -> &'static [LedgerRow] {
    ROWS
}

/// The `(name, dialect)` row, if one exists.
pub fn find(name: &str, dialect: DialectId) -> Option<&'static LedgerRow> {
    ROWS.iter().find(|r| r.name == name && r.dialect == dialect)
}

/// DuckDB gaps found by the first sweep of this audit.
///
/// Each is a name smelt's `BuiltinRegistry` recognises for which DuckDB has no
/// such function and the registry records no emission verdict — so the printer
/// emits it verbatim and DuckDB rejects it at runtime. Closing one means adding
/// an `Emission::Rename`, an `Emission::Rewrite`, or an
/// `Emission::Unsupported` row to the entry, and deleting its row here.
static ROWS: &[LedgerRow] = &[
    gap(
        "INITCAP",
        DialectId::DuckDb,
        "no `initcap` in DuckDB 1.5.4; the closest is a manual UPPER/LOWER split",
    ),
    gap(
        "TO_CHAR",
        DialectId::DuckDb,
        "no `to_char` in DuckDB; `strftime` is the temporal half of it",
    ),
    gap(
        "TRUNCATE",
        DialectId::DuckDb,
        "no `truncate` scalar in DuckDB; `trunc` is the numeric one",
    ),
    gap("QUOTE_IDENT", DialectId::DuckDb, "PostgreSQL-only builtin"),
    gap(
        "QUOTE_LITERAL",
        DialectId::DuckDb,
        "PostgreSQL-only builtin",
    ),
    gap(
        "JSON_EXTRACT_TEXT",
        DialectId::DuckDb,
        "DuckDB spells it `json_extract_string`",
    ),
    gap(
        "JSON_OBJECT_KEYS",
        DialectId::DuckDb,
        "DuckDB spells it `json_keys`",
    ),
    gap(
        "PERCENTILE_CONT",
        DialectId::DuckDb,
        "DuckDB spells the ordered-set aggregate `quantile_cont`",
    ),
    gap(
        "PERCENTILE_DISC",
        DialectId::DuckDb,
        "DuckDB spells the ordered-set aggregate `quantile_disc`",
    ),
    gap(
        "DATE_SUB",
        DialectId::DuckDb,
        "no `date_sub` in DuckDB; interval subtraction is infix `-`",
    ),
];
