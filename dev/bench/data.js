window.BENCHMARK_DATA = {
  "lastUpdate": 1774017614730,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "42de40ee2cd2830f2fcd3431228e087088d6adb2",
          "message": "Merge pull request #44 from adbrowne/feature/python-optimizer-rules\n\nAdd Python optimizer rules via PyO3 bridge",
          "timestamp": "2026-03-15T11:32:42+11:00",
          "tree_id": "6f4591377f87657a3ccbcd3f4d270f472a55319f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/42de40ee2cd2830f2fcd3431228e087088d6adb2"
        },
        "date": 1773535064765,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.360014,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.124411,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.607668,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.317278,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.00549,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.959877,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020909,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.014497,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011421,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.308026,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.395389,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.82876,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.23008,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.804501,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "60294b5f827679b6605afd7060ac4191e3376b89",
          "message": "Merge pull request #45 from adbrowne/chore/update-actions-node24\n\nBump GitHub Actions to Node.js 24-compatible versions",
          "timestamp": "2026-03-15T12:09:57+11:00",
          "tree_id": "be187364301d6fe053d6920e522a4e45b934aa88",
          "url": "https://github.com/adbrowne/smelt-sql/commit/60294b5f827679b6605afd7060ac4191e3376b89"
        },
        "date": 1773537072356,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.958735,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.776631,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.5685,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.306502,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003236,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.795610999999994,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.023854,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.023854,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008326,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.959358,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.8069219999999997,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.195790000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.351549999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.825132,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6cf6c4c6031b4ae3272b4f5a6b5e7eebcdf27c71",
          "message": "Merge pull request #48 from adbrowne/feature/dialect-rewrites-phase5\n\nAdd trailing comma removal and EXPLODE/UNNEST renaming",
          "timestamp": "2026-03-15T14:36:53+11:00",
          "tree_id": "6e40826e440c949b8f7fc3aceb933c9b597c9f49",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6cf6c4c6031b4ae3272b4f5a6b5e7eebcdf27c71"
        },
        "date": 1773545926152,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.862546,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.675699,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.577488,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30745399999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003156,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.646692,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018544,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012723,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.010359,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.09287,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.086804,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.35142,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.25546,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.701979,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0e4f8a77918362232d4a72ec465542b5e42bc213",
          "message": "Merge pull request #47 from adbrowne/feature/incremental-model-improvements\n\nUnify IncrementalConfig, add granularity and SQL safety checks",
          "timestamp": "2026-03-15T14:53:56+11:00",
          "tree_id": "3d973a78e7c8be757583c4d34223bb08f5bd9d52",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0e4f8a77918362232d4a72ec465542b5e42bc213"
        },
        "date": 1773546932908,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.558870999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.374367,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.553474,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30201,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0026249999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.801623000000006,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.01583,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010169,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008266,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.863438,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.974088,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.33966,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.63931,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.590093,
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
          "id": "d0e181b8804f16f03345c6485c8faf76c2e0af81",
          "message": "Add Release & Distribution roadmap section (R1-R7)\n\nPhased plan for maturin-based Python distribution, cross-platform\nCI builds, PyPI/VSCode Marketplace publishing, docs site, and\noptional crates.io publishing.\n\nCo-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>",
          "timestamp": "2026-03-15T15:21:23+11:00",
          "tree_id": "c62c576a8d18931e61f3fb1911dba1dcd4419003",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d0e181b8804f16f03345c6485c8faf76c2e0af81"
        },
        "date": 1773548578415,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.062955,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.818942,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.626995,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30561299999999997,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0031650000000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 37.580165,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020548,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01085,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008376000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.002239,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.030173,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.855300000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.97806,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.808694999999998,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "1e34bbf639238d9b01079da98a46d1c3718c9a71",
          "message": "Merge pull request #49 from adbrowne/worktree-release\n\nAdd release infrastructure (R1-R3)",
          "timestamp": "2026-03-15T15:43:45+11:00",
          "tree_id": "c970d367402acc5bd9a37afd3e6ddac0d73790fe",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1e34bbf639238d9b01079da98a46d1c3718c9a71"
        },
        "date": 1773549908375,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.05132,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.799084,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6061909999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30266499999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002615,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.712354000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.029064000000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.014567,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009689,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.323099,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.344611,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.81137,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.46922,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.187282000000002,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "d26e58c05ef246a46eaf090205544aae28d9ef71",
          "message": "Merge pull request #50 from adbrowne/worktree-release\n\nAdd end-user distribution (R4-R7)",
          "timestamp": "2026-03-15T16:13:17+11:00",
          "tree_id": "e5d4256211445a2a27d9ef7545c2c05cfa0d1ba1",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d26e58c05ef246a46eaf090205544aae28d9ef71"
        },
        "date": 1773551705633,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.923184000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.732315,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.573914,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.308382,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002845,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.255745000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017053000000000002,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.009899,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.007944999999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.899168,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.948439,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.53726,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.816640000000003,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.635157,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "b48cb8499bc6552925c7af31c1be3df483f5a466",
          "message": "Add continuous dev releases from main (R8) (#51)\n\nEvery merge to main now produces an installable dev release via\n`pip install smelt-sql --pre`. Adds dev-release.yml workflow with\nCI-patched version (0.1.0-dev.YYYYMMDDHHMM), GitHub pre-release,\nand PyPI publishing. Switches pyproject.toml to dynamic versioning\nso Cargo.toml is the single source of truth.\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-15T16:31:47+11:00",
          "tree_id": "9f599c7d2553502096a34d102c3984a2e886c36a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b48cb8499bc6552925c7af31c1be3df483f5a466"
        },
        "date": 1773552774741,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.138794000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.932618,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.563274,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.305461,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002885,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.326405,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.022952,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013525,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011422,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.2632819999999998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.322905,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.82493,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.86581,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.73749,
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
          "id": "ac7ff281774cf15ce7529647c40a3070ca624e1c",
          "message": "Actually check sqlglot result",
          "timestamp": "2026-03-21T00:08:32+11:00",
          "tree_id": "468c0cb2e35dc66a27b1220085b8572e0fb42ac3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ac7ff281774cf15ce7529647c40a3070ca624e1c"
        },
        "date": 1774012219873,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.457282,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.224569,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.584442,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.321841,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002575,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.725364,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.016009,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.02164,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008977,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.859577,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.077512,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 3.74972,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.65385,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.401701,
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
          "id": "c5fc1e209ad6576c6d3e6f151dc57e5713704218",
          "message": "Remove qualify spark tests - spark doesn't support qualify",
          "timestamp": "2026-03-21T00:16:53+11:00",
          "tree_id": "4c38e3ca22f4de1fea87b049790c12833503461a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c5fc1e209ad6576c6d3e6f151dc57e5713704218"
        },
        "date": 1774012709613,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.658133,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.467519,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.575606,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30208399999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002514,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.632807,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017071999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010379,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008185999999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.8916860000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.85275,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.8707,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.61308,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.791941,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "ec8081753c9fdabbf5e2a72c6ceb8419272da609",
          "message": "Add smelt type CLI command (#52)\n\n* Add `smelt type` CLI command to show model function signatures\n\nView each model as a function: what columns it requires from input refs\n(with type constraints) and what columns it produces (with inferred types).\nTypes are derived purely from the SQL structure.\n\n- Add GroupByClause AST wrapper to smelt-parser\n- Enhance model_input_constraints to cover GROUP BY, HAVING, ORDER BY\n- Add context-based type hints in collect_column_refs (SUM/AVG → numeric)\n- Add ModelFunctionType, FunctionInput, FunctionOutput, TypedField types\n- Add model_function_type() Salsa query combining inputs and outputs\n- Add `smelt type [model] --project-dir` CLI command\n- Add 11 new tests for function type inference\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Support smelt.source() inputs in model function signatures\n\nModels using smelt.source() now show their source table columns as\ninputs in `smelt type` output, matching the existing smelt.ref() behavior.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Extract init_db() to deduplicate Salsa database setup in CLI commands\n\nThe table, ui, and show_type commands all repeated the same pattern:\nload sources.yml, create database, register models. Extract this into\na shared init_db() function. Also fixes show_type missing Python model\ndiscovery.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix trailing whitespace in parser-compat lib.rs\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T00:42:48+11:00",
          "tree_id": "ac236257c38bd03d839476663a915eafb698d4ed",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ec8081753c9fdabbf5e2a72c6ceb8419272da609"
        },
        "date": 1774014271788,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.75801,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.526483000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.606151,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.31244299999999997,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0026850000000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.770573999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017302,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010529,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008166,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.897144,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.4091400000000003,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.73163,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.72433,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.653052,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6f7d609c7a2939a8894d235f974325a4327a6063",
          "message": "Centralize SQL function names into SqlFunction enum in smelt-types (#54)\n\nReplace scattered string literals for SQL function names across smelt-db,\nsmelt-optimizer, and smelt-parser-compat with a single SqlFunction enum.\nThis eliminates duplication and makes it easy to find all references to a\nfunction, with exhaustive matching ensuring new variants are handled everywhere.\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T01:38:14+11:00",
          "tree_id": "aecace191512464f31268a9c9b03cef01377fdce",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6f7d609c7a2939a8894d235f974325a4327a6063"
        },
        "date": 1774017614187,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.586629,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.319278000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6505620000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30571,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0033859999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.866823,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.03185,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015368,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011141,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.397273,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.385035,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.35969,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.06828,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.924082,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "42de40ee2cd2830f2fcd3431228e087088d6adb2",
          "message": "Merge pull request #44 from adbrowne/feature/python-optimizer-rules\n\nAdd Python optimizer rules via PyO3 bridge",
          "timestamp": "2026-03-15T11:32:42+11:00",
          "tree_id": "6f4591377f87657a3ccbcd3f4d270f472a55319f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/42de40ee2cd2830f2fcd3431228e087088d6adb2"
        },
        "date": 1773535065730,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.021684609963607,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "60294b5f827679b6605afd7060ac4191e3376b89",
          "message": "Merge pull request #45 from adbrowne/chore/update-actions-node24\n\nBump GitHub Actions to Node.js 24-compatible versions",
          "timestamp": "2026-03-15T12:09:57+11:00",
          "tree_id": "be187364301d6fe053d6920e522a4e45b934aa88",
          "url": "https://github.com/adbrowne/smelt-sql/commit/60294b5f827679b6605afd7060ac4191e3376b89"
        },
        "date": 1773537074676,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.979774602093233,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6cf6c4c6031b4ae3272b4f5a6b5e7eebcdf27c71",
          "message": "Merge pull request #48 from adbrowne/feature/dialect-rewrites-phase5\n\nAdd trailing comma removal and EXPLODE/UNNEST renaming",
          "timestamp": "2026-03-15T14:36:53+11:00",
          "tree_id": "6e40826e440c949b8f7fc3aceb933c9b597c9f49",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6cf6c4c6031b4ae3272b4f5a6b5e7eebcdf27c71"
        },
        "date": 1773545927253,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.23214056357476,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0e4f8a77918362232d4a72ec465542b5e42bc213",
          "message": "Merge pull request #47 from adbrowne/feature/incremental-model-improvements\n\nUnify IncrementalConfig, add granularity and SQL safety checks",
          "timestamp": "2026-03-15T14:53:56+11:00",
          "tree_id": "3d973a78e7c8be757583c4d34223bb08f5bd9d52",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0e4f8a77918362232d4a72ec465542b5e42bc213"
        },
        "date": 1773546934463,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.4660677010961,
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
          "id": "d0e181b8804f16f03345c6485c8faf76c2e0af81",
          "message": "Add Release & Distribution roadmap section (R1-R7)\n\nPhased plan for maturin-based Python distribution, cross-platform\nCI builds, PyPI/VSCode Marketplace publishing, docs site, and\noptional crates.io publishing.\n\nCo-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>",
          "timestamp": "2026-03-15T15:21:23+11:00",
          "tree_id": "c62c576a8d18931e61f3fb1911dba1dcd4419003",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d0e181b8804f16f03345c6485c8faf76c2e0af81"
        },
        "date": 1773548579388,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.013153019872227,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "1e34bbf639238d9b01079da98a46d1c3718c9a71",
          "message": "Merge pull request #49 from adbrowne/worktree-release\n\nAdd release infrastructure (R1-R3)",
          "timestamp": "2026-03-15T15:43:45+11:00",
          "tree_id": "c970d367402acc5bd9a37afd3e6ddac0d73790fe",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1e34bbf639238d9b01079da98a46d1c3718c9a71"
        },
        "date": 1773549910184,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.267205928278347,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "d26e58c05ef246a46eaf090205544aae28d9ef71",
          "message": "Merge pull request #50 from adbrowne/worktree-release\n\nAdd end-user distribution (R4-R7)",
          "timestamp": "2026-03-15T16:13:17+11:00",
          "tree_id": "e5d4256211445a2a27d9ef7545c2c05cfa0d1ba1",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d26e58c05ef246a46eaf090205544aae28d9ef71"
        },
        "date": 1773551707366,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.371308440444764,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "b48cb8499bc6552925c7af31c1be3df483f5a466",
          "message": "Add continuous dev releases from main (R8) (#51)\n\nEvery merge to main now produces an installable dev release via\n`pip install smelt-sql --pre`. Adds dev-release.yml workflow with\nCI-patched version (0.1.0-dev.YYYYMMDDHHMM), GitHub pre-release,\nand PyPI publishing. Switches pyproject.toml to dynamic versioning\nso Cargo.toml is the single source of truth.\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-15T16:31:47+11:00",
          "tree_id": "9f599c7d2553502096a34d102c3984a2e886c36a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b48cb8499bc6552925c7af31c1be3df483f5a466"
        },
        "date": 1773552775941,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.158827824347455,
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
          "id": "ac7ff281774cf15ce7529647c40a3070ca624e1c",
          "message": "Actually check sqlglot result",
          "timestamp": "2026-03-21T00:08:32+11:00",
          "tree_id": "468c0cb2e35dc66a27b1220085b8572e0fb42ac3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ac7ff281774cf15ce7529647c40a3070ca624e1c"
        },
        "date": 1774012222197,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.870324173559723,
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
          "id": "c5fc1e209ad6576c6d3e6f151dc57e5713704218",
          "message": "Remove qualify spark tests - spark doesn't support qualify",
          "timestamp": "2026-03-21T00:16:53+11:00",
          "tree_id": "4c38e3ca22f4de1fea87b049790c12833503461a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c5fc1e209ad6576c6d3e6f151dc57e5713704218"
        },
        "date": 1774012710806,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.047270928509562,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "ec8081753c9fdabbf5e2a72c6ceb8419272da609",
          "message": "Add smelt type CLI command (#52)\n\n* Add `smelt type` CLI command to show model function signatures\n\nView each model as a function: what columns it requires from input refs\n(with type constraints) and what columns it produces (with inferred types).\nTypes are derived purely from the SQL structure.\n\n- Add GroupByClause AST wrapper to smelt-parser\n- Enhance model_input_constraints to cover GROUP BY, HAVING, ORDER BY\n- Add context-based type hints in collect_column_refs (SUM/AVG → numeric)\n- Add ModelFunctionType, FunctionInput, FunctionOutput, TypedField types\n- Add model_function_type() Salsa query combining inputs and outputs\n- Add `smelt type [model] --project-dir` CLI command\n- Add 11 new tests for function type inference\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Support smelt.source() inputs in model function signatures\n\nModels using smelt.source() now show their source table columns as\ninputs in `smelt type` output, matching the existing smelt.ref() behavior.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Extract init_db() to deduplicate Salsa database setup in CLI commands\n\nThe table, ui, and show_type commands all repeated the same pattern:\nload sources.yml, create database, register models. Extract this into\na shared init_db() function. Also fixes show_type missing Python model\ndiscovery.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix trailing whitespace in parser-compat lib.rs\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T00:42:48+11:00",
          "tree_id": "ac236257c38bd03d839476663a915eafb698d4ed",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ec8081753c9fdabbf5e2a72c6ceb8419272da609"
        },
        "date": 1774014272834,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.333882660096254,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}