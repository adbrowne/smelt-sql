window.BENCHMARK_DATA = {
  "lastUpdate": 1788520584762,
  "repoUrl": "https://github.com/adbrowne/smelt-sql",
  "entries": {
    "Smelt Latency Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "b7740de4c4e35ef4e3a110fc19ac68ce279d3e9a",
          "message": "outcome-loop: replace bash stream formatter with a Rust stream-view tool\n\nThe bash+jq formatter plus a cooperating /dev/tty-listening background\nsubshell (for a ctrl+o show/hide toggle on tool-call detail) was fragile:\norphaned listener processes accumulated across iterations with no clean\nlifecycle. Replace both with a single standalone binary,\n.claude/tools/stream-view (deliberately outside the main Cargo workspace,\nso none of its clippy/hardening-budget/test gates apply — this is a dev-\nloop display tool, not shipped product code).\n\nstream-view owns termios raw-mode setup on /dev/tty, a background thread\nreading the ctrl+o toggle, and restoring terminal settings from main() on\nexit (a thread's own teardown can't be relied on here — it stays blocked in\na 1-byte read for the process's whole life and only sees EOF/error, never\nthe happy path). It skips taking over the tty entirely when not in its\nforeground process group, since tcsetattr/read would otherwise raise\nSIGTTOU/SIGTTIN and stop the process. Stream lines are read as raw bytes\nwith lossy UTF-8 conversion rather than via BufRead::lines(), so one bad\nbyte can't blank the live display for the rest of an iteration.\n\noutcome-loop.sh rebuilds the binary (a no-op once current) at the start of\neach run and falls back to raw JSONL via cat if the build fails, so the\nloop is never blocked on it. Toggle state persists to a file under\nLOG_DIR keyed by repo checkout, kept outside any worktree so it can't trip\nthe loop's own git-dirty check.\n\n.gitignore's `.claude/*` blanket-ignore plus whitelist previously had no\nentry for .claude/tools/, which would have silently discarded the whole\ntool on commit; added `!.claude/tools/**` with target/ re-excluded.\n\nReviewed by an Opus subagent, which caught: the crate being gitignored,\nthe termios restore reading already-raw settings (a no-op), the restore\npath never running on the happy path, the missing foreground-pgroup guard,\nand the BufRead::lines() UTF-8 fragility — all fixed here.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T21:12:16+10:00",
          "tree_id": "b941004fd1e750bced139890c3baf972adcc0b1e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b7740de4c4e35ef4e3a110fc19ac68ce279d3e9a"
        },
        "date": 1788520582869,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.264102,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.019615,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.999316,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.627533,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29921499999999995,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1157.485336,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.648644,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.174873,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.166931,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.668814,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 964.279351,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.85859,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.323229999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.801919,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}