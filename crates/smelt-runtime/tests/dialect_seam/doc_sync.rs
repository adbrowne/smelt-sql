//! The user docs quote exact `UnsupportedOnBackend` refusal text verbatim;
//! these tests pin each quoted block against what the compile path actually
//! emits, so the guide cannot drift from the diagnostic
//! (`docs/plans/20260827-statement-level-lowering.md` Phase 7).

use crate::fixtures::{make_model, registry};

/// Extracts the fenced `text` block immediately following `marker` in
/// `diagnostics.md`, compiles `model_sql` against `backend`, and asserts the
/// live `UnsupportedOnBackend` text is byte-identical to the doc's quote.
fn assert_doc_quote_matches_live_diagnostic(marker: &str, model_sql: &str, backend: &str) {
    const DOC_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs-site/docs/reference/diagnostics.md"
    );
    let doc = std::fs::read_to_string(DOC_PATH)
        .unwrap_or_else(|e| panic!("failed to read {DOC_PATH}: {e}"));

    let after_marker = doc
        .split_once(marker)
        .unwrap_or_else(|| panic!("docs no longer carry the {marker} marker"))
        .1;
    let fence_start = after_marker
        .find("```text")
        .expect("marker must be immediately followed by a ```text fenced block")
        + "```text".len();
    let fenced = &after_marker[fence_start..];
    let fence_end = fenced
        .find("```")
        .expect("the ```text block quoting the refusal must be closed");
    let quoted = fenced[..fence_end].trim_matches('\n');

    let model = make_model("q", model_sql);
    let err = registry()
        .get(backend)
        .compile(&model, "main")
        .expect_err("model must be refused so its diagnostic can be pinned");
    let live = format!("{err}");

    assert_eq!(
        live, quoted,
        "the docs' quoted UnsupportedOnBackend text (marker {marker}) has drifted from what \
         the compile path actually emits. Live:\n{live}\n\nDocs (from {DOC_PATH}):\n{quoted}"
    );
}

#[test]
fn docs_quoted_refusal_text_matches_the_live_diagnostic() {
    // The same running-window model as `running_window_refused_at_compile_time`.
    // Its single `PERCENTILE_CONT(...) WITHIN GROUP (...) OVER (...)` call is
    // flagged twice — once for the ordered-set aggregate, once for the window
    // it sits under — so the message reads "2 constructs" with two identical
    // detail lines. That is the live shape the doc's quote must match.
    assert_doc_quote_matches_live_diagnostic(
        "<!-- unsupported-on-backend-refusal-text -->",
        "SELECT id, g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
         OVER (PARTITION BY g ORDER BY t) AS med FROM tbl",
        "duckdb",
    );
}

/// A `Template` row can carry only a plain positional call: `DATE_SUB` is the
/// function-call template row on DuckDB (phase 4 of
/// `docs/outcomes/20260904-dialect-emission-vocabulary`), and a `DISTINCT`
/// modifier is refused before the printer ever sees the call.
#[test]
fn docs_quoted_template_modifier_refusal_matches_the_live_diagnostic() {
    assert_doc_quote_matches_live_diagnostic(
        "<!-- unsupported-on-backend-template-modifier-refusal-text -->",
        "SELECT DATE_SUB(DISTINCT d, INTERVAL 1 DAY) AS x FROM events",
        "duckdb",
    );
}

/// `//`'s `Conditional` verdict on Spark falls through to its `otherwise` arm
/// — `Unsupported` — whenever an operand's class cannot be resolved, because
/// a wrong guess here would compute a different, silently wrong number
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts"). A column
/// from an unschematised table infers as `Unknown`, which classifies as
/// `Unresolved`.
#[test]
fn docs_quoted_operand_class_refusal_matches_the_live_diagnostic() {
    assert_doc_quote_matches_live_diagnostic(
        "<!-- unsupported-on-backend-operand-class-refusal-text -->",
        "SELECT a // b AS x FROM t",
        "spark",
    );
}
