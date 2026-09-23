#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer
import json as _json
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

# LE CATALOGUE D'IMAGES VIT DANS SON PROPRE FICHIER, et c'est voulu : le test
# `test_images_fixtures.py` verifie que les pixels qu'il annonce sont bien
# ceux que les fichiers contiennent. Un banc qui se trompe accuse le port a
# la place de son propre encodeur.
import os as _os
import sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import images_fixtures as _img

# Compteur de requetes PAR FICHIER. Il repond a une question que le journal du
# navigateur ne peut pas trancher : une image reservie l'a-t-elle ete depuis
# la memoire, ou retelechargee ?
from collections import defaultdict as _defaultdict
REQUETES = _defaultdict(int)

CATALOGUE_PAGE = [
    {
        "id": e["id"], "nom": e["nom"], "fichier": e["fichier"],
        "url": "/img/" + e["fichier"], "largeur": e["largeur"], "hauteur": e["hauteur"],
        "tolerance": e["tolerance"], "pixels": e["pixels"],
        "alpha": e.get("alpha_attendu"),
    }
    for e in _img.CATALOGUE
]

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


  // ====================================================================
  // LE BANC D'IMAGES
  //
  // Une image n'est PAS « OK » parce que `onload` s'est declenche. Le defaut
  // observe sur la machine physique est exactement celui-la : HTTP 200,
  // ImageDecoder qui tourne, `onload` qui part -- et un rectangle BLANC a
  // l'ecran.
  //
  // Chaque image est donc dessinee dans un canvas a sa taille naturelle, et
  // les pixels sont RELUS. C'est la seule etape qui distingue « le decodeur
  // a rendu succes » de « les bons octets sont arrives jusqu'a un bitmap ».
  // ====================================================================
  const lisPixels = (image, largeur, hauteur) => {
    const c = document.createElement("canvas");
    c.width = largeur;
    c.height = hauteur;
    const ctx = c.getContext("2d");
    ctx.clearRect(0, 0, largeur, hauteur);
    ctx.drawImage(image, 0, 0);
    return ctx;
  };

  const chargeImage = (url, garde) => new Promise((resolve, reject) => {
    const image = new Image();
    const timer = setTimeout(() => reject(new Error("timeout")), garde);
    image.onload = () => { clearTimeout(timer); resolve(image); };
    image.onerror = () => { clearTimeout(timer); reject(new Error("erreur de chargement")); };
    image.src = url;
  });

  const essaieImage = async (entree) => {
    const t0 = performance.now();
    const dit = (etape, extra) =>
      console.log(`HOST_IMAGE id=${entree.id} nom=${entree.nom} etape=${etape}`
        + ` ms=${Math.round(performance.now() - t0)}${extra ? " " + extra : ""}`);
    try {
      dit("demande", `url=${entree.url}`);
      const image = await chargeImage(entree.url, 30000);
      const largeur = image.naturalWidth;
      const hauteur = image.naturalHeight;
      dit("charge", `dimensions=${largeur}x${hauteur}`);

      if (largeur === 0 || hauteur === 0) {
        dit("echec", "raison=dimensions_nulles");
        return false;
      }
      if (entree.largeur !== null && (largeur !== entree.largeur || hauteur !== entree.hauteur)) {
        dit("echec", `raison=dimensions attendu=${entree.largeur}x${entree.hauteur}`);
        return false;
      }

      // L'ALPHA SE VERIFIE A PART, et pas par curiosite : le canvas
      // PREMULTIPLIE. Un pixel blanc a moitie transparent n'en ressort pas
      // blanc, si bien qu'une assertion de couleur echouerait sur une image
      // pourtant correctement decodee. C'est le canal alpha qui porte
      // l'information, et c'est lui qu'on lit.
      if (entree.alpha) {
        const ctx = lisPixels(image, largeur, hauteur);
        const [ax, ay, aa] = entree.alpha;
        const d = ctx.getImageData(ax, ay, 1, 1).data;
        if (Math.abs(d[3] - aa) > 2) {
          dit("echec", `raison=alpha en=(${ax},${ay}) lu=${d[3]} attendu=${aa}`);
          return false;
        }
        dit("ok", `verifie=alpha=${d[3]}`);
        return true;
      }

      if (entree.pixels.length === 0) {
        // Amont n'affirme que le decodage pour cette entree ; le banc ne
        // revendique donc que les dimensions, et le DIT.
        dit("ok", "verifie=dimensions_seules");
        return true;
      }

      const ctx = lisPixels(image, largeur, hauteur);
      for (const p of entree.pixels) {
        const d = ctx.getImageData(p[0], p[1], 1, 1).data;
        const ecart = Math.max(Math.abs(d[0] - p[2]), Math.abs(d[1] - p[3]), Math.abs(d[2] - p[4]));
        if (ecart > entree.tolerance) {
          dit("echec", `raison=pixel en=(${p[0]},${p[1]}) lu=${d[0]},${d[1]},${d[2]}`
            + ` attendu=${p[2]},${p[3]},${p[4]} ecart=${ecart}`);
          return false;
        }
      }
      dit("ok", `verifie=${entree.pixels.length}_pixels`);
      return true;
    } catch (e) {
      dit("echec", `raison=${e}`);
      return false;
    }
  };

  // LE CATALOGUE EST INJECTE PAR LE SERVEUR : une seule source de verite,
  // partagee avec le test hote qui verifie que le banc lui-meme est juste.
  const CATALOGUE = ''' + _json.dumps(CATALOGUE_PAGE) + ''';

  let imagesOK = 0;
  for (const entree of CATALOGUE) {
    if (await essaieImage(entree)) imagesOK++;
  }
  console.log(`HOST_IMAGES codecs ok=${imagesOK} sur=${CATALOGUE.length}`);

  // ====================================================================
  // LES CHEMINS CSS, qui ne passent pas par `new Image()`.
  //
  // `background-image` est servi par un autre chemin du moteur que `<img>` :
  // il passe par le style calcule et le display list, pas par HTMLImageElement.
  // Un port peut faire marcher l'un et pas l'autre, et c'est precisement le
  // genre de trou qui laisse une page « a moitie » illustree.
  // ====================================================================
  const CIBLE = CATALOGUE[0];
  let fondOK = false;
  try {
    const boite = document.createElement("div");
    boite.style.width = "32px";
    boite.style.height = "32px";
    boite.style.backgroundImage = `url(${CIBLE.url})`;
    boite.style.backgroundSize = "32px 32px";
    document.body.appendChild(boite);
    // Laisser un tour de boucle au moteur pour charger la ressource de style.
    await new Promise(r => setTimeout(r, 1500));
    const calcule = getComputedStyle(boite).backgroundImage;
    fondOK = calcule && calcule !== "none" && calcule.includes(CIBLE.fichier);
    console.log(fondOK ? `HOST_IMAGE_FOND OK calcule=${calcule}`
                       : `HOST_IMAGE_FOND FAIL calcule=${calcule}`);
  } catch (e) {
    console.log("HOST_IMAGE_FOND FAIL " + e);
  }

  // REDIMENSIONNEMENT : l'image doit rester juste apres mise a l'echelle.
  //
  // Le damier est choisi expres : une couleur unie resterait juste meme si
  // le moteur dessinait n'importe quoi a l'echelle. Un damier 8x8 mis a
  // l'echelle 2x place du rouge en (8,8) et du bleu en (24,8).
  let echelleOK = false;
  try {
    const image = await chargeImage(CIBLE.url, 30000);
    const c = document.createElement("canvas");
    c.width = 64; c.height = 64;
    const ctx = c.getContext("2d");
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(image, 0, 0, 64, 64);
    const a = ctx.getImageData(8, 8, 1, 1).data;
    const b = ctx.getImageData(24, 8, 1, 1).data;
    echelleOK = a[0] > 200 && a[2] < 60 && b[2] > 200 && b[0] < 60;
    console.log(echelleOK ? "HOST_IMAGE_ECHELLE OK 2x"
      : `HOST_IMAGE_ECHELLE FAIL a=${a[0]},${a[1]},${a[2]} b=${b[0]},${b[1]},${b[2]}`);
  } catch (e) {
    console.log("HOST_IMAGE_ECHELLE FAIL " + e);
  }

  // REUTILISATION : la meme URL, une seconde fois.
  //
  // Le cache disque est desactive au lancement ; ce que ce point mesure est
  // donc la reutilisation EN MEMOIRE, pas le cache HTTP. Le serveur compte
  // ses requetes et le banc lit ce compteur -- c'est la seule facon de
  // distinguer « reservi depuis la memoire » de « retelecharge ».
  let reutiliseOK = false;
  try {
    const avant = await (await fetch("/compteur?fichier=" + CIBLE.fichier)).text();
    await chargeImage(CIBLE.url, 30000);
    const apres = await (await fetch("/compteur?fichier=" + CIBLE.fichier)).text();
    reutiliseOK = true;
    console.log(`HOST_IMAGE_REUTILISE requetes_avant=${avant.trim()} apres=${apres.trim()}`);
  } catch (e) {
    console.log("HOST_IMAGE_REUTILISE FAIL " + e);
  }

  const imagesToutesOK = imagesOK === CATALOGUE.length && fondOK && echelleOK;
  console.log(`HOST_IMAGES_${imagesToutesOK ? "OK" : "FAIL"} codecs=${imagesOK}/${CATALOGUE.length}`
    + ` fond=${fondOK ? 1 : 0} echelle=${echelleOK ? 1 : 0}`);

  // ====================================================================
  // LE BANC JAVASCRIPT
  //
  // `HOST_CANVAS OK` prouvait qu'un peu de JS tourne. Ca ne dit rien de la
  // boucle d'evenements, des micro-taches, des minuteries ni du DOM -- et
  // c'est precisement ce dont une page reelle depend.
  //
  // Chaque comportement est EXECUTE et son resultat verifie. Une epreuve qui
  // se contenterait de tester l'existence de `setTimeout` passerait sur un
  // moteur qui ne le declenche jamais.
  // ====================================================================
  const epreuves = [];
  const essai = (nom, fn) => epreuves.push([nom, fn]);

  essai("dom-mutation", async () => {
    const hote = document.createElement("div");
    document.body.appendChild(hote);
    const fils = document.createElement("span");
    fils.textContent = "bouchaud";
    hote.appendChild(fils);
    if (hote.children.length !== 1) throw new Error("appendChild");
    if (hote.textContent !== "bouchaud") throw new Error("textContent");
    fils.setAttribute("data-x", "42");
    if (fils.getAttribute("data-x") !== "42") throw new Error("setAttribute");
    hote.removeChild(fils);
    if (hote.children.length !== 0) throw new Error("removeChild");
    if (!document.querySelector("body")) throw new Error("querySelector");
  });

  essai("click-event", async () => {
    const bouton = document.createElement("button");
    document.body.appendChild(bouton);
    let vu = 0;
    bouton.addEventListener("click", () => { vu++; });
    bouton.click();
    if (vu !== 1) throw new Error(`recu ${vu} clic(s)`);
  });

  essai("event-dispatch-bubble", async () => {
    const parent = document.createElement("div");
    const enfant = document.createElement("div");
    parent.appendChild(enfant);
    document.body.appendChild(parent);
    let ordre = [];
    parent.addEventListener("essai", () => ordre.push("parent"));
    enfant.addEventListener("essai", () => ordre.push("enfant"));
    enfant.dispatchEvent(new Event("essai", { bubbles: true }));
    if (ordre.join(",") !== "enfant,parent") throw new Error("ordre " + ordre.join(","));
  });

  essai("keydown", async () => {
    const champ = document.createElement("input");
    document.body.appendChild(champ);
    let touche = null;
    champ.addEventListener("keydown", e => { touche = e.key; });
    champ.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    if (touche !== "a") throw new Error("touche=" + touche);
  });

  essai("setTimeout", async () => {
    const t0 = performance.now();
    await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("jamais declenche")), 10000);
      setTimeout(() => { clearTimeout(garde); res(); }, 50);
    });
    // LE DELAI DOIT AVOIR ETE ATTENDU. Un moteur qui declencherait
    // immediatement passerait une verification de simple existence.
    if (performance.now() - t0 < 20) throw new Error("declenche trop tot");
  });

  essai("setInterval", async () => {
    let n = 0;
    await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("moins de 3 tours")), 10000);
      const id = setInterval(() => {
        if (++n >= 3) { clearInterval(id); clearTimeout(garde); res(); }
      }, 20);
    });
    if (n !== 3) throw new Error("tours=" + n);
  });

  essai("promise", async () => {
    const v = await Promise.resolve(7);
    if (v !== 7) throw new Error("resolve");
    const tous = await Promise.all([Promise.resolve(1), 2, new Promise(r => setTimeout(() => r(3), 10))]);
    if (tous.join(",") !== "1,2,3") throw new Error("all " + tous.join(","));
    try { await Promise.reject(new Error("attendu")); throw new Error("reject non propage"); }
    catch (e) { if (e.message !== "attendu") throw e; }
  });

  essai("queueMicrotask-ordre", async () => {
    // L'ORDRE EST LE TEST. Les micro-taches doivent passer AVANT la
    // prochaine macro-tache ; un moteur qui les confondrait executerait le
    // `setTimeout` en premier et une page reelle verrait des etats a moitie
    // appliques.
    const ordre = [];
    await new Promise(res => {
      setTimeout(() => { ordre.push("macro"); res(); }, 0);
      queueMicrotask(() => ordre.push("micro"));
    });
    if (ordre.join(",") !== "micro,macro") throw new Error("ordre " + ordre.join(","));
  });

  essai("async-await", async () => {
    const tarde = (v, ms) => new Promise(r => setTimeout(() => r(v), ms));
    const a = await tarde(1, 10);
    const b = await tarde(2, 10);
    if (a + b !== 3) throw new Error("somme " + (a + b));
  });

  essai("exception", async () => {
    try { null.x; throw new Error("pas de TypeError"); }
    catch (e) { if (!(e instanceof TypeError)) throw new Error("type " + e); }
    try { JSON.parse("{"); throw new Error("pas de SyntaxError"); }
    catch (e) { if (!(e instanceof SyntaxError)) throw new Error("type " + e); }
  });

  essai("json", async () => {
    const o = { a: 1, b: [2, 3], c: { d: "e" }, f: null, g: true };
    const t = JSON.stringify(o);
    const r = JSON.parse(t);
    if (JSON.stringify(r) !== t) throw new Error("aller-retour");
    if (r.b[1] !== 3 || r.c.d !== "e" || r.f !== null || r.g !== true) throw new Error("valeurs");
  });

  essai("url", async () => {
    const u = new URL("/a/b?x=1&y=2#z", location.href);
    if (u.pathname !== "/a/b") throw new Error("pathname " + u.pathname);
    if (u.searchParams.get("y") !== "2") throw new Error("searchParams");
    if (u.hash !== "#z") throw new Error("hash " + u.hash);
  });

  essai("fetch-texte", async () => {
    const r = await fetch("/echo?v=bouchaud");
    if (!r.ok) throw new Error("statut " + r.status);
    const t = await r.text();
    if (t.trim() !== "bouchaud") throw new Error("corps " + t);
  });

  essai("fetch-json", async () => {
    const r = await fetch("/json");
    const j = await r.json();
    if (j.nom !== "bouchaud" || j.n !== 42) throw new Error("json " + JSON.stringify(j));
  });

  essai("fetch-404", async () => {
    // UN 404 DOIT ETRE UN 404, pas une exception. Un moteur qui rejetterait
    // la promesse ferait echouer toute page qui sonde une ressource.
    const r = await fetch("/absent-volontairement");
    if (r.status !== 404) throw new Error("statut " + r.status);
    if (r.ok) throw new Error("ok=true sur un 404");
  });

  essai("iframe-js", async () => {
    const cadre = document.createElement("iframe");
    cadre.src = "/frame-js.html";
    document.body.appendChild(cadre);
    await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("timeout")), 20000);
      cadre.onload = () => { clearTimeout(garde); res(); };
      cadre.onerror = () => { clearTimeout(garde); rej(new Error("erreur")); };
    });
    // Le JS du cadre ecrit son resultat dans son propre document : le lire
    // prouve a la fois qu'il s'est execute ET que le parent peut l'atteindre.
    const doc = cadre.contentDocument;
    if (!doc) throw new Error("contentDocument inaccessible");
    const marque = doc.getElementById("marque");
    if (!marque) throw new Error("le script du cadre n'a rien ecrit");
    if (marque.textContent !== "cadre-ok") throw new Error("marque " + marque.textContent);
  });

  essai("postMessage-cadre", async () => {
    const cadre = document.createElement("iframe");
    cadre.src = "/frame-js.html";
    document.body.appendChild(cadre);
    await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("timeout chargement")), 20000);
      cadre.onload = () => { clearTimeout(garde); res(); };
    });
    const reponse = await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("pas de reponse")), 20000);
      window.addEventListener("message", function ecoute(e) {
        if (e.data && e.data.pong) {
          clearTimeout(garde);
          window.removeEventListener("message", ecoute);
          res(e.data.pong);
        }
      });
      cadre.contentWindow.postMessage({ ping: "bouchaud" }, "*");
    });
    if (reponse !== "bouchaud") throw new Error("reponse " + reponse);
  });

  // requestAnimationFrame EST MESURE A PART, et ce n'est pas une facilite.
  //
  // Les autres epreuves interrogent le moteur JS et le DOM ; celle-ci
  // interroge la BOUCLE DE PRESENTATION. Un navigateur sans compositeur actif
  // -- un Chromium `headless_shell` en temps virtuel, par exemple -- ne la
  // declenche jamais, et la compter avec les autres ferait croire a un defaut
  // du moteur la ou il n'y a pas d'ecran.
  //
  // Elle garde donc sa propre ligne. Sous Ladybird, qui compose reellement,
  // une absence de declenchement est un VRAI defaut et se lit isolement.
  let rafOK = false;
  try {
    const t = await new Promise((res, rej) => {
      const garde = setTimeout(() => rej(new Error("jamais declenche")), 15000);
      requestAnimationFrame(x => { clearTimeout(garde); res(x); });
    });
    rafOK = typeof t === "number";
    console.log(`HOST_RAF ${rafOK ? "OK" : "FAIL"} horodatage=${typeof t}`);
  } catch (e) {
    console.log(`HOST_RAF FAIL raison=${e && e.message ? e.message : e}`);
  }

  let jsOK = 0;
  for (const [nom, fn] of epreuves) {
    const t0 = performance.now();
    try {
      await fn();
      jsOK++;
      console.log(`HOST_JS nom=${nom} etat=OK ms=${Math.round(performance.now() - t0)}`);
    } catch (e) {
      console.log(`HOST_JS nom=${nom} etat=FAIL ms=${Math.round(performance.now() - t0)} raison=${e && e.message ? e.message : e}`);
    }
  }
  console.log(`HOST_JS_${jsOK === epreuves.length ? "OK" : "FAIL"} ok=${jsOK}/${epreuves.length}`);

  // ====================================================================
  // LES WORKERS PASSENT EN DERNIER, ET CE N'EST PAS UN DETAIL D'ORDRE.
  //
  // Ils etaient en deuxieme position. Un worker qui ne repond pas coute
  // SOIXANTE SECONDES par origine, et il y en a deux : deux minutes
  // pendant lesquelles rien d'autre ne s'execute.
  //
  // La mesure du run 35825013435 montre ce que cela coute vraiment :
  //
  //     T+205 s  HOST_CANVAS OK
  //     T+255 s  worker http : echec ms=61070 raison=timeout
  //     T+256 s  worker blob : construction
  //     T+267 s  la machine s'eteint
  //
  // Les onze images et les dix-sept epreuves JavaScript n'ont jamais eu
  // leur tour. Un banc qui ne rend AUCUN resultat parce que son epreuve
  // la plus lente est passee en premier ne mesure rien.
  //
  // L'ordre est donc : ce qui est rapide et informatif d'abord, ce qui
  // peut expirer en dernier. Le verdict global reste le meme -- il a
  // seulement des chiffres a rapporter quand le worker echoue.
  // ====================================================================
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


  console.log(`HOST_SMOKE_${canvasOK && workerOK && imageOK && frameOK ? "OK" : "FAIL"} canvas=${canvasOK ? 1 : 0} worker=${workerOK ? 1 : 0} image=${imageOK ? 1 : 0} frame=${frameOK ? 1 : 0}`
    + ` images=${imagesOK}/${CATALOGUE.length} js=${jsOK}/${epreuves.length} raf=${rafOK ? 1 : 0}`);
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
        if path in _img.PAR_CHEMIN:
            entree = _img.PAR_CHEMIN[path]
            REQUETES[entree["fichier"]] += 1
            self.send_response(200)
            self.send_header("Content-Type", entree["mime"])
            self.send_header("Content-Length", str(len(entree["octets"])))
            # PAS DE CACHE HTTP : le lancement passe `--disable-http-disk-cache`,
            # et annoncer un cache que le navigateur ne tient pas rendrait le
            # compteur de requetes illisible.
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(entree["octets"])
            print(f"BROWSER_HOST_FIXTURE_IMAGE id={entree['id']} nom={entree['nom']} "
                  f"octets={len(entree['octets'])} requetes={REQUETES[entree['fichier']]}", flush=True)
            return
        if path == "/favicon.ico":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(_img.FAVICON)))
            self.end_headers()
            self.wfile.write(_img.FAVICON)
            print("BROWSER_HOST_FIXTURE_FAVICON_OK", flush=True)
            return
        if path == "/compteur":
            from urllib.parse import parse_qs, urlparse
            fichier = parse_qs(urlparse(self.path).query).get("fichier", [""])[0]
            corps = str(REQUETES.get(fichier, 0)).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(corps)))
            self.end_headers()
            self.wfile.write(corps)
            return
        if path == "/echo":
            from urllib.parse import parse_qs, urlparse
            corps = parse_qs(urlparse(self.path).query).get("v", [""])[0].encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(corps)))
            self.end_headers()
            self.wfile.write(corps)
            return
        if path == "/json":
            corps = b'{"nom":"bouchaud","n":42}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(corps)))
            self.end_headers()
            self.wfile.write(corps)
            return
        if path == "/frame-js.html":
            corps = (b"<!doctype html><meta charset=utf-8><body><span id=marque></span>"
                     b"<script>"
                     b"document.getElementById('marque').textContent='cadre-ok';"
                     b"onmessage=function(e){if(e.data&&e.data.ping)"
                     b"e.source.postMessage({pong:e.data.ping},'*');};"
                     b"</script></body>")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(corps)))
            self.end_headers()
            self.wfile.write(corps)
            print("BROWSER_HOST_FIXTURE_FRAMEJS_OK", flush=True)
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
