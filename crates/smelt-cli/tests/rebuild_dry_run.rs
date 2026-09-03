//! `smelt rebuild --dry-run` renders the emitted maintenance statements with
//! per-chunk boundaries (`docs/specs/cli.md` §"`--dry-run` prints the
//! maintenance statements"): when the batch-safety classification splits the
//! range, statements print once per chunk, each chunk introduced by a boundary
//! line naming its `[start, end)` window and position, in real execution order.
//!
//! Real fixture: `examples/web_analytics/silver.sessions` has a derived
//! `partition_column` (`session_start_date`) whose Form B relation
//! (`event_date BETWEEN session_start_date AND session_start_date + INTERVAL
//! '1 day'`) skews the driving `event_date` column forward by one day, so
//! its **derived output window** (`docs/specs/model_transforms.md`
//! §Semantics "The output window is derived, never assumed") widens the
//! declared `[2026-03-01, 2026-03-29)` run window one day *backward* (the
//! Form B skew inverted): `[2026-02-28, 2026-03-29)`. That 29-day derived
//! window is what the bound-based batch-safety roll-up
//! (`crate::compile::batch_safety_for_model`, over the *expanded* SQL — a
//! call site fixed in phase 2 of `docs/outcomes/
//! 20260815-partition-grain-residue` to actually see through
//! `smelt.functions.sessionize`'s body rather than silently no-op'ing on a
//! frontmatter parse failure) auto-splits — no `--per-partition`/
//! `--batch-size` override is used. The composed bound over
//! `silver.events_deduped` is 4 days (verified via `smelt explain
//! silver.sessions`'s scan clamp `before=P4D`), giving a 12-day chunk size
//! (`context_days * 3`, `crates/smelt-logical/src/rules/incremental.rs`'s
//! `min_chunk.clamp(7, 90)` — 12 clears the 7-day floor, so no clamping
//! applies).

use std::path::Path;
use std::process::Command;

/// The 28-day declared run window `[2026-03-01, 2026-03-29)` derives a 29-day
/// output window `[2026-02-28, 2026-03-29)` (the model's own Form B skew,
/// inverted), which the bound-based batch-safety roll-up's 12-day chunk size
/// (`context_days * 3`, composed over `silver.events_deduped`'s 4-day bound —
/// see the module doc comment) auto-splits into three chunks (two full
/// 12-day chunks plus a 5-day remainder); the dry-run prints one
/// `DELETE`+`INSERT` block per chunk for `silver.sessions`, each introduced
/// by `-- chunk k/N: [start, end)` in execution order, and no backend is
/// opened (the run succeeds against a project whose `.duckdb` target need
/// not exist).
#[test]
fn chunked_range_prints_per_chunk_boundaries() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("rebuild")
        .arg("silver.sessions")
        .arg("--start")
        .arg("2026-03-01")
        .arg("--end")
        .arg("2026-03-29")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt rebuild silver.sessions --dry-run");

    assert!(
        output.status.success(),
        "rebuild --dry-run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Each chunk introduced by its boundary line, in order. The derived
    // output window [2026-02-28, 2026-03-29) — the declared window widened
    // one day backward by the model's own Form B skew, inverted — splits
    // into two full 12-day chunks plus a 5-day remainder.
    for (k, (start, end)) in [
        ("2026-02-28", "2026-03-12"),
        ("2026-03-12", "2026-03-24"),
        ("2026-03-24", "2026-03-29"),
    ]
    .iter()
    .enumerate()
    {
        let line = format!("-- chunk {}/3: [{}, {})", k + 1, start, end);
        assert!(
            stdout.contains(&line),
            "expected per-chunk boundary line `{line}` in the dry-run output:\n{stdout}"
        );
    }

    // The chunks print in execution order (chunk 1 before chunk 3).
    let pos1 = stdout.find("-- chunk 1/3:").expect("chunk 1 present");
    let pos3 = stdout.find("-- chunk 3/3:").expect("chunk 3 present");
    assert!(pos1 < pos3, "chunks must print in execution order");

    // Each chunk carries the emitted maintenance statements, transactionally
    // bracketed, with real literals (no symbolic placeholders).
    assert!(
        stdout.contains("DELETE FROM main.silver_sessions WHERE")
            && stdout.contains("INSERT INTO main.silver_sessions "),
        "expected the region DELETE+INSERT for silver.sessions:\n{stdout}"
    );
    assert!(
        stdout.contains("BEGIN") && stdout.contains("COMMIT"),
        "expected transactional BEGIN/COMMIT bracketing:\n{stdout}"
    );
    assert!(
        !stdout.contains("{{window_start}}") && !stdout.contains("{{window_end}}"),
        "dry-run literals must be real, not placeholders:\n{stdout}"
    );
    // The real (derived output) window literals appear in the DELETE predicates.
    assert!(
        stdout.contains("'2026-02-28'") && stdout.contains("'2026-03-24'"),
        "expected real chunk window literals in the statements:\n{stdout}"
    );
}

/// `smelt backbuild` no longer exists — the verb renamed to `smelt rebuild`
/// with no compatibility alias (`docs/outcomes/20260815-definition-delta-migrate/`
/// phase 4). Running the old verb must fail as an unrecognised subcommand,
/// exiting with clap's standard usage-error code (2).
#[test]
fn backbuild_verb_is_gone() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("backbuild")
        .arg("silver.sessions")
        .arg("--start")
        .arg("2026-03-01")
        .arg("--end")
        .arg("2026-03-29")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt backbuild silver.sessions --dry-run");

    assert_eq!(
        output.status.code(),
        Some(2),
        "smelt backbuild must exit 2 (unrecognised subcommand); stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backbuild"),
        "expected clap's unrecognised-subcommand error to name `backbuild`:\n{stderr}"
    );
}

/// `smelt --help` must list the `rebuild` subcommand and must not mention the
/// retired `backbuild` name anywhere in its output.
#[test]
fn help_lists_rebuild() {
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("--help")
        .output()
        .expect("spawn smelt --help");

    assert!(
        output.status.success(),
        "smelt --help failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rebuild"),
        "expected `rebuild` in --help output:\n{stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("backbuild"),
        "expected no mention of retired `backbuild` verb in --help output:\n{stdout}"
    );
}

/// The criterion-8 rename sweep (`smelt backbuild` → `smelt rebuild`) and the
/// phase-1 diagnostic rename (the old skeleton-column-added spelling →
/// `MaintenanceSkeletonChanged`) have no other standing gate over prose, so a
/// spec or user-doc edit could silently reintroduce either stale name.
/// Scoped to `docs/specs/` and `docs-site/docs/` only — `docs/plans/` and
/// `docs/research/` are historical records that legitimately still name the
/// retired verb/code, and this file's own negative tests above are the
/// intentional exception (they assert the retired verb is rejected). The old
/// diagnostic spelling is built via `concat!` rather than spelled literally
/// so this very file does not trip `smelt-db`'s own
/// `no_stale_skeleton_column_added_spelling` gate.
#[test]
fn no_stale_backbuild_verb_or_diagnostic_name_in_docs() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root exists");
    let stale_diagnostic_spelling = ["Maintenance", "Skeleton", "Column", "Added"].concat();

    let mut hits = Vec::new();
    for dir in [
        repo_root.join("docs/specs"),
        repo_root.join("docs-site/docs"),
    ] {
        for entry in walk_markdown_files(&dir) {
            let content = std::fs::read_to_string(&entry)
                .unwrap_or_else(|e| panic!("read {}: {e}", entry.display()));
            for (lineno, line) in content.lines().enumerate() {
                if line.contains("smelt backbuild") || line.contains(&stale_diagnostic_spelling) {
                    hits.push(format!(
                        "{}:{}: {}",
                        entry.display(),
                        lineno + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "stale `smelt backbuild`/skeleton-column-added-spelling mention(s):\n{}",
        hits.join("\n")
    );
}

fn walk_markdown_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                out.push(path);
            }
        }
    }
    out
}
