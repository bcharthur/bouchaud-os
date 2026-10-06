#!/usr/bin/env python3
"""Serveur du banc « le cache HTTP et la base SQL survivent au redemarrage du
navigateur » (BOUCHAUD_CACHE_REDEMARRAGE_V1).

    python3 tools/ci/ladybird_cache_fixture.py PORT JOURNAL

Le serveur est le TEMOIN : il compte ce qu'il a reellement envoye. Une
ressource servie depuis le cache disque ne lui parvient pas ; une
revalidation lui parvient avec `If-None-Match`.

  /cache-test.html       la page (no-store) ; ?passage=1|2
  /cache/stable.bin      64 Kio deterministes, max-age=86400, ETag fixe :
                         passage 2 doit la lire DEPUIS LE DISQUE (0 GET)
  /cache/inchange.txt    no-cache, ETag fixe : passage 2 revalide -> 304
  /cache/change.txt      no-cache ; la version change a chaque reponse
                         complete : passage 2 revalide -> 200 v2 (invalidation)
  /cookie                pose un cookie persistant (Max-Age) au passage 1 ;
                         au passage 2 le navigateur doit le RENVOYER (SQLite)

Chaque requete est journalisee en une ligne `CACHE_FIXTURE <methode> <chemin>
inm=<If-None-Match> cookie=<present|absent> statut=<code> octets=<n>`.
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
JOURNAL = open(sys.argv[2], "a", buffering=1)
STABLE = bytes((i * 131 + 7) % 251 for i in range(65536))
etat = {"change": 1}

PAGE = r'''<!doctype html><meta charset="utf-8"><title>cache</title><body>cache<script>
(async () => {
  const passage = new URLSearchParams(location.search).get("passage");
  const somme = b => { let s = 0; for (const x of b) s = (s * 31 + x) >>> 0; return s; };
  for (const nom of ["stable.bin", "inchange.txt", "change.txt"]) {
    try {
      const r = await fetch(`/cache/${nom}`);
      const b = new Uint8Array(await r.arrayBuffer());
      console.log(`HOST_CACHE passage=${passage} res=${nom} statut=${r.status} taille=${b.length} somme=${somme(b)}`
        + (nom.endsWith(".txt") ? ` texte=${new TextDecoder().decode(b).trim()}` : ""));
    } catch (e) {
      console.log(`HOST_CACHE passage=${passage} res=${nom} FAIL ${e}`);
    }
  }
  // La base SQL du profil : localStorage et les cookies y vivent.
  if (passage === "1") {
    localStorage.setItem("bouchaud-sql", "ecrit-au-passage-1");
    await fetch("/cookie?pose=1");
    console.log(`HOST_SQL passage=1 WRITE localStorage=${localStorage.getItem("bouchaud-sql")}`);
  } else {
    const relu = localStorage.getItem("bouchaud-sql");
    await fetch("/cookie?pose=0");
    console.log(`HOST_SQL passage=2 ${relu === "ecrit-au-passage-1" ? "REOPEN_OK" : "REOPEN_FAIL"} localStorage=${relu}`);
  }
  // Laisser RequestServer terminer l'ecriture des entrees, puis quitter.
  setTimeout(() => {
    console.log(`HOST_CACHE_FIN passage=${passage}`);
    document.title = "BOUCHAUD_BANC_QUITTE";
  }, 3000);
})();
</script></body>'''.encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def envoie(self, statut, corps, entetes):
        self.send_response(statut)
        for cle, valeur in entetes:
            self.send_header(cle, valeur)
        self.send_header("Content-Length", str(len(corps)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(corps)
        inm = self.headers.get("If-None-Match", "-")
        cookie = "present" if "bouchaud_sql=" in (self.headers.get("Cookie") or "") else "absent"
        JOURNAL.write(f"CACHE_FIXTURE {self.command} {self.path} inm={inm} cookie={cookie} statut={statut} octets={len(corps)}\n")

    def do_GET(self):
        chemin = self.path.split("?")[0]
        inm = self.headers.get("If-None-Match")
        if chemin == "/cache-test.html":
            return self.envoie(200, PAGE, [("Content-Type", "text/html; charset=utf-8"), ("Cache-Control", "no-store")])
        if chemin == "/cache/stable.bin":
            if inm == '"stable-v1"':
                return self.envoie(304, b"", [("ETag", '"stable-v1"'), ("Cache-Control", "max-age=86400")])
            return self.envoie(200, STABLE, [("Content-Type", "application/octet-stream"),
                                             ("Cache-Control", "max-age=86400"), ("ETag", '"stable-v1"')])
        if chemin == "/cache/inchange.txt":
            if inm == '"inchange-v1"':
                return self.envoie(304, b"", [("ETag", '"inchange-v1"'), ("Cache-Control", "no-cache")])
            return self.envoie(200, b"inchange v1\n", [("Content-Type", "text/plain"),
                                                      ("Cache-Control", "no-cache"), ("ETag", '"inchange-v1"')])
        if chemin == "/cache/change.txt":
            version = etat["change"]
            if inm == f'"change-v{version}"' and version > 1:
                return self.envoie(304, b"", [("ETag", f'"change-v{version}"'), ("Cache-Control", "no-cache")])
            corps = f"change v{version}\n".encode()
            reponse = (200, corps, [("Content-Type", "text/plain"), ("Cache-Control", "no-cache"),
                                    ("ETag", f'"change-v{version}"')])
            etat["change"] = version + 1
            return self.envoie(*reponse)
        if chemin == "/cookie":
            entetes = [("Content-Type", "text/plain"), ("Cache-Control", "no-store")]
            if "pose=1" in self.path:
                entetes.append(("Set-Cookie", "bouchaud_sql=persistant; Max-Age=86400; Path=/"))
            return self.envoie(200, b"ok\n", entetes)
        return self.envoie(404, b"absent\n", [("Content-Type", "text/plain")])

    def log_message(self, fmt, *args):
        pass


ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
