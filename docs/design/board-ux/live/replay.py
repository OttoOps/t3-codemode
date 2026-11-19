import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PAGE = open(sys.argv[1], "rb").read().replace(b"{{CSRF}}", b"replay")
STATE = open(sys.argv[2], "rb").read().replace(b"\n", b"")
PORT = int(sys.argv[3]) if len(sys.argv) > 3 else 47792


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_: object) -> None:
        pass

    def send(self, code: int, kind: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        path = self.path.split("?")[0]
        if path == "/":
            self.send(200, "text/html; charset=utf-8", PAGE)
        elif path == "/api/state":
            self.send(200, "application/json", STATE)
        elif path == "/api/events":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            self.wfile.write(b"event: state\ndata: " + STATE + b"\n\n")
            self.wfile.flush()
            try:
                while True:
                    time.sleep(15)
                    self.wfile.write(b": keepalive\n\n")
                    self.wfile.flush()
            except OSError:
                return
        else:
            self.send(404, "text/plain", b"not found")

    def do_POST(self) -> None:
        self.send(405, "text/plain", b"read-only replay")


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
