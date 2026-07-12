//! `smelt backbuild --dry-run` renders the emitted maintenance statements with
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
//! window is what the `bounded_safe(chunk=9d,…)` classification auto-splits
//! — no `--per-partition`/`--batch-size` override is used. The 9-day chunk
//! size comes from the model's own 3-day context (a 2-day backward source
//! read, from `sessionize`'s `max_lookback` window frames, plus the 1-day
//! forward Form B skew): `context_days * 3`, clamped to `[7, 90]`.

use std::path::Path;
use std::process::Command;

/// The 28-day declared run window `[2026-03-01, 2026-03-29)` derives a 29-day
/// output window `[2026-02-28, 2026-03-29)` (the model's own Form B skew,
/// inverted), which a `bounded_safe(chunk=9d,…)` classification auto-splits
/// into four chunks (three full 9-day chunks plus a 2-day remainder); the
/// dry-run prints one `DELETE`+`INSERT` block per chunk for `silver.sessions`,
/// each introduced by `-- chunk k/N: [start, end)` in execution order, and no
/// backend is opened (the run succeeds against a project whose `.duckdb`
/// target need not exist).
#[test]
fn chunked_range_prints_per_chunk_boundaries() {
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

    assert!(
        output.status.success(),
        "backbuild --dry-run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Each chunk introduced by its boundary line, in order. The derived
    // output window [2026-02-28, 2026-03-29) — the declared window widened
    // one day backward by the model's own Form B skew, inverted — splits
    // into three full 9-day chunks plus a 2-day remainder.
    for (k, (start, end)) in [
        ("2026-02-28", "2026-03-09"),
        ("2026-03-09", "2026-03-18"),
        ("2026-03-18", "2026-03-27"),
        ("2026-03-27", "2026-03-29"),
    ]
    .iter()
    .enumerate()
    {
        let line = format!("-- chunk {}/4: [{}, {})", k + 1, start, end);
        assert!(
            stdout.contains(&line),
            "expected per-chunk boundary line `{line}` in the dry-run output:\n{stdout}"
        );
    }

    // The chunks print in execution order (chunk 1 before chunk 4).
    let pos1 = stdout.find("-- chunk 1/4:").expect("chunk 1 present");
    let pos4 = stdout.find("-- chunk 4/4:").expect("chunk 4 present");
    assert!(pos1 < pos4, "chunks must print in execution order");

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
        stdout.contains("'2026-02-28'") && stdout.contains("'2026-03-18'"),
        "expected real chunk window literals in the statements:\n{stdout}"
    );
}
