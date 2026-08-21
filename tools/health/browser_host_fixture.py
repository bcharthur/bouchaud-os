#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer
import struct
import zlib


def _png_chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def _pixel_png() -> bytes:
    # PNG RGBA 1x1 deterministe, pixel #112233 opaque.
    signature = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    scanline = b"\x00\x11\x22\x33\xff"
    return signature + _png_chunk(b"IHDR", ihdr) + _png_chunk(b"IDAT", zlib.compress(scanline)) + _png_chunk(b"IEND", b"")


PIXEL_PNG = _pixel_png()

HTML = r'''<!doctype html>
<meta charset="utf-8">
<title>Bouchaud BrowserHost smoke</title>
<body>BrowserHost smoke<script>
(async () => {
  let canvasOK = false;
  let workerOK = false;
  let imageOK = false;

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

  try {
    const image = new Image();
    const loaded = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("image timeout")), 10000);
      image.onload = () => { clearTimeout(timer); resolve(); };
      image.onerror = () => { clearTimeout(timer); reject(new Error("image decode error")); };
    });
    image.src = "/pixel.png";
    await loaded;
    imageOK = image.naturalWidth === 1 && image.naturalHeight === 1;
    console.log(imageOK ? "HOST_IMAGE OK 1x1" : `HOST_IMAGE FAIL ${image.naturalWidth}x${image.naturalHeight}`);
  } catch (e) {
    console.log("HOST_IMAGE FAIL " + e);
  }

  console.log(`HOST_SMOKE_${canvasOK && workerOK && imageOK ? "OK" : "FAIL"} canvas=${canvasOK ? 1 : 0} worker=${workerOK ? 1 : 0} image=${imageOK ? 1 : 0}`);
})();
</script></body>'''

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/pixel.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(PIXEL_PNG)))
            self.end_headers()
            self.wfile.write(PIXEL_PNG)
            print("BROWSER_HOST_FIXTURE_IMAGE_OK path=/pixel.png", flush=True)
            return
        if path != "/browser-host.html":
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
