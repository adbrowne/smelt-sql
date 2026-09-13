#!/usr/bin/env python3
"""A minimal, stateful stub of the three Databricks REST endpoints
`databricks bundle validate` unconditionally calls, regardless of what a
bundle's own config references: SCIM `Me` (the CLI's `PopulateCurrentUser`
mutator runs on every bundle command) and `workspace/get-status` +
`workspace/mkdirs` (checking/creating the bundle's remote root directory).

Exists so `databricks bundle validate` — criterion 11's per-PR gate,
docs/outcomes/20260912-databricks-dogfood-spine/outcome.md — needs no real
Databricks workspace: measured against the real CLI (v1.16.1), `validate`
calls these three endpoints even for a bundle target with no `host:` in its
own config, so the offline claim requires *something* to answer them, not
merely omitting a real credential.

Binds an ephemeral port on 127.0.0.1, prints it alone on stdout, then serves
until killed. Callers (scripts/dbx-bundle.sh, crates/smelt-cli/tests/
databricks_bundle.rs) read that line to build DATABRICKS_HOST.
"""

import http.server
import json
import socketserver
import sys
from urllib.parse import parse_qs, urlparse

created_dirs = set()


class Handler(http.server.BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urlparse(self.path)
        if "scim/v2/Me" in parsed.path:
            self._send(200, {"userName": "dbx-bundle-stub", "id": "1", "displayName": "stub"})
        elif "workspace/get-status" in parsed.path:
            path = parse_qs(parsed.query).get("path", [""])[0]
            if path in created_dirs:
                self._send(200, {"path": path, "object_type": "DIRECTORY", "object_id": 1})
            else:
                self._send(404, {"error_code": "RESOURCE_DOES_NOT_EXIST", "message": "not found"})
        else:
            self._send(200, {})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length)
        if "workspace/mkdirs" in urlparse(self.path).path:
            try:
                path = json.loads(raw or b"{}").get("path")
            except ValueError:
                path = None
            if path:
                created_dirs.add(path)
        self._send(200, {})

    def log_message(self, *_args):
        pass


def main():
    with socketserver.TCPServer(("127.0.0.1", 0), Handler) as httpd:
        print(httpd.server_address[1], flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    sys.exit(main())
