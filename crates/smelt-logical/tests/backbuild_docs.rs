//! Doc-sync gate for `docs-site/docs/guide/backbuild-synthesis.md` (research
//! `docs/research/20260802-backbuild-synthesis.md`, plan
//! `docs/plans/20260802-backbuild-followups.md` Phase 8). The guide's
//! before/after/emitted-script SQL is generated and verified here, not
//! hand-authored prose that can silently drift from what the real
//! `definition_diff` → `derive_backbuild_options` → `assemble` pipeline
//! actually produces.
//!
//! ## Marker scheme
//!
//! Every fenced ```sql block the guide relies on for a worked example is
//! preceded by one or more HTML comment markers, invisible in rendered
//! MkDocs output:
//!
//! ```markdown
//! <!-- backbuild-example(<id>): before -->
//! <!-- backbuild-example(<id>): after -->
//! ```sql
//! ...
//! ```
//! ```
//!
//! A single fenced block may carry more than one marker (most sections show
//! `before`/`after` inside one combined fence, split internally on the `--
//! before` / `-- after` sentinel comment lines the guide already uses for
//! readability — those sentinels are literal content, reused here as the
//! split point rather than invented). The `intro` example is the one
//! section that shows `before` and `after` as two separate fences, each
//! still carrying its own `-- before` / `-- after` leading sentinel line
//! (stripped before parsing).
//!
//! A block that genuinely cannot be derived end-to-end (none exist on this
//! page as of this phase — see `every_script_block_is_marked`'s doc comment)
//! would instead carry `<!-- backbuild-example-exempt: <reason> -->`.
//!
//! ## Regeneration mode
//!
//! `SMELT_REGEN_DOCS=1 cargo test -p smelt-logical --test backbuild_docs`
//! rewrites every marked `script` block in the guide in place, from the real
//! emitters, before running the assertions (which then pass trivially).
//! Regeneration touches only the text between a `script`-marked fence's
//! delimiters — prose, headings, and `before`/`after` blocks are never
//! written. Statement formatting: every statement is written on its own
//! line; every statement is semicolon-terminated except when the whole
//! script is a single statement (matching this guide's existing house style
//! of leaving a lone statement's trailing `;` off). Regeneration does not
//! attempt to preserve hand-authored in-fence prose comments (e.g. the
//! `-- option 1: update-from` labels in the LEFT JOIN dual-option example) —
//! those are restored by hand after a regen run, then reverified by a normal
//! (non-regen) test run, which is comment- and whitespace-insensitive (see
//! `normalize_statements`).
//!
//! ## Comparison normalization
//!
//! A script block's raw text is turned into a `Vec<String>` of statements by
//! dropping blank lines and comment-only lines (`-- ...`), then splitting on
//! `;` (a statement ends where a `;`-terminated line is seen, or at the end
//! of the block for a final unterminated statement), collapsing internal
//! whitespace runs to a single space in each statement. This tolerates the
//! guide's multi-line pretty-printing of a single emitted statement (e.g.
//! the aggregate/window backfills) and the dual-option join example's
//! interleaved `-- option N: ...` labels, while still catching any real
//! difference in the SQL itself.

#[path = "backbuild_conformance/harness.rs"]
mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use duckdb::Connection;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, BackbuildInputs, BackbuildOptions,
    Selection, SourceRef, Technique,
};

const GUIDE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs-site/docs/guide/backbuild-synthesis.md"
);

const DEFINITION_DELTAS_SPEC_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/definition_deltas.md"
);

const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

fn plain_inputs(after_sql: &str) -> BackbuildInputs {
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: BTreeMap::new(),
        sources: BTreeMap::new(),
    }
}

fn source_ref(physical_name: &str, unique_key: &[&str], not_null_columns: &[&str]) -> SourceRef {
    SourceRef {
        physical_name: physical_name.to_string(),
        unique_key: Some(unique_key.iter().map(|s| s.to_string()).collect()),
        not_null_columns: not_null_columns.iter().map(|s| s.to_string()).collect(),
    }
}

fn added_types(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, ty)| (name.to_string(), ty.to_string()))
        .collect()
}

// ===== Extraction =====

#[derive(Debug, Clone, Default)]
struct ExtractedBlocks {
    before: Option<String>,
    after: Option<String>,
    script: Option<String>,
}

/// One marker line's parsed content: an example id + role, or a standalone
/// exemption reason.
enum Marker {
    Role {
        id: String,
        role: String,
    },
    /// The reason string is not consumed today (no exemptions exist yet —
    /// see `every_script_block_is_marked`'s doc comment) but is parsed and
    /// kept so a future exemption's reason text is validated as present.
    Exempt {
        #[allow(dead_code)]
        reason: String,
    },
}

fn parse_marker_line(line: &str) -> Option<Marker> {
    let line = line.trim();
    if let Some(rest) = line
        .strip_prefix("<!-- backbuild-example-exempt:")
        .and_then(|s| s.strip_suffix("-->"))
    {
        return Some(Marker::Exempt {
            reason: rest.trim().to_string(),
        });
    }
    let rest = line
        .strip_prefix("<!-- backbuild-example(")
        .and_then(|s| s.strip_suffix("-->"))?;
    let close_paren = rest.find(')')?;
    let id = rest[..close_paren].trim().to_string();
    let role = rest[close_paren + 1..]
        .trim()
        .strip_prefix(':')?
        .trim()
        .to_string();
    Some(Marker::Role { id, role })
}

/// Join non-blank, trimmed lines with a single space — SQL text is
/// whitespace-insensitive to `smelt_parser::parse`, so this is safe for
/// reconstructing a before/after definition split across several
/// pretty-printed lines in the guide.
fn join_sql_lines(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split one fence's raw content into (before, after) SQL text using the
/// guide's own `-- before` / `-- after` leading sentinel comment lines —
/// present whether the fence holds one side alone or both combined.
fn split_before_after(content: &str) -> (Option<String>, Option<String>) {
    let lines: Vec<&str> = content.lines().collect();
    let after_idx = lines.iter().position(|l| l.trim() == "-- after");
    match after_idx {
        Some(idx) => {
            let mut before_lines = lines[..idx].to_vec();
            if before_lines.first().map(|l| l.trim()) == Some("-- before") {
                before_lines.remove(0);
            }
            let after_lines = &lines[idx + 1..];
            (
                Some(join_sql_lines(&before_lines)),
                Some(join_sql_lines(after_lines)),
            )
        }
        None => {
            let mut ls = lines.clone();
            if ls.first().map(|l| l.trim()) == Some("-- before") {
                ls.remove(0);
                (Some(join_sql_lines(&ls)), None)
            } else if ls.first().map(|l| l.trim()) == Some("-- after") {
                ls.remove(0);
                (None, Some(join_sql_lines(&ls)))
            } else {
                // No sentinel: caller asked for a single role that isn't
                // before/after (e.g. `script`) — nothing to split here.
                (None, None)
            }
        }
    }
}

/// Sweep result for one fenced ```sql block: which line it starts on, and
/// whether it was covered by a marker/exempt comment immediately above it.
struct FenceSweep {
    start_line: usize,
    marked: bool,
}

/// Walk the guide's lines once, matching each ```sql fence to any marker
/// comment(s) immediately preceding it (no blank line in between — the
/// convention every marker in this guide follows). Returns the extracted
/// per-id blocks plus the sweep result for every fence found, for
/// `every_script_block_is_marked` to check without re-parsing.
fn extract(doc: &str) -> (BTreeMap<String, ExtractedBlocks>, Vec<FenceSweep>) {
    let lines: Vec<&str> = doc.lines().collect();
    let mut examples: BTreeMap<String, ExtractedBlocks> = BTreeMap::new();
    let mut sweep = Vec::new();

    let mut i = 0usize;
    let mut pending_roles: Vec<(String, String)> = Vec::new();
    let mut pending_exempt = false;
    while i < lines.len() {
        let line = lines[i];
        match parse_marker_line(line) {
            Some(Marker::Role { id, role }) => {
                pending_roles.push((id, role));
                i += 1;
                continue;
            }
            Some(Marker::Exempt { reason: _ }) => {
                pending_exempt = true;
                i += 1;
                continue;
            }
            None => {}
        }

        if line.trim() == "```sql" {
            let start_line = i + 1; // 1-indexed for humans
            let close = lines[i + 1..]
                .iter()
                .position(|l| l.trim() == "```")
                .map(|p| i + 1 + p)
                .unwrap_or_else(|| {
                    panic!(
                        "unterminated ```sql fence starting at line {start_line} in {GUIDE_PATH}"
                    )
                });
            let content = lines[i + 1..close].join("\n");

            let marked = !pending_roles.is_empty() || pending_exempt;
            sweep.push(FenceSweep { start_line, marked });

            for (id, role) in &pending_roles {
                let entry = examples.entry(id.clone()).or_default();
                match role.as_str() {
                    "before" | "after" => {
                        let (b, a) = split_before_after(&content);
                        if role == "before" {
                            entry.before = b.or(entry.before.take());
                        } else {
                            entry.after = a.or(entry.after.take());
                        }
                    }
                    "script" => {
                        entry.script = Some(content.clone());
                    }
                    other => panic!(
                        "unknown backbuild-example role '{other}' for id '{id}' at line \
                         {start_line} in {GUIDE_PATH} — expected before, after, or script"
                    ),
                }
            }

            pending_roles.clear();
            pending_exempt = false;
            i = close + 1;
            continue;
        }

        // Any non-marker, non-fence line resets pending markers — they only
        // ever apply to the fence immediately following them.
        pending_roles.clear();
        pending_exempt = false;
        i += 1;
    }

    (examples, sweep)
}

fn normalize_statements(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for raw_line in block.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line);
        if line.ends_with(';') {
            let stmt = current.trim_end_matches(';').trim();
            out.push(collapse_ws(stmt));
            current.clear();
        }
    }
    let rest = current.trim();
    if !rest.is_empty() {
        out.push(collapse_ws(rest.trim_end_matches(';').trim()));
    }
    out
}

/// Collapse whitespace runs to a single space, then close up the space a
/// line-break introduces immediately inside a paren pair (e.g. the guide's
/// `FROM (\n  SELECT ...\n) s` pretty-printing, which the line-join step
/// otherwise turns into `FROM ( SELECT ... ) s` — the emitted statement text
/// has no such space).
fn collapse_ws(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
}

// ===== Per-example registry =====

enum BeforeAfterSource {
    /// Extracted from the guide's own `before`/`after`-marked block(s).
    Doc,
    /// Hand-authored here because the guide shows only a `script` block for
    /// this sub-example (no standalone before/after fence) — the wrapped-
    /// expression LEFT JOIN follow-on, which continues the prior example's
    /// before-definition with a different after-definition described only
    /// in prose.
    Synthetic {
        before: &'static str,
        after: &'static str,
    },
}

struct DocExample {
    id: &'static str,
    fixture: &'static str,
    source: BeforeAfterSource,
    inputs: fn(after_sql: &str) -> BackbuildInputs,
    /// Every independently-runnable statement script this example proves —
    /// almost always one; the LEFT JOIN dual-option example shows two
    /// alternative single-atom scripts back to back, so this returns two.
    /// Flattened in order for the byte-compare against the guide's `script`
    /// block; each entry is separately oracle-verified.
    scripts: fn(&BackbuildOptions) -> Vec<Vec<String>>,
    /// Whether the guide shows a `script`-marked fence for this id at all.
    /// `chain_joins` proves its composed script only via the oracle test —
    /// the guide's prose describes it without printing the SQL.
    has_script_block: bool,
}

fn registry() -> Vec<DocExample> {
    vec![
        DocExample {
            id: "intro",
            fixture: "CREATE TABLE orders (id INTEGER, amount INTEGER, rate INTEGER);
                      INSERT INTO orders VALUES (1, 100, 2), (2, 50, 3), (3, 10, 5);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "fix_in_place",
            fixture: "CREATE TABLE orders (id INTEGER, amount INTEGER, rate INTEGER);
                      INSERT INTO orders VALUES (1, 100, 2), (2, 50, 3), (3, 10, 5);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "rename",
            fixture: "CREATE TABLE orders (id INTEGER, amount INTEGER);
                      INSERT INTO orders VALUES (1, 10), (2, 20);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "add_stored",
            fixture: "CREATE TABLE orders (id INTEGER, price INTEGER, qty INTEGER);
                      INSERT INTO orders VALUES (1, 10, 2), (2, 5, 3), (3, 7, 0);",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[("total", "INTEGER")]),
                ..plain_inputs(after)
            },
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "pullthrough",
            fixture:
                "CREATE TABLE orders (order_id INTEGER, customer TEXT, discount INTEGER);
                      INSERT INTO orders VALUES (1, 'alice', 10), (2, 'bob', 20), (3, 'carol', 30);",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[("discount", "INTEGER")]),
                sources: [(
                    "o".to_string(),
                    source_ref("orders", &["order_id"], &["order_id"]),
                )]
                .into_iter()
                .collect(),
                ..plain_inputs(after)
            },
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "aggregate",
            fixture: "CREATE TABLE orders (customer_id INTEGER, qty INTEGER);
                      INSERT INTO orders VALUES
                        (1, 5), (1, -100),
                        (2, 10), (2, 3),
                        (3, -1);",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                not_null_columns: ["customer_id".to_string()].into_iter().collect(),
                added_column_types: added_types(&[("total_qty", "HUGEINT")]),
                ..plain_inputs(after)
            },
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "left_join_enrich",
            fixture: "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER);
                      CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
                      INSERT INTO orders VALUES (1, 100), (2, 100), (3, 200);
                      INSERT INTO customers VALUES (100, 'alice'), (200, 'bob');",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[("customer_name", "TEXT")]),
                sources: [(
                    "c".to_string(),
                    source_ref("customers", &["customer_id"], &["customer_id"]),
                )]
                .into_iter()
                .collect(),
                ..plain_inputs(after)
            },
            scripts: |options| {
                let atom = &options.atoms[0];
                let update_from = atom
                    .options
                    .iter()
                    .find(|o| o.technique == Technique::JoinEnrichmentUpdateFrom)
                    .expect("update-from option");
                let scalar_subquery = atom
                    .options
                    .iter()
                    .find(|o| o.technique == Technique::JoinEnrichmentScalarSubquery)
                    .expect("scalar-subquery option");
                vec![
                    update_from.statements.clone(),
                    scalar_subquery.statements.clone(),
                ]
            },
            has_script_block: true,
        },
        DocExample {
            id: "left_join_wrapped",
            fixture: "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER);
                      CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
                      INSERT INTO orders VALUES (1, 100), (2, 300);
                      INSERT INTO customers VALUES (100, 'alice');",
            source: BeforeAfterSource::Synthetic {
                before: "SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM orders o",
                after: "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
                         COALESCE(c.customer_name, 'none') AS customer_label FROM orders o LEFT \
                         JOIN customers c ON o.customer_id = c.customer_id",
            },
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[("customer_label", "TEXT")]),
                sources: [(
                    "c".to_string(),
                    source_ref("customers", &["customer_id"], &["customer_id"]),
                )]
                .into_iter()
                .collect(),
                ..plain_inputs(after)
            },
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "chain_joins",
            fixture: "CREATE TABLE orders (order_id INTEGER, dim1_id INTEGER);
                      CREATE TABLE dim1 (dim1_id INTEGER, region_id INTEGER);
                      CREATE TABLE dim2 (region_id INTEGER, region_name TEXT);
                      INSERT INTO orders VALUES (1, 10), (2, 20), (3, 10);
                      INSERT INTO dim1 VALUES (10, 100), (20, 200);
                      INSERT INTO dim2 VALUES (100, 'north'), (200, 'south');",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[
                    ("region_id", "INTEGER"),
                    ("region_name", "TEXT"),
                ]),
                sources: [
                    (
                        "d1".to_string(),
                        source_ref("dim1", &["dim1_id"], &["dim1_id"]),
                    ),
                    (
                        "d2".to_string(),
                        source_ref("dim2", &["region_id"], &["region_id"]),
                    ),
                ]
                .into_iter()
                .collect(),
                ..plain_inputs(after)
            },
            scripts: |options| {
                vec![assemble(
                    options,
                    &Selection::Targeted {
                        atom_choices: vec![0; options.atoms.len()],
                    },
                )]
            },
            has_script_block: false,
        },
        DocExample {
            id: "window",
            fixture: "CREATE TABLE orders (order_id INTEGER, status TEXT, amount INTEGER);
                      INSERT INTO orders VALUES
                        (1, 'open', 10), (2, 'open', 20), (3, 'closed', 5), (4, 'closed', 7),
                        (5, 'open', 1);",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                row_identity: Some(vec!["order_id".to_string()]),
                not_null_columns: ["order_id".to_string()].into_iter().collect(),
                added_column_types: added_types(&[("rn", "BIGINT")]),
                ..plain_inputs(after)
            },
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "tighten",
            fixture: "CREATE TABLE orders (id INTEGER, status TEXT);
                      INSERT INTO orders VALUES (1, 'active'), (2, 'cancelled'), (3, 'active');",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "extend_history",
            fixture: "CREATE TABLE events (ts DATE, amount INTEGER);
                      INSERT INTO events VALUES
                        ('2023-06-01', 1),
                        ('2024-06-01', 2),
                        ('2025-06-01', 3);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "union_add",
            fixture: "CREATE TABLE events_a (id INTEGER);
                      CREATE TABLE events_b (id INTEGER);
                      INSERT INTO events_a VALUES (1), (2);
                      INSERT INTO events_b VALUES (3);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "union_remove",
            fixture: "CREATE TABLE events_a (id INTEGER);
                      CREATE TABLE events_b (id INTEGER);
                      INSERT INTO events_a VALUES (1), (2);
                      INSERT INTO events_b VALUES (3);",
            source: BeforeAfterSource::Doc,
            inputs: plain_inputs,
            scripts: single_atom_option,
            has_script_block: true,
        },
        DocExample {
            id: "composite",
            fixture: "CREATE TABLE orders (id INTEGER, price INTEGER, extra INTEGER, qty INTEGER);
                      INSERT INTO orders VALUES (1, 10, 999, 3), (2, 20, 888, 0), (3, 30, 777, 5);",
            source: BeforeAfterSource::Doc,
            inputs: |after| BackbuildInputs {
                added_column_types: added_types(&[("unit_price", "INTEGER")]),
                ..plain_inputs(after)
            },
            scripts: |options| {
                vec![assemble(
                    options,
                    &Selection::Targeted {
                        atom_choices: vec![0; options.atoms.len()],
                    },
                )]
            },
            has_script_block: true,
        },
    ]
}

fn single_atom_option(options: &BackbuildOptions) -> Vec<Vec<String>> {
    vec![options.atoms[0].options[0].statements.clone()]
}

/// Resolve one example's (before_sql, after_sql) — from the guide's own
/// extracted blocks, or the registry's synthetic fallback for the one
/// sub-example the guide shows no standalone before/after fence for.
fn resolve_before_after(example: &DocExample, extracted: &ExtractedBlocks) -> (String, String) {
    match &example.source {
        BeforeAfterSource::Synthetic { before, after } => (before.to_string(), after.to_string()),
        BeforeAfterSource::Doc => {
            let before = extracted.before.clone().unwrap_or_else(|| {
                panic!(
                    "example '{}' expects a doc-extracted `before` block but none was marked \
                     in {GUIDE_PATH}",
                    example.id
                )
            });
            let after = extracted.after.clone().unwrap_or_else(|| {
                panic!(
                    "example '{}' expects a doc-extracted `after` block but none was marked in \
                     {GUIDE_PATH}",
                    example.id
                )
            });
            (before, after)
        }
    }
}

fn compute_options(example: &DocExample, before_sql: &str, after_sql: &str) -> BackbuildOptions {
    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    let inputs = (example.inputs)(after_sql);
    derive_backbuild_options(&diff, &inputs)
}

// ===== Tests =====

#[test]
fn doc_examples_match_emitters() {
    let doc = fs::read_to_string(GUIDE_PATH).unwrap_or_else(|e| panic!("read {GUIDE_PATH}: {e}"));
    let (extracted, _sweep) = extract(&doc);

    let regen = std::env::var("SMELT_REGEN_DOCS").as_deref() == Ok("1");
    let mut regenerated = doc.clone();
    let mut any_regenerated = false;

    for example in registry() {
        let blocks = extracted.get(example.id).cloned().unwrap_or_default();
        let (before_sql, after_sql) = resolve_before_after(&example, &blocks);
        let options = compute_options(&example, &before_sql, &after_sql);
        let expected: Vec<String> = (example.scripts)(&options).into_iter().flatten().collect();

        if !example.has_script_block {
            // Proven by `doc_examples_pass_the_oracle` instead — the guide
            // doesn't print this example's script text.
            continue;
        }

        let script_block = blocks.script.clone().unwrap_or_else(|| {
            panic!(
                "example '{}' is registered with has_script_block: true but no `script` marker \
                 was found in {GUIDE_PATH} — regeneration hint: add \
                 `<!-- backbuild-example({}): script -->` above its fenced ```sql block",
                example.id, example.id
            )
        });

        if regen {
            let rendered = render_script(&expected);
            if collapse_block(&script_block) != collapse_block(&rendered) {
                regenerated = replace_script_block(&regenerated, example.id, &rendered);
                any_regenerated = true;
            }
            continue;
        }

        let actual = normalize_statements(&script_block);
        assert_eq!(
            actual, expected,
            "example '{}': the guide's `script` block does not match \
             definition_diff -> derive_backbuild_options -> assemble's real output.\n\
             Regeneration hint: run `SMELT_REGEN_DOCS=1 cargo test -p smelt-logical --test \
             backbuild_docs doc_examples_match_emitters` to rewrite it in place, then restore \
             any hand-authored comments/formatting and rerun without the env var.",
            example.id
        );
    }

    if regen && any_regenerated {
        fs::write(GUIDE_PATH, regenerated)
            .unwrap_or_else(|e| panic!("write regenerated {GUIDE_PATH}: {e}"));
    }
}

/// Render a flattened statement list the same way regeneration writes a
/// `script` block: one statement per line, semicolon-terminated unless the
/// whole script is a single statement (this guide's existing house style
/// for a lone statement).
fn render_script(statements: &[String]) -> String {
    if statements.len() <= 1 {
        statements.join("\n")
    } else {
        statements
            .iter()
            .map(|s| format!("{s};"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn collapse_block(block: &str) -> Vec<String> {
    normalize_statements(block)
}

/// Rewrite id's `script`-marked fence content to `new_content`, leaving
/// every other line (including this id's own `before`/`after` fences and
/// all prose) untouched. Only used in `SMELT_REGEN_DOCS=1` mode.
fn replace_script_block(doc: &str, id: &str, new_content: &str) -> String {
    let marker = format!("<!-- backbuild-example({id}): script -->");
    let lines: Vec<&str> = doc.lines().collect();
    let marker_idx = lines
        .iter()
        .position(|l| l.trim() == marker)
        .unwrap_or_else(|| panic!("regen: no script marker found for '{id}'"));
    let fence_start = marker_idx + 1;
    assert_eq!(
        lines[fence_start].trim(),
        "```sql",
        "regen: expected a ```sql fence immediately after '{id}'s script marker"
    );
    let fence_end = lines[fence_start + 1..]
        .iter()
        .position(|l| l.trim() == "```")
        .map(|p| fence_start + 1 + p)
        .unwrap_or_else(|| panic!("regen: unterminated fence for '{id}'"));

    let mut out: Vec<String> = lines[..=fence_start]
        .iter()
        .map(|s| s.to_string())
        .collect();
    out.extend(new_content.lines().map(|s| s.to_string()));
    out.extend(lines[fence_end..].iter().map(|s| s.to_string()));
    out.join("\n") + "\n"
}

#[test]
fn doc_examples_pass_the_oracle() {
    let Ok(_) = std::env::var("DUCKDB_LIB_DIR").or_else(|_| std::env::var("HOME")) else {
        panic!("DUCKDB_LIB_DIR / HOME not set — cannot locate DuckDB");
    };
    let conn = match Connection::open_in_memory() {
        Ok(conn) => conn,
        Err(e) => {
            panic!(
                "could not open an in-memory DuckDB connection ({e}) — set DUCKDB_LIB_DIR per \
                 CLAUDE.md before running backbuild_docs"
            );
        }
    };

    let doc = fs::read_to_string(GUIDE_PATH).unwrap_or_else(|e| panic!("read {GUIDE_PATH}: {e}"));
    let (extracted, _sweep) = extract(&doc);

    for example in registry() {
        let blocks = extracted.get(example.id).cloned().unwrap_or_default();
        let (before_sql, after_sql) = resolve_before_after(&example, &blocks);
        let options = compute_options(&example, &before_sql, &after_sql);
        let variants = (example.scripts)(&options);

        harness::stage_inputs(&conn, example.fixture);
        for variant in &variants {
            harness::verify_script(&conn, "t", &before_sql, &after_sql, variant);
        }
        conn.execute_batch("DROP TABLE IF EXISTS t;")
            .unwrap_or_else(|e| panic!("drop t after example '{}': {e}", example.id));
        // Drop every fixture table the example may have staged, so the next
        // example's `CREATE TABLE` doesn't collide.
        for tbl in [
            "orders",
            "events",
            "events_a",
            "events_b",
            "events_c",
            "customers",
            "dim1",
            "dim2",
        ] {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS {tbl};"))
                .unwrap_or_else(|e| panic!("drop fixture table {tbl}: {e}"));
        }
    }
}

/// Sweep rule: every fenced ```sql block in the guide must sit immediately
/// under at least one `<!-- backbuild-example(<id>): <role> -->` marker or a
/// `<!-- backbuild-example-exempt: <reason> -->` marker — no exceptions.
/// This is deliberately stronger than "every *emitted-script* block is
/// marked" (the plan's minimum bar): requiring `before`/`after` blocks to
/// carry a marker too means a new hand-written example cannot be added
/// without either wiring it into `registry()` (so `doc_examples_match_emitters`
/// / `doc_examples_pass_the_oracle` prove it) or explicitly exempting it —
/// there is no unmarked middle ground for drift to hide in.
///
/// As of this phase, every ```sql block on the page is derivable end-to-end
/// through the real classifier, so zero exemption markers exist. If a truly
/// illustrative, non-derivable snippet is ever added (e.g. showing a refused
/// shape's *would-be* SQL), it should carry
/// `<!-- backbuild-example-exempt: <reason> -->` rather than a role marker.
#[test]
fn every_script_block_is_marked() {
    let doc = fs::read_to_string(GUIDE_PATH).unwrap_or_else(|e| panic!("read {GUIDE_PATH}: {e}"));
    let (_extracted, sweep) = extract(&doc);

    assert!(
        !sweep.is_empty(),
        "expected at least one ```sql fence in {GUIDE_PATH}"
    );

    let unmarked: Vec<usize> = sweep
        .iter()
        .filter(|f| !f.marked)
        .map(|f| f.start_line)
        .collect();
    assert!(
        unmarked.is_empty(),
        "unmarked ```sql fence(s) in {GUIDE_PATH} at line(s) {unmarked:?} — every fenced SQL \
         block must be preceded by a `<!-- backbuild-example(<id>): before|after|script -->` \
         marker or a `<!-- backbuild-example-exempt: <reason> -->` marker (see this test's doc \
         comment)"
    );
}

/// Every id `registry()` declares must actually be marked in the guide, and
/// every id marked in the guide must be registered — an orphaned marker (a
/// section renamed without updating its id, or a stale example) is caught
/// here rather than silently ignored by `doc_examples_match_emitters`'s
/// `unwrap_or_default`.
#[test]
fn registry_matches_guide_markers() {
    let doc = fs::read_to_string(GUIDE_PATH).unwrap_or_else(|e| panic!("read {GUIDE_PATH}: {e}"));
    let (extracted, _sweep) = extract(&doc);

    let registered: BTreeSet<&str> = registry().iter().map(|e| e.id).collect();
    let marked: BTreeSet<&str> = extracted.keys().map(|s| s.as_str()).collect();

    assert_eq!(
        registered, marked,
        "registry() ids and the guide's marked example ids must match exactly"
    );
}

/// Expand one `{a,b,c}` brace-alternation group in `pattern` into every
/// concrete alternative (`definition_deltas.md` §References uses this to
/// list sibling files compactly, e.g.
/// `crates/smelt-logical/src/backbuild/{mod,diff,classify}.rs`). A pattern
/// with no brace group expands to itself.
fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(close) = pattern[open..].find('}').map(|i| open + i) else {
        return vec![pattern.to_string()];
    };
    let prefix = &pattern[..open];
    let suffix = &pattern[close + 1..];
    pattern[open + 1..close]
        .split(',')
        .map(|alt| format!("{prefix}{alt}{suffix}"))
        .collect()
}

/// Every backtick-quoted, slash-containing path inside `definition_deltas.md`
/// §References → Code / Tests must exist on disk — the doc-sync equivalent
/// of `registry_matches_guide_markers` for the spec's own bookkeeping
/// section, so a renamed or deleted module silently rots the §References
/// list instead of failing a test.
#[test]
fn spec_references_are_live_paths() {
    let doc = fs::read_to_string(DEFINITION_DELTAS_SPEC_PATH)
        .unwrap_or_else(|e| panic!("read {DEFINITION_DELTAS_SPEC_PATH}: {e}"));

    let references_start = doc
        .find("\n## References")
        .unwrap_or_else(|| panic!("no '## References' heading in {DEFINITION_DELTAS_SPEC_PATH}"));
    let section = &doc[references_start..];
    let section_end = section[1..]
        .find("\n## ")
        .map(|i| i + 1)
        .unwrap_or(section.len());
    let section = &section[..section_end];

    let mut missing = Vec::new();
    for bullet_label in ["**Code**", "**Tests**"] {
        let bullet_start = section
            .find(&format!("- {bullet_label}"))
            .unwrap_or_else(|| {
                panic!(
                "no '- {bullet_label}' bullet under §References in {DEFINITION_DELTAS_SPEC_PATH}"
            )
            });
        let bullet = &section[bullet_start..];
        let bullet_end = bullet[bullet_label.len()..]
            .find("\n- **")
            .map(|i| i + bullet_label.len())
            .unwrap_or(bullet.len());
        let bullet = &bullet[..bullet_end];

        let mut checked_any = false;
        let mut rest = bullet;
        while let Some(open) = rest.find('`') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find('`') else {
                break;
            };
            let span = &rest[..close];
            rest = &rest[close + 1..];
            if !span.contains('/') {
                continue;
            }
            checked_any = true;
            for candidate in expand_braces(span) {
                let full = std::path::Path::new(WORKSPACE_ROOT).join(&candidate);
                if !full.exists() {
                    missing.push(format!("{bullet_label}: `{candidate}`"));
                }
            }
        }
        assert!(
            checked_any,
            "no slash-containing backtick path found in the '- {bullet_label}' bullet under \
             §References in {DEFINITION_DELTAS_SPEC_PATH}"
        );
    }

    assert!(
        missing.is_empty(),
        "definition_deltas.md §References names path(s) that don't exist on disk: {missing:?}"
    );
}
