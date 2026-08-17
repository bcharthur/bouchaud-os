#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse

BODY = b"BOUCHAUD_HEALTH_HTTP_OK\n"

M9_HTML = b"""<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Bouchaud M9</title>
<style>
html,body{margin:0;background:#0b1220;color:#eef6ff;font-family:sans-serif}
main{padding:56px}
h1{font-size:42px;margin:0 0 24px;color:#79b8ff}
.card{max-width:760px;padding:28px;border:2px solid #31557f;background:#14243a}
strong{color:#ffd166}
code{color:#8be9fd}
</style>
</head>
<body>
<main>
<div class="card">
<h1>Ladybird M9 sur Bouchaud OS</h1>
<p>Cette page est chargee par <strong>RequestServer</strong> via HTTP.</p>
<p>Chemin: <code>WebContent -&gt; RequestServer -&gt; Bouchaud TCP/IP</code></p>
<p><strong>BOUCHAUD_M9_HTTP_OK</strong></p>
</div>
</main>
</body>
</html>
"""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if self.path == "/health":
            body = BODY
            content_type = "text/plain; charset=utf-8"
            self.send_response(200)
        elif self.path == "/m9.html":
            body = M9_HTML
            content_type = "text/html; charset=utf-8"
            self.send_response(200)
            print("[health-fixture] M9_FIXTURE_HTTP_OK path=/m9.html", flush=True)
        else:
            body = b"not found\n"
            content_type = "text/plain; charset=utf-8"
            self.send_response(404)

        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        print("[health-fixture] " + (fmt % args), flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=18080)
    args = parser.parse_args()

    server = ThreadingHTTPServer(("0.0.0.0", args.port), Handler)
    print(f"[health-fixture] listening on 0.0.0.0:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
