//! `smelt ui` network-hardening arg-parse tests (`docs/specs/cli.md`
//! §"`smelt ui`"): binding to a non-loopback host requires the explicit
//! `--allow-remote` opt-in, fail-loud rather than silently rebinding.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn test_workspace() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/test_workspace")
        .canonicalize()
        .expect("examples/test_workspace exists")
}

#[test]
fn non_loopback_host_without_allow_remote_fails() {
    let project_dir = test_workspace();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("ui")
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg("38173")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt ui --host 0.0.0.0");

    assert!(
        !output.status.success(),
        "smelt ui --host 0.0.0.0 without --allow-remote should fail, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--allow-remote"),
        "error should name the --allow-remote flag, got: {stderr}"
    );
}

#[test]
fn non_loopback_host_with_allow_remote_parses_and_proceeds() {
    let project_dir = test_workspace();

    let mut child = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("ui")
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg("38174")
        .arg("--allow-remote")
        .arg("--project-dir")
        .arg(&project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn smelt ui --host 0.0.0.0 --allow-remote");

    // The command starts a long-running server; if the arg parsed and the
    // bind-host check passed, the process is still running after a short
    // delay rather than having exited with a usage/validation error.
    std::thread::sleep(Duration::from_millis(700));

    match child.try_wait().expect("try_wait") {
        None => {
            // Still running — arg parsed, validation passed. Clean up.
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(status) => {
            let mut stdout = String::new();
            let mut stderr = String::new();
            use std::io::Read;
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut stdout);
            }
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr);
            }
            panic!(
                "smelt ui --host 0.0.0.0 --allow-remote exited early with {status}, \
                 stdout: {stdout}, stderr: {stderr}"
            );
        }
    }
}
