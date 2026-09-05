window.BENCHMARK_DATA = {
  "lastUpdate": 1788652518729,
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
          "id": "d2eee320aff61a8ccfde70b07ddd355dd2e85970",
          "message": "fix(ci): scope the property-diff comment lookup to the PR's own comments\n\nThe lookup listed `repos/{owner}/{repo}/issues/comments` — the\nrepository-wide issue-comment feed — and took `| last`. That resolves to\nwhichever PR was commented on most recently across the whole repo, so the\nfirst PR the job ever commented on becomes the permanent PATCH target:\nevery later PR renders its own diff correctly and then overwrites that one\ncomment, while its own PR shows nothing.\n\nCaught live by run 33997596604: PR #191's property diff was PATCHed onto\nPR #188.\n\nFixed in the workflow and in the documented job users copy\n(docs-site/docs/guide/ci.md), with a text gate over both and a spec\nsentence saying the marker identifies which of a PR's comments is the\ndiff's, not which PR a repo-wide listing belongs to.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T09:36:04+10:00",
          "tree_id": "e6a75344c05ec1ed3395f8de5a762b699e9d306a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d2eee320aff61a8ccfde70b07ddd355dd2e85970"
        },
        "date": 1788651590091,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.24815400000001,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.083209,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.8458199999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.642631,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.378998,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1199.214449,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.244752,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.690166,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.534465,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.740553,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 991.861129,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.44171,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.690439999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.891874,
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
          "id": "eaf7065f2ec56bfb0a1793628ea441f640d608d1",
          "message": "fix(runtime): include child PID in Python model error messages (#189)\n\nThe subprocess path's error message for a failed Python model\n(`run_python_model`) previously named only the model file. Issue #189\nreported one occurrence where a test's panic showed an error context\nfor one temp-dir model file paired with a traceback that referenced a\ndifferent (concurrently running) test's model file — a misattributed\nerror that would otherwise be nearly impossible to diagnose.\n\nRuled out via code review: file_path and stderr are both local to a\nsingle run_python_model call, each child gets its own OS pipe, and\nnone of find_python_sdk/build_pythonpath/setup_sdk share state across\ncalls — so cross-contamination inside this function is not currently\nexplainable from the code. Attempted to reproduce under heavy\nconcurrent load (~40k test executions across parallel process/thread\ncombinations) without success.\n\nSince the root cause remains unconfirmed, add the child's PID to every\nerror message this function can produce. If the misattribution recurs,\nthe PID immediately tells us whether it's genuinely a different child\n(pointing at an OS/std process-spawn bug) or something else entirely.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T09:50:37+10:00",
          "tree_id": "040b1521dbbe5e7b4bfe915665d7d2a62a2e4936",
          "url": "https://github.com/adbrowne/smelt-sql/commit/eaf7065f2ec56bfb0a1793628ea441f640d608d1"
        },
        "date": 1788652512731,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.807389,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 58.371524,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.095712,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.658058,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.346503,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1207.890116,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.446329,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.551054,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.250117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.679439,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 999.754893,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.68336,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.118249999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.964776,
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
          "id": "d2eee320aff61a8ccfde70b07ddd355dd2e85970",
          "message": "fix(ci): scope the property-diff comment lookup to the PR's own comments\n\nThe lookup listed `repos/{owner}/{repo}/issues/comments` — the\nrepository-wide issue-comment feed — and took `| last`. That resolves to\nwhichever PR was commented on most recently across the whole repo, so the\nfirst PR the job ever commented on becomes the permanent PATCH target:\nevery later PR renders its own diff correctly and then overwrites that one\ncomment, while its own PR shows nothing.\n\nCaught live by run 33997596604: PR #191's property diff was PATCHed onto\nPR #188.\n\nFixed in the workflow and in the documented job users copy\n(docs-site/docs/guide/ci.md), with a text gate over both and a spec\nsentence saying the marker identifies which of a PR's comments is the\ndiff's, not which PR a repo-wide listing belongs to.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T09:36:04+10:00",
          "tree_id": "e6a75344c05ec1ed3395f8de5a762b699e9d306a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d2eee320aff61a8ccfde70b07ddd355dd2e85970"
        },
        "date": 1788651594546,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.814938574881978,
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
          "id": "eaf7065f2ec56bfb0a1793628ea441f640d608d1",
          "message": "fix(runtime): include child PID in Python model error messages (#189)\n\nThe subprocess path's error message for a failed Python model\n(`run_python_model`) previously named only the model file. Issue #189\nreported one occurrence where a test's panic showed an error context\nfor one temp-dir model file paired with a traceback that referenced a\ndifferent (concurrently running) test's model file — a misattributed\nerror that would otherwise be nearly impossible to diagnose.\n\nRuled out via code review: file_path and stderr are both local to a\nsingle run_python_model call, each child gets its own OS pipe, and\nnone of find_python_sdk/build_pythonpath/setup_sdk share state across\ncalls — so cross-contamination inside this function is not currently\nexplainable from the code. Attempted to reproduce under heavy\nconcurrent load (~40k test executions across parallel process/thread\ncombinations) without success.\n\nSince the root cause remains unconfirmed, add the child's PID to every\nerror message this function can produce. If the misattribution recurs,\nthe PID immediately tells us whether it's genuinely a different child\n(pointing at an OS/std process-spawn bug) or something else entirely.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T09:50:37+10:00",
          "tree_id": "040b1521dbbe5e7b4bfe915665d7d2a62a2e4936",
          "url": "https://github.com/adbrowne/smelt-sql/commit/eaf7065f2ec56bfb0a1793628ea441f640d608d1"
        },
        "date": 1788652517388,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.685394166007388,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}