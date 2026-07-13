window.BENCHMARK_DATA = {
  "lastUpdate": 1783917112756,
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
          "id": "5e9f1202c0f60cfe3b751acbe28c438c88ef198a",
          "message": "Merge pull request #161 from adbrowne/ci-docs-pr-previews",
          "timestamp": "2026-07-13T14:29:06+10:00",
          "tree_id": "845d19a003f42bc19c4eb303181f35a8a90ca630",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5e9f1202c0f60cfe3b751acbe28c438c88ef198a"
        },
        "date": 1783917108777,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.017878,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.772225000000006,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.925081,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.670014,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.348072,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 926.956722,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.4708270000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.905387,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.7118849999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.678841,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 772.5719389999999,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.63222,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.91768,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.996766,
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
          "id": "5e9f1202c0f60cfe3b751acbe28c438c88ef198a",
          "message": "Merge pull request #161 from adbrowne/ci-docs-pr-previews",
          "timestamp": "2026-07-13T14:29:06+10:00",
          "tree_id": "845d19a003f42bc19c4eb303181f35a8a90ca630",
          "url": "https://github.com/adbrowne/smelt-sql/commit/5e9f1202c0f60cfe3b751acbe28c438c88ef198a"
        },
        "date": 1783917112062,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.628975007512448,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}