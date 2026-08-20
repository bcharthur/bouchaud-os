#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer

HTML = r'''<!doctype html>
<meta charset="utf-8">
<title>Bouchaud BrowserHost smoke</title>
<body>BrowserHost smoke<script>
(async () => {
  let canvasOK = false;
  let workerOK = false;

  try {
    const canvas = document.createElement("canvas");
    canvas.width = 2;
    canvas.height = 2;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "rgb(17, 34, 51)";
    ctx.fillRect(0, 0, 2, 2);
    const p = ctx.getImageData(0, 0, 1, 1).data;
    canvasOK = p[0] === 17 && p[1] === 34 && p[2] === 51 && p[3] === 255;
    console.log(canvasOK ? "HOST_CANVAS OK" : `HOST_CANVAS FAIL ${p[0]},${p[1]},${p[2]},${p[3]}`);
  } catch (e) {
    console.log("HOST_CANVAS FAIL " + e);
  }

  try {
    const source = `onmessage = e => { if (e.data === "ping") postMessage("pong"); };`;
    const blob = new Blob([source], { type: "text/javascript" });
    const worker = new Worker(URL.createObjectURL(blob));
    const answer = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("worker timeout")), 10000);
      worker.onmessage = e => { clearTimeout(timer); resolve(e.data); };
      worker.onerror = () => { clearTimeout(timer); reject(new Error("worker error")); };
      worker.postMessage("ping");
    });
    workerOK = answer === "pong";
    console.log(workerOK ? "HOST_WORKER OK pong" : "HOST_WORKER FAIL " + answer);
    worker.terminate();
  } catch (e) {
    console.log("HOST_WORKER FAIL " + e);
  }

  console.log(`HOST_SMOKE_${canvasOK && workerOK ? "OK" : "FAIL"} canvas=${canvasOK ? 1 : 0} worker=${workerOK ? 1 : 0}`);
})();
</script></body>'''

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.split("?", 1)[0] != "/browser-host.html":
            self.send_response(404)
            self.end_headers()
            return
        body = HTML.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        print("BROWSER_HOST_FIXTURE_OK path=/browser-host.html", flush=True)

    def log_message(self, fmt, *args):
        print("[fixture]", fmt % args, flush=True)

HTTPServer(("0.0.0.0", 18082), Handler).serve_forever()
