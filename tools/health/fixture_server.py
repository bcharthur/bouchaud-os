#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import base64
import ssl

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


M12_HTML = b"""<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Bouchaud M12</title>
<style>
html,body{margin:0;background:#07130c;color:#e8fff1;font-family:sans-serif}
main{padding:56px}
h1{font-size:42px;margin:0 0 24px;color:#4ade80}
.card{max-width:760px;padding:28px;border:2px solid #1f7a4a;background:#0e2418}
strong{color:#fde047}
code{color:#7dd3fc}
</style>
</head>
<body>
<main>
<div class="card">
<h1>Ladybird M12 sur Bouchaud OS</h1>
<p>Cette page est chargee en <strong>HTTPS</strong>, certificat verifie.</p>
<p>Chemin: <code>WebContent -&gt; RequestServer -&gt; OpenSSL -&gt; TCP Bouchaud</code></p>
<p><strong>BOUCHAUD_M12_HTTPS_OK</strong></p>
</div>
</main>
</body>
</html>
"""


# ---------------------------------------------------------------------------
# Suite fonctionnelle du moteur.
#
# Les quatre images sont de vrais fichiers, produits une fois avec Pillow puis
# figes ici en base64. Les figer plutot que les regenerer donne trois choses :
# aucune dependance de generation dans la CI, les memes octets a chaque
# execution, et une couleur differente par format — ce qui permet a la page de
# dire *lequel* a ete decode, et pas seulement qu'une image l'a ete.
#
#   PNG   16x16  #E03030      WebP  16x16  #3060E0
#   JPEG  16x16  #30C040      GIF   16x16  #E0C020
# ---------------------------------------------------------------------------

PNG_B64 = (
    "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAGUlEQVR4nGN8YGDAQApgIkn1qIZRDUNKAwAGfAFg"
    "k0eyuQAAAABJRU5ErkJggg=="
)

JPEG_B64 = (
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAIBAQEBAQIBAQECAgICAgQDAgICAgUEBAMEBgUGBgYFBgYGBwkIBgcJ"
    "BwYGCAsICQoKCgoKBggLDAsKDAkKCgr/2wBDAQICAgICAgUDAwUKBwYHCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoK"
    "CgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgr/wAARCAAQABADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAA"
    "AAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAk"
    "M2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKT"
    "lJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QA"
    "HwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdh"
    "cRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hp"
    "anN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
    "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDYooor+Xz+Bz//2Q=="
)

WEBP_B64 = (
    "UklGRh4AAABXRUJQVlA4TBEAAAAvD8ADAAdQsMIUvP+BiOh/AAA="
)

GIF_B64 = (
    "R0lGODdhEAAQAIEAAODAIAAAAAAAAAAAACwAAAAAEAAQAEAIHQABCBxIsKDBgwgTKlzIsKHDhxAjSpxIsaLFgQEB"
    "ADs="
)


MOTEUR_HTML = b"""<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<title>Bouchaud - suite moteur</title>
<style>
html,body{margin:0;background:#101820;color:#e8f0f8;font-family:sans-serif}
main{padding:24px}
h1{font-size:28px;margin:0 0 16px;color:#7ec8ff}
#vignettes img{width:32px;height:32px;margin-right:8px;image-rendering:pixelated}
#bilan{margin-top:16px;padding:12px;border:2px solid #2f5f8f;background:#16232f}
.ok{color:#7ee787}.ko{color:#ff7b72}
</style>
</head>
<body>
<main>
<h1>Suite fonctionnelle du moteur</h1>
<p id="vignettes">
<img id="i_png"  src="/img/carre.png"  alt="png">
<img id="i_jpeg" src="/img/carre.jpg"  alt="jpeg">
<img id="i_webp" src="/img/carre.webp" alt="webp">
<img id="i_gif"  src="/img/carre.gif"  alt="gif">
<img id="i_svg"  src="/img/carre.svg"  alt="svg">
</p>
<canvas id="toile" width="16" height="16"></canvas>
<div id="bilan">en cours...</div>
</main>
<script>
// Chaque verification imprime une ligne, et une seule. La CI lit ces lignes ;
// elle ne compare pas de pixels a l'ecran. Un echec dit donc *quoi*, pas
// seulement *que*.
var reussis = 0, total = 0;
function verifie(nom, condition, detail) {
  total += 1;
  if (condition) { reussis += 1; console.log("MOTEUR OK " + nom); }
  else { console.log("MOTEUR ECHEC " + nom + " " + (detail === undefined ? "" : detail)); }
  return !!condition;
}

// --- 1. DOM et CSS ---------------------------------------------------------
verifie("dom_querySelector", document.querySelector("h1") !== null);
verifie("dom_textContent", document.querySelector("h1").textContent.indexOf("moteur") > 0);
var calcule = getComputedStyle(document.body).backgroundColor;
verifie("css_computed_style", calcule.replace(/ /g, "") === "rgb(16,24,32)", calcule);
var elem = document.createElement("div");
elem.style.width = "42px";
document.body.appendChild(elem);
verifie("css_layout_offsetWidth", elem.offsetWidth === 42, elem.offsetWidth);

// --- 2. Evenements ---------------------------------------------------------
var vu = false;
elem.addEventListener("essai", function () { vu = true; });
elem.dispatchEvent(new Event("essai"));
verifie("evenements_dispatch", vu);

// --- 3. Stockage -----------------------------------------------------------
// Ces appels bloquaient WebContent pour toujours avant que l'hote local ne
// reponde : ce sont des messages IPC synchrones vers un processus absent.
try {
  localStorage.setItem("bouchaud", "valeur-1");
  verifie("localStorage_aller_retour", localStorage.getItem("bouchaud") === "valeur-1",
          localStorage.getItem("bouchaud"));
  sessionStorage.setItem("bouchaud", "valeur-2");
  verifie("sessionStorage_aller_retour", sessionStorage.getItem("bouchaud") === "valeur-2");
  localStorage.removeItem("bouchaud");
  verifie("localStorage_suppression", localStorage.getItem("bouchaud") === null);
} catch (e) {
  verifie("localStorage_aller_retour", false, String(e));
}

// --- 4. Images : telechargees ET decodees ----------------------------------
// `naturalWidth` ne vaut la vraie largeur qu'apres decodage reussi. Une image
// simplement telechargee rend 0. C'est donc la preuve que le service
// ImageDecoder a repondu, et pas seulement que RequestServer a livre.
function verifie_image(nom, id) {
  var img = document.getElementById(id);
  verifie("image_" + nom + "_decodee", img.complete && img.naturalWidth === 16,
          "complete=" + img.complete + " largeur=" + img.naturalWidth);
  return img;
}

// --- 5. Canvas : le contenu, pas seulement les dimensions ------------------
function verifie_pixel(nom, img, r, v, b) {
  var toile = document.getElementById("toile");
  var ctx = toile.getContext("2d");
  ctx.clearRect(0, 0, 16, 16);
  ctx.drawImage(img, 0, 0);
  var p = ctx.getImageData(8, 8, 1, 1).data;
  // Marge de 24 : le JPEG est avec pertes, les autres sont exacts.
  var proche = Math.abs(p[0]-r) < 24 && Math.abs(p[1]-v) < 24 && Math.abs(p[2]-b) < 24;
  verifie("canvas_" + nom + "_pixel", proche, p[0] + "," + p[1] + "," + p[2]);
}

// --- 6. Minuteurs, promesses, fetch ----------------------------------------
function apres_chargement() {
  var png  = verifie_image("png",  "i_png");
  var jpeg = verifie_image("jpeg", "i_jpeg");
  var webp = verifie_image("webp", "i_webp");
  var gif  = verifie_image("gif",  "i_gif");
  var svg  = document.getElementById("i_svg");
  verifie("image_svg_decodee", svg.complete && svg.naturalWidth === 16,
          "complete=" + svg.complete + " largeur=" + svg.naturalWidth);

  var toile = document.getElementById("toile");
  if (toile.getContext) {
    verifie("canvas_contexte_2d", toile.getContext("2d") !== null);
    verifie_pixel("png",  png,  224, 48,  48);
    verifie_pixel("jpeg", jpeg, 48,  192, 64);
    verifie_pixel("webp", webp, 48,  96,  224);
    verifie_pixel("gif",  gif,  224, 192, 32);
  } else {
    verifie("canvas_contexte_2d", false, "getContext absent");
  }

  Promise.resolve(7)
    .then(function (v) { return v * 6; })
    .then(function (v) {
      verifie("promesse_chainee", v === 42, v);
      return new Promise(function (r) { setTimeout(function () { r("minuteur"); }, 30); });
    })
    .then(function (v) {
      verifie("minuteur_setTimeout", v === "minuteur", v);
      return fetch("/api/pixel.json");
    })
    .then(function (reponse) {
      verifie("fetch_statut", reponse.status === 200, reponse.status);
      return reponse.json();
    })
    .then(function (json) {
      verifie("fetch_json", json.rouge === 224 && json.vert === 48, JSON.stringify(json));
      return new Promise(function (resoudre, rejeter) {
        var x = new XMLHttpRequest();
        x.open("GET", "/api/pixel.json");
        x.onload = function () { resoudre(x.status); };
        x.onerror = function () { rejeter(new Error("XHR en erreur")); };
        x.send();
      });
    })
    .then(function (statut) {
      verifie("xhr_statut", statut === 200, statut);
    })
    .catch(function (e) {
      verifie("chaine_asynchrone", false, String(e));
    })
    .then(function () {
      // Cookies : poses par le serveur a la reponse, relus par le document.
      verifie("cookie_serveur_visible", document.cookie.indexOf("bouchaud=moteur") >= 0,
              document.cookie);
      document.cookie = "depuis_js=1";
      verifie("cookie_ecrit_par_js", document.cookie.indexOf("depuis_js=1") >= 0,
              document.cookie);

      var bilan = document.getElementById("bilan");
      bilan.className = (reussis === total) ? "ok" : "ko";
      bilan.textContent = "MOTEUR_BILAN reussis=" + reussis + " total=" + total;
      document.title = "Bouchaud " + reussis + "/" + total;
      console.log("MOTEUR_BILAN reussis=" + reussis + " total=" + total);
    });
}

if (document.readyState === "complete") apres_chargement();
else window.addEventListener("load", apres_chargement);
</script>
</main>
</body>
</html>
"""

SVG_CARRE = b"""<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">
<rect width="16" height="16" fill="#9040e0"/>
</svg>
"""

PIXEL_JSON = b'{"rouge":224,"vert":48,"bleu":48}\n'


ROUTES_MOTEUR = {
    "/moteur.html": (MOTEUR_HTML, "text/html; charset=utf-8"),
    "/img/carre.png": (base64.b64decode(PNG_B64), "image/png"),
    "/img/carre.jpg": (base64.b64decode(JPEG_B64), "image/jpeg"),
    "/img/carre.webp": (base64.b64decode(WEBP_B64), "image/webp"),
    "/img/carre.gif": (base64.b64decode(GIF_B64), "image/gif"),
    "/img/carre.svg": (SVG_CARRE, "image/svg+xml"),
    "/api/pixel.json": (PIXEL_JSON, "application/json"),
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if self.path in ROUTES_MOTEUR:
            body, content_type = ROUTES_MOTEUR[self.path]
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            # Un cookie pose par le serveur : la page verifie ensuite qu'elle le
            # relit. C'est ce qui distingue « les cookies sont branches » de
            # « le pot existe ».
            if self.path == "/moteur.html":
                self.send_header("Set-Cookie", "bouchaud=moteur; Path=/")
                print("[health-fixture] MOTEUR_FIXTURE_OK path=/moteur.html", flush=True)
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
            return

        if self.path == "/health":
            body = BODY
            content_type = "text/plain; charset=utf-8"
            self.send_response(200)
        elif self.path == "/m12.html":
            body = M12_HTML
            content_type = "text/html; charset=utf-8"
            self.send_response(200)
            print("[health-fixture] M12_FIXTURE_HTTPS_OK path=/m12.html", flush=True)
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
    # TLS local. La CI ne doit dependre d'aucun site externe : le certificat est
    # fabrique sur place et son autorite est embarquee dans l'image Bouchaud.
    # C'est une verification TLS **reelle** — la chaine est validee — sans
    # sortir de la machine.
    parser.add_argument("--cert", help="certificat serveur (PEM) ; active TLS")
    parser.add_argument("--key", help="cle privee (PEM)")
    args = parser.parse_args()

    server = ThreadingHTTPServer(("0.0.0.0", args.port), Handler)
    schema = "http"
    if args.cert:
        if not args.key:
            parser.error("--cert exige --key")
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certfile=args.cert, keyfile=args.key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        schema = "https"
    print(f"[health-fixture] listening on {schema}://0.0.0.0:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
