window.BENCHMARK_DATA = {
  "lastUpdate": 1784427388378,
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
          "id": "f8acdd637c95f24eabea51c6a537705442d1b05a",
          "message": "research: production-release review — state, blockers, v0.5 shape, blog series\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-19T12:05:52+10:00",
          "tree_id": "02e1fd505c7637ab8568c67c4daf79b74f942a47",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f8acdd637c95f24eabea51c6a537705442d1b05a"
        },
        "date": 1784427387163,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.387392,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.11792,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.025289,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.621701,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.31109,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 927.941552,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.791195,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.196504,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.148372,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.712746,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 762.958497,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.23553,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.25466,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.922442,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}