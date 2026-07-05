//! Tripwire (property-discovery FIX-2): `input_delta_discovery` classifies a
//! clocked `Mutable` source as `InputDeltaKind::WindowForward` — read only
//! new partitions forward, never revisit an already-processed one. A source
//! declared `Mutable` can be updated IN PLACE in an already-processed
//! partition; `WindowForward` alone has no branch that would catch that
//! (`docs/research/property-discovery/ledger.md` SC-2 confirmed the
//! divergence this shape produces for a hand-simulated forward-only
//! consumer).
//!
//! `input_delta_discovery` itself has **zero production call sites** today —
//! it is proof-stage-only, referenced by its own unit tests. Wiring its
//! `WindowForward` verdict to a real maintenance mode for a `Mutable` source
//! is *behaviour-defining design* (new refresh semantics for mutable
//! sources), out of scope for this loop's mechanical-fix authority
//! (design §8(4)). This test is the guard: it fails the moment a production
//! path starts calling `input_delta_discovery`, so that wiring cannot land
//! silently — the PR that makes this test fail must consult
//! `docs/research/property-discovery/ledger.md` (cell SC-2) and either add an
//! explicit `MutationProfile::Mutable` guard or get human sign-off on the new
//! semantics before updating this test's expected call-site set.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every file (relative to repo root) allowed to reference
/// `input_delta_discovery` — its own definition + unit tests. Any other match
/// is a new production (or non-tripwire test) call site.
const ALLOWED_CALLERS: &[&str] = &["crates/smelt-logical/src/analysis/input_delta.rs"];

#[test]
fn input_delta_discovery_has_no_production_call_sites() {
    let root = repo_root();

    let output = Command::new("rg")
        .args([
            "--line-number",
            "--with-filename",
            "-g",
            "*.rs",
            "-g",
            "!crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs",
            "input_delta_discovery",
            "crates",
        ])
        .current_dir(&root)
        .output()
        .expect("failed to run rg — is ripgrep installed?");

    // rg exits 1 when there are no matches; that would itself be surprising
    // (the definition + its own unit tests should always match) but is not
    // this test's concern.
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut unexpected: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some((file, _rest)) = line.split_once(':') else {
            continue;
        };
        let normalized = Path::new(file).to_string_lossy().replace('\\', "/");
        if !ALLOWED_CALLERS
            .iter()
            .any(|allowed| normalized == *allowed || normalized.ends_with(allowed))
        {
            unexpected.push(line.to_string());
        }
    }

    assert!(
        unexpected.is_empty(),
        "input_delta_discovery gained a new caller outside {ALLOWED_CALLERS:?}:\n{}\n\n\
         This function classifies a clocked Mutable source as WindowForward with NO branch \
         for in-place updates of already-processed partitions (property-discovery cell SC-2 — \
         see docs/research/property-discovery/ledger.md). Wiring its verdict into a real \
         maintenance path is a behaviour-defining design decision (new refresh semantics for \
         mutable sources), not a mechanical fix. Before updating ALLOWED_CALLERS: confirm the \
         new caller has an explicit MutationProfile::Mutable guard (never trusts WindowForward \
         alone for a mutable source), and get human sign-off per design §8(4).",
        unexpected.join("\n")
    );
}
