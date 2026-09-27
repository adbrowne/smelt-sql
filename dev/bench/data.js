window.BENCHMARK_DATA = {
  "lastUpdate": 1790478562691,
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
          "id": "4c535e08075e127ad3690a3d7ed62ad6e8562573",
          "message": "docs(maes-words): handoff for pictogram bank + sight-word activity\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-27T13:05:24+10:00",
          "tree_id": "ef9a882a32185e2bc8a52a5035b1435313fdc353",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4c535e08075e127ad3690a3d7ed62ad6e8562573"
        },
        "date": 1790478555751,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.03628,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.968945,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.829727,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.640695,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.296012,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1204.073843,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.374444,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.8341450000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.675751,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.713371,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 986.399369,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.275880000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.516909999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.883704,
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
          "id": "4c535e08075e127ad3690a3d7ed62ad6e8562573",
          "message": "docs(maes-words): handoff for pictogram bank + sight-word activity\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-27T13:05:24+10:00",
          "tree_id": "ef9a882a32185e2bc8a52a5035b1435313fdc353",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4c535e08075e127ad3690a3d7ed62ad6e8562573"
        },
        "date": 1790478561353,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.82954116567164,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}