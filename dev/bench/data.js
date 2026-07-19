window.BENCHMARK_DATA = {
  "lastUpdate": 1784460976936,
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
          "id": "0be59d08d2d33e751b91e9a8e97813d2081aa240",
          "message": "Merge pull request #164 from adbrowne/worktree-roadmap_todo\n\nQuality grind: parser ledger fixes, comma-joins, generator coverage, planner/logical consolidation",
          "timestamp": "2026-07-19T21:33:26+10:00",
          "tree_id": "7f58f8898b057ce24193b443463bef8d7d3e0828",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0be59d08d2d33e751b91e9a8e97813d2081aa240"
        },
        "date": 1784460973373,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 32.818811000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.43821,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.60023,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.402502,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.186252,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 662.708415,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 1.839295,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.607174,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.53117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.495035,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 571.203614,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.05394,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 23.2612,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.129408,
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
          "id": "0be59d08d2d33e751b91e9a8e97813d2081aa240",
          "message": "Merge pull request #164 from adbrowne/worktree-roadmap_todo\n\nQuality grind: parser ledger fixes, comma-joins, generator coverage, planner/logical consolidation",
          "timestamp": "2026-07-19T21:33:26+10:00",
          "tree_id": "7f58f8898b057ce24193b443463bef8d7d3e0828",
          "url": "https://github.com/adbrowne/smelt-sql/commit/0be59d08d2d33e751b91e9a8e97813d2081aa240"
        },
        "date": 1784460976338,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 34.032196155984636,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}