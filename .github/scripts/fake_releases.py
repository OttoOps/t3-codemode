import functools
import http.server
import sys

port, root, tag = int(sys.argv[1]), sys.argv[2], sys.argv[3]


class Releases(http.server.SimpleHTTPRequestHandler):
    def route(self) -> bool:
        if self.path == "/latest":
            self.send_response(302)
            self.send_header("Location", f"/tag/{tag}")
        elif self.path == f"/tag/{tag}":
            self.send_response(200)
        else:
            return False
        self.send_header("Content-Length", "0")
        self.end_headers()
        return True

    def do_GET(self) -> None:
        if not self.route():
            super().do_GET()

    def do_HEAD(self) -> None:
        if not self.route():
            super().do_HEAD()


handler = functools.partial(Releases, directory=root)
http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
