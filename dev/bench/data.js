window.BENCHMARK_DATA = {
  "lastUpdate": 1788434921643,
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
          "id": "0e69c303f5926502169f73d07b04a80d8a569a64",
          "message": "fix(annes-words): corrected word list, always-visible practice button, mobile layout\n\nBrings the branch's remaining fixes onto main. The word list previously\nmerged via #183/#184 contained words unfit to be daily answers.\n\n- word list regenerated: answers are now restricted to entries that are\n  already lowercase in the SCOWL dictionaries and also present in dwyl's\n  list, with three profanity lists applied to the answer pool. Removes\n  nonce, queer, prick, raped, squaw, bimbo, labia, sperm, jihad and the\n  non-words mccoy, ascii, mckay, cobol. Guess list stays permissive.\n- practice button is always visible, with a \"Back to today's word\"\n  button to return to a saved daily game\n- mobile layout: safe-area insets (viewport-fit=cover was set with none),\n  a 100vh fallback before 100dvh, a max-height compaction block for\n  landscape phones, and main scrolls rather than clipping its own footer\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-02T20:22:22+10:00",
          "tree_id": "1774c4d38da2351a9b82f95b73a22aaa8557b3f4",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0e69c303f5926502169f73d07b04a80d8a569a64"
        },
        "date": 1788344731720,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.151604000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.023519,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.8660500000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.634124,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.32841800000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1092.410904,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.383999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.112536,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.156379,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.663589,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 896.567329,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.59642,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.332840000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.721246,
            "unit": "ms"
          }
        ]
      },
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
          "id": "223d4aafbc346af84af46e3a73aaee24e7382571",
          "message": "Add mise for tool versions, DuckDB lib setup, and dev tasks\n\nPins Rust/Node via mise.toml, sets DUCKDB_LIB_DIR/LD_LIBRARY_PATH\ndynamically, and wraps verify-phase.sh/clippy-gate.sh as mise tasks so\na fresh checkout no longer needs the manual export block in CLAUDE.md.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-03T20:50:15+10:00",
          "tree_id": "014d71c2b080ca2cbefbe2d941171aba7e1d090a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/223d4aafbc346af84af46e3a73aaee24e7382571"
        },
        "date": 1788432738179,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 55.995962000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 53.52912499999999,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.249913,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.609754,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29258300000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1105.874821,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.962205,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.506537,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.275263,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.781108,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 915.763423,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.537999999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.4442,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.365486,
            "unit": "ms"
          }
        ]
      },
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
          "id": "0de6608eec1fb0d859121a855346e90b3ba44f7f",
          "message": "CI: use mise for Rust/Node toolchain setup instead of separate actions\n\nReplaces dtolnay/rust-toolchain and actions/setup-node with jdx/mise-action\neverywhere, including the nightly fuzz jobs (via MISE_RUST_VERSION=nightly),\nso CI now installs the exact versions pinned in mise.toml — the same ones\na local `mise install` gives a developer. DuckDB setup runs before mise in\nevery job so mise's dynamic DUCKDB_LIB_DIR detection resolves correctly.\nrustup target/component flags move to explicit `rustup target/component add`\nsteps since mise-action doesn't take rust-toolchain's `with:` inputs.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-03T21:23:56+10:00",
          "tree_id": "e3db7539c211f46d445a76f0eb202cd78f940c44",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0de6608eec1fb0d859121a855346e90b3ba44f7f"
        },
        "date": 1788434919881,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 64.829942,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 61.74846,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.555554,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.769992,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.397505,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1158.006124,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 6.596638,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 5.212505,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 5.896445999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.469302,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 984.301968,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 9.23199,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 35.687850000000005,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.265003,
            "unit": "ms"
          }
        ]
      }
    ],
    "Smelt Throughput Benchmarks": [
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
          "id": "0e69c303f5926502169f73d07b04a80d8a569a64",
          "message": "fix(annes-words): corrected word list, always-visible practice button, mobile layout\n\nBrings the branch's remaining fixes onto main. The word list previously\nmerged via #183/#184 contained words unfit to be daily answers.\n\n- word list regenerated: answers are now restricted to entries that are\n  already lowercase in the SCOWL dictionaries and also present in dwyl's\n  list, with three profanity lists applied to the answer pool. Removes\n  nonce, queer, prick, raped, squaw, bimbo, labia, sperm, jihad and the\n  non-words mccoy, ascii, mckay, cobol. Guess list stays permissive.\n- practice button is always visible, with a \"Back to today's word\"\n  button to return to a saved daily game\n- mobile layout: safe-area insets (viewport-fit=cover was set with none),\n  a 100vh fallback before 100dvh, a max-height compaction block for\n  landscape phones, and main scrolls rather than clipping its own footer\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-02T20:22:22+10:00",
          "tree_id": "1774c4d38da2351a9b82f95b73a22aaa8557b3f4",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0e69c303f5926502169f73d07b04a80d8a569a64"
        },
        "date": 1788344736394,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.123520123464008,
            "unit": "MB/s"
          }
        ]
      },
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
          "id": "223d4aafbc346af84af46e3a73aaee24e7382571",
          "message": "Add mise for tool versions, DuckDB lib setup, and dev tasks\n\nPins Rust/Node via mise.toml, sets DUCKDB_LIB_DIR/LD_LIBRARY_PATH\ndynamically, and wraps verify-phase.sh/clippy-gate.sh as mise tasks so\na fresh checkout no longer needs the manual export block in CLAUDE.md.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-03T20:50:15+10:00",
          "tree_id": "014d71c2b080ca2cbefbe2d941171aba7e1d090a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/223d4aafbc346af84af46e3a73aaee24e7382571"
        },
        "date": 1788432742341,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.79225327085001,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}