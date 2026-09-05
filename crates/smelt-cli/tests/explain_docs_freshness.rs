//! Standing drift gate for `docs/outcomes/20260904-delta-signature-front-door`
//! criterion 4 (and the docs half of criterion 1): every `smelt explain`
//! excerpt committed under `docs-site/docs/` must lead with the model's
//! delta-signature headline (`docs/specs/incremental_models.md` §Surface
//! "CLI"), and the two *hand-pasted* excerpts (`reference/cli.md`,
//! `guide/incremental-models.md`) must stay byte-identical to what the real
//! binary prints for the model they show. Pipeline-generated excerpts
//! (`docs-site/docs/examples/web-analytics/`) are covered by
//! `tutorial_freshness.rs` instead — this test only asserts the headline
//! invariant on them, not full freshness.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn docs_site_dir() -> PathBuf {
    repo_root().join("docs-site/docs")
}

fn walk_markdown_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            walk_markdown_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn run_explain(model: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args(["explain", model, "--project-dir", "examples/timeseries"])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("spawn smelt explain {model}: {e}"));
    assert!(
        output.status.success(),
        "smelt explain {model} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Extracts fenced code blocks' bodies (the lines strictly between the
/// opening ` ``` <lang>` line and the closing ` ``` ` line — the language
/// tag itself is not content).
fn fenced_blocks(text: &str) -> Vec<Vec<&str>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_fence {
                out.push(std::mem::take(&mut current));
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            current.push(line);
        }
    }
    out
}

/// Every fenced block in the docs-site that shows a `Maintenance plan: <name>`
/// report must have that report's headline (`model <name>  (emits: …)`) as
/// its first non-`$`-prompt line — never the retired `Maintenance plan:`
/// line printed on its own first.
#[test]
fn every_maintenance_plan_excerpt_leads_with_the_headline() {
    let mut files = Vec::new();
    walk_markdown_files(&docs_site_dir(), &mut files);
    files.sort();

    let mut offenders = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        for block in fenced_blocks(&text) {
            if !block.iter().any(|l| l.contains("Maintenance plan: ")) {
                continue;
            }
            let first_content_line = block
                .iter()
                .map(|l| l.trim_end())
                .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('$'));
            let Some(first_line) = first_content_line else {
                continue;
            };
            if !first_line.starts_with("model ") {
                offenders.push(format!(
                    "{}: block's first line is {:?}, expected `model <name>  (emits: …)`",
                    path.strip_prefix(repo_root()).unwrap().display(),
                    first_line
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "docs-site excerpts with a maintenance-plan report that don't lead with the \
         delta-signature headline: {offenders:#?}"
    );
}

/// `docs-site/docs/reference/cli.md`'s `smelt explain daily_events` sample
/// must stay byte-identical to what the real binary prints today.
#[test]
fn cli_reference_sample_matches_real_explain_output() {
    let real = run_explain("daily_events");
    let path = docs_site_dir().join("reference/cli.md");
    let text = fs::read_to_string(&path).unwrap();

    let marker = "$ smelt explain daily_events\n";
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("{}: no `{marker:?}` prompt line found", path.display()))
        + marker.len();
    let rest = &text[start..];
    let end = rest
        .find("\n```")
        .unwrap_or_else(|| panic!("{}: no closing fence after the sample", path.display()));
    let committed = &rest[..end];

    assert_eq!(
        committed.trim_end_matches('\n'),
        real.trim_end_matches('\n'),
        "{}'s `smelt explain daily_events` sample has drifted from real output — \
         regenerate it from a fresh `smelt explain daily_events --project-dir examples/timeseries` run",
        path.display()
    );
}

/// `docs-site/docs/guide/incremental-models.md`'s `smelt explain
/// daily_events_enriched` excerpt is deliberately elided after its first two
/// cells with `...`; only the un-elided prefix (headline + `Maintenance
/// plan:` line + first cell) is pinned against real output.
#[test]
fn incremental_guide_headline_matches_real_explain_output() {
    let real = run_explain("daily_events_enriched");
    let real_lines: Vec<&str> = real.lines().collect();

    let path = docs_site_dir().join("guide/incremental-models.md");
    let text = fs::read_to_string(&path).unwrap();

    let marker = "$ smelt explain daily_events_enriched\n";
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("{}: no `{marker:?}` prompt line found", path.display()))
        + marker.len();
    let rest = &text[start..];
    let end = rest
        .find("\n```")
        .unwrap_or_else(|| panic!("{}: no closing fence after the excerpt", path.display()));
    let committed = &rest[..end];
    let committed_lines: Vec<&str> = committed.lines().collect();

    // Compare every committed line up to (not including) the first elision
    // marker `...` against the real output at the same line index.
    let prefix_len = committed_lines
        .iter()
        .position(|l| l.trim() == "...")
        .unwrap_or(committed_lines.len());

    assert!(
        prefix_len >= 2,
        "{}: excerpt has no un-elided prefix to check",
        path.display()
    );
    assert!(
        real_lines.len() >= prefix_len,
        "real `smelt explain daily_events_enriched` output has fewer lines ({}) than the \
         committed excerpt's un-elided prefix ({prefix_len})",
        real_lines.len()
    );

    assert_eq!(
        &committed_lines[..prefix_len],
        &real_lines[..prefix_len],
        "{}'s `smelt explain daily_events_enriched` excerpt's un-elided prefix has drifted \
         from real output — regenerate it from a fresh \
         `smelt explain daily_events_enriched --project-dir examples/timeseries` run",
        path.display()
    );
}
