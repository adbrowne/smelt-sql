//! The standing coverage table issue #171 asked for.
//!
//! Derived from `BuiltinRegistry` and the ledger only — no warehouse, fully
//! deterministic, so the table can be regenerated and drift-gated per-PR. What
//! it publishes is *what smelt claims*: the emission verdict for each
//! `(entry, dialect)` pair, annotated with any recorded divergence a live sweep
//! found. It is not a claim that every cell has been verified; the trailing
//! verification-tier section says which dialects a live leg actually visits.

use smelt_types::{BuiltinRegistry, DialectId, Emission, SyntaxForm};

use crate::ledger::{self, Verdict};

fn form_label(form: SyntaxForm) -> &'static str {
    match form {
        SyntaxForm::Call => "call",
        SyntaxForm::Infix => "infix",
        SyntaxForm::Postfix => "postfix",
        SyntaxForm::TableFn => "table-fn",
        SyntaxForm::Special => "special",
    }
}

/// The cell for one `(entry, dialect)` pair.
///
/// The registry's verdict comes first because it is what smelt *does*; a
/// ledger annotation is appended because it is what a live engine *found*. A
/// pair can carry both — `LOG` is `native` on Spark and also a recorded gap.
fn cell(name: &str, dialect: DialectId) -> String {
    let emission = BuiltinRegistry::resolve(name)
        .map(|sig| sig.emission_for(dialect))
        .unwrap_or(Emission::Native);
    let base = match emission {
        Emission::Native => "native".to_string(),
        Emission::Rename(to) => format!("rename:{to}"),
        Emission::Rewrite(id) => format!("rewrite:{id:?}"),
        Emission::Unsupported { .. } => "unsupported".to_string(),
    };
    let annotation: Vec<&'static str> = ledger::dialect_divergences()
        .iter()
        .filter(|r| r.name == name && r.dialect == dialect)
        .map(|r| match r.verdict {
            Verdict::Gap { issue, .. } => issue,
            Verdict::Divergent { .. } => "divergent",
        })
        .collect();
    if annotation.is_empty() {
        base
    } else {
        format!("{base} (gap {})", annotation.join(", "))
    }
}

/// Whether the entry is probed schema-only, and why.
fn schema_only_note(name: &str) -> Option<&'static str> {
    crate::overrides::find(name).and_then(|o| o.schema_only)
}

/// Render the whole document, header included.
pub fn render() -> String {
    let mut names: Vec<&'static str> = BuiltinRegistry::names().collect();
    // `BuiltinRegistry::names()` is HashMap order. Sorting is what makes the
    // generated file stable enough to diff-gate.
    names.sort_unstable();

    let mut out = String::new();
    out.push_str(
        "<!-- GENERATED FILE — do not edit by hand.\n\
         Regenerate with:\n\
         \x20    SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \\\n\
         \x20      the_coverage_table_matches_the_registry\n\
         -->\n\n",
    );
    out.push_str("# Dialect emission coverage\n\n");
    out.push_str(
        "How every built-in smelt recognises is spelled on each backend. Each cell is the\n\
         `Emission` verdict the registry carries for that `(entry, dialect)` pair\n\
         (`crates/smelt-types/src/signatures.rs`), which is the single place the printer\n\
         reads — there is no name-matched dialect arm in `printer.rs`.\n\n",
    );
    out.push_str("Cell vocabulary:\n\n");
    out.push_str(
        "- `native` — same spelling, same semantics; smelt emits the name unchanged.\n\
         - `rename:X` — same call shape, emitted as `X`.\n\
         - `rewrite:Id` — structurally rewritten by the printer's `RewriteId::Id` arm.\n\
         - `unsupported` — the compiler refuses the model (`UnsupportedOnBackend`) rather\n\
         \x20 than emitting SQL the engine would reject or misread.\n\
         - `(gap #N)` — a live sweep found this pair does not work as claimed, tracked by\n\
         \x20 issue #N. The count ratchets down only\n\
         \x20 (`.claude/dialect-gaps-baseline.txt`).\n\
         - `(gap divergent)` — an accepted, permanent semantic difference no rename or\n\
         \x20 rewrite can close.\n\n",
    );

    out.push_str("| Entry | Form | DuckDB | Spark SQL | PostgreSQL | BigQuery |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for name in &names {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        out.push_str(&format!(
            "| `{name}` | {} | {} | {} | {} | {} |\n",
            form_label(sig.syntax_form),
            cell(name, DialectId::DuckDb),
            cell(name, DialectId::SparkSql),
            cell(name, DialectId::PostgreSql),
            cell(name, DialectId::BigQuery),
        ));
    }

    out.push_str("\n## Schema-only entries\n\n");
    out.push_str(
        "These are probed for acceptance but never value-compared: they return a different\n\
         answer on every run, or on every engine, for reasons that say nothing about\n\
         emission.\n\n",
    );
    let mut any_schema_only = false;
    for name in &names {
        if let Some(reason) = schema_only_note(name) {
            any_schema_only = true;
            out.push_str(&format!("- `{name}` — {reason}\n"));
        }
    }
    if !any_schema_only {
        out.push_str("_None._\n");
    }

    out.push_str("\n## Verification tiers\n\n");
    out.push_str(
        "A verdict in the table is what smelt *claims*. What a live engine has actually\n\
         confirmed differs per dialect:\n\n",
    );
    out.push_str("| Dialect | Live leg | Tier |\n|---|---|---|\n");
    out.push_str("| DuckDB | schema + value | every PR (in-process, no warehouse) |\n");
    out.push_str("| Spark SQL | schema + value | nightly, or a PR labelled `run-docker-tests` |\n");
    out.push_str(
        "| BigQuery | schema + value | manual sweep only — `scripts/bigquery-dialect-audit.sh`; \
         the value leg executes rather than dry-runs, so it bills |\n",
    );
    out.push_str(
        "| PostgreSQL | none | **unverified** — a `SqlDialect` variant with no backend crate \
         and no oracle, so nothing exercises its verdicts |\n",
    );
    out.push_str(
        "\nAn untested `native` is reported as *unverified*, never as *passing*: the value leg\n\
         exists to test the claim, and a default-passing assumption would recreate exactly the\n\
         silent hole this audit was built to close.\n",
    );
    out
}
