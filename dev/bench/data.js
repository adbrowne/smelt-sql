window.BENCHMARK_DATA = {
  "lastUpdate": 1773535065957,
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
      }
    ]
  }
}