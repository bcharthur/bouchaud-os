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

# BOUCHAUD_C26_WORKER_DEUX_ORIGINES
#
# Le meme worker, servi en HTTP. Il existe pour DISCRIMINER, pas pour doubler
# la couverture.
#
# Le smoke test ne creait son worker que depuis une URL `blob:`. Or un
# `blob:` est un objet de l'agent QUI L'A CREE, et un WebWorker Ladybird est
# un PROCESSUS SEPARE : le faire resoudre demande que le magasin d'URL de blob
# traverse la frontiere de processus. C'est une mecanique entierement
# differente de celle qui lance le processus.
#
# Un seul worker ne pouvait donc pas dire lequel des deux etages est casse.
# Deux le disent en une seule execution :
#
#     http OK, blob KO  -> le processus demarre ; c'est le blob: qui ne
#                          traverse pas
#     les deux KO       -> le processus ne demarre pas, ou son IPC n'arrive
#                          jamais
#     http KO, blob OK  -> le chargement du script par le reseau est casse
WORKER_JS = b'onmessage = e => { if (e.data === "ping") postMessage("pong"); };\n'

HTML = r'''<!doctype html>
<meta charset="utf-8">
<title>Bouchaud BrowserHost smoke</title>
<body>BrowserHost smoke<script>
(async () => {
  let canvasOK = false;
  let workerOK = false;
  let imageOK = false;
  let frameOK = false;

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

  // BOUCHAUD_C26_WORKER_DEUX_ORIGINES
  //
  // Le meme worker, lance de deux facons. Voir WORKER_JS plus haut : un seul
  // ne pouvait pas dire si c'est le PROCESSUS qui ne demarre pas ou le
  // `blob:` qui ne traverse pas la frontiere de processus.
  //
  // Chaque etape se dit, et avec son horodatage. « worker timeout » ne
  // distingue pas « le processus n'a jamais demarre » de « il a demarre et
  // n'a pas repondu », et ces deux-la n'ont pas le meme remede.
  const essaieWorker = async (nom, url, garde) => {
    const t0 = performance.now();
    const dit = (quoi, extra) =>
      console.log(`HOST_WORKER_ETAPE origine=${nom} etape=${quoi} ms=${Math.round(performance.now() - t0)}${extra ? " " + extra : ""}`);
    try {
      dit("construction");
      const worker = new Worker(url);
      dit("construit");
      const answer = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("timeout")), garde);
        worker.onmessage = e => { clearTimeout(timer); dit("message_recu"); resolve(e.data); };
        worker.onerror = ev => {
          clearTimeout(timer);
          dit("erreur", `message=${ev && ev.message ? ev.message : "?"}`);
          reject(new Error("worker error"));
        };
        dit("postMessage");
        worker.postMessage("ping");
      });
      worker.terminate();
      const ok = answer === "pong";
      dit(ok ? "pong" : "reponse_inattendue", `reponse=${answer}`);
      return ok;
    } catch (e) {
      dit("echec", `raison=${e}`);
      return false;
    }
  };

  // LE GARDE-FOU N'EST PAS UN BUDGET DE PERFORMANCE.
  //
  // WebWorker est un binaire Ladybird complet, avec son edition de liens et
  // son initialisation ICU, et la machine emulee de la CI met deja vingt-huit
  // secondes a initialiser le navigateur lui-meme. Soixante secondes chacun.
  const GARDE_WORKER_MS = 60000;

  // L'ORDRE COMPTE : HTTP d'abord.
  //
  // Si le processus ne demarre pas du tout, c'est celui-la qui le dira, et
  // l'essai `blob:` qui suit n'aura pas a payer une seconde attente de
  // soixante secondes pour apprendre la meme chose.
  let workerHttpOK = false;
  let workerBlobOK = false;
  try {
    workerHttpOK = await essaieWorker("http", "/worker.js", GARDE_WORKER_MS);
  } catch (e) {
    console.log("HOST_WORKER_HTTP FAIL " + e);
  }
  try {
    const source = `onmessage = e => { if (e.data === "ping") postMessage("pong"); };`;
    const blob = new Blob([source], { type: "text/javascript" });
    workerBlobOK = await essaieWorker("blob", URL.createObjectURL(blob), GARDE_WORKER_MS);
  } catch (e) {
    console.log("HOST_WORKER_BLOB FAIL " + e);
  }

  console.log(`HOST_WORKER_HTTP ${workerHttpOK ? "OK pong" : "FAIL"}`);
  console.log(`HOST_WORKER_BLOB ${workerBlobOK ? "OK pong" : "FAIL"}`);

  // L'ASSERTION HISTORIQUE, ET CE QU'ELLE EXIGE DESORMAIS.
  //
  // `HOST_WORKER OK pong` reste le jalon que la CI attend. Il est satisfait
  // des qu'un worker repond, PAR N'IMPORTE LAQUELLE des deux origines : ce
  // que ce jalon affirme est « le navigateur sait executer du JavaScript dans
  // un processus WebWorker et en recevoir un message ». Le cas `blob:` est
  // une capacite distincte, et il a sa propre ligne -- la confondre avec la
  // premiere ferait echouer le lot entier sur un defaut de magasin d'URL.
  workerOK = workerHttpOK || workerBlobOK;
  console.log(workerOK ? "HOST_WORKER OK pong" : "HOST_WORKER FAIL les deux origines ont echoue");

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

  try {
    const frame = document.createElement("iframe");
    const loaded = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("iframe timeout")), 10000);
      frame.onload = () => { clearTimeout(timer); resolve(); };
      frame.onerror = () => { clearTimeout(timer); reject(new Error("iframe error")); };
    });
    frame.src = "/frame.html";
    document.body.appendChild(frame);
    await loaded;
    frameOK = true;
    console.log("HOST_IFRAME OK");
  } catch (e) {
    console.log("HOST_IFRAME FAIL " + e);
  }

  console.log(`HOST_SMOKE_${canvasOK && workerOK && imageOK && frameOK ? "OK" : "FAIL"} canvas=${canvasOK ? 1 : 0} worker=${workerOK ? 1 : 0} image=${imageOK ? 1 : 0} frame=${frameOK ? 1 : 0}`);
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
        if path == "/worker.js":
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript")
            self.send_header("Content-Length", str(len(WORKER_JS)))
            self.end_headers()
            self.wfile.write(WORKER_JS)
            print("BROWSER_HOST_FIXTURE_WORKER_OK path=/worker.js", flush=True)
            return
        if path == "/frame.html":
            body = b"<!doctype html><meta charset=utf-8><body>Bouchaud iframe</body>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            print("BROWSER_HOST_FIXTURE_FRAME_OK path=/frame.html", flush=True)
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
