window.BENCHMARK_DATA = {
  "lastUpdate": 1790550921371,
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
          "id": "0893f780b673bd2f86a5cd61090f434320203258",
          "message": "docs(maes-words): spec — pictogram bank, colour tiles, Fill activity, try-again\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-28T09:10:54+10:00",
          "tree_id": "155e02536ee7a3745e8b2f5932d2a08d4cef47e7",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0893f780b673bd2f86a5cd61090f434320203258"
        },
        "date": 1790550915113,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.374451,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.217032,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.870234,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.620826,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.332303,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1210.681868,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.35147,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.216861,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.1768349999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.6796260000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 993.818329,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.78504,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.67461,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.943461,
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
          "id": "0893f780b673bd2f86a5cd61090f434320203258",
          "message": "docs(maes-words): spec — pictogram bank, colour tiles, Fill activity, try-again\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-28T09:10:54+10:00",
          "tree_id": "155e02536ee7a3745e8b2f5932d2a08d4cef47e7",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0893f780b673bd2f86a5cd61090f434320203258"
        },
        "date": 1790550920338,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.72313007509398,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}