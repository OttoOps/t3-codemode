#!/usr/bin/env python3
"""Optional zero-dependency local live bridge for docs/design/board-dispatch.html.

Proxies read-only GET endpoints (/api/state, /api/flow, /api/usage-stats, /api/msg/*)
from the running t3cm board (http://127.0.0.1:47731) so the prototype updates live
without modifying t3cm's CORS policy.

Run:
    python3 docs/design/board-dispatch-live.py
Then open:
    http://127.0.0.1:47738/
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

HOST = os.environ.get("T3CM_PROTO_HOST", "127.0.0.1")
PORT = int(os.environ.get("T3CM_PROTO_PORT", "47738"))
UPSTREAM = os.environ.get("T3CM_BOARD_ORIGIN", "http://127.0.0.1:47731").rstrip("/")
HTML_PATH = Path(__file__).resolve().parent / "board-dispatch.html"
ALLOWED_PREFIXES = ("/api/state", "/api/flow", "/api/usage-stats", "/api/msg/")
ALLOWED_HOSTS = {f"{HOST}:{PORT}", f"127.0.0.1:{PORT}", f"localhost:{PORT}", f"[::1]:{PORT}"}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        return

    def _send(self, code: int, content_type: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.headers.get("Host", "").lower() not in ALLOWED_HOSTS:
            self._send(403, "text/plain; charset=utf-8", b"Forbidden host")
            return
        if self.path in ("/", "/index.html", "/board-dispatch.html"):
            self._send(200, "text/html; charset=utf-8", HTML_PATH.read_bytes())
            return
        if any(self.path.startswith(p) for p in ALLOWED_PREFIXES):
            url = f"{UPSTREAM}{self.path}"
            try:
                req = urllib.request.Request(url, headers={"Accept": "application/json"})
                with urllib.request.urlopen(req, timeout=4.0) as resp:
                    self._send(resp.status, "application/json; charset=utf-8", resp.read())
            except urllib.error.HTTPError as err:
                self._send(err.code, "application/json; charset=utf-8", err.read())
            except Exception as exc:
                payload = json.dumps({"error": f"Cannot reach t3cm at {UPSTREAM}: {exc}"}).encode("utf-8")
                self._send(502, "application/json; charset=utf-8", payload)
            return
        self._send(404, "text/plain; charset=utf-8", b"Not found")


def main() -> None:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"t3 board Dispatch prototype (live bridge -> {UPSTREAM}): http://{HOST}:{PORT}/")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
