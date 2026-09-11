//! Every ledger-mutating transaction this backend opens is serialised
//! against this process's others.
//!
//! BigQuery cancels a multi-statement transaction that mutates a table
//! another in-flight transaction is also mutating, and every maintained
//! model's bookkeeping transaction mutates the one `_smelt_ledger` table.
//! A live run of `examples/github_activity` at the default `--jobs` lost
//! models at random to "Transaction is aborted due to concurrent update
//! against table … `_smelt_ledger`" until `BigQueryBackend::ledger_gate`
//! serialised them; the same run at `--jobs 1` was green, which is what
//! identified the cause.
//!
//! The gate is a liveness property of a concurrent run against a real
//! warehouse, so nothing offline can prove it *works*. What an offline gate
//! can do — and what this one does — is prove it is still *there*: the two
//! seams that open a ledger transaction must both acquire it, so deleting
//! the acquisition (or adding a third such seam without it) fails here
//! rather than at a warehouse. Same species as `smelt-runtime`'s
//! `state_guard_census`.

/// The seams that open a transaction over `_smelt_ledger`.
const LEDGER_TRANSACTION_SEAMS: &[&str] = &["fold_ledger_delta", "execute_write_with_bookkeeping"];

const ACQUISITION: &str = "self.ledger_gate.lock().await";

/// Extract the source of one `async fn <name>` from `lib.rs`, from its
/// signature to the start of the next item at the same indentation.
fn method_body<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("async fn {name}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("no `async fn {name}` in lib.rs — was it renamed?"));
    let rest = &source[start..];
    // The next method at the same (4-space) indentation ends this one.
    match rest[1..].find("\n    async fn ") {
        Some(offset) => &rest[..offset + 1],
        None => rest,
    }
}

#[test]
fn every_ledger_transaction_seam_takes_the_gate() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("read lib.rs");

    let mut missing = Vec::new();
    for seam in LEDGER_TRANSACTION_SEAMS {
        let body = method_body(&source, seam);
        if !body.contains(ACQUISITION) {
            missing.push(*seam);
        }
    }
    assert!(
        missing.is_empty(),
        "these seams open a transaction over `_smelt_ledger` without acquiring \
         `ledger_gate` — two of them in flight at once are cancelled by BigQuery, which a \
         live `examples/github_activity` run demonstrated: {missing:?}"
    );
}

/// The extractor must actually be reading the named method's body — with a
/// naive whole-file `contains` the test above passes no matter which method
/// holds the acquisition.
#[test]
fn the_body_extractor_is_scoped_to_one_method() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("read lib.rs");
    let body = method_body(&source, "execute_write_with_bookkeeping");
    assert!(
        body.len() < source.len() / 2,
        "the extractor returned most of the file ({} of {} bytes) — it is not scoped",
        body.len(),
        source.len()
    );
    assert!(body.contains("write_with_bookkeeping_plan"), "wrong method");
    assert!(
        !body.contains("async fn load_table("),
        "the extractor ran past the end of the method"
    );
}
