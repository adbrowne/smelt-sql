window.BENCHMARK_DATA = {
  "lastUpdate": 1788647642035,
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
          "id": "4d51727a76fad522b8b00a51e1fe97b535111cb8",
          "message": "Merge pull request #188 from adbrowne/outcome-20260905-property-diff\n\nProperty diff — explain what a model edit did to smelt's proofs",
          "timestamp": "2026-09-06T08:29:06+10:00",
          "tree_id": "6ca7ceab50cf344ff027b3f076e6d897a7965a93",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4d51727a76fad522b8b00a51e1fe97b535111cb8"
        },
        "date": 1788647637229,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.991588,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.517222999999994,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.21801,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.624407,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.28876900000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1193.742021,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.772795,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.456179,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.2285310000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.7455470000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 996.426288,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.88124,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.75614,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.794521,
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
          "id": "4d51727a76fad522b8b00a51e1fe97b535111cb8",
          "message": "Merge pull request #188 from adbrowne/outcome-20260905-property-diff\n\nProperty diff — explain what a model edit did to smelt's proofs",
          "timestamp": "2026-09-06T08:29:06+10:00",
          "tree_id": "6ca7ceab50cf344ff027b3f076e6d897a7965a93",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4d51727a76fad522b8b00a51e1fe97b535111cb8"
        },
        "date": 1788647640899,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.990066708369216,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}