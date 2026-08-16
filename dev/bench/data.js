window.BENCHMARK_DATA = {
  "lastUpdate": 1786843604788,
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
          "id": "ae63cc4f62cf14988a5b3cd8d9c7c6699fa79d2d",
          "message": "research: record decisions in open-questions triage (item 1 posture-derived deletion)\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-16T11:23:18+10:00",
          "tree_id": "d373ebb8305e3167acdfd291126df0e93507a29f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ae63cc4f62cf14988a5b3cd8d9c7c6699fa79d2d"
        },
        "date": 1786843569560,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 32.56986,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.174759,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.61802,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.407309,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.180581,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 752.292724,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.783748,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.934763,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.674681,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.541936,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 644.7087849999999,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.22832,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 23.58823,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.146232,
            "unit": "ms"
          }
        ]
      },
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
          "id": "f3de2cc5ab395a806beffd4f27b03a5166129655",
          "message": "research: item 7 decision — equivalence scoped to deterministic outputs, NOW() runs as-is\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-16T11:24:02+10:00",
          "tree_id": "1ce26670220d6f4dfe95bb92923bf675c2113927",
          "url": "https://github.com/adbrowne/smelt-sql/commit/f3de2cc5ab395a806beffd4f27b03a5166129655"
        },
        "date": 1786843602933,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 57.618972,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 55.20107,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.143605,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.629998,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.33878,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1120.345256,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.863423,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.175836,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.182686,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.7223040000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 923.805507,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.998239999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.66353,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.757972,
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
          "id": "ae63cc4f62cf14988a5b3cd8d9c7c6699fa79d2d",
          "message": "research: record decisions in open-questions triage (item 1 posture-derived deletion)\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-16T11:23:18+10:00",
          "tree_id": "d373ebb8305e3167acdfd291126df0e93507a29f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ae63cc4f62cf14988a5b3cd8d9c7c6699fa79d2d"
        },
        "date": 1786843573223,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 33.97576558470179,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}