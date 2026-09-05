window.BENCHMARK_DATA = {
  "lastUpdate": 1788568129571,
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
          "id": "292954ccaabd90f18727150dfd32b9f4cf641ae2",
          "message": "docs: self-directed scheduler research; fix stale orchestration/state citations in roadmap, cli.md, run_state.md\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-05T10:22:49+10:00",
          "tree_id": "f37e946bb669286dbe556ba7a05b0cc2287ea6b0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/292954ccaabd90f18727150dfd32b9f4cf641ae2"
        },
        "date": 1788568126272,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 49.589513,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 47.386320000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.0590529999999998,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.566969,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.290305,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1182.867988,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.912291,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 3.578808,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 3.371734,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 1.202883,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1040.66709,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.10023,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.83287,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.682705,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}