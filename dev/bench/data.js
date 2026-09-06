window.BENCHMARK_DATA = {
  "lastUpdate": 1788683588413,
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
          "id": "81b5abb96029ebfe3af69398fdd182e0481c54e1",
          "message": "Merge pull request #196 from adbrowne/worktree-build-speed\n\nbuild: cap cargo parallelism, add missing CI gates, and measure the verify suite",
          "timestamp": "2026-09-06T18:28:36+10:00",
          "tree_id": "b02ac9a11b74da313092d7aaa7d8a6cb5d70915f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/81b5abb96029ebfe3af69398fdd182e0481c54e1"
        },
        "date": 1788683581322,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 48.096689,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 46.170261999999994,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.850686,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.546851,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.265734,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1178.08591,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.144719,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.415621,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.14268,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.7472989999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 999.343733,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.30893,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 36.406189999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.355538,
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
          "id": "81b5abb96029ebfe3af69398fdd182e0481c54e1",
          "message": "Merge pull request #196 from adbrowne/worktree-build-speed\n\nbuild: cap cargo parallelism, add missing CI gates, and measure the verify suite",
          "timestamp": "2026-09-06T18:28:36+10:00",
          "tree_id": "b02ac9a11b74da313092d7aaa7d8a6cb5d70915f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/81b5abb96029ebfe3af69398fdd182e0481c54e1"
        },
        "date": 1788683586939,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.01345041892543,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}