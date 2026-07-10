//! External corpus conformance gate — architecture spec §Constraints #13
//! "corpus grounding": a vendored SELECT-only corpus extracted from DuckDB's
//! sqllogictest suite and PostgreSQL's regression suite
//! (`scripts/extract-sql-corpus.py`, `tests/corpus/external/{duckdb,postgres}.sql`)
//! runs through the smelt parser (and, where the statement is self-contained
//! enough for DuckDB to accept it with no schema, the Phase 3 fidelity check)
//! with a checked-in failure ledger (`tests/corpus/external_ledger.toml`).
//!
//! - `corpus_statements_parse_or_ledgered` — every corpus statement either
//!   parses cleanly (and, where DuckDB accepts it standalone, round-trips)
//!   or appears in the ledger with a category. An unledgered failure fails
//!   the test.
//! - `ledger_has_no_stale_entries` — a ledger entry whose statement now
//!   passes (or that no longer matches any corpus statement) fails the test,
//!   forcing shrinkage as gaps close or the corpus is refreshed.
//! - `extraction_filter_self_test` — invokes
//!   `scripts/extract-sql-corpus.py --self-test`, which is the single source
//!   of truth for the extraction filter's `is_select_only` / `normalize` /
//!   `dedup_and_cap` logic (see "Filter logic ownership" below).
//!
//! ## Filter logic ownership
//!
//! The extraction filter (SELECT/WITH/VALUES only, no top-level DDL/DML,
//! dedup, size cap) is implemented once, in `scripts/extract-sql-corpus.py`.
//! It is not mirrored in Rust: the script is the only thing that runs it
//! (extraction is a one-time/occasional refresh, not a CI step), so a second
//! Rust copy would be pure drift risk with no corresponding coverage benefit.
//! `extraction_filter_self_test` below shells out to the script's
//! `--self-test` mode, which exercises `is_select_only`, `normalize`, and
//! `dedup_and_cap` with unit-style assertions and a non-zero exit on failure.

use std::collections::HashMap;
use std::path::Path;

use smelt_parser_compat::duckdb_oracle::duckdb_accepts;
use smelt_parser_compat::SmeltParseResult;

/// One statement from the external corpus, tagged with its source file (for
/// error messages only).
struct CorpusStatement {
    source: &'static str,
    sql: String,
}

fn load_corpus_file(source: &'static str, text: &str) -> Vec<CorpusStatement> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| CorpusStatement {
            source,
            sql: l.to_string(),
        })
        .collect()
}

fn corpus_statements() -> Vec<CorpusStatement> {
    let mut stmts = load_corpus_file("duckdb.sql", include_str!("corpus/external/duckdb.sql"));
    stmts.extend(load_corpus_file(
        "postgres.sql",
        include_str!("corpus/external/postgres.sql"),
    ));
    stmts
}

/// Stable hash key for a normalized statement, used to key the ledger.
///
/// This is FNV-1a (64-bit), vendored inline: a fully specified, frozen
/// algorithm (offset basis `0xcbf29ce484222325`, prime `0x100000001b3`),
/// so the key for a given statement text is stable across runs, machines,
/// **and Rust toolchain releases**. Do NOT swap this for
/// `std::collections::hash_map::DefaultHasher` — its algorithm is
/// explicitly unspecified and may change between Rust versions, which
/// would silently rekey every ledger entry on a toolchain upgrade.
///
/// The extraction script (`scripts/extract-sql-corpus.py`) does not compute
/// these hashes — this function is the only implementation. If a second
/// implementation is ever added (e.g. in the script for pre-seeding a
/// ledger), it must match this one byte-for-byte and both sites must
/// cross-reference each other.
fn stmt_hash(sql: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in sql.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// A parsed ledger entry: `category` + free-text `note`.
#[derive(Debug, Clone)]
struct LedgerEntry {
    category: String,
    note: String,
}

/// Hand-rolled parser for the ledger's TOML subset: a sequence of
/// `["<hash>"]` table headers, each followed by `key = "value"` lines. This
/// is valid TOML (quoted table names, quoted string values) — just a
/// restricted grammar, parsed directly rather than pulling in a `toml` crate
/// dependency for a two-field, self-authored file format.
///
/// Hand-added `note` values must not contain embedded `"` characters or
/// lines starting with `[` — this restricted parser does not handle escapes
/// or multi-line strings.
fn parse_ledger(text: &str) -> HashMap<String, LedgerEntry> {
    let mut out = HashMap::new();
    let mut current_hash: Option<String> = None;
    let mut current_category: Option<String> = None;
    let mut current_note: Option<String> = None;

    fn flush(
        out: &mut HashMap<String, LedgerEntry>,
        hash: Option<String>,
        category: Option<String>,
        note: Option<String>,
    ) {
        if let Some(hash) = hash {
            out.insert(
                hash,
                LedgerEntry {
                    category: category.unwrap_or_default(),
                    note: note.unwrap_or_default(),
                },
            );
        }
    }

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            // New table header — flush the previous entry first.
            flush(
                &mut out,
                current_hash.take(),
                current_category.take(),
                current_note.take(),
            );
            let hash = rest.trim().trim_matches('"').to_string();
            current_hash = Some(hash);
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "category" => current_category = Some(value.to_string()),
                "note" => current_note = Some(value.to_string()),
                _ => {}
            }
        }
    }
    flush(&mut out, current_hash, current_category, current_note);
    out
}

fn ledger() -> HashMap<String, LedgerEntry> {
    parse_ledger(include_str!("corpus/external_ledger.toml"))
}

/// True if this statement, standalone with no schema, is accepted by
/// DuckDB — i.e. it is fully self-contained (no external table reference)
/// and so is in scope for the fidelity round-trip check. Statements that
/// reference tables from their upstream test suite (which are not part of
/// our schema prelude) correctly fall outside this check: DuckDB rejects
/// them for an unrelated catalog reason, so they create no fidelity
/// pressure here (same principle as the Phase 3 seed-corpus harness, which
/// scopes itself to a canned schema prelude).
fn duckdb_accepts_standalone(sql: &str) -> bool {
    duckdb_accepts(sql, "").is_ok()
}

/// Evaluate one corpus statement against the pass/fail criteria used by
/// both the main gate and the stale-entry check:
/// - smelt must parse the statement cleanly, AND
/// - if DuckDB accepts the *original* statement standalone (no schema), the
///   printed form smelt produces must also be accepted by DuckDB (the
///   fidelity direction, scoped to self-contained statements the way the
///   Phase 3 seed harness scopes to its schema prelude).
fn statement_passes(sql: &str) -> Result<(), String> {
    let smelt = SmeltParseResult::parse(sql);
    if !smelt.success {
        return Err(format!("smelt failed to parse: {:?}", smelt.errors));
    }
    if duckdb_accepts_standalone(sql) {
        let printed = smelt
            .normalized_sql
            .as_deref()
            .expect("a successful parse always has normalized SQL");
        if let Err(err) = duckdb_accepts(printed, "") {
            return Err(format!(
                "smelt parsed cleanly but printed SQL is rejected by DuckDB \
                 (silent mis-parse); printed: {printed}; duckdb: {err}"
            ));
        }
    }
    Ok(())
}

#[test]
fn corpus_statements_parse_or_ledgered() {
    let ledger = ledger();
    let mut unledgered_failures = Vec::new();

    for stmt in corpus_statements() {
        if let Err(reason) = statement_passes(&stmt.sql) {
            let hash = stmt_hash(&stmt.sql);
            if !ledger.contains_key(&hash) {
                unledgered_failures.push(format!(
                    "[{}] hash={} sql={}\n  reason: {}",
                    stmt.source, hash, stmt.sql, reason
                ));
            }
        }
    }

    assert!(
        unledgered_failures.is_empty(),
        "{} unledgered external-corpus failure(s). Add an entry to \
         tests/corpus/external_ledger.toml (category + note) or fix the gap:\n{}",
        unledgered_failures.len(),
        unledgered_failures.join("\n")
    );
}

#[test]
fn ledger_has_no_stale_entries() {
    let ledger = ledger();
    let stmts = corpus_statements();
    let by_hash: HashMap<String, &str> = stmts
        .iter()
        .map(|s| (stmt_hash(&s.sql), s.sql.as_str()))
        .collect();

    let mut stale = Vec::new();
    for (hash, entry) in &ledger {
        match by_hash.get(hash) {
            None => stale.push(format!(
                "hash={hash} (category={}) no longer matches any corpus statement \
                 — remove it from the ledger",
                entry.category
            )),
            Some(sql) => {
                if statement_passes(sql).is_ok() {
                    stale.push(format!(
                        "hash={hash} (category={}, note={}) now PASSES — remove it \
                         from the ledger: {sql}",
                        entry.category, entry.note
                    ));
                }
            }
        }
    }

    assert!(
        stale.is_empty(),
        "{} stale ledger entr(y/ies) — a gap closed or the corpus changed, \
         shrink tests/corpus/external_ledger.toml:\n{}",
        stale.len(),
        stale.join("\n")
    );
}

#[test]
fn extraction_filter_self_test() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("smelt-parser-compat is two levels under the workspace root");
    let script = repo_root.join("scripts").join("extract-sql-corpus.py");
    assert!(
        script.exists(),
        "extraction script not found at {}",
        script.display()
    );

    // Fail closed: a missing interpreter must be a loud failure, not a
    // silent skip that lets the filter logic rot uncovered. Environments
    // that genuinely cannot provide Python opt out explicitly.
    if std::env::var_os("SMELT_SKIP_PY_SELFTEST").is_some_and(|v| v == "1") {
        eprintln!("SMELT_SKIP_PY_SELFTEST=1 set; skipping extraction_filter_self_test");
        return;
    }
    let python3 = which_python3().unwrap_or_else(|| {
        panic!(
            "extraction_filter_self_test requires a Python interpreter \
             (`python3` or `python`) on PATH to run \
             scripts/extract-sql-corpus.py --self-test, and none was found. \
             Install Python, or set SMELT_SKIP_PY_SELFTEST=1 to skip this \
             test explicitly (see crates/smelt-parser-compat/CLAUDE.md)."
        )
    });

    let output = std::process::Command::new(python3)
        .arg(&script)
        .arg("--self-test")
        .output()
        .expect("failed to invoke extract-sql-corpus.py --self-test");

    assert!(
        output.status.success(),
        "extract-sql-corpus.py --self-test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn which_python3() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[test]
fn corpus_is_nonempty() {
    let stmts = corpus_statements();
    assert!(
        stmts.len() >= 500,
        "external corpus should have a meaningful sample size, got {}",
        stmts.len()
    );
}

#[test]
fn parse_ledger_handles_basic_shape() {
    let text = r#"
["abc123"]
category = "smelt_fails"
note = "some construct"

["def456"]
category = "roundtrip_mismatch"
note = "another construct"
"#;
    let parsed = parse_ledger(text);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed["abc123"].category, "smelt_fails");
    assert_eq!(parsed["def456"].note, "another construct");
}

#[test]
fn stmt_hash_is_deterministic() {
    assert_eq!(stmt_hash("SELECT 1"), stmt_hash("SELECT 1"));
    assert_ne!(stmt_hash("SELECT 1"), stmt_hash("SELECT 2"));
}
