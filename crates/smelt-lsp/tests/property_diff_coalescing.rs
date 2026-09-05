//! S1 (Phase 7 review fix round 1): a `didChangeWatchedFiles` notification
//! carrying many `.git` change events under one project root must trigger
//! at most one property-diff derivation for that notification — not one
//! per event (`docs/outcomes/20260905-property-diff/phases/07-plan.md`
//! risk R3, and the plan's own claim: "the `running` flag coalesces; a
//! trailing re-run must be scheduled once, not per event").
//!
//! This calls `Backend::did_change_watched_files` DIRECTLY on a real
//! `Backend` (via `LspService::inner()`, never boxed/mocked) rather than
//! over the duplex-stream wire the other `property_diff_*` test files use.
//! While first writing this test, an ad hoc (uncommitted) over-the-wire
//! variant appeared to only deliver the first `FileEvent` of a multi-event
//! burst to the handler; re-investigated with a committed, reproducible
//! probe for `docs/outcomes/20260905-property-diff/phases/08-plan.md` task
//! 6 (evidence in `crates/smelt-lsp/CLAUDE.md`) and found NOT to reproduce
//! — a 10-event burst sent over the real `tower_lsp::Server::serve` +
//! `tokio::io::duplex` wire, including this test's own staged-repo
//! scenario, delivers all 10 events to the handler. The direct-call
//! approach here is kept anyway on its own merits — it isolates the
//! coalescing logic from wire framing and needs no duplex plumbing — not
//! because of a proven transport defect. `initialize`/`initialized` are
//! still called directly on the same `Backend` beforehand so the
//! notification hits the real, fully-initialized handler.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use tower_lsp::{LanguageServer, LspService};

use smelt_lsp::Backend;

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}: {}",
        args,
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_commit(dir: &Path, message: &str) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            message,
        ])
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git commit failed in {:?}: {}",
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".smelt" || name == ".git" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

fn stage_timeseries_repo(tmp: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp);
    git(tmp, &["init", "-q", "-b", "main"]);
    git(tmp, &["add", "-A"]);
    git_commit(tmp, "initial import of examples/timeseries");
}

/// Poll `counter` until it stops changing for `settle_ms`, or `timeout_ms`
/// elapses.
async fn wait_for_settled(counter: &AtomicUsize, settle_ms: u64, timeout_ms: u64) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut last = counter.load(Ordering::Relaxed);
    let mut last_change = std::time::Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let current = counter.load(Ordering::Relaxed);
        if current != last {
            last = current;
            last_change = std::time::Instant::now();
        } else if last_change.elapsed() >= std::time::Duration::from_millis(settle_ms) {
            return last;
        }
        if std::time::Instant::now() >= deadline {
            return last;
        }
    }
}

#[tokio::test]
async fn a_burst_of_git_change_events_under_one_project_triggers_one_derivation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();
    let counter = backend.property_diff_derivation_counter();

    let init_params: lsp_types::InitializeParams = serde_json::from_value(json!({
        "processId": null,
        "capabilities": { "workspace": { "workspaceFolders": true } },
        "workspaceFolders": [
            { "uri": format!("file://{}", tmp.path().display()), "name": "test" }
        ]
    }))
    .expect("well-formed InitializeParams");
    backend
        .initialize(init_params)
        .await
        .expect("initialize must succeed");
    backend.initialized(lsp_types::InitializedParams {}).await;

    // Wait out the `initialized`-triggered derivation before measuring the
    // burst, so its count doesn't contaminate the delta below.
    wait_for_settled(&counter, 500, 30_000).await;
    let before = counter.load(Ordering::Relaxed);
    assert!(
        before >= 1,
        "the initial workspace-load refresh must have derived at least once"
    );

    // One notification, ten `.git` change events, all under the same
    // project root — the scenario risk R3 names (a rebase or branch switch
    // firing many watched-file events at once).
    let git_dir = tmp.path().join(".git");
    let changes: Vec<lsp_types::FileEvent> = (0..10)
        .map(|i| {
            let path = if i % 2 == 0 {
                git_dir.join("HEAD")
            } else {
                git_dir.join("refs/heads/main")
            };
            lsp_types::FileEvent {
                uri: lsp_types::Url::from_file_path(path).expect("absolute path"),
                typ: lsp_types::FileChangeType::CHANGED,
            }
        })
        .collect();
    backend
        .did_change_watched_files(lsp_types::DidChangeWatchedFilesParams { changes })
        .await;

    let after = wait_for_settled(&counter, 500, 30_000).await;

    assert_eq!(
        after - before,
        1,
        "ten .git change events under one project root in a single \
         notification must trigger exactly one derivation, not one per \
         event (before={before}, after={after})"
    );
}
