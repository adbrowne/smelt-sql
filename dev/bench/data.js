window.BENCHMARK_DATA = {
  "lastUpdate": 1784883535836,
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
          "id": "2a02b22e4fa4c562649f023bf994742fc0cbfe52",
          "message": "Merge pull request #165 from adbrowne/worktree-production\n\nProduction readiness (v0.5): W1 fail-loud, W2 operability, W4 Spark first-class, W3 adoption (tracking PR)",
          "timestamp": "2026-07-21T06:16:45+10:00",
          "tree_id": "b37ea82516d2ac5f2958d976a03348a82731de80",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2a02b22e4fa4c562649f023bf994742fc0cbfe52"
        },
        "date": 1784578883526,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.13637,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.997726,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.8524719999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6061719999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.380451,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1012.303484,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.534461,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.912159,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.690816,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.68548,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 816.959828,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.20249,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.299589999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.835038,
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
          "id": "4c0ed72ca7129b7aa3661358dc04e888c7538d06",
          "message": "ci(compat): fix scheduled-run failure in Detect Spark-relevant changes job\n\nOn schedule events github.event.before is unset, so the base-ref\nfallback resolved to the literal string 'HEAD~1', which paths-filter\ntried to fetch as a remote refspec and failed with exit 128. Skip\ncheckout/filter on schedule entirely — the downstream spark jobs\nalready run unconditionally when github.event_name == 'schedule'.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-24T18:33:56+10:00",
          "tree_id": "bac9dc6a26d428967104f69caec21456c5e158ba",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4c0ed72ca7129b7aa3661358dc04e888c7538d06"
        },
        "date": 1784882250335,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 62.300918,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 59.899211,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.076356,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.634202,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.371974,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1010.797926,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.463423,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.321415,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.195441,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.657225,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 820.950169,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.93015,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.06477,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.917934,
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
          "id": "bc8a0b04a0fd0b72986797559d001c009ee0649a",
          "message": "ci(vscode): bump CI Node.js from 20 to 24 (current LTS)\n\nactions/setup-node was pinned to Node 20 in test.yml/release.yml/\ndev-release.yml, which is now deprecated on GitHub-hosted runners.\nBump to 24 (current Active LTS) and update @types/node to match.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-24T18:55:43+10:00",
          "tree_id": "83c5c85855786e5e76f2737633ecff44780b89af",
          "url": "https://github.com/adbrowne/smelt-sql/commit/bc8a0b04a0fd0b72986797559d001c009ee0649a"
        },
        "date": 1784883533208,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 61.252291,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 59.031595,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.954271,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.61849,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.345979,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1012.673578,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.631694,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 3.0806,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.843014,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.700714,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 827.918842,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.82749,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.46228,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.890211,
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
          "id": "2a02b22e4fa4c562649f023bf994742fc0cbfe52",
          "message": "Merge pull request #165 from adbrowne/worktree-production\n\nProduction readiness (v0.5): W1 fail-loud, W2 operability, W4 Spark first-class, W3 adoption (tracking PR)",
          "timestamp": "2026-07-21T06:16:45+10:00",
          "tree_id": "b37ea82516d2ac5f2958d976a03348a82731de80",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2a02b22e4fa4c562649f023bf994742fc0cbfe52"
        },
        "date": 1784578887245,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.916881326961303,
            "unit": "MB/s"
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
          "id": "4c0ed72ca7129b7aa3661358dc04e888c7538d06",
          "message": "ci(compat): fix scheduled-run failure in Detect Spark-relevant changes job\n\nOn schedule events github.event.before is unset, so the base-ref\nfallback resolved to the literal string 'HEAD~1', which paths-filter\ntried to fetch as a remote refspec and failed with exit 128. Skip\ncheckout/filter on schedule entirely — the downstream spark jobs\nalready run unconditionally when github.event_name == 'schedule'.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-24T18:33:56+10:00",
          "tree_id": "bac9dc6a26d428967104f69caec21456c5e158ba",
          "url": "https://github.com/adbrowne/smelt-sql/commit/4c0ed72ca7129b7aa3661358dc04e888c7538d06"
        },
        "date": 1784882254229,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.76847497624288,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}