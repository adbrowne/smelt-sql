window.BENCHMARK_DATA = {
  "lastUpdate": 1790560872848,
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
          "distinct": false,
          "id": "aa2ba8164e435e6bb306fffa16bfe43e33ca3511",
          "message": "fix(maes-words): final review — fits exhaustiveness, frame wording, ambiguous pictograms, guard tests\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-28T11:52:37+10:00",
          "tree_id": "28937c27a8946bb157e3331c64854b89ba139119",
          "url": "https://github.com/adbrowne/smelt-sql/commit/aa2ba8164e435e6bb306fffa16bfe43e33ca3511"
        },
        "date": 1790560870136,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.838511,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 58.181482,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.326476,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.688155,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.315603,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1226.791386,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.986469000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 3.877386,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 4.182488,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.037622,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1007.030775,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.41776,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.10817,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.880783,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}