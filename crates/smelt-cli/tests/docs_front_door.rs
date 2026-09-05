//! Standing gate for `docs/outcomes/20260904-delta-signature-front-door`
//! phase 3: the incremental-models guide must open on delta signatures, not
//! on the DELETE+INSERT mechanics, and the docs-site must never regress to
//! the retired "four corners" framing.

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
/// opening ` ``` <lang>` line and the closing ` ``` ` line).
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

#[test]
fn incremental_guide_first_section_introduces_delta_signatures() {
    let path = docs_site_dir().join("guide/incremental-models.md");
    let text = fs::read_to_string(&path).unwrap();

    let first_h2 = text.find("\n## ").expect("guide has no `##` section");
    let second_h2 = text[first_h2 + 1..]
        .find("\n## ")
        .map(|i| first_h2 + 1 + i)
        .unwrap_or(text.len());
    let first_section_body = &text[first_h2..second_h2];
    assert!(
        first_section_body
            .to_lowercase()
            .contains("delta signature"),
        "{}: the guide's first `##` section must introduce delta signatures",
        path.display()
    );

    let signature_pos = text
        .to_lowercase()
        .find("signature")
        .expect("guide never mentions \"signature\"");
    let configuration_pos = text
        .find("\n## Configuration")
        .expect("guide has no `## Configuration` heading");
    assert!(
        signature_pos < configuration_pos,
        "{}: \"signature\" must appear before `## Configuration`",
        path.display()
    );
}

#[test]
fn incremental_guide_front_door_headline_matches_real_explain_output() {
    let real = run_explain("user_daily_spend");
    let real_first_line = real.lines().next().unwrap();

    let path = docs_site_dir().join("guide/incremental-models.md");
    let text = fs::read_to_string(&path).unwrap();

    let blocks = fenced_blocks(&text);
    let explain_block = blocks
        .iter()
        .find(|b| b.iter().any(|l| l.starts_with("$ smelt explain")))
        .expect("guide has no `$ smelt explain` fenced block");
    let headline = explain_block
        .iter()
        .find(|l| l.starts_with("model "))
        .unwrap_or_else(|| panic!("front-door explain block has no headline line"));

    assert_eq!(
        *headline,
        real_first_line,
        "{}'s front-door `smelt explain user_daily_spend` headline has drifted from real \
         output — regenerate it from a fresh \
         `smelt explain user_daily_spend --project-dir examples/timeseries` run",
        path.display()
    );
}

/// Ratchet: the retired "four corners" framing must never reappear under
/// `docs-site/docs/`. The framing was already fully purged before this gate
/// was written; this test exists to keep it that way.
#[test]
fn no_four_corners_framing_under_docs_site() {
    let mut files = Vec::new();
    walk_markdown_files(&docs_site_dir(), &mut files);
    files.sort();

    let mut offenders = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        if text.to_lowercase().contains("four corners")
            || text.to_lowercase().contains("four-corners")
        {
            offenders.push(
                path.strip_prefix(repo_root())
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    assert!(
        offenders.is_empty(),
        "docs-site files with retired \"four corners\" framing: {offenders:#?}"
    );
}
