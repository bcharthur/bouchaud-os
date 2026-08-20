#!/usr/bin/env python3
"""Fixture deterministe pour la plateforme Ladybird sous Bouchaud OS.

Chaque marqueur n'est imprime QUE lorsque l'API testee a reellement produit le
resultat attendu. Un test qui n'aboutit pas rend FAIL, jamais rien d'autre.

Deux regles de construction :

- **Chaque test a sa propre limite de temps.** Les tests s'enchainent dans une
  seule fonction asynchrone ; sans limite, le premier qui reste en attente
  empeche tous les suivants de s'exprimer, et le journal ne montre alors plus
  rien -- pas meme les tests qui passaient. Une limite transforme un blocage en
  echec localise.

- **Les images sont verifiees par leurs PIXELS**, pas par leur evenement `load`.
  Un `load` prouve que le flux est arrive ; il ne prouve pas que le decodeur a
  rendu la bonne couleur. `drawImage` puis `getImageData` verifie la chaine
  complete RequestServer -> ImageDecoder -> Skia.
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import base64

# PNG 2x2 entierement rouge opaque, et JPEG 8x8 entierement bleu. Assez petits
# pour tenir ici, assez vrais pour exercer les deux decodeurs.
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf"
    "7gP9i18U1AAAAABJRU5ErkJggg=="
)
JPEG = base64.b64decode(
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoM"
    "DAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsN"
    "FBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAAR"
    "CAAIAAgDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAA"
    "AgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkK"
    "FhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWG"
    "h4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl"
    "5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA"
    "AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYk"
    "NOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
    "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
    "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD896KKK/1TPhz/2Q=="
)

# Les deux pages que la navigation fait defiler dans une iframe. Chacune
# annonce son identite au document parent : c'est ce qui permet de savoir vers
# quelle page `history.back()` est reellement revenu.
def page_navigation(nom: str) -> bytes:
    # Le message part sur `load`, pas pendant l'analyse du document. Une page
    # qui s'annonce avant d'avoir fini de charger fait enchainer la navigation
    # suivante trop tot : l'entree d'historique de la premiere n'est pas encore
    # validee, la seconde la REMPLACE au lieu de s'empiler, et un `back()`
    # ulterieur remonte alors au-dela de l'iframe -- jusqu'au document de tete.
    return (
        "<!doctype html><meta charset=utf-8><title>%s</title>"
        "<script>addEventListener('load', () => parent.postMessage('NAV:%s', '*'))</script>"
        % (nom, nom)
    ).encode()


# Le telechargement est declenche depuis l'IFRAME, pas depuis le document de
# tete : `window.close()` n'est autorise que si le traversable de tete n'a
# qu'une seule entree d'historique (LocalNavigable::is_script_closable), et la
# fermeture propre est ce qui permet au noyau d'ecrire /persist avant de couper.
PAGE_TELECHARGEMENT = (
    "<!doctype html><meta charset=utf-8><title>telechargement</title>"
    "<a id=lien href='/download.bin' download='preuve-bouchaud.bin'>t</a>"
    "<script>"
    "document.getElementById('lien').click();"
    "parent.postMessage('NAV:telechargement', '*');"
    "</script>"
).encode()


# Sonde HTTPS : un fichier du depot, fige par un SHA, donc au contenu connu.
# On y verifie le SHA upstream epingle -- c'est-a-dire que la page a bien lu le
# fichier qui le porte, et pas une page d'erreur.
HTTPS_SONDE_URL = (
    "https://raw.githubusercontent.com/bcharthur/bouchaud-os/"
    "df706e739cb98ca959aa29ba627462ac89eefa8c/third_party/UPSTREAM.md"
)
HTTPS_SONDE_ATTENDU = "cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6"

PAGE = r'''<!doctype html>
<meta charset="utf-8">
<title>Bouchaud Ladybird full platform</title>
<body>
<iframe id="cadre" style="width:120px;height:60px"></iframe>
<script>
(async () => {
  const HTTPS_SONDE_URL = "__HTTPS_SONDE_URL__";
  const HTTPS_SONDE_ATTENDU = "__HTTPS_SONDE_ATTENDU__";
  const results = {};
  const mark = (name, ok, detail = "") => {
    results[name] = !!ok;
    console.log(`PLATFORM_${name.toUpperCase()} ${ok ? "OK" : "FAIL"} ${detail}`.trim());
  };

  // Un test qui n'aboutit pas doit ECHOUER, pas retenir les suivants.
  const limite = (promesse, ms, quoi) => Promise.race([
    promesse,
    new Promise((_, rejeter) => setTimeout(() => rejeter(new Error(`limite ${quoi} ${ms}ms`)), ms)),
  ]);

  const essaie = async (nom, ms, corps) => {
    try {
      const ok = await limite(Promise.resolve().then(corps), ms, nom);
      mark(nom, ok === true, ok === true ? "" : `valeur=${ok}`);
    } catch (e) {
      mark(nom, false, String(e));
    }
  };

  // --- survie au redemarrage ---------------------------------------------
  //
  // Ce bloc lit AVANT d'ecrire. Au premier demarrage il ne trouve rien, ce qui
  // est normal et se dit ; au demarrage suivant, sur la meme image, il doit
  // retrouver exactement ce que le precedent a laisse. C'est la seule facon de
  // prouver une persistance : la meme valeur, de part et d'autre d'une
  // extinction.
  const TEMOIN = "profil-survivant-42";

  const lit_idb = () => new Promise((resoudre) => {
    let requete;
    try { requete = indexedDB.open("bouchaud-persistance", 1); } catch (e) { return resoudre(null); }
    requete.onupgradeneeded = () => requete.result.createObjectStore("kv");
    requete.onerror = () => resoudre(null);
    requete.onsuccess = () => {
      const db = requete.result;
      const rx = db.transaction("kv").objectStore("kv").get("temoin");
      rx.onerror = () => { db.close(); resoudre(null); };
      rx.onsuccess = () => { const v = rx.result; db.close(); resoudre(v ?? null); };
    };
  });

  const ecrit_idb = (valeur) => new Promise((resoudre) => {
    const requete = indexedDB.open("bouchaud-persistance", 1);
    requete.onupgradeneeded = () => requete.result.createObjectStore("kv");
    requete.onerror = () => resoudre(false);
    requete.onsuccess = () => {
      const db = requete.result;
      const tx = db.transaction("kv", "readwrite");
      tx.objectStore("kv").put(valeur, "temoin");
      tx.oncomplete = () => { db.close(); resoudre(true); };
      tx.onerror = () => { db.close(); resoudre(false); };
    };
  });

  try {
    const vu_local = localStorage.getItem("bouchaud_temoin");
    const vu_cookie = document.cookie.includes(`bouchaud_temoin=${TEMOIN}`);
    const vu_idb = await limite(lit_idb(), 20000, "persistance-lecture");
    const complet = vu_local === TEMOIN && vu_cookie && vu_idb === TEMOIN;
    if (complet) {
      console.log("PLATFORM_PERSIST_APRES_REDEMARRAGE OK localStorage+cookie+indexeddb");
    } else {
      console.log(`PLATFORM_PERSIST_PREMIER_BOOT localStorage=${vu_local} cookie=${vu_cookie} indexeddb=${vu_idb}`);
    }
    localStorage.setItem("bouchaud_temoin", TEMOIN);
    document.cookie = `bouchaud_temoin=${TEMOIN}; Max-Age=86400; SameSite=Lax`;
    const ecrit = await limite(ecrit_idb(TEMOIN), 20000, "persistance-ecriture");
    console.log(`PLATFORM_PERSIST_ECRIT ${ecrit ? "OK" : "FAIL"}`);
  } catch (e) {
    console.log(`PLATFORM_PERSIST_ECRIT FAIL ${e}`);
  }

  // --- stockage, temoins, reseau -----------------------------------------

  await essaie("localstorage", 5000, () => {
    localStorage.setItem("bouchaud", "ladybird");
    return localStorage.getItem("bouchaud") === "ladybird";
  });

  await essaie("sessionstorage", 5000, () => {
    sessionStorage.setItem("session", "ok");
    return sessionStorage.getItem("session") === "ok";
  });

  await essaie("cookie", 5000, () => {
    document.cookie = "bouchaud_cookie=ok; SameSite=Lax";
    return document.cookie.includes("bouchaud_cookie=ok");
  });

  await essaie("fetch", 15000, async () => {
    const response = await fetch("/api");
    return response.ok && (await response.text()) === "fetch-ok";
  });

  // --- reseau reel : DNS, TLS 1.3, chaine de certification ----------------
  //
  // La fixture locale prouve HTTP jusqu'a 10.0.2.2, pas davantage. Ces deux
  // tests-ci sortent de la machine :
  //
  // - `https` exige une resolution DNS reelle, une poignee de main TLS et la
  //   validation de la chaine contre /etc/ssl/certs/ca-certificates.crt. Le
  //   contenu attendu est fige par un SHA, donc le test reste deterministe :
  //   il verifie le SHA upstream epingle, dans le fichier qui le porte.
  //
  // - `reseau_echec` verifie qu'un nom qui NE PEUT PAS resoudre -- `.invalid`
  //   est reserve pour cela par la RFC 2606 -- fait echouer la requete SANS
  //   emporter le moteur. Le test ne vaut que par sa seconde moitie : apres
  //   l'echec, une requete valide doit encore aboutir.

  await essaie("https", 45000, async () => {
    const reponse = await fetch(HTTPS_SONDE_URL);
    if (!reponse.ok) return false;
    const texte = await reponse.text();
    return texte.includes(HTTPS_SONDE_ATTENDU);
  });

  await essaie("reseau_echec", 45000, async () => {
    let rompu = false;
    try {
      await fetch("https://nom-qui-ne-resout-pas.bouchaud.invalid/");
    } catch (e) {
      rompu = true;
    }
    // Le moteur doit encore repondre apres l'echec.
    const apres = await fetch("/api");
    return rompu && apres.ok && (await apres.text()) === "fetch-ok";
  });

  // --- DOM, JS, promesses, minuteries ------------------------------------

  await essaie("dom", 5000, () => {
    const hote = document.createElement("div");
    hote.innerHTML = '<p class="cible"><span>a</span><span>b</span></p>';
    document.body.appendChild(hote);
    const trouve = hote.querySelectorAll("p.cible span");
    const texte = hote.querySelector("p.cible").textContent;
    hote.remove();
    return trouve.length === 2 && texte === "ab" && !document.querySelector("p.cible");
  });

  await essaie("async", 5000, async () => {
    // Promesse tenue, promesse rompue rattrapee, et exception synchrone.
    const tenue = await Promise.resolve(21).then(v => v * 2);
    let rattrapee = null;
    try { await Promise.reject(new Error("prevue")); } catch (e) { rattrapee = e.message; }
    let levee = null;
    try { (() => { throw new TypeError("aussi prevue"); })(); } catch (e) { levee = e.name; }
    const groupe = await Promise.all([1, Promise.resolve(2)]);
    return tenue === 42 && rattrapee === "prevue" && levee === "TypeError"
        && groupe[0] === 1 && groupe[1] === 2;
  });

  await essaie("timer", 5000, async () => {
    // L'ordre doit suivre les delais, pas l'ordre de creation.
    const ordre = [];
    await new Promise(resoudre => {
      setTimeout(() => { ordre.push("tard"); resoudre(); }, 60);
      setTimeout(() => ordre.push("tot"), 10);
    });
    return ordre.length === 2 && ordre[0] === "tot" && ordre[1] === "tard";
  });

  await essaie("timezone", 5000, () => {
    // --default-time-zone doit atteindre le moteur, pas seulement le shell.
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const date = new Date(0);
    return zone === "Europe/Paris" && !Number.isNaN(date.getTime());
  });

  // --- rendu ---------------------------------------------------------------

  await essaie("canvas", 10000, () => {
    const canvas = document.createElement("canvas");
    canvas.width = 2; canvas.height = 2;
    const c = canvas.getContext("2d");
    c.fillStyle = "rgb(11,22,33)";
    c.fillRect(0,0,2,2);
    const p = c.getImageData(0,0,1,1).data;
    return p[0] === 11 && p[1] === 22 && p[2] === 33 && p[3] === 255;
  });

  // --- images --------------------------------------------------------------

  const charge = (src) => new Promise((resoudre, rejeter) => {
    const image = new Image();
    image.onload = () => resoudre(image);
    image.onerror = () => rejeter(new Error(`chargement echoue ${src}`));
    image.src = src;
  });

  let png = null;
  await essaie("image_png", 20000, async () => {
    png = await charge("/image.png");
    return png.naturalWidth === 2 && png.naturalHeight === 2;
  });

  await essaie("image_jpeg", 20000, async () => {
    const jpeg = await charge("/image.jpg");
    return jpeg.naturalWidth === 8 && jpeg.naturalHeight === 8;
  });

  // La preuve qui compte : les PIXELS decodes, pas l'evenement `load`.
  await essaie("image_pixels", 15000, () => {
    if (!png) return false;
    const canvas = document.createElement("canvas");
    canvas.width = 2; canvas.height = 2;
    const c = canvas.getContext("2d");
    c.drawImage(png, 0, 0);
    const p = c.getImageData(0, 0, 1, 1).data;
    return p[0] === 255 && p[1] === 0 && p[2] === 0 && p[3] === 255;
  });

  // Une image absente doit declencher `error` et laisser le moteur debout.
  await essaie("image_erreur", 20000, async () => {
    let erreur = false;
    try { await charge("/image-qui-nexiste-pas.png"); } catch (e) { erreur = true; }
    // Le moteur repond encore apres l'echec : c'est cela qu'on verifie.
    const encore = await charge("/image.png");
    return erreur && encore.naturalWidth === 2;
  });

  // --- navigation, historique, rechargement --------------------------------

  const cadre = document.getElementById("cadre");
  const attend_page = (nom, action) => new Promise((resoudre, rejeter) => {
    const ecoute = (evenement) => {
      if (evenement.data === `NAV:${nom}`) {
        window.removeEventListener("message", ecoute);
        resoudre(true);
      }
    };
    window.addEventListener("message", ecoute);
    try { action(); } catch (e) { window.removeEventListener("message", ecoute); rejeter(e); }
  });

  await essaie("navigation", 30000, async () => {
    await attend_page("un", () => { cadre.src = "/nav-un.html"; });
    await attend_page("deux", () => { cadre.src = "/nav-deux.html"; });
    return true;
  });

  await essaie("historique", 30000, async () => {
    // `history.back()` traverse l'historique de session CONJOINT. Si l'iframe
    // n'a pas empile sa propre entree, le retour remonte au document de tete et
    // detruit la page de test : plus aucun marqueur ne peut alors s'imprimer,
    // pas meme un echec, et le journal devient muet. Mesure dans Chromium :
    //
    //     13.1s  NAV[iframe] /nav-deux.html
    //     13.1s  PLATFORM_NAVIGATION OK
    //     13.2s  NAV[TETE] about:blank      <- la page de test a disparu
    //
    // On verifie donc que l'iframe a bien deux entrees avant de reculer. Si ce
    // n'est pas le cas, le test ECHOUE au lieu d'emporter tout le reste.
    const avant = cadre.contentWindow.history.length;
    if (avant < 2) return `historique iframe trop court (${avant})`;

    const tete = location.href;
    await attend_page("un", () => cadre.contentWindow.history.back());
    if (location.href !== tete) return "le retour a deplace le document de tete";
    await attend_page("deux", () => cadre.contentWindow.history.forward());
    return location.href === tete;
  });

  await essaie("rechargement", 30000, async () => {
    await attend_page("deux", () => cadre.contentWindow.location.reload());
    return true;
  });

  // --- verdict --------------------------------------------------------------

  const required = [
    "localstorage", "sessionstorage", "cookie", "fetch",
    "dom", "async", "timer", "timezone",
    "canvas", "worker", "wasm", "indexeddb",
    "image_png", "image_jpeg", "image_pixels", "image_erreur",
    "navigation", "historique", "rechargement",
    "https", "reseau_echec",
  ];
  const rates = required.filter(k => !results[k]);
  const bons = required.length - rates.length;
  if (rates.length)
    console.log(`PLATFORM_ECHECS ${rates.join(",")}`);
  console.log(`PLATFORM_FULL_${rates.length ? "FAIL" : "OK"} passed=${bons}/${required.length}`);

  // --- telechargement, puis fermeture propre --------------------------------
  //
  // Le telechargement ne peut pas se verifier depuis le JavaScript : c'est le
  // processus hote qui ecrit le fichier, et c'est donc l'OS qui doit le
  // constater. On le declenche ici, et `/persist/Downloads` est relu par le
  // shell au demarrage suivant.
  try {
    await limite(attend_page("telechargement", () => { cadre.src = "/telecharger.html"; }),
                 20000, "telechargement");
    console.log("PLATFORM_DOWNLOAD_DEMANDE preuve-bouchaud.bin");
  } catch (e) {
    console.log(`PLATFORM_DOWNLOAD_DEMANDE FAIL ${e}`);
  }

  // Laisser le telechargement s'ecrire avant de rendre la main.
  await new Promise(r => setTimeout(r, 8000));

  // `window.close()` fait quitter la boucle d'evenements du BrowserHost
  // (HeadlessMode::Manual -> load_page_and_exit_on_close). L'autorun reprend
  // alors la main, le noyau ecrit /persist et coupe : c'est ce qui rend le
  // demarrage suivant capable de retrouver le profil et le fichier.
  console.log("PLATFORM_FERMETURE demandee");
  window.close();
})();
</script>

<script>
// Worker, Wasm et IndexedDB restent dans leur propre fonction : ils touchent
// des processus et des bases separes, et on ne veut pas qu'un blocage de l'un
// retienne l'inventaire ci-dessus.
</script>
</body>
'''

# Les trois tests qui exigent d'autres processus ou une base, inseres dans la
# meme sequence pour que le verdict les couvre.
BLOC_PROCESSUS = r'''
  await essaie("worker", 25000, async () => {
    const src = `onmessage=e=>{if(e.data==="ping")postMessage("pong")}`;
    const worker = new Worker(URL.createObjectURL(new Blob([src], {type:"text/javascript"})));
    try {
      const reponse = await new Promise((resoudre, rejeter) => {
        worker.onmessage = e => resoudre(e.data);
        worker.onerror = () => rejeter(new Error("worker"));
        worker.postMessage("ping");
      });
      return reponse === "pong";
    } finally {
      worker.terminate();
    }
  });

  await essaie("wasm", 20000, async () => {
    const bytes = new Uint8Array([
      0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,
      7,5,1,1,102,0,0,10,6,1,4,0,65,42,11
    ]);
    const {instance} = await WebAssembly.instantiate(bytes);
    return instance.exports.f() === 42;
  });

  await essaie("indexeddb", 25000, async () => {
    const openRequest = indexedDB.open("bouchaud-platform", 1);
    const db = await new Promise((resoudre, rejeter) => {
      openRequest.onupgradeneeded = () => openRequest.result.createObjectStore("kv");
      openRequest.onsuccess = () => resoudre(openRequest.result);
      openRequest.onerror = () => rejeter(openRequest.error);
    });
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("ok", "key");
    await new Promise((resoudre, rejeter) => {
      tx.oncomplete = resoudre; tx.onerror = () => rejeter(tx.error);
    });
    const rx = db.transaction("kv").objectStore("kv").get("key");
    const valeur = await new Promise((resoudre, rejeter) => {
      rx.onsuccess = () => resoudre(rx.result); rx.onerror = () => rejeter(rx.error);
    });
    db.close();
    return valeur === "ok";
  });
'''

PAGE = PAGE.replace("  // --- navigation, historique, rechargement --------------------------------",
                    BLOC_PROCESSUS + "\n  // --- navigation, historique, rechargement --------------------------------")
PAGE = PAGE.replace("__HTTPS_SONDE_URL__", HTTPS_SONDE_URL)
PAGE = PAGE.replace("__HTTPS_SONDE_ATTENDU__", HTTPS_SONDE_ATTENDU)


class Handler(BaseHTTPRequestHandler):
    ROUTES = {
        "/api": (b"fetch-ok", "text/plain"),
        "/download.bin": (b"Bouchaud download proof\n", "application/octet-stream"),
        "/image.png": (PNG, "image/png"),
        "/image.jpg": (JPEG, "image/jpeg"),
        "/nav-un.html": (page_navigation("un"), "text/html; charset=utf-8"),
        "/nav-deux.html": (page_navigation("deux"), "text/html; charset=utf-8"),
        "/telecharger.html": (PAGE_TELECHARGEMENT, "text/html; charset=utf-8"),
    }

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/platform.html":
            body, ctype = PAGE.encode(), "text/html; charset=utf-8"
        elif path in self.ROUTES:
            body, ctype = self.ROUTES[path]
        else:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            print(f"PLATFORM_FIXTURE 404 path={path}", flush=True)
            return

        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        if path == "/download.bin":
            self.send_header("Content-Disposition",
                             'attachment; filename="preuve-bouchaud.bin"')
        self.end_headers()
        self.wfile.write(body)
        print(f"PLATFORM_FIXTURE path={path}", flush=True)

    def log_message(self, fmt, *args):
        print("[platform-fixture]", fmt % args, flush=True)


HTTPServer(("0.0.0.0", 18083), Handler).serve_forever()
