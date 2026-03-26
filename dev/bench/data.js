window.BENCHMARK_DATA = {
  "lastUpdate": 1774526142468,
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
          "id": "cb22c6ac9973fdbbf144dd232aa36f812a06aebc",
          "message": "Fix maturin workspace build: add manifest-path and bundle smelt-lsp\n\nMaturin failed with \"missing field `package`\" because the root Cargo.toml\nis a workspace without a [package] section. Fix by:\n- Adding manifest-path to point maturin at crates/smelt-cli/Cargo.toml\n- Pre-building smelt-lsp and placing it in smelt_sql.data/scripts/ so\n  both binaries are included in the wheel\n- Using before-script-linux for manylinux container builds\n- Using a separate step for macOS/Windows builds\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T01:41:58+11:00",
          "tree_id": "e549a513b0aec6f26e82711edabd0b4831a48846",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb22c6ac9973fdbbf144dd232aa36f812a06aebc"
        },
        "date": 1774017794720,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.723429,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.560217,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.554831,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.304185,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003356,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.016462999999995,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020608,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012574,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008415,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.0493499999999998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.9294529999999996,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.40475,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.74094,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.73233,
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
          "id": "1a90ac78eecc630a6fc5b6d4c110a49f384e000d",
          "message": "Add docs link to README (#55)\n\n* Add link to GitHub Pages documentation in README\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Configure smeltsql.com custom domain for GitHub Pages\n\n- Add CNAME file for GitHub Pages custom domain\n- Update site_url in mkdocs.yml to smeltsql.com\n- Update documentation link in README.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T02:02:16+11:00",
          "tree_id": "5335151d9b32ac1b34ff1b7513ae71c96ef3bb96",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1a90ac78eecc630a6fc5b6d4c110a49f384e000d"
        },
        "date": 1774019035546,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.630849999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.409387,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.601484,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30734399999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003166,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.2266,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.029565,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015098,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008585,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.299529,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.249954,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.050039999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.62643,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.61026,
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
          "id": "3cd5a2a34f3ae57b96d322b32f5b959a67d6726e",
          "message": "Drop macos-x86_64 builds: macos-13 runner deprecated\n\nGitHub Actions no longer supports the macos-13 runner. Remove\nmacos-x86_64 from build matrices in release and dev-release workflows.\nmacOS ARM64 (Apple Silicon) is the only supported macOS target; x86_64\nusers can run via Rosetta 2.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T02:03:36+11:00",
          "tree_id": "48804c926af8d0dd2a6dcba9106e4de54a7d6e92",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3cd5a2a34f3ae57b96d322b32f5b959a67d6726e"
        },
        "date": 1774019116108,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.754832,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.540278,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.59563,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.306917,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002745,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.259871,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.02679,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.014427,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.022062,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.274325,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.161133,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.846369999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 46.87732,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.61842,
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
          "id": "604e86e495a5e13c72d14df116910b13c9019148",
          "message": "Update duckdb to 1.4.4 and arrow/parquet to 57 (#56)\n\nMatches the system-installed DuckDB version (1.4.4) and updates\narrow/parquet to stay in sync with duckdb's arrow dependency.\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T11:47:15+11:00",
          "tree_id": "cfd6f53f926ff32e710941c43cf899490ff2a3a3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/604e86e495a5e13c72d14df116910b13c9019148"
        },
        "date": 1774054160706,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.890899000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.667904,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6031120000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.31430400000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.004849,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.587225,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.028753,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015679,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008365000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.144028,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.011743,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.60303,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.645730000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.773252,
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
          "id": "eb17b3761f37fe19dcd3b9c14e44884956e57731",
          "message": "Add property-based type inference tests against DuckDB (#57)\n\nVerify smelt's type inference matches real database behavior using proptest\nto generate random typed CTE queries and compare inferred types against\nDuckDB's actual Arrow schema output.\n\n- 256-case proptest + 5 deterministic smoke tests + unit tests (21 total)\n- TypeOracle trait for future PostgreSQL/Spark backends\n- Known divergence registry for expected mismatches (SUM, CEIL, EXTRACT, etc.)\n- Compatible type handling (Text/Varchar, Decimal precision, integer widths)\n- Arrow-to-smelt type mapping module\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T11:52:39+11:00",
          "tree_id": "1a197a65de48c0d46493e68dd46d64fcb93c57a3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/eb17b3761f37fe19dcd3b9c14e44884956e57731"
        },
        "date": 1774054473291,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.564564,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.380906,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.581915,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.304739,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0026249999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.643536000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017853,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01036,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.007924,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.876575,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.8008439999999997,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.5761,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.58745,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.705661,
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
          "id": "b14ac0591883cdace08076b46efdc397794bda0f",
          "message": "Fix dev release: patch inter-crate dependency versions (#58)\n\nThe dev version patching only updated the workspace root Cargo.toml,\nbut smelt-dialect has a hardcoded `version = \"0.1.0\"` dependency on\nsmelt-parser. Semver pre-release versions (0.1.0-dev.xxx) don't match\n^0.1.0, causing all builds to fail with \"failed to select a version\".\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T13:15:41+11:00",
          "tree_id": "be5f50e8b512360bd92c2241e35a750250dadb95",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b14ac0591883cdace08076b46efdc397794bda0f"
        },
        "date": 1774059447280,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.148177000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.912971999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.619451,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30414,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002705,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.390051,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.025358,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013365,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.01029,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.225256,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.144324,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.70258,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.69251,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.613997,
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
          "id": "5f3fe31be16e25eaa3f7d5c3044225b0d9b4a62a",
          "message": "Add Spark SQL oracle for type property tests (#59)",
          "timestamp": "2026-03-21T16:39:17+11:00",
          "tree_id": "38a3435dd312337bca4c5fc4b8bcae7f8691a4e0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5f3fe31be16e25eaa3f7d5c3044225b0d9b4a62a"
        },
        "date": 1774071628683,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.223936,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.88242,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6789930000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.32538100000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.51414,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.025387,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013946,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011281,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.2741,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.5607680000000004,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.907299999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.63443,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.682269,
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
          "id": "0343c12f4ade278b6bfff90e8a8de76dc9e036f8",
          "message": "Add end-of-conversation checklist to CLAUDE.md\n\nInstructs Claude to write unfinished work and open decisions to\ndocs/TODO.md and update CLAUDE.md before ending a conversation.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T16:45:09+11:00",
          "tree_id": "43e10f771a9fc9cce96222b8ea03d24791fc94a0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0343c12f4ade278b6bfff90e8a8de76dc9e036f8"
        },
        "date": 1774072028656,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.594822,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.418685999999994,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.540429,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.317803,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002635,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.586139,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.016711,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010499,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008425,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.869853,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.178914,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.414000000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.824339999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.600434,
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
          "id": "7dea6f891a5a7ff5e8177d8c4df65d86d6fd6741",
          "message": "Add type conformance casts to ensure backend output matches smelt inference (#60)",
          "timestamp": "2026-03-21T18:14:16+11:00",
          "tree_id": "7ac78aad2e63dea039db3aad37705267204b5f09",
          "url": "https://github.com/adbrowne/smelt-sql/commit/7dea6f891a5a7ff5e8177d8c4df65d86d6fd6741"
        },
        "date": 1774077374199,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.557254,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.376811,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.5746020000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.302464,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0027949999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.057597,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018655,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012333,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008215,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.956636,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.187779,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.73253,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.667439999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.630797,
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
          "id": "bc1273a9d0cf645e28e74cb7afd809e6bfad4970",
          "message": "Add cross-model type propagation tests (#61)",
          "timestamp": "2026-03-21T18:34:59+11:00",
          "tree_id": "33ebe9537d79352933732c0e249a1b3583827789",
          "url": "https://github.com/adbrowne/smelt-sql/commit/bc1273a9d0cf645e28e74cb7afd809e6bfad4970"
        },
        "date": 1774078597692,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.38595299999999,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.216146,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.566424,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.29980799999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002665,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.410889,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.016501000000000002,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010259,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008195000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.860522,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.869271,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.361180000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.6695,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.691522,
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
          "id": "4ec93eae0dcf3dcfaf0637beee0b6f04b1c0e688",
          "message": "Fix Unknown type gaps and add type diagnostics (#62)",
          "timestamp": "2026-03-21T19:50:58+11:00",
          "tree_id": "e440bbd98377a9e06acdb3576c7c5348a727ed6e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4ec93eae0dcf3dcfaf0637beee0b6f04b1c0e688"
        },
        "date": 1774083127738,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.512868000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.334379999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.564694,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30475800000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002555,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.47114,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.016881,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010419,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008075,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.885592,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.0089829999999997,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.80426,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.73233,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.68162,
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
          "id": "f75035bf7385d5e154ee05ca8a73860f175af28e",
          "message": "Fix KnownBug type inference divergences (#63)",
          "timestamp": "2026-03-21T19:50:34+11:00",
          "tree_id": "fdd94d0f707f1eb52625708d70d137f319fe4e7c",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f75035bf7385d5e154ee05ca8a73860f175af28e"
        },
        "date": 1774083133540,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.881626,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.61474,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.632065,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.311388,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.005159,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.149453,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.027681,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015519000000000002,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.012293,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.386117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.285177,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.1122,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.077,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.78528,
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
          "id": "fde96655274afde115b803b0e882541c1db00b5d",
          "message": "Add plans documentation convention to CLAUDE.md (#64)",
          "timestamp": "2026-03-21T20:02:40+11:00",
          "tree_id": "fe4db904d9ed97beed499ceae8c20b7aff0aa772",
          "url": "https://github.com/adbrowne/smelt-sql/commit/fde96655274afde115b803b0e882541c1db00b5d"
        },
        "date": 1774083863927,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.635728,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.358098,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.621433,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.323174,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003536,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.418243,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.029365,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015909,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011972,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.397205,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.472738,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.431290000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.59059,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.952715,
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
          "id": "f26726e256331c2064b29801a01cf6f6af4e3e49",
          "message": "Add property test generator coverage gaps to TODO\n\nAudit of generators vs parser/type inference support revealed extensive\ngaps: ~80% of string/math functions, all window functions, temporal\nfunctions, advanced aggregates, and several expression types untested.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T20:13:56+11:00",
          "tree_id": "5d7b2bbbe1acc9eb6fdcc87cf5012ac8308d3fc8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f26726e256331c2064b29801a01cf6f6af4e3e49"
        },
        "date": 1774084499649,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.120287,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 32.942402,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.548817,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30937,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002565,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.941421,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017433,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01058,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008055,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.878254,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.90092,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.12119,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.624809999999997,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.667269,
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
          "id": "535a42cdea105276d505f5c642039cf9832b6736",
          "message": "Expand property test generator coverage (#65)\n\n* Add trig and log math functions to property test generators\n\nAdd 16 single-arg math functions: POWER, POW, EXP, LN, LOG, LOG10,\nLOG2, SIN, COS, TAN, ASIN, ACOS, ATAN, SINH, COSH, TANH.\nAll take numeric input and return Double.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add SIGN function to generators and fix SIGN type inference\n\nSIGN always returns a small integer (-1, 0, 1) in DuckDB (TINYINT).\nFix smelt's type inference to return SmallInt instead of arg type.\nRemove POWER/POW (needs 2 args) and ASIN/ACOS (domain-restricted)\nfrom single-arg generators; they'll be added in multi-arg step.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add single-arg string functions to property test generators\n\nAdd LTRIM, RTRIM, INITCAP, CONCAT (single-arg), CHAR_LENGTH, and\nCHARACTER_LENGTH to the generator function list.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add COALESCE and AnyScalar input variant to generators\n\nAdd FuncInput::AnyScalar for non-aggregate functions that accept any\ntype. Add COALESCE (single-arg, returns arg type). Remove INITCAP\n(not available in DuckDB).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add GREATEST and LEAST to property test generators\n\nBoth use AnyScalar input and return the argument type.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add statistical aggregates and refactor aggregate detection\n\nAdd STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP\nto generators. Refactor assemble_cte_query() to use SqlFunction::\nfrom_name + is_aggregate() instead of fragile starts_with() checks.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add boolean aggregate functions to property test generators\n\nAdd BOOL_AND, BOOL_OR, EVERY with new FuncInput::BooleanAggregate\nvariant that only accepts Boolean columns.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add bit and boolean aggregates to property test generators\n\nAdd BIT_AND, BIT_OR, BIT_XOR with new FuncInput::IntegerAggregate.\nAdd BOOL_AND, BOOL_OR with FuncInput::BooleanAggregate.\nRemove EVERY (not available in DuckDB).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add PI() zero-arg function to property test generators\n\nAdd FuncInput::NoArg variant and PI() function. Update generate_expr()\nto emit function calls without arguments for NoArg functions.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add multi-arg function infrastructure and functions\n\nAdd ExtraArg enum (SameAsFirst, IntLiteral, StringLiteral) and\nextra_args field to FuncDesc. Update generate_expr() to build\nmulti-arg function calls.\n\nNew functions: REPLACE, LPAD, RPAD, LEFT, RIGHT, REPEAT (string),\nNULLIF (any-type), POWER, MOD, ATAN2 (numeric).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add SUBSTRING, SUBSTR, SPLIT_PART, STRPOS to generators\n\nMulti-arg string functions using the ExtraArg infrastructure.\nSTRPOS covers the same inference path as POSITION (special syntax).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add DATE_PART, DATE_TRUNC and prepend_literal support\n\nAdd prepend_literal field to FuncDesc for functions where a string\nliteral must come before the column argument (e.g. DATE_PART('year', col)).\nAdd DATE_PART (returns BigInt) and DATE_TRUNC (returns Timestamp).\nRemove LEFT/RIGHT (SQL keyword conflicts cause parser issues).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add expanded CAST targets and BETWEEN/IN expressions\n\nExpand CAST to target INTEGER, BIGINT, DOUBLE, VARCHAR, BOOLEAN,\nDATE, and TIMESTAMP (based on source type compatibility).\nAdd ExprKind::Between and ExprKind::InList for numeric columns,\nboth returning Boolean.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update TODO.md with completed generator coverage items\n\nMark completed items and document DuckDB incompatibilities discovered\nduring the generator expansion work.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add implementation plan for property test generator expansion\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Clarify that plans must be committed to docs/plans/\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add function name remapping to dialect printer\n\nReplace ad-hoc EXPLODE/UNNEST if-else chain with a general\nremap_function_name() lookup. Add new mappings:\n- DuckDB/PostgreSQL: EVERY -> BOOL_AND\n- SparkSQL: BOOL_AND -> EVERY, BOOL_OR -> SOME\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Wire conformance tests through dialect printer\n\nPass generated SQL through smelt_dialect::print() before executing\nagainst DuckDB, so function name remappings (EVERY->BOOL_AND, etc.)\nare applied. Type inference still runs on the original SQL.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add EVERY back to generators (remapped via dialect printer)\n\nNow that the dialect printer remaps EVERY -> BOOL_AND for DuckDB,\nwe can test EVERY in the generators. The conformance tests pass\nbecause they go through the printer before executing.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix LEFT/RIGHT keyword-as-function parsing and add to generators\n\nAdd LEFT_KW and RIGHT_KW to at_keyword_as_function_name() so the\nparser treats LEFT(...) and RIGHT(...) as function calls when\nfollowed by '(', while still handling LEFT JOIN / RIGHT JOIN as\nkeywords.\n\nRe-add LEFT and RIGHT string functions to the property test\ngenerators now that the parser handles them correctly.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update TODO.md and add dialect remapping plan\n\nUpdate coverage status: EVERY now tested via dialect remapping,\nLEFT/RIGHT now tested after parser fix. Add implementation plan.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix review findings: EVERY remap, NULLIF args, CAST boolean, TODO dup\n\n- Split PostgreSQL from DuckDB in EVERY remapping — PostgreSQL\n  natively supports EVERY, only DuckDB needs BOOL_AND remap\n- Change NULLIF to use Numeric input with IntLiteral(\"0\") instead\n  of SameAsFirst (which always returned NULL)\n- Remove CAST(numeric AS BOOLEAN) — non-standard, not portable\n- Fix duplicate SIGN line in TODO.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T21:46:59+11:00",
          "tree_id": "2e8a94c9d0bd99928d0fd18ab6ea1ce30cdd1fdd",
          "url": "https://github.com/adbrowne/smelt-sql/commit/535a42cdea105276d505f5c642039cf9832b6736"
        },
        "date": 1774090110914,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.353325999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.132867000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.590192,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30450900000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003757,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.273655,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.019205999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012373,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.01085,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.118058,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.091313,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.550060000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.31207,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.818752,
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
          "id": "35f2ce1e9e4b48981dca11263dcc0e7e97d2aad1",
          "message": "Add window function generators to property tests (#66) (#66)",
          "timestamp": "2026-03-21T22:06:24+11:00",
          "tree_id": "aee180c40cdebec7928cf148d25c6b10e2c71e3d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/35f2ce1e9e4b48981dca11263dcc0e7e97d2aad1"
        },
        "date": 1774091279899,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.828891,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.531484999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6391110000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.31351399999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.004258,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.978443,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.019937,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013485,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.01086,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.168136,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.128768,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.620069999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.62221,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.000295,
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
          "id": "b62ad6fbd08755466fdb41698444dcfe8e4f1082",
          "message": "Canonicalize JSON functions and add type inference (#67)\n\n* Canonicalize JSON functions and add type inference (#67)\n\nRedesign JSON function support to accept all dialect variants (PostgreSQL,\nDuckDB, Spark) and map them to canonical smelt functions internally:\n\n- JsonObject (json_build_object, json_object)\n- JsonArray (json_build_array, json_array)\n- ToJson (to_json, to_jsonb, row_to_json)\n- JsonExtract (json_extract, json_extract_path)\n- JsonExtractText (json_extract_string, json_extract_path_text, get_json_object, json_value)\n- JsonArrayLength, JsonObjectKeys (json_keys), JsonContains\n\nAdd type inference for JSON operators: ->, ->>, #>, #>> (Text), @>, <@ (Boolean).\nAdd JSON functions and -> / ->> operators to property test generators.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Address review: add comments for JSON type collapse, update ROADMAP\n\n- Add comment explaining why -> and ->> both return Text (no DataType::Json)\n- Update ROADMAP.md with JSON canonicalization entry\n- Note generator coverage gap for JSON_EXTRACT etc. in TODO.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add JSON example model for event property extraction\n\nDemonstrates canonical JSON functions (->>, ->, json_array_length)\nin a realistic model that extracts structured fields from JSON event\nproperties.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-22T00:17:24+11:00",
          "tree_id": "910441dccd7fed7f086976e255be64a029565e68",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b62ad6fbd08755466fdb41698444dcfe8e4f1082"
        },
        "date": 1774099138125,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.20843,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 32.987131000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.610946,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.304361,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0029349999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.580056,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.028704,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01067,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008446,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.852748,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.293364,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.25915,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.60214,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.793038,
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
          "id": "c79c8e9665b1b4b07e5a91743a37ba0327d23fc4",
          "message": "Add plan: GROUP BY / HAVING property test generators (#81)",
          "timestamp": "2026-03-22T09:16:08+11:00",
          "tree_id": "6979fe69ae2dafaf911c9acf88ed4fd7de4a9978",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c79c8e9665b1b4b07e5a91743a37ba0327d23fc4"
        },
        "date": 1774131432065,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.352144,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.168926,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.554254,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30526000000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.004168,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.403755000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017453,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011081,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009738,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.303001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.467349,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.01206,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.21664,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.01115,
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
          "id": "3063a310a0ff466a2302512178aef4408bd06ae4",
          "message": "Add GROUP BY / HAVING generators to property tests (#82)",
          "timestamp": "2026-03-22T09:15:40+11:00",
          "tree_id": "6979fe69ae2dafaf911c9acf88ed4fd7de4a9978",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3063a310a0ff466a2302512178aef4408bd06ae4"
        },
        "date": 1774131450059,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.789913999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.585621,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.591322,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.303216,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002615,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.140812,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.023955,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013274,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008705999999999998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.087867,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.50455,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.75065,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.827350000000003,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.870338,
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
          "id": "0cfb16e653b283885ef6022ef720a464f5fa1acf",
          "message": "Plan: Comprehensive Incremental Model Support (#84)\n\n* Add plan: Comprehensive Incremental Model Support (#83)\n\nCovers strategy expansion (MERGE/APPEND/INSERT_OVERWRITE), config\nunification, backfill intelligence with batch safety analysis,\nlookback windows for late-arriving data, operational metadata,\nschema evolution, orchestrator integration (Dagster/Airflow),\nand testing infrastructure.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: AST-inferred temporal dependencies replace explicit lookback\n\nPhase 3 redesigned around two orthogonal concerns:\n- Temporal dependencies (inferred from SQL AST): window functions,\n  LAG/LEAD, self-joins with date offsets — automatic, no config needed\n- Data latency (configured): how late upstream data can arrive —\n  operational knowledge that can't be inferred from the query\n\nEffective window = max(ast_inferred, data_latency). Lookahead support\nfor LEAD/forward joins. Unbounded dependencies detected and reported.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: data latency is per-column on upstream sources/models\n\nLatency is a property of the producing table's columns, not the\nconsuming model. Different columns on the same table can have\ndifferent latencies (e.g., event_time=3 days vs ingestion_time=0).\nsmelt traces the downstream model's event_time_column to the\nupstream source column and resolves the appropriate latency.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: add dbt microbatch analysis, upstream ref filtering, begin date\n\n- Added detailed dbt microbatch comparison table showing where smelt\n  improves (AST inference vs explicit event_time, per-column latency\n  vs fixed lookback, interval tracking vs stateless)\n- Added Phase 3f: upstream ref filtering (learned from microbatch's\n  automatic upstream WHERE injection, but without silent full-scan\n  failure mode)\n- Added begin date note to Phase 5 interval tracking\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: address review feedback on strategy, syntax, and execution modes\n\n- Strategy moves from model config to backend (resolve_strategy() on trait)\n- data_latency uses SQL interval syntax (\"3 days\") instead of structured YAML\n- Drop unit: partitions, require explicit time units\n- Upstream ref filtering promoted to Phase 3 MVP\n- Add max_lookback thresholds (project → model → per-dependency)\n- Add allow_unfiltered_refs config acknowledgment + LSP warnings\n- Replace --cascade with dbt-style +model/model+ selector syntax\n- Define three execution modes: run, backbuild, range run\n- Backbuild walks DAG backwards expanding ranges per temporal deps\n- Phase 5 (state tracking) marked as optional\n- Custom granularity extension point via plugin API (future)\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update docs: codebase discoveries, Phase 6 status, DESIGN.md clarifications\n\n- Plan doc: document existing selector syntax and graph traversal infrastructure,\n  add line references to Key Files table, note Phase 4 can reuse existing code\n- ROADMAP.md: mark Phase 6 as partially complete, cross-reference Phase 9,\n  link to incremental plan for advanced features\n- DESIGN.md: clarify that @materialize annotation syntax is not implemented\n  (YAML frontmatter is current config surface), note lookback_days superseded\n  by AST-inferred temporal dependencies + per-column data_latency\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-22T20:21:24+11:00",
          "tree_id": "aeadcda5601e96df569df900b48bc7b4e416f7e6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0cfb16e653b283885ef6022ef720a464f5fa1acf"
        },
        "date": 1774171376029,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 28.772813000000003,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 27.733079,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.473172,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.289635,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.00421,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 31.591161,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.028464,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015456,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.010473,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.244285,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.5590610000000003,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.5895600000000005,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 30.037419999999997,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.200053,
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
          "id": "c86c519da77225b7657e3b1f2aa4f31aa4cb8de4",
          "message": "Merge pull request #85 from adbrowne/incremental-model-support\n\nComprehensive Incremental Model Support",
          "timestamp": "2026-03-24T18:47:19+11:00",
          "tree_id": "a7f387afc31dd903fc6038211f86d009bed71141",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c86c519da77225b7657e3b1f2aa4f31aa4cb8de4"
        },
        "date": 1774338567898,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.681507,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.476986000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.586145,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30322699999999997,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002765,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.980716,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.019065,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011672,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008446,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.949744,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.005646,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.80397,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.0982,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.828279,
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
          "id": "1260d658cbdd703fcd7d6c18908e0ef7a8faadc2",
          "message": "Merge branch 'incremental-model-support'",
          "timestamp": "2026-03-24T18:49:29+11:00",
          "tree_id": "080203ea5cc60f0c1f3f180d29d2943925fb8728",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1260d658cbdd703fcd7d6c18908e0ef7a8faadc2"
        },
        "date": 1774338642077,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.354093,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.170975,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.567539,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.308635,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002766,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.335759,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017943,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010709,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008566,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.8762340000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.08203,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.40899,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.32538,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.673909,
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
          "id": "a03ea28ef1eea8703b530f42297b5da76fd38325",
          "message": "Phase 3: Run Planner — interactive run preview UI\n\nBackend:\n- POST /api/run/plan endpoint accepts time range, batch size, per-partition\n  flag, and model selection\n- build_run_plan() computes execution plan using batch safety analysis\n  from smelt-optimizer, generates batches per model\n- Returns models with batch counts, safety levels, and per-batch ranges\n\nFrontend:\n- Run Planner page with date range inputs, batch size override,\n  per-partition toggle, and model selector (click to toggle)\n- Preview button triggers plan computation\n- Plan table shows models with type, safety badge, batch count, range\n- Expandable rows show individual batch read/write ranges\n- Navigation tabs in header: Graph | Run Planner\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:36:44+11:00",
          "tree_id": "c2d6c604cfece006f47ecfe0cd714483ac9d4214",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a03ea28ef1eea8703b530f42297b5da76fd38325"
        },
        "date": 1774341536252,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.553464999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.383462,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.5586760000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.302177,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002715,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.020139,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018124,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012073,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.01046,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.086607,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.0893689999999996,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.94917,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.68969,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.830547,
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
          "id": "bc72334eead61c6fb748054db7a2a640e4ca31f5",
          "message": "Add UI dashboard expansion plan to docs/plans\n\nDocuments completed phases 1-3 and remaining phases 4-6.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:42:48+11:00",
          "tree_id": "75b2367ab709edb9cf18d26a5ddc65bf96780eb5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/bc72334eead61c6fb748054db7a2a640e4ca31f5"
        },
        "date": 1774341866777,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 29.298492999999997,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 28.229568,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.489799,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.290578,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003114,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 32.468937000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.033627000000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.017949,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.012094,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.339718,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.426984,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.66192,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 29.969990000000003,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.326563,
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
          "id": "3eee0c8f2dd76204fca7cd252175fd1611f9da9f",
          "message": "Register SIGN() Spark type divergences in property tests\n\nSpark's SIGN() preserves the input type (DOUBLE→DOUBLE, INTEGER→INTEGER,\netc.) while DuckDB returns TINYINT. smelt infers SmallInt matching DuckDB\nbehavior. Register all numeric input variants as BackendSpecific divergences\nso the Spark property tests pass.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:52:36+11:00",
          "tree_id": "5323ce943dd1972fdecaebb958d1d362279eb2ec",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3eee0c8f2dd76204fca7cd252175fd1611f9da9f"
        },
        "date": 1774342968945,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.242437,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.044162,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.5483830000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.327371,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002675,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.728839,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.023574,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011942,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008886,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.985538,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.248368,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.79975,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.82259,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.800568,
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
          "id": "5694783dec427376a5ad51f97609e85a5fd0d1e1",
          "message": "Phase 4: Run Execution + Monitoring from the UI dashboard\n\nRunManager orchestrates model execution in a background tokio task with\nreal-time progress streaming via WebSocket. Supports cancellation between\nbatches and saves run manifests + interval updates on completion.\n\nBackend: RunManager, 5 new API endpoints (execute/cancel/status/history),\nWebSocket run event streaming, RunProgressEvent types.\n\nFrontend: useRunStatus hook, RunProgress component with progress bars,\nRunHistory page, Execute button in RunPlanner, History nav tab.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T20:05:16+11:00",
          "tree_id": "ca5cafa61eba3ba96fa1041f749c923b546bb6f5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5694783dec427376a5ad51f97609e85a5fd0d1e1"
        },
        "date": 1774343245649,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.238787,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.013360999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.616686,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.301259,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002716,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.663536,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.026299,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013875,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008726000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.163311,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.1488620000000003,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.50102,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.594479999999997,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.510648,
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
          "id": "437ea51b8229728f04f6ce717598622b1c200f39",
          "message": "Show model function type signature in UI sidebar\n\nAdds the model's (inputs) -> outputs type signature to the detail\nsidebar, rendered just above the SQL block. Uses the existing\nModelFunctionType Display impl from smelt-db.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T08:32:05+11:00",
          "tree_id": "22ea8fcff259a63288aa56d22c6350934823cb3e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/437ea51b8229728f04f6ce717598622b1c200f39"
        },
        "date": 1774388041465,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.288768999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 34.061742,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.610648,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.30878999999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003857,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.77694700000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020359,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011672,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009458,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.082906,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.116118,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.0012,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.20432,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.834748,
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
          "id": "f42fe4d9c1651ed88917eb6d82fe27166a815cf6",
          "message": "Consolidate all examples under examples/ directory\n\nMove scattered example/test workspaces into a unified structure:\n- examples/timeseries/ (was examples/) — 12 SQL user/event analytics models\n- examples/retail_analytics/ (was benchmarks/retail-analytics/) — 25 TPC-DS models\n- examples/broken/ (new) — 5 intentionally broken models for error testing\n- examples/test_workspace/ (was test-workspace/) — minimal VSCode/LSP testing\n- examples/huge/ (new) — 2000 auto-generated models for stress testing\n\nRefactored smelt-bench model_gen to support persistent output directories\nand added generate_static_workspace binary. Updated all integration test\npaths, documentation references, and .gitignore.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T14:02:19+11:00",
          "tree_id": "6ce62defcf586a61e26d9d722292c006109cddc2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f42fe4d9c1651ed88917eb6d82fe27166a815cf6"
        },
        "date": 1774407842960,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 35.091352,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.823328,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.63573,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.311323,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.005411,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.653574,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020098,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.014528,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.012874,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.19119,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 3.30085,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.128640000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 48.31555,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.809497,
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
          "id": "e9462fddaef02c42c838ebbf95ad5ed2dcf94c8b",
          "message": "Run Planner: select/exclude with CLI command preview\n\n- Add --exclude flag to CLI (RunArgs, BuildArgs) with selector syntax\n- Add exclude_models() and all_model_names() to DependencyGraph (both smelt-core and smelt-cli)\n- Use proper selector parsing in UI backend (build_run_plan, run_manager)\n  instead of simple name matching — supports tags, upstream/downstream\n- Add POST /api/resolve endpoint for lightweight selector resolution\n- Generate CLI command string in RunPlanResponse\n- Redesign RunPlanner UI:\n  - Text inputs for select/exclude (space-separated selector syntax)\n  - Model pills that toggle tokens in the text inputs (single source of truth)\n  - Shift+click pills to exclude (red/strikethrough)\n  - Pills highlight resolved models via /api/resolve (including upstream/downstream)\n  - CLI command box with copy button shown after preview\n  - Resolved models list shown after preview",
          "timestamp": "2026-03-25T14:45:43+11:00",
          "tree_id": "f7902d8488647981b91950503bed59a4307abd0f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/e9462fddaef02c42c838ebbf95ad5ed2dcf94c8b"
        },
        "date": 1774410468998,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 37.766657,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.609398,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.5400280000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.308315,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0026249999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.67017,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018194,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011141,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009819,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.947297,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.113424,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.45588,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.04737,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.522515,
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
          "id": "d55da26f01ce22bf874669ddf3ed603ca3046231",
          "message": "Merge pull request #86 from adbrowne/docs3\n\nUpdate outdated documentation",
          "timestamp": "2026-03-25T15:26:11+11:00",
          "tree_id": "ae50ae5b8c4f1576fa08bab122ca37f341b67925",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d55da26f01ce22bf874669ddf3ed603ca3046231"
        },
        "date": 1774412884316,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.688593,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.515505999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.545712,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.31176899999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003056,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.100615000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.024436000000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012884,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008607,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.08883,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.452202,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.06209,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.36359,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.255676,
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
          "id": "5f13317efbd935a707ed4b64bc2a7d773389f1b8",
          "message": "Rename smelt-optimizer to smelt-planner\n\nThe crate implements planning (execution strategy, materialization,\nbatching) rather than query optimization. This rename clarifies intent.\n\n- Crate: smelt-optimizer → smelt-planner\n- Struct: Optimizer → Planner, .optimize() → .plan()\n- Python: OptimizerRule → PlannerRule, entry point smelt.planner_rules\n- Docs: updated CLAUDE.md, ROADMAP.md, architecture_overview.md\n- Renamed optimization_rule_api_design.md → planner_rule_api_design.md\n- Test file: optimizer_test.rs → planner_test.rs\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T15:49:32+11:00",
          "tree_id": "e899b17edab3bf7e8e9394518688d85a3c40d093",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5f13317efbd935a707ed4b64bc2a7d773389f1b8"
        },
        "date": 1774414295041,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 37.951205,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.768629,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.538128,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.320491,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002696,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.190859999999994,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018224,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011161,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008585,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.971881,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.122677,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.88736,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.17131,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.274568,
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
          "id": "f31ca08bed91764bcb6fa960814010c7910c270b",
          "message": "Per-model backend routing: route models to different targets in a single run\n\nAdd per-model target assignment so `smelt run` can execute models against\ndifferent backends. Users specify targets via SQL frontmatter (`target: spark_prod`)\nor smelt.yml model config, with precedence: frontmatter > smelt.yml > CLI --target.\n\n- Add `target` field to ModelConfig and ModelMetadata\n- Add Config::get_target() with 3-level precedence resolution\n- Add BackendRegistry (creates backends per-target) and CompilerRegistry (dialect-aware compilation per-target)\n- Add cross-backend ref validation (clear error when model refs span targets)\n- Update run, backbuild, and UI execution loops to use per-model backend/compiler/schema\n- Cross-backend data transfer deferred to future work\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T16:45:29+11:00",
          "tree_id": "52140c850d511500aba3c3ae7de744781577d0b3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f31ca08bed91764bcb6fa960814010c7910c270b"
        },
        "date": 1774417659990,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.04784,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.765972000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.648969,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.312963,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003577,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.814947,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.029225,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015729,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011822,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.466893,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.651068,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.389740000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.34618,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.964488,
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
          "id": "03964482464c7d25a61b81cc4978955e190275c7",
          "message": "Removed log files",
          "timestamp": "2026-03-25T16:49:14+11:00",
          "tree_id": "69a09012a4e45dc8b7f829996a7db314128ea7c7",
          "url": "https://github.com/adbrowne/smelt-sql/commit/03964482464c7d25a61b81cc4978955e190275c7"
        },
        "date": 1774417842140,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.035872,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.733402,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.674734,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.306103,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0034460000000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 36.081949,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.031048,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015269,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.012183,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.401064,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.593648,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.25143,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.345840000000003,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.006878,
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
          "id": "240cfe7659c18082fa1a14e29ca23072c03b6cb5",
          "message": "Removed script",
          "timestamp": "2026-03-25T16:49:33+11:00",
          "tree_id": "5645c5b1ad398d0b798135ac8579c9ce4160a537",
          "url": "https://github.com/adbrowne/smelt-sql/commit/240cfe7659c18082fa1a14e29ca23072c03b6cb5"
        },
        "date": 1774417896842,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.782517000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.552192,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.612107,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.305183,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0034869999999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.176038999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.029435,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.015189,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.010971,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.370939,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.5298909999999997,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.88462,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.54584,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.021158,
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
          "id": "a19feda9e0199170d1fdd92f5ae6f28c29d8d6b2",
          "message": "Fix CI: clippy field_reassign_with_default and DuckDB manylinux CXX ABI\n\n- Use struct initializer syntax instead of field reassignment in config test\n- Set DUCKDB_PLATFORM env var in manylinux Docker containers to fix\n  DuckDB bundled build failing on legacy CXX ABI\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:09:53+11:00",
          "tree_id": "c9d73afbdf5a91c0511118e1a07c8e69fc8bbcf5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a19feda9e0199170d1fdd92f5ae6f28c29d8d6b2"
        },
        "date": 1774422672149,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 43.345685,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 41.891962,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.760104,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.342612,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0034470000000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 41.102353,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.034064,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.017122,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011962,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.468641,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.497979,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.24099,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.24742,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.739494,
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
          "id": "9f59cafcb648fba442c5c0a7dc3802c064fb1fb7",
          "message": "Fix Linux wheel builds: use CXXFLAGS define for DuckDB legacy ABI\n\nDUCKDB_EXPLICIT_PLATFORM must be a C++ preprocessor define, not a shell\nenv var. Pass it via CXXFLAGS which the cc crate reads for C++ builds.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:30:36+11:00",
          "tree_id": "935a2f616c50fe61076ff905b7d4e987dac112f6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/9f59cafcb648fba442c5c0a7dc3802c064fb1fb7"
        },
        "date": 1774423919174,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.271371,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.082256,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.572501,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.305882,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.004127,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.440666,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017482,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012223,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009378,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.108574,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.359302,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.01888,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.68498,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.063941,
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
          "id": "ae1dda244abeae11b8d5a26b01506996ded2488a",
          "message": "Fix Linux wheel builds: use DUCKDB_CUSTOM_PLATFORM to bypass ABI check\n\nThe DuckDB platform.hpp #error for legacy CXX ABI cannot be bypassed\nwith DUCKDB_EXPLICIT_PLATFORM (it's only mentioned in the error text,\nnot actually checked). Use DUCKDB_CUSTOM_PLATFORM which short-circuits\nthe entire platform detection function.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:49:47+11:00",
          "tree_id": "d51b5faec57f727ae7388189f009898f4ebb5248",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ae1dda244abeae11b8d5a26b01506996ded2488a"
        },
        "date": 1774425100134,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.288903,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.075174,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.573132,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.316957,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002575,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.340473,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.01651,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010168,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.007984,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.908514,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.141499,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.62422,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.83414,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.025545,
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
          "id": "ea7b6d023d657bc07f752289ec61f832c9458057",
          "message": "Switch dev release publishing from PyPI to TestPyPI\n\nDev releases should publish to TestPyPI. Production PyPI publishing\nremains in the release.yml workflow for tagged releases.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T19:42:39+11:00",
          "tree_id": "77f895258209f1d39fff678050edb449665f64a9",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ea7b6d023d657bc07f752289ec61f832c9458057"
        },
        "date": 1774428235697,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.377465,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.22541,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.529541,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.31695300000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002615,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.842331,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017442,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01047,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008265,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.878363,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.279494,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.2754,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.83611,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.257661,
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
          "id": "3bc48925032f80402a9d6a9ba95486aa27496c29",
          "message": "Merge branch 'worktree-client-feedback'",
          "timestamp": "2026-03-25T20:51:19+11:00",
          "tree_id": "025a59fa478ef6104fe35ebd715345277255521a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3bc48925032f80402a9d6a9ba95486aa27496c29"
        },
        "date": 1774432389820,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.33311,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.820496,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.791265,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.393605,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002914,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.413521,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.024908,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.010947,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008872999999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.154814,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.287304,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.94717,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.4073,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.139272,
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
          "id": "01bd9226154c0a3dc725c5e5e4b18566f3a1b5d1",
          "message": "Fix formatting in discovery.rs tests\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T20:57:57+11:00",
          "tree_id": "561edc0f8bee1d210da69974f8c4ca020cca5b3c",
          "url": "https://github.com/adbrowne/smelt-sql/commit/01bd9226154c0a3dc725c5e5e4b18566f3a1b5d1"
        },
        "date": 1774432779235,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 32.498537,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.352236,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.53166,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.337661,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003872,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 31.277689999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.025844,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013373,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.010002,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.114867,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1.879693,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.9469,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.20899,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.586967,
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
          "id": "162cca5d729f276b07c2570b02b31b06f538e831",
          "message": "Add plan for logical-to-physical graph architecture\n\nIntroduces a two-stage graph design separating user-authored models\n(logical graph) from the execution plan (physical graph). The physical\ngraph removes ephemeral nodes, adds planner-created intermediates as\nfirst-class nodes, and carries concrete execution strategies. This\ngives planner rule authors a clear contract and follows patterns from\nDataFusion, Spark Catalyst, and Apache Calcite.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T21:39:01+11:00",
          "tree_id": "5b1ee3e27ffce2c58c250fdabb6b6a8a64a71beb",
          "url": "https://github.com/adbrowne/smelt-sql/commit/162cca5d729f276b07c2570b02b31b06f538e831"
        },
        "date": 1774435314818,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 37.679714,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.345878,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6342519999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.384202,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002775,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.587352,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.02151,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013435,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008506,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.177513,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.303059,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.00919,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.55781,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.167977,
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
          "id": "fff2a57b46b99083d668070db8343a09650a3298",
          "message": "Update materialization plan with completed phases and remaining integration work",
          "timestamp": "2026-03-26T08:26:44+11:00",
          "tree_id": "d9550a2a344e068319e85125711fcb0e5cf1d237",
          "url": "https://github.com/adbrowne/smelt-sql/commit/fff2a57b46b99083d668070db8343a09650a3298"
        },
        "date": 1774474074894,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.975608,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.525012,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.69996,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.356367,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0025450000000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.102188,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.019346,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011682,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008617000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.123252,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.2878030000000003,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.65869,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.84755,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.24477,
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
          "distinct": false,
          "id": "4812ff3c405ebb50ad7a2257ca6ab45c650272af",
          "message": "Wire ephemeral models into CLI execution loop with example\n\n- Build EphemeralResolver per target and use compile_with_ephemerals for\n  all model compilation in run() and backbuild() loops\n- Skip ephemeral models during execution (print info, continue)\n- Validate materialization configs at startup (ephemeral+incremental etc)\n- Warn on unused ephemeral models with no downstream consumers\n- Error when --select directly targets an ephemeral model\n- Add compile_with_sql_and_ephemerals for incremental code path\n- Fix type-cast column name inference to use Expr::infer_name() instead\n  of \"?\" placeholder for bare column references\n- Add examples/ephemeral_demo/ demonstrating CTE inlining\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T18:32:57+11:00",
          "tree_id": "3c494311abe186f68ca089a72ab66e65203e56eb",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4812ff3c405ebb50ad7a2257ca6ab45c650272af"
        },
        "date": 1774510514270,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.723645,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.331218,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.707854,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.369832,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.004558,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.605227000000006,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.02704,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013535,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.011281,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.450325,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.609224,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.98881,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.661609999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.997447,
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
          "id": "3a7c82f5fd6f4c668143120c4f76b76335c42872",
          "message": "Update graph stages plan with landed ephemeral support\n\nRefresh the logical-to-physical graph plan now that ephemeral model\nsupport (Materialization::Ephemeral, EphemeralResolver, CTE inlining)\nhas landed on main. Key updates: reference existing EphemeralResolver\nfor reuse, add CreateMaterializedView strategy, note which validations\nalready exist, and detail the graph consolidation (delete smelt-cli's\nduplicate DependencyGraph).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T18:51:57+11:00",
          "tree_id": "e1e9ef52ba4e6dec28f7d687e51b8d3f3c26b664",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3a7c82f5fd6f4c668143120c4f76b76335c42872"
        },
        "date": 1774511608769,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.301932,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.862258,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.713502,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.391569,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002764,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.178115000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.024426999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009104,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.081756,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.357192,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.54373,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.331679999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.321226,
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
          "distinct": false,
          "id": "8efaff8240a6abc9c3e774766fe18664eec924be",
          "message": "Add physical graph section to smelt explain output (Phase D)\n\nsmelt explain now runs the planner and shows the physical execution plan\nalongside the logical graph — strategies, ephemerals, and planner\noptimizations are visible without connecting to any database.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T20:49:25+11:00",
          "tree_id": "ceeb10ab3ed6b4b878117299c87f6eb27489ed90",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8efaff8240a6abc9c3e774766fe18664eec924be"
        },
        "date": 1774518689705,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 37.402663,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.095438,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6347619999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.3693479999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002515,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 33.640443,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017372,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.01049,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008145000000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.936935,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.305916,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.77379,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.54586,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.245534,
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
          "distinct": false,
          "id": "296e5afeda5721c8ab3d91e760f4e38fd179b21b",
          "message": "Remove smelt-cli DependencyGraph, use PhysicalGraph in backbuild\n\n- Delete crates/smelt-cli/src/graph.rs (duplicate of smelt-core's)\n- Migrate python.rs tests and integration tests to LogicalGraph\n- backbuild() now uses PhysicalGraph for ephemeral resolver ownership\n  instead of manually constructing resolvers\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T20:57:53+11:00",
          "tree_id": "e5e084d5277fe45c2fb635acc11ad82c6a399dff",
          "url": "https://github.com/adbrowne/smelt-sql/commit/296e5afeda5721c8ab3d91e760f4e38fd179b21b"
        },
        "date": 1774519165798,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 37.095063999999994,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 35.851565,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.598937,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.341858,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.002695,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.012094,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.017553,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.0107,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008456,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.94368,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.365831,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.65548,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.56373,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.04944,
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
          "distinct": false,
          "id": "132425beac5a4f387d9d6c678d2e52e822946048",
          "message": "Update graph stages plan: all deferred items resolved\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T21:04:04+11:00",
          "tree_id": "a3c218b5195498542168b4f1b44182f13cd8b8b8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/132425beac5a4f387d9d6c678d2e52e822946048"
        },
        "date": 1774519576131,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.997747,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.624085,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.701814,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.355696,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003126,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.744484,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.022913,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.013796,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.26044,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.458684,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.94854,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.48267,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.053732,
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
          "id": "5186d00289b577d19744d3163b7bede7f8e3b353",
          "message": "Add comprehensive multi-perspective codebase review report\n\nReview from 10 professional viewpoints (dbt user, Director of Engineering,\nSQLMesh user, Data Architect, Analytics Engineer, Spark Engineer, Data Analyst,\nRust Architect, Rust Developer, Python Developer) with verdicts, evidence-backed\nanalysis, cross-cutting themes, and prioritized recommendations.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T21:31:17+11:00",
          "tree_id": "0a66137a398cf4a2ce73c9faa29281c696edcd4a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5186d00289b577d19744d3163b7bede7f8e3b353"
        },
        "date": 1774521154658,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.503978,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 37.115893,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.714926,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.360238,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 35.449432,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.018264,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.012143,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008426,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.085874,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.32373,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.531460000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.83838,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.32422,
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
          "id": "cb698145f06e5b118bc709f1d3e623fff3d6b9f9",
          "message": "Merge branch 'worktree-docs'",
          "timestamp": "2026-03-26T21:39:53+11:00",
          "tree_id": "548ab8b8c25dfbbba9d82d814d34b6cc4a761be3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb698145f06e5b118bc709f1d3e623fff3d6b9f9"
        },
        "date": 1774521693732,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 32.975135,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.811969,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.573033,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.311319,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.003965,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 31.796654,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.031089,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.017529,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.009871,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.323658,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.052454,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.54,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 27.33569,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.528525,
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
          "id": "7dd8fbb6ed023accf73a058f80e70986df4d6fef",
          "message": "Merge pull request #87 from adbrowne/worktree-roadmap_from_review\n\nRestructure roadmap based on codebase review",
          "timestamp": "2026-03-26T22:53:32+11:00",
          "tree_id": "d3b74784042411d96824793c01c1e605a395bae0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/7dd8fbb6ed023accf73a058f80e70986df4d6fef"
        },
        "date": 1774526141852,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 38.335626,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 36.973905,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.687524,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.365602,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.0026249999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 34.593401,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 0.020197999999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 0.011221,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 0.008486,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.033068,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 2.3121259999999997,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.92159,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.74582,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.171269,
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
        "date": 1774017616157,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.78078245352556,
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
          "id": "cb22c6ac9973fdbbf144dd232aa36f812a06aebc",
          "message": "Fix maturin workspace build: add manifest-path and bundle smelt-lsp\n\nMaturin failed with \"missing field `package`\" because the root Cargo.toml\nis a workspace without a [package] section. Fix by:\n- Adding manifest-path to point maturin at crates/smelt-cli/Cargo.toml\n- Pre-building smelt-lsp and placing it in smelt_sql.data/scripts/ so\n  both binaries are included in the wheel\n- Using before-script-linux for manylinux container builds\n- Using a separate step for macOS/Windows builds\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T01:41:58+11:00",
          "tree_id": "e549a513b0aec6f26e82711edabd0b4831a48846",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb22c6ac9973fdbbf144dd232aa36f812a06aebc"
        },
        "date": 1774017796082,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.16945312653156,
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
          "id": "1a90ac78eecc630a6fc5b6d4c110a49f384e000d",
          "message": "Add docs link to README (#55)\n\n* Add link to GitHub Pages documentation in README\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Configure smeltsql.com custom domain for GitHub Pages\n\n- Add CNAME file for GitHub Pages custom domain\n- Update site_url in mkdocs.yml to smeltsql.com\n- Update documentation link in README.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T02:02:16+11:00",
          "tree_id": "5335151d9b32ac1b34ff1b7513ae71c96ef3bb96",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1a90ac78eecc630a6fc5b6d4c110a49f384e000d"
        },
        "date": 1774019036794,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.423570187058687,
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
          "id": "3cd5a2a34f3ae57b96d322b32f5b959a67d6726e",
          "message": "Drop macos-x86_64 builds: macos-13 runner deprecated\n\nGitHub Actions no longer supports the macos-13 runner. Remove\nmacos-x86_64 from build matrices in release and dev-release workflows.\nmacOS ARM64 (Apple Silicon) is the only supported macOS target; x86_64\nusers can run via Rosetta 2.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T02:03:36+11:00",
          "tree_id": "48804c926af8d0dd2a6dcba9106e4de54a7d6e92",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3cd5a2a34f3ae57b96d322b32f5b959a67d6726e"
        },
        "date": 1774019117486,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.40641670726312,
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
          "id": "604e86e495a5e13c72d14df116910b13c9019148",
          "message": "Update duckdb to 1.4.4 and arrow/parquet to 57 (#56)\n\nMatches the system-installed DuckDB version (1.4.4) and updates\narrow/parquet to stay in sync with duckdb's arrow dependency.\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T11:47:15+11:00",
          "tree_id": "cfd6f53f926ff32e710941c43cf899490ff2a3a3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/604e86e495a5e13c72d14df116910b13c9019148"
        },
        "date": 1774054163145,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.08544385187712,
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
          "id": "eb17b3761f37fe19dcd3b9c14e44884956e57731",
          "message": "Add property-based type inference tests against DuckDB (#57)\n\nVerify smelt's type inference matches real database behavior using proptest\nto generate random typed CTE queries and compare inferred types against\nDuckDB's actual Arrow schema output.\n\n- 256-case proptest + 5 deterministic smoke tests + unit tests (21 total)\n- TypeOracle trait for future PostgreSQL/Spark backends\n- Known divergence registry for expected mismatches (SUM, CEIL, EXTRACT, etc.)\n- Compatible type handling (Text/Varchar, Decimal precision, integer widths)\n- Arrow-to-smelt type mapping module\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T11:52:39+11:00",
          "tree_id": "1a197a65de48c0d46493e68dd46d64fcb93c57a3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/eb17b3761f37fe19dcd3b9c14e44884956e57731"
        },
        "date": 1774054474239,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.22451837619422,
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
          "id": "b14ac0591883cdace08076b46efdc397794bda0f",
          "message": "Fix dev release: patch inter-crate dependency versions (#58)\n\nThe dev version patching only updated the workspace root Cargo.toml,\nbut smelt-dialect has a hardcoded `version = \"0.1.0\"` dependency on\nsmelt-parser. Semver pre-release versions (0.1.0-dev.xxx) don't match\n^0.1.0, causing all builds to fail with \"failed to select a version\".\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T13:15:41+11:00",
          "tree_id": "be5f50e8b512360bd92c2241e35a750250dadb95",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b14ac0591883cdace08076b46efdc397794bda0f"
        },
        "date": 1774059448400,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.41571149019584,
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
          "id": "5f3fe31be16e25eaa3f7d5c3044225b0d9b4a62a",
          "message": "Add Spark SQL oracle for type property tests (#59)",
          "timestamp": "2026-03-21T16:39:17+11:00",
          "tree_id": "38a3435dd312337bca4c5fc4b8bcae7f8691a4e0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5f3fe31be16e25eaa3f7d5c3044225b0d9b4a62a"
        },
        "date": 1774071629775,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.273024358538567,
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
          "id": "0343c12f4ade278b6bfff90e8a8de76dc9e036f8",
          "message": "Add end-of-conversation checklist to CLAUDE.md\n\nInstructs Claude to write unfinished work and open decisions to\ndocs/TODO.md and update CLAUDE.md before ending a conversation.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T16:45:09+11:00",
          "tree_id": "43e10f771a9fc9cce96222b8ea03d24791fc94a0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0343c12f4ade278b6bfff90e8a8de76dc9e036f8"
        },
        "date": 1774072029792,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.44425786138691,
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
          "id": "7dea6f891a5a7ff5e8177d8c4df65d86d6fd6741",
          "message": "Add type conformance casts to ensure backend output matches smelt inference (#60)",
          "timestamp": "2026-03-21T18:14:16+11:00",
          "tree_id": "7ac78aad2e63dea039db3aad37705267204b5f09",
          "url": "https://github.com/adbrowne/smelt-sql/commit/7dea6f891a5a7ff5e8177d8c4df65d86d6fd6741"
        },
        "date": 1774077375392,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.38044443557909,
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
          "id": "bc1273a9d0cf645e28e74cb7afd809e6bfad4970",
          "message": "Add cross-model type propagation tests (#61)",
          "timestamp": "2026-03-21T18:34:59+11:00",
          "tree_id": "33ebe9537d79352933732c0e249a1b3583827789",
          "url": "https://github.com/adbrowne/smelt-sql/commit/bc1273a9d0cf645e28e74cb7afd809e6bfad4970"
        },
        "date": 1774078598801,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.25381400300149,
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
          "id": "4ec93eae0dcf3dcfaf0637beee0b6f04b1c0e688",
          "message": "Fix Unknown type gaps and add type diagnostics (#62)",
          "timestamp": "2026-03-21T19:50:58+11:00",
          "tree_id": "e440bbd98377a9e06acdb3576c7c5348a727ed6e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4ec93eae0dcf3dcfaf0637beee0b6f04b1c0e688"
        },
        "date": 1774083128622,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.274372903758213,
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
          "id": "f75035bf7385d5e154ee05ca8a73860f175af28e",
          "message": "Fix KnownBug type inference divergences (#63)",
          "timestamp": "2026-03-21T19:50:34+11:00",
          "tree_id": "fdd94d0f707f1eb52625708d70d137f319fe4e7c",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f75035bf7385d5e154ee05ca8a73860f175af28e"
        },
        "date": 1774083135904,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.060862363898014,
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
          "id": "fde96655274afde115b803b0e882541c1db00b5d",
          "message": "Add plans documentation convention to CLAUDE.md (#64)",
          "timestamp": "2026-03-21T20:02:40+11:00",
          "tree_id": "fe4db904d9ed97beed499ceae8c20b7aff0aa772",
          "url": "https://github.com/adbrowne/smelt-sql/commit/fde96655274afde115b803b0e882541c1db00b5d"
        },
        "date": 1774083865023,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.723815049551504,
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
          "id": "f26726e256331c2064b29801a01cf6f6af4e3e49",
          "message": "Add property test generator coverage gaps to TODO\n\nAudit of generators vs parser/type inference support revealed extensive\ngaps: ~80% of string/math functions, all window functions, temporal\nfunctions, advanced aggregates, and several expression types untested.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T20:13:56+11:00",
          "tree_id": "5d7b2bbbe1acc9eb6fdcc87cf5012ac8308d3fc8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f26726e256331c2064b29801a01cf6f6af4e3e49"
        },
        "date": 1774084500526,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.30423092156357,
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
          "id": "535a42cdea105276d505f5c642039cf9832b6736",
          "message": "Expand property test generator coverage (#65)\n\n* Add trig and log math functions to property test generators\n\nAdd 16 single-arg math functions: POWER, POW, EXP, LN, LOG, LOG10,\nLOG2, SIN, COS, TAN, ASIN, ACOS, ATAN, SINH, COSH, TANH.\nAll take numeric input and return Double.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add SIGN function to generators and fix SIGN type inference\n\nSIGN always returns a small integer (-1, 0, 1) in DuckDB (TINYINT).\nFix smelt's type inference to return SmallInt instead of arg type.\nRemove POWER/POW (needs 2 args) and ASIN/ACOS (domain-restricted)\nfrom single-arg generators; they'll be added in multi-arg step.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add single-arg string functions to property test generators\n\nAdd LTRIM, RTRIM, INITCAP, CONCAT (single-arg), CHAR_LENGTH, and\nCHARACTER_LENGTH to the generator function list.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add COALESCE and AnyScalar input variant to generators\n\nAdd FuncInput::AnyScalar for non-aggregate functions that accept any\ntype. Add COALESCE (single-arg, returns arg type). Remove INITCAP\n(not available in DuckDB).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add GREATEST and LEAST to property test generators\n\nBoth use AnyScalar input and return the argument type.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add statistical aggregates and refactor aggregate detection\n\nAdd STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP\nto generators. Refactor assemble_cte_query() to use SqlFunction::\nfrom_name + is_aggregate() instead of fragile starts_with() checks.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add boolean aggregate functions to property test generators\n\nAdd BOOL_AND, BOOL_OR, EVERY with new FuncInput::BooleanAggregate\nvariant that only accepts Boolean columns.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add bit and boolean aggregates to property test generators\n\nAdd BIT_AND, BIT_OR, BIT_XOR with new FuncInput::IntegerAggregate.\nAdd BOOL_AND, BOOL_OR with FuncInput::BooleanAggregate.\nRemove EVERY (not available in DuckDB).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add PI() zero-arg function to property test generators\n\nAdd FuncInput::NoArg variant and PI() function. Update generate_expr()\nto emit function calls without arguments for NoArg functions.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add multi-arg function infrastructure and functions\n\nAdd ExtraArg enum (SameAsFirst, IntLiteral, StringLiteral) and\nextra_args field to FuncDesc. Update generate_expr() to build\nmulti-arg function calls.\n\nNew functions: REPLACE, LPAD, RPAD, LEFT, RIGHT, REPEAT (string),\nNULLIF (any-type), POWER, MOD, ATAN2 (numeric).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add SUBSTRING, SUBSTR, SPLIT_PART, STRPOS to generators\n\nMulti-arg string functions using the ExtraArg infrastructure.\nSTRPOS covers the same inference path as POSITION (special syntax).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add DATE_PART, DATE_TRUNC and prepend_literal support\n\nAdd prepend_literal field to FuncDesc for functions where a string\nliteral must come before the column argument (e.g. DATE_PART('year', col)).\nAdd DATE_PART (returns BigInt) and DATE_TRUNC (returns Timestamp).\nRemove LEFT/RIGHT (SQL keyword conflicts cause parser issues).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add expanded CAST targets and BETWEEN/IN expressions\n\nExpand CAST to target INTEGER, BIGINT, DOUBLE, VARCHAR, BOOLEAN,\nDATE, and TIMESTAMP (based on source type compatibility).\nAdd ExprKind::Between and ExprKind::InList for numeric columns,\nboth returning Boolean.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update TODO.md with completed generator coverage items\n\nMark completed items and document DuckDB incompatibilities discovered\nduring the generator expansion work.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add implementation plan for property test generator expansion\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Clarify that plans must be committed to docs/plans/\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add function name remapping to dialect printer\n\nReplace ad-hoc EXPLODE/UNNEST if-else chain with a general\nremap_function_name() lookup. Add new mappings:\n- DuckDB/PostgreSQL: EVERY -> BOOL_AND\n- SparkSQL: BOOL_AND -> EVERY, BOOL_OR -> SOME\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Wire conformance tests through dialect printer\n\nPass generated SQL through smelt_dialect::print() before executing\nagainst DuckDB, so function name remappings (EVERY->BOOL_AND, etc.)\nare applied. Type inference still runs on the original SQL.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add EVERY back to generators (remapped via dialect printer)\n\nNow that the dialect printer remaps EVERY -> BOOL_AND for DuckDB,\nwe can test EVERY in the generators. The conformance tests pass\nbecause they go through the printer before executing.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix LEFT/RIGHT keyword-as-function parsing and add to generators\n\nAdd LEFT_KW and RIGHT_KW to at_keyword_as_function_name() so the\nparser treats LEFT(...) and RIGHT(...) as function calls when\nfollowed by '(', while still handling LEFT JOIN / RIGHT JOIN as\nkeywords.\n\nRe-add LEFT and RIGHT string functions to the property test\ngenerators now that the parser handles them correctly.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update TODO.md and add dialect remapping plan\n\nUpdate coverage status: EVERY now tested via dialect remapping,\nLEFT/RIGHT now tested after parser fix. Add implementation plan.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Fix review findings: EVERY remap, NULLIF args, CAST boolean, TODO dup\n\n- Split PostgreSQL from DuckDB in EVERY remapping — PostgreSQL\n  natively supports EVERY, only DuckDB needs BOOL_AND remap\n- Change NULLIF to use Numeric input with IntLiteral(\"0\") instead\n  of SameAsFirst (which always returned NULL)\n- Remove CAST(numeric AS BOOLEAN) — non-standard, not portable\n- Fix duplicate SIGN line in TODO.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-21T21:46:59+11:00",
          "tree_id": "2e8a94c9d0bd99928d0fd18ab6ea1ce30cdd1fdd",
          "url": "https://github.com/adbrowne/smelt-sql/commit/535a42cdea105276d505f5c642039cf9832b6736"
        },
        "date": 1774090111872,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.992719366647172,
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
          "id": "35f2ce1e9e4b48981dca11263dcc0e7e97d2aad1",
          "message": "Add window function generators to property tests (#66) (#66)",
          "timestamp": "2026-03-21T22:06:24+11:00",
          "tree_id": "aee180c40cdebec7928cf148d25c6b10e2c71e3d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/35f2ce1e9e4b48981dca11263dcc0e7e97d2aad1"
        },
        "date": 1774091280831,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.629752435252634,
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
          "id": "b62ad6fbd08755466fdb41698444dcfe8e4f1082",
          "message": "Canonicalize JSON functions and add type inference (#67)\n\n* Canonicalize JSON functions and add type inference (#67)\n\nRedesign JSON function support to accept all dialect variants (PostgreSQL,\nDuckDB, Spark) and map them to canonical smelt functions internally:\n\n- JsonObject (json_build_object, json_object)\n- JsonArray (json_build_array, json_array)\n- ToJson (to_json, to_jsonb, row_to_json)\n- JsonExtract (json_extract, json_extract_path)\n- JsonExtractText (json_extract_string, json_extract_path_text, get_json_object, json_value)\n- JsonArrayLength, JsonObjectKeys (json_keys), JsonContains\n\nAdd type inference for JSON operators: ->, ->>, #>, #>> (Text), @>, <@ (Boolean).\nAdd JSON functions and -> / ->> operators to property test generators.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Address review: add comments for JSON type collapse, update ROADMAP\n\n- Add comment explaining why -> and ->> both return Text (no DataType::Json)\n- Update ROADMAP.md with JSON canonicalization entry\n- Note generator coverage gap for JSON_EXTRACT etc. in TODO.md\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Add JSON example model for event property extraction\n\nDemonstrates canonical JSON functions (->>, ->, json_array_length)\nin a realistic model that extracts structured fields from JSON event\nproperties.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-22T00:17:24+11:00",
          "tree_id": "910441dccd7fed7f086976e255be64a029565e68",
          "url": "https://github.com/adbrowne/smelt-sql/commit/b62ad6fbd08755466fdb41698444dcfe8e4f1082"
        },
        "date": 1774099139019,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.045034027703466,
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
          "id": "c79c8e9665b1b4b07e5a91743a37ba0327d23fc4",
          "message": "Add plan: GROUP BY / HAVING property test generators (#81)",
          "timestamp": "2026-03-22T09:16:08+11:00",
          "tree_id": "6979fe69ae2dafaf911c9acf88ed4fd7de4a9978",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c79c8e9665b1b4b07e5a91743a37ba0327d23fc4"
        },
        "date": 1774131433794,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.608397197603892,
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
          "id": "3063a310a0ff466a2302512178aef4408bd06ae4",
          "message": "Add GROUP BY / HAVING generators to property tests (#82)",
          "timestamp": "2026-03-22T09:15:40+11:00",
          "tree_id": "6979fe69ae2dafaf911c9acf88ed4fd7de4a9978",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3063a310a0ff466a2302512178aef4408bd06ae4"
        },
        "date": 1774131450951,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.88845203902366,
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
          "id": "0cfb16e653b283885ef6022ef720a464f5fa1acf",
          "message": "Plan: Comprehensive Incremental Model Support (#84)\n\n* Add plan: Comprehensive Incremental Model Support (#83)\n\nCovers strategy expansion (MERGE/APPEND/INSERT_OVERWRITE), config\nunification, backfill intelligence with batch safety analysis,\nlookback windows for late-arriving data, operational metadata,\nschema evolution, orchestrator integration (Dagster/Airflow),\nand testing infrastructure.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: AST-inferred temporal dependencies replace explicit lookback\n\nPhase 3 redesigned around two orthogonal concerns:\n- Temporal dependencies (inferred from SQL AST): window functions,\n  LAG/LEAD, self-joins with date offsets — automatic, no config needed\n- Data latency (configured): how late upstream data can arrive —\n  operational knowledge that can't be inferred from the query\n\nEffective window = max(ast_inferred, data_latency). Lookahead support\nfor LEAD/forward joins. Unbounded dependencies detected and reported.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: data latency is per-column on upstream sources/models\n\nLatency is a property of the producing table's columns, not the\nconsuming model. Different columns on the same table can have\ndifferent latencies (e.g., event_time=3 days vs ingestion_time=0).\nsmelt traces the downstream model's event_time_column to the\nupstream source column and resolves the appropriate latency.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: add dbt microbatch analysis, upstream ref filtering, begin date\n\n- Added detailed dbt microbatch comparison table showing where smelt\n  improves (AST inference vs explicit event_time, per-column latency\n  vs fixed lookback, interval tracking vs stateless)\n- Added Phase 3f: upstream ref filtering (learned from microbatch's\n  automatic upstream WHERE injection, but without silent full-scan\n  failure mode)\n- Added begin date note to Phase 5 interval tracking\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update plan: address review feedback on strategy, syntax, and execution modes\n\n- Strategy moves from model config to backend (resolve_strategy() on trait)\n- data_latency uses SQL interval syntax (\"3 days\") instead of structured YAML\n- Drop unit: partitions, require explicit time units\n- Upstream ref filtering promoted to Phase 3 MVP\n- Add max_lookback thresholds (project → model → per-dependency)\n- Add allow_unfiltered_refs config acknowledgment + LSP warnings\n- Replace --cascade with dbt-style +model/model+ selector syntax\n- Define three execution modes: run, backbuild, range run\n- Backbuild walks DAG backwards expanding ranges per temporal deps\n- Phase 5 (state tracking) marked as optional\n- Custom granularity extension point via plugin API (future)\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n* Update docs: codebase discoveries, Phase 6 status, DESIGN.md clarifications\n\n- Plan doc: document existing selector syntax and graph traversal infrastructure,\n  add line references to Key Files table, note Phase 4 can reuse existing code\n- ROADMAP.md: mark Phase 6 as partially complete, cross-reference Phase 9,\n  link to incremental plan for advanced features\n- DESIGN.md: clarify that @materialize annotation syntax is not implemented\n  (YAML frontmatter is current config surface), note lookback_days superseded\n  by AST-inferred temporal dependencies + per-column data_latency\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-22T20:21:24+11:00",
          "tree_id": "aeadcda5601e96df569df900b48bc7b4e416f7e6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0cfb16e653b283885ef6022ef720a464f5fa1acf"
        },
        "date": 1774171377569,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.242849846635913,
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
          "id": "c86c519da77225b7657e3b1f2aa4f31aa4cb8de4",
          "message": "Merge pull request #85 from adbrowne/incremental-model-support\n\nComprehensive Incremental Model Support",
          "timestamp": "2026-03-24T18:47:19+11:00",
          "tree_id": "a7f387afc31dd903fc6038211f86d009bed71141",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c86c519da77225b7657e3b1f2aa4f31aa4cb8de4"
        },
        "date": 1774338569782,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.973394607956067,
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
          "id": "1260d658cbdd703fcd7d6c18908e0ef7a8faadc2",
          "message": "Merge branch 'incremental-model-support'",
          "timestamp": "2026-03-24T18:49:29+11:00",
          "tree_id": "080203ea5cc60f0c1f3f180d29d2943925fb8728",
          "url": "https://github.com/adbrowne/smelt-sql/commit/1260d658cbdd703fcd7d6c18908e0ef7a8faadc2"
        },
        "date": 1774338643391,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.29040692367912,
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
          "id": "a03ea28ef1eea8703b530f42297b5da76fd38325",
          "message": "Phase 3: Run Planner — interactive run preview UI\n\nBackend:\n- POST /api/run/plan endpoint accepts time range, batch size, per-partition\n  flag, and model selection\n- build_run_plan() computes execution plan using batch safety analysis\n  from smelt-optimizer, generates batches per model\n- Returns models with batch counts, safety levels, and per-batch ranges\n\nFrontend:\n- Run Planner page with date range inputs, batch size override,\n  per-partition toggle, and model selector (click to toggle)\n- Preview button triggers plan computation\n- Plan table shows models with type, safety badge, batch count, range\n- Expandable rows show individual batch read/write ranges\n- Navigation tabs in header: Graph | Run Planner\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:36:44+11:00",
          "tree_id": "c2d6c604cfece006f47ecfe0cd714483ac9d4214",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a03ea28ef1eea8703b530f42297b5da76fd38325"
        },
        "date": 1774341538155,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.968798737708408,
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
          "id": "bc72334eead61c6fb748054db7a2a640e4ca31f5",
          "message": "Add UI dashboard expansion plan to docs/plans\n\nDocuments completed phases 1-3 and remaining phases 4-6.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:42:48+11:00",
          "tree_id": "75b2367ab709edb9cf18d26a5ddc65bf96780eb5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/bc72334eead61c6fb748054db7a2a640e4ca31f5"
        },
        "date": 1774341868495,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.00430379498324,
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
          "id": "3eee0c8f2dd76204fca7cd252175fd1611f9da9f",
          "message": "Register SIGN() Spark type divergences in property tests\n\nSpark's SIGN() preserves the input type (DOUBLE→DOUBLE, INTEGER→INTEGER,\netc.) while DuckDB returns TINYINT. smelt infers SmallInt matching DuckDB\nbehavior. Register all numeric input variants as BackendSpecific divergences\nso the Spark property tests pass.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T19:52:36+11:00",
          "tree_id": "5323ce943dd1972fdecaebb958d1d362279eb2ec",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3eee0c8f2dd76204fca7cd252175fd1611f9da9f"
        },
        "date": 1774342969972,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.0296907742068,
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
          "id": "5694783dec427376a5ad51f97609e85a5fd0d1e1",
          "message": "Phase 4: Run Execution + Monitoring from the UI dashboard\n\nRunManager orchestrates model execution in a background tokio task with\nreal-time progress streaming via WebSocket. Supports cancellation between\nbatches and saves run manifests + interval updates on completion.\n\nBackend: RunManager, 5 new API endpoints (execute/cancel/status/history),\nWebSocket run event streaming, RunProgressEvent types.\n\nFrontend: useRunStatus hook, RunProgress component with progress bars,\nRunHistory page, Execute button in RunPlanner, History nav tab.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-24T20:05:16+11:00",
          "tree_id": "ca5cafa61eba3ba96fa1041f749c923b546bb6f5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5694783dec427376a5ad51f97609e85a5fd0d1e1"
        },
        "date": 1774343247760,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.63492932804478,
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
          "id": "437ea51b8229728f04f6ce717598622b1c200f39",
          "message": "Show model function type signature in UI sidebar\n\nAdds the model's (inputs) -> outputs type signature to the detail\nsidebar, rendered just above the SQL block. Uses the existing\nModelFunctionType Display impl from smelt-db.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T08:32:05+11:00",
          "tree_id": "22ea8fcff259a63288aa56d22c6350934823cb3e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/437ea51b8229728f04f6ce717598622b1c200f39"
        },
        "date": 1774388043486,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.960290493722383,
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
          "id": "f42fe4d9c1651ed88917eb6d82fe27166a815cf6",
          "message": "Consolidate all examples under examples/ directory\n\nMove scattered example/test workspaces into a unified structure:\n- examples/timeseries/ (was examples/) — 12 SQL user/event analytics models\n- examples/retail_analytics/ (was benchmarks/retail-analytics/) — 25 TPC-DS models\n- examples/broken/ (new) — 5 intentionally broken models for error testing\n- examples/test_workspace/ (was test-workspace/) — minimal VSCode/LSP testing\n- examples/huge/ (new) — 2000 auto-generated models for stress testing\n\nRefactored smelt-bench model_gen to support persistent output directories\nand added generate_static_workspace binary. Updated all integration test\npaths, documentation references, and .gitignore.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T14:02:19+11:00",
          "tree_id": "6ce62defcf586a61e26d9d722292c006109cddc2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f42fe4d9c1651ed88917eb6d82fe27166a815cf6"
        },
        "date": 1774407845619,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.01152225196382,
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
          "id": "e9462fddaef02c42c838ebbf95ad5ed2dcf94c8b",
          "message": "Run Planner: select/exclude with CLI command preview\n\n- Add --exclude flag to CLI (RunArgs, BuildArgs) with selector syntax\n- Add exclude_models() and all_model_names() to DependencyGraph (both smelt-core and smelt-cli)\n- Use proper selector parsing in UI backend (build_run_plan, run_manager)\n  instead of simple name matching — supports tags, upstream/downstream\n- Add POST /api/resolve endpoint for lightweight selector resolution\n- Generate CLI command string in RunPlanResponse\n- Redesign RunPlanner UI:\n  - Text inputs for select/exclude (space-separated selector syntax)\n  - Model pills that toggle tokens in the text inputs (single source of truth)\n  - Shift+click pills to exclude (red/strikethrough)\n  - Pills highlight resolved models via /api/resolve (including upstream/downstream)\n  - CLI command box with copy button shown after preview\n  - Resolved models list shown after preview",
          "timestamp": "2026-03-25T14:45:43+11:00",
          "tree_id": "f7902d8488647981b91950503bed59a4307abd0f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/e9462fddaef02c42c838ebbf95ad5ed2dcf94c8b"
        },
        "date": 1774410470892,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 26.71907360462335,
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
          "id": "d55da26f01ce22bf874669ddf3ed603ca3046231",
          "message": "Merge pull request #86 from adbrowne/docs3\n\nUpdate outdated documentation",
          "timestamp": "2026-03-25T15:26:11+11:00",
          "tree_id": "ae50ae5b8c4f1576fa08bab122ca37f341b67925",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d55da26f01ce22bf874669ddf3ed603ca3046231"
        },
        "date": 1774412886367,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.300819636550447,
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
          "id": "5f13317efbd935a707ed4b64bc2a7d773389f1b8",
          "message": "Rename smelt-optimizer to smelt-planner\n\nThe crate implements planning (execution strategy, materialization,\nbatching) rather than query optimization. This rename clarifies intent.\n\n- Crate: smelt-optimizer → smelt-planner\n- Struct: Optimizer → Planner, .optimize() → .plan()\n- Python: OptimizerRule → PlannerRule, entry point smelt.planner_rules\n- Docs: updated CLAUDE.md, ROADMAP.md, architecture_overview.md\n- Renamed optimization_rule_api_design.md → planner_rule_api_design.md\n- Test file: optimizer_test.rs → planner_test.rs\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T15:49:32+11:00",
          "tree_id": "e899b17edab3bf7e8e9394518688d85a3c40d093",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5f13317efbd935a707ed4b64bc2a7d773389f1b8"
        },
        "date": 1774414296186,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.25880047265207,
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
          "id": "f31ca08bed91764bcb6fa960814010c7910c270b",
          "message": "Per-model backend routing: route models to different targets in a single run\n\nAdd per-model target assignment so `smelt run` can execute models against\ndifferent backends. Users specify targets via SQL frontmatter (`target: spark_prod`)\nor smelt.yml model config, with precedence: frontmatter > smelt.yml > CLI --target.\n\n- Add `target` field to ModelConfig and ModelMetadata\n- Add Config::get_target() with 3-level precedence resolution\n- Add BackendRegistry (creates backends per-target) and CompilerRegistry (dialect-aware compilation per-target)\n- Add cross-backend ref validation (clear error when model refs span targets)\n- Update run, backbuild, and UI execution loops to use per-model backend/compiler/schema\n- Cross-backend data transfer deferred to future work\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T16:45:29+11:00",
          "tree_id": "52140c850d511500aba3c3ae7de744781577d0b3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f31ca08bed91764bcb6fa960814010c7910c270b"
        },
        "date": 1774417661726,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.965258521718606,
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
          "id": "03964482464c7d25a61b81cc4978955e190275c7",
          "message": "Removed log files",
          "timestamp": "2026-03-25T16:49:14+11:00",
          "tree_id": "69a09012a4e45dc8b7f829996a7db314128ea7c7",
          "url": "https://github.com/adbrowne/smelt-sql/commit/03964482464c7d25a61b81cc4978955e190275c7"
        },
        "date": 1774417844225,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.866527835129165,
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
          "id": "240cfe7659c18082fa1a14e29ca23072c03b6cb5",
          "message": "Removed script",
          "timestamp": "2026-03-25T16:49:33+11:00",
          "tree_id": "5645c5b1ad398d0b798135ac8579c9ce4160a537",
          "url": "https://github.com/adbrowne/smelt-sql/commit/240cfe7659c18082fa1a14e29ca23072c03b6cb5"
        },
        "date": 1774417898742,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.833425032763063,
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
          "id": "a19feda9e0199170d1fdd92f5ae6f28c29d8d6b2",
          "message": "Fix CI: clippy field_reassign_with_default and DuckDB manylinux CXX ABI\n\n- Use struct initializer syntax instead of field reassignment in config test\n- Set DUCKDB_PLATFORM env var in manylinux Docker containers to fix\n  DuckDB bundled build failing on legacy CXX ABI\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:09:53+11:00",
          "tree_id": "c9d73afbdf5a91c0511118e1a07c8e69fc8bbcf5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a19feda9e0199170d1fdd92f5ae6f28c29d8d6b2"
        },
        "date": 1774422673379,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 26.26399447262191,
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
          "id": "9f59cafcb648fba442c5c0a7dc3802c064fb1fb7",
          "message": "Fix Linux wheel builds: use CXXFLAGS define for DuckDB legacy ABI\n\nDUCKDB_EXPLICIT_PLATFORM must be a C++ preprocessor define, not a shell\nenv var. Pass it via CXXFLAGS which the cc crate reads for C++ builds.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:30:36+11:00",
          "tree_id": "935a2f616c50fe61076ff905b7d4e987dac112f6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/9f59cafcb648fba442c5c0a7dc3802c064fb1fb7"
        },
        "date": 1774423921175,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.734717867071797,
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
          "id": "ae1dda244abeae11b8d5a26b01506996ded2488a",
          "message": "Fix Linux wheel builds: use DUCKDB_CUSTOM_PLATFORM to bypass ABI check\n\nThe DuckDB platform.hpp #error for legacy CXX ABI cannot be bypassed\nwith DUCKDB_EXPLICIT_PLATFORM (it's only mentioned in the error text,\nnot actually checked). Use DUCKDB_CUSTOM_PLATFORM which short-circuits\nthe entire platform detection function.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T18:49:47+11:00",
          "tree_id": "d51b5faec57f727ae7388189f009898f4ebb5248",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ae1dda244abeae11b8d5a26b01506996ded2488a"
        },
        "date": 1774425102199,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.823271211408713,
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
          "id": "ea7b6d023d657bc07f752289ec61f832c9458057",
          "message": "Switch dev release publishing from PyPI to TestPyPI\n\nDev releases should publish to TestPyPI. Production PyPI publishing\nremains in the release.yml workflow for tagged releases.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T19:42:39+11:00",
          "tree_id": "77f895258209f1d39fff678050edb449665f64a9",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ea7b6d023d657bc07f752289ec61f832c9458057"
        },
        "date": 1774428237720,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.296398554340833,
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
          "id": "3bc48925032f80402a9d6a9ba95486aa27496c29",
          "message": "Merge branch 'worktree-client-feedback'",
          "timestamp": "2026-03-25T20:51:19+11:00",
          "tree_id": "025a59fa478ef6104fe35ebd715345277255521a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3bc48925032f80402a9d6a9ba95486aa27496c29"
        },
        "date": 1774432391322,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.56260836728924,
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
          "id": "01bd9226154c0a3dc725c5e5e4b18566f3a1b5d1",
          "message": "Fix formatting in discovery.rs tests\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T20:57:57+11:00",
          "tree_id": "561edc0f8bee1d210da69974f8c4ca020cca5b3c",
          "url": "https://github.com/adbrowne/smelt-sql/commit/01bd9226154c0a3dc725c5e5e4b18566f3a1b5d1"
        },
        "date": 1774432781208,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 26.582257663820048,
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
          "id": "162cca5d729f276b07c2570b02b31b06f538e831",
          "message": "Add plan for logical-to-physical graph architecture\n\nIntroduces a two-stage graph design separating user-authored models\n(logical graph) from the execution plan (physical graph). The physical\ngraph removes ephemeral nodes, adds planner-created intermediates as\nfirst-class nodes, and carries concrete execution strategies. This\ngives planner rule authors a clear contract and follows patterns from\nDataFusion, Spark Catalyst, and Apache Calcite.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-25T21:39:01+11:00",
          "tree_id": "5b1ee3e27ffce2c58c250fdabb6b6a8a64a71beb",
          "url": "https://github.com/adbrowne/smelt-sql/commit/162cca5d729f276b07c2570b02b31b06f538e831"
        },
        "date": 1774435316026,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.49758649280813,
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
          "id": "fff2a57b46b99083d668070db8343a09650a3298",
          "message": "Update materialization plan with completed phases and remaining integration work",
          "timestamp": "2026-03-26T08:26:44+11:00",
          "tree_id": "d9550a2a344e068319e85125711fcb0e5cf1d237",
          "url": "https://github.com/adbrowne/smelt-sql/commit/fff2a57b46b99083d668070db8343a09650a3298"
        },
        "date": 1774474076989,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.32513554766647,
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
          "distinct": false,
          "id": "4812ff3c405ebb50ad7a2257ca6ab45c650272af",
          "message": "Wire ephemeral models into CLI execution loop with example\n\n- Build EphemeralResolver per target and use compile_with_ephemerals for\n  all model compilation in run() and backbuild() loops\n- Skip ephemeral models during execution (print info, continue)\n- Validate materialization configs at startup (ephemeral+incremental etc)\n- Warn on unused ephemeral models with no downstream consumers\n- Error when --select directly targets an ephemeral model\n- Add compile_with_sql_and_ephemerals for incremental code path\n- Fix type-cast column name inference to use Expr::infer_name() instead\n  of \"?\" placeholder for bare column references\n- Add examples/ephemeral_demo/ demonstrating CTE inlining\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T18:32:57+11:00",
          "tree_id": "3c494311abe186f68ca089a72ab66e65203e56eb",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4812ff3c405ebb50ad7a2257ca6ab45c650272af"
        },
        "date": 1774510516403,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.888433264176953,
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
          "id": "3a7c82f5fd6f4c668143120c4f76b76335c42872",
          "message": "Update graph stages plan with landed ephemeral support\n\nRefresh the logical-to-physical graph plan now that ephemeral model\nsupport (Materialization::Ephemeral, EphemeralResolver, CTE inlining)\nhas landed on main. Key updates: reference existing EphemeralResolver\nfor reuse, add CreateMaterializedView strategy, note which validations\nalready exist, and detail the graph consolidation (delete smelt-cli's\nduplicate DependencyGraph).\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T18:51:57+11:00",
          "tree_id": "e1e9ef52ba4e6dec28f7d687e51b8d3f3c26b664",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3a7c82f5fd6f4c668143120c4f76b76335c42872"
        },
        "date": 1774511610403,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.155576888209016,
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
          "distinct": false,
          "id": "8efaff8240a6abc9c3e774766fe18664eec924be",
          "message": "Add physical graph section to smelt explain output (Phase D)\n\nsmelt explain now runs the planner and shows the physical execution plan\nalongside the logical graph — strategies, ephemerals, and planner\noptimizations are visible without connecting to any database.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T20:49:25+11:00",
          "tree_id": "ceeb10ab3ed6b4b878117299c87f6eb27489ed90",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8efaff8240a6abc9c3e774766fe18664eec924be"
        },
        "date": 1774518691061,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.323430729929783,
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
          "distinct": false,
          "id": "296e5afeda5721c8ab3d91e760f4e38fd179b21b",
          "message": "Remove smelt-cli DependencyGraph, use PhysicalGraph in backbuild\n\n- Delete crates/smelt-cli/src/graph.rs (duplicate of smelt-core's)\n- Migrate python.rs tests and integration tests to LogicalGraph\n- backbuild() now uses PhysicalGraph for ephemeral resolver ownership\n  instead of manually constructing resolvers\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T20:57:53+11:00",
          "tree_id": "e5e084d5277fe45c2fb635acc11ad82c6a399dff",
          "url": "https://github.com/adbrowne/smelt-sql/commit/296e5afeda5721c8ab3d91e760f4e38fd179b21b"
        },
        "date": 1774519167083,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.76809544675935,
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
          "distinct": false,
          "id": "132425beac5a4f387d9d6c678d2e52e822946048",
          "message": "Update graph stages plan: all deferred items resolved\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T21:04:04+11:00",
          "tree_id": "a3c218b5195498542168b4f1b44182f13cd8b8b8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/132425beac5a4f387d9d6c678d2e52e822946048"
        },
        "date": 1774519578019,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.758207997323982,
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
          "id": "5186d00289b577d19744d3163b7bede7f8e3b353",
          "message": "Add comprehensive multi-perspective codebase review report\n\nReview from 10 professional viewpoints (dbt user, Director of Engineering,\nSQLMesh user, Data Architect, Analytics Engineer, Spark Engineer, Data Analyst,\nRust Architect, Rust Developer, Python Developer) with verdicts, evidence-backed\nanalysis, cross-cutting themes, and prioritized recommendations.\n\nCo-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-03-26T21:31:17+11:00",
          "tree_id": "0a66137a398cf4a2ce73c9faa29281c696edcd4a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5186d00289b577d19744d3163b7bede7f8e3b353"
        },
        "date": 1774521156824,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 27.14897981373263,
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
          "id": "cb698145f06e5b118bc709f1d3e623fff3d6b9f9",
          "message": "Merge branch 'worktree-docs'",
          "timestamp": "2026-03-26T21:39:53+11:00",
          "tree_id": "548ab8b8c25dfbbba9d82d814d34b6cc4a761be3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb698145f06e5b118bc709f1d3e623fff3d6b9f9"
        },
        "date": 1774521695744,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 26.706256323070754,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}