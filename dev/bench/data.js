window.BENCHMARK_DATA = {
  "lastUpdate": 1773545927565,
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
      }
    ]
  }
}