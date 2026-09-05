window.BENCHMARK_DATA = {
  "lastUpdate": 1788572536301,
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
          "id": "128edb54a48d117f03175a95c863218683c32d89",
          "message": "docs(research): ten candidate directions for smelt, recommend explain-the-diff",
          "timestamp": "2026-09-05T11:37:35+10:00",
          "tree_id": "18800dcaf68f96b64361b13261c7d7cc40e4e0a2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/128edb54a48d117f03175a95c863218683c32d89"
        },
        "date": 1788572530599,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.443615,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.152225,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.04407,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.598318,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.33496600000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1156.012306,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.358333,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.184801,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.18412,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.6912389999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 962.420577,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.84981,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.80025,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.707459,
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
          "id": "128edb54a48d117f03175a95c863218683c32d89",
          "message": "docs(research): ten candidate directions for smelt, recommend explain-the-diff",
          "timestamp": "2026-09-05T11:37:35+10:00",
          "tree_id": "18800dcaf68f96b64361b13261c7d7cc40e4e0a2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/128edb54a48d117f03175a95c863218683c32d89"
        },
        "date": 1788572535052,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.148789429171373,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}