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

# LA MIRE DE SURFACE : trois aplats de couleur PURE.
#
# BOUCHAUD_C29_SURFACE_PRESENTEE
#
# Tout ce qui precede lit les pixels dans un CANVAS. Cela prouve que le
# decodeur a rendu les bons octets -- pas qu'ils arrivent a l'ecran. Le
# defaut observe sur la machine physique est precisement celui-la : une
# image qui se decode et reste BLANCHE dans la frame presentee.
#
# Les trois couleurs sont pures et saturees expres. Le chrome d'un
# navigateur est fait de gris, de blancs et de bleus d'accentuation ; un
# rouge #FF0000 ou un vert #00FF00 exact n'y apparait pas par accident. Le
# controleur peut donc COMPTER les pixels exacts dans une capture d'ecran
# sans avoir a connaitre la position de la mire -- ce qui le rend insensible
# a la hauteur de la barre d'adresse, au decalage de la fenetre et a
# l'echelle.
MIRE = {
    "/mire/rouge.png": _img.png_rgba(64, 64, lambda x, y: (255, 0, 0, 255)),
    "/mire/vert.png": _img.png_rgba(64, 64, lambda x, y: (0, 255, 0, 255)),
    "/mire/bleu.png": _img.png_rgba(64, 64, lambda x, y: (0, 0, 255, 255)),
}

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
  // il passe par le style calcule et le display list, pas par
  // HTMLImageElement. Un port peut faire marcher l'un et pas l'autre, et
  // c'est le genre de trou qui laisse une page « a moitie » illustree.
  //
  // BOUCHAUD_C29_FOND_PAR_PIXEL
  //
  // La version precedente se contentait de `getComputedStyle(...).backgroundImage`
  // et verifiait que la chaine contenait le nom du fichier. Cela ne prouve
  // RIEN sur le rendu : le style calcule rend l'URL telle qu'elle a ete
  // posee, que la ressource ait ete chargee, decodee, ou meme trouvee. Un
  // 404 donne exactement la meme chaine.
  //
  // Le fond est donc verifie par ses PIXELS. Le seul moyen d'en lire dans
  // une page est de repasser par un canvas : on dessine un `<img>` de la
  // MEME ressource et l'on compare. Ce n'est toujours pas la surface
  // presentee -- c'est `HOST_SURFACE` plus bas qui s'en charge, depuis la
  // capture d'ecran de la machine -- mais cela ferme le trou du « la chaine
  // contient le bon nom ».
  // ====================================================================
  const CIBLE = CATALOGUE[0];
  let fondOK = false;
  try {
    const boite = document.createElement("div");
    boite.id = "fond-epreuve";
    boite.style.width = "32px";
    boite.style.height = "32px";
    boite.style.backgroundImage = `url(${CIBLE.url})`;
    boite.style.backgroundSize = "32px 32px";
    boite.style.backgroundRepeat = "no-repeat";
    document.body.appendChild(boite);
    await new Promise(r => setTimeout(r, 1500));

    const calcule = getComputedStyle(boite).backgroundImage;
    const styleCite = calcule && calcule !== "none" && calcule.includes(CIBLE.fichier);

    // LA RESSOURCE DU FOND DOIT ETRE REELLEMENT DECODABLE, et donner les
    // pixels que le catalogue annonce. Le style seul ne le dit pas.
    let pixelsJustes = false;
    if (styleCite && CIBLE.pixels.length > 0) {
      const image = await chargeImage(CIBLE.url, 30000);
      const ctx = lisPixels(image, image.naturalWidth, image.naturalHeight);
      pixelsJustes = CIBLE.pixels.every(p => {
        const d = ctx.getImageData(p[0], p[1], 1, 1).data;
        return Math.max(Math.abs(d[0]-p[2]), Math.abs(d[1]-p[3]), Math.abs(d[2]-p[4])) <= CIBLE.tolerance;
      });
    }
    fondOK = styleCite && pixelsJustes;
    console.log(`HOST_IMAGE_FOND ${fondOK ? "OK" : "FAIL"}`
      + ` style_cite=${styleCite ? 1 : 0} pixels_justes=${pixelsJustes ? 1 : 0}`
      + ` calcule=${calcule}`);
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
  // BOUCHAUD_C29_REUTILISE_COMPARE
  //
  // La version precedente posait `reutiliseOK = true` juste apres avoir lu
  // les deux compteurs, sans JAMAIS les comparer, et le resultat n'entrait
  // pas dans le verdict. C'etait une ligne de journal deguisee en epreuve :
  // elle ne pouvait pas echouer, donc elle ne mesurait rien.
  //
  // Elle compare maintenant, et elle entre dans le verdict. Ce qu'elle
  // affirme est borne et honnete : le cache disque est desactive au
  // lancement (`--disable-http-disk-cache`), donc ce qui est teste ici est
  // la reutilisation EN MEMOIRE d'une ressource deja chargee dans la meme
  // page. Une requete de plus veut dire que le moteur est retourne au
  // reseau pour une image qu'il tenait deja.
  let reutiliseOK = false;
  try {
    // LA RESSOURCE DE CETTE EPREUVE EST CACHABLE, ET C'EST INDISPENSABLE.
    //
    // Les images du catalogue sont servies `Cache-Control: no-store`, pour
    // que le compteur de requetes reste lisible. Avec `no-store`, un
    // navigateur CONFORME doit retourner au reseau a chaque acces : exiger
    // zero requete supplementaire sur ces images-la reviendrait a exiger
    // qu'il viole la norme. La premiere version le faisait, et Chromium --
    // qui a raison -- rendait `supplement=1`.
    //
    // `/reutilise.png` est donc servie avec un `max-age` ordinaire. Ce que
    // l'epreuve affirme devient alors vrai et verifiable : une ressource
    // cachable deja chargee dans ce document ne doit pas etre redemandee.
    const lisCompteur = async () =>
      parseInt((await (await fetch("/compteur?fichier=reutilise.png")).text()).trim(), 10);
    await chargeImage("/reutilise.png", 30000);
    const avant = await lisCompteur();
    await chargeImage("/reutilise.png", 30000);
    const apres = await lisCompteur();
    const supplement = apres - avant;
    reutiliseOK = Number.isFinite(avant) && Number.isFinite(apres) && supplement === 0;
    console.log(`HOST_IMAGE_REUTILISE ${reutiliseOK ? "OK" : "FAIL"}`
      + ` requetes_avant=${avant} apres=${apres} supplement=${supplement}`);
  } catch (e) {
    console.log("HOST_IMAGE_REUTILISE FAIL " + e);
  }

  // ====================================================================
  // LA SURFACE PRESENTEE, et pourquoi elle ne peut pas se lire d'ici.
  //
  // BOUCHAUD_C29_SURFACE_PRESENTEE
  //
  // Tout ce qui precede lit des pixels dans un CANVAS. Cela prouve que le
  // decodeur rend les bons octets. Cela ne prouve PAS qu'ils arrivent a
  // l'ecran -- et le defaut observe sur la machine physique est exactement
  // celui-la : une image qui se decode et reste BLANCHE dans la frame
  // presentee.
  //
  // Aucune interface de page ne permet de relire la surface reellement
  // composee. La verification se fait donc DEHORS : la page pose une mire
  // de trois aplats purs, annonce qu'elle est posee, et le banc prend une
  // capture de l'ecran de la machine par le moniteur QEMU. C'est
  // `tools/ci/analyse-surface-mire.py` qui compte les pixels.
  //
  // Les trois aplats empruntent DEUX chemins differents a dessein :
  //
  //     rouge   <img>              -- HTMLImageElement
  //     vert    background-image   -- style calcule et display list
  //     bleu    <img> mis a l'echelle
  //
  // Si le rouge arrive et pas le vert, c'est le chemin CSS qui ne peint
  // pas ; l'inverse accuse HTMLImageElement.
  // ====================================================================
  try {
    const mire = document.createElement("div");
    mire.id = "mire";
    mire.style.cssText = "position:fixed;left:0;top:0;z-index:2147483647;"
      + "background:#000;padding:0;margin:0;line-height:0;font-size:0";
    const rouge = new Image();
    rouge.src = "/mire/rouge.png";
    rouge.width = 64; rouge.height = 64;
    rouge.style.cssText = "display:inline-block;vertical-align:top";
    const vert = document.createElement("div");
    vert.style.cssText = "display:inline-block;vertical-align:top;width:64px;height:64px;"
      + "background-image:url(/mire/vert.png);background-size:64px 64px;background-repeat:no-repeat";
    const bleu = new Image();
    bleu.src = "/mire/bleu.png";
    bleu.width = 128; bleu.height = 64;
    bleu.style.cssText = "display:inline-block;vertical-align:top";
    mire.appendChild(rouge);
    mire.appendChild(vert);
    mire.appendChild(bleu);
    document.body.appendChild(mire);

    await Promise.all([rouge, bleu].map(im => new Promise(res => {
      if (im.complete && im.naturalWidth > 0) return res();
      im.onload = res; im.onerror = res;
    })));
    // LA BORNE QUE LE BANC PEUT CORRELER A UNE TRAME.
    //
    // `HOST_SURFACE_MIRE_POSEE` arrive APRES deux `requestAnimationFrame` et
    // deux secondes d'attente : il dit que la page a fini d'esperer, pas qu'une
    // trame contenant la mire a ete composee. Le banc capturait donc un ecran
    // sans savoir ce qu'il capturait.
    //
    // Cette ligne-ci est emise a l'instant ou la mire est dans le DOM et ou ses
    // images sont DECODEES. Toute trame numerotee posterieure la contient
    // necessairement ; c'est cette trame-la que le banc attend avant de
    // demander le `screendump`.
    console.log("HOST_SURFACE_MIRE_INSEREE largeur=" + mire.offsetWidth + " hauteur=" + mire.offsetHeight);
    // DEUX TRAMES DE BATTEMENT, MAIS BORNEES.
    //
    // Attendre deux `requestAnimationFrame` laisse au compositeur le temps de
    // presenter la mire. Mais un navigateur sans boucle de presentation ne
    // les declenche JAMAIS -- un Chromium `headless_shell` en temps virtuel,
    // par exemple. La premiere version attendait sans garde et la page s'y
    // arretait pour toujours : le banc n'a rendu aucune ligne apres
    // `HOST_IMAGE_REUTILISE`.
    //
    // Une epreuve ne doit pas pouvoir suspendre le banc. La course contre un
    // delai rend la borne explicite, et le journal dit laquelle a gagne.
    const deuxTrames = new Promise(r =>
      requestAnimationFrame(() => requestAnimationFrame(() => r("trames"))));
    const battement = await Promise.race([
      deuxTrames,
      new Promise(r => setTimeout(() => r("delai"), 5000)),
    ]);
    await new Promise(r => setTimeout(r, 2000));
    console.log(`HOST_SURFACE_MIRE_POSEE battement=${battement}` + " rouge=img vert=background-image bleu=img_echelle"
      + ` attendu_min_pixels=2048 largeur=${mire.offsetWidth} hauteur=${mire.offsetHeight}`);
  } catch (e) {
    console.log("HOST_SURFACE_MIRE_POSEE FAIL " + e);
  }


  // LA REUTILISATION ENTRE DANS LE VERDICT.
  //
  // Elle n'y entrait pas : son resultat etait calcule puis ignore. Une
  // epreuve dont le resultat n'a aucune consequence n'en est pas une.
  const imagesToutesOK = imagesOK === CATALOGUE.length && fondOK && echelleOK && reutiliseOK;
  console.log(`HOST_IMAGES_${imagesToutesOK ? "OK" : "FAIL"} codecs=${imagesOK}/${CATALOGUE.length}`
    + ` fond=${fondOK ? 1 : 0} echelle=${echelleOK ? 1 : 0} reutilise=${reutiliseOK ? 1 : 0}`);

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
  // BOUCHAUD_C29_WORKER_MATRICE
  //
  // # Ce que le « vert » precedent cachait
  //
  //     HOST_WORKER_BLOB OK pong
  //     HOST_WORKER_HTTP FAIL
  //     HOST_WORKER      OK pong      <- faux vert
  //
  // Le jalon global etait `workerHttpOK || workerBlobOK`. Une origine sur
  // deux suffisait donc a le satisfaire, et la CI passait au vert avec le
  // chargement de script par le reseau CASSE. Un jalon qui peut etre vert
  // alors qu'une moitie de la fonction ne marche pas ne mesure pas cette
  // fonction.
  //
  // Les trois verdicts sont desormais STRICTEMENT separes, et le global est
  // une CONJONCTION :
  //
  //     HOST_WORKER_HTTP    le script vient du reseau
  //     HOST_WORKER_BLOB    le script vient d'un blob: de l'agent
  //     HOST_WORKER_GLOBAL  les DEUX
  //
  // # La matrice A/B, et la question qu'elle tranche
  //
  // Le releve precedent est ambigu. L'ordre etait http puis blob :
  //
  //     http  premier   echec a 61 s (garde), reponse observee vers 148 s
  //     blob  second    reponse immediate
  //
  // Deux lectures s'opposent, et elles n'ont pas le meme remede :
  //
  //   ORIGINE     le chargement par le RESEAU est lent -- ResourceLoader,
  //               RequestServer, DNS, boucle de fetch.
  //   DEMARRAGE   le PREMIER WebWorker paie un demarrage a froid -- edition
  //               de liens du binaire, ICU, fontconfig -- et son origine n'y
  //               est pour rien.
  //
  // Quatre workers, dans cet ordre, les distinguent en une seule execution :
  //
  //     blob_1   http_1   blob_2   http_2
  //
  //   blob_1 lent, les trois autres rapides   -> DEMARRAGE A FROID
  //   http_1 ET http_2 lents, blob_* rapides  -> ORIGINE
  //
  // L'ordre est INVERSE par rapport au releve precedent, expres : si la
  // lenteur suit la premiere position au lieu de suivre `http`, elle est
  // du demarrage et non de l'origine.
  //
  // # Le plafond n'est pas le budget, et c'est ce qui evite de masquer
  //
  // Elargir la garde a 180 s rendrait le banc vert sur un worker qui met
  // deux minutes et demie -- ce serait masquer le defaut, pas le mesurer.
  //
  // La garde sert a OBSERVER : elle est assez large pour laisser la reponse
  // de 148 s arriver et etre horodatee. Le BUDGET, lui, decide du verdict.
  // Un worker qui repond en 148 s rend donc `FAIL ms=148000`, ce qui est a
  // la fois un echec et une mesure.
  const GARDE_WORKER_MS = 200000;
  const BUDGET_WORKER_MS = 30000;

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
      const ms = Math.round(performance.now() - t0);
      const ok = answer === "pong";
      dit(ok ? "pong" : "reponse_inattendue", `reponse=${answer}`);
      return { ok, ms };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      dit("echec", `raison=${e}`);
      return { ok: false, ms };
    }
  };

  const urlBlob = () => URL.createObjectURL(
    new Blob(['onmessage = e => { if (e.data === "ping") postMessage("pong"); };'],
             { type: "text/javascript" }));

  const MATRICE = [
    ["blob", urlBlob],
    ["http", () => "/worker.js"],
    ["blob", urlBlob],
    ["http", () => "/worker.js"],
  ];

  const releves = [];
  for (let rang = 0; rang < MATRICE.length; rang++) {
    const [origine, fabrique] = MATRICE[rang];
    let r;
    try {
      r = await essaieWorker(`${origine}_${rang + 1}`, fabrique(), GARDE_WORKER_MS);
    } catch (e) {
      r = { ok: false, ms: -1 };
      console.log(`HOST_WORKER_ETAPE origine=${origine}_${rang + 1} etape=echec raison=${e}`);
    }
    releves.push({ origine, rang: rang + 1, ok: r.ok, ms: r.ms });
    console.log(`HOST_WORKER_AB rang=${rang + 1} origine=${origine}`
      + ` repond=${r.ok ? 1 : 0} ms=${r.ms} budget=${BUDGET_WORKER_MS}`
      + ` verdict=${r.ok && r.ms <= BUDGET_WORKER_MS ? "OK" : "HORS_BUDGET"}`);
  }

  // ====================================================================
  // CAPACITE ET PERFORMANCE SONT DEUX QUESTIONS, ET ON NE LES MELANGE PLUS.
  //
  // BOUCHAUD_C30_CAPACITE_ET_PERFORMANCE
  //
  // La version precedente publiait :
  //
  //     HOST_WORKER_BLOB FAIL ms=68779 repond=1
  //
  // `repond=1` : le worker a REELLEMENT renvoye son pong. La capacite
  // fonctionne. Le marquer `FAIL` parce qu'il a mis soixante-huit secondes
  // transforme un probleme de performance en panne fonctionnelle -- et la
  // difference compte : l'une se corrige en optimisant, l'autre en reparant
  // un chemin casse, et l'on ne cherche pas au meme endroit.
  //
  // Deux familles de verdicts, donc :
  //
  //     _FUNCTIONAL   le pong est-il revenu ? (capacite)
  //     _PERF         est-il revenu dans le budget ? (performance)
  //
  // Un budget de performance ne peut plus eteindre une capacite.
  // ====================================================================
  const premier = o => releves.find(r => r.origine === o) || { ok: false, ms: -1 };
  const http = premier("http");
  const blob = premier("blob");
  const dansBudget = r => r.ok && r.ms >= 0 && r.ms <= BUDGET_WORKER_MS;

  // LA CAPACITE : n'importe quelle tentative de cette origine a-t-elle
  // repondu ? Une origine qui marche a la troisieme tentative marche.
  // CHAQUE LIGNE NE DIT QU'UNE CHOSE, ET LA DIT DE SES PROPRES CHIFFRES.
  //
  // La version precedente publiait :
  //
  //     HOST_WORKER_HTTP_FUNCTIONAL OK pong=0
  //
  // `OK` venait de « une tentative HTTP a repondu » ; `pong=0` venait de la
  // PREMIERE tentative. Deux faits differents sur une meme ligne, dont l'un
  // contredit l'autre a la lecture. Une ligne de verdict qui se contredit
  // elle-meme ne se lit plus : elle se devine.
  //
  //     _FUNCTIONAL      une tentative de cette origine a-t-elle repondu ?
  //     successes=N/M    combien, sur combien de tentatives
  //     _FIRST_ATTEMPT   la PREMIERE a-t-elle repondu ? (independant du budget)
  //     _PERF_FIRST      cette premiere tenait-elle dans le budget ?
  const tentatives = o => releves.filter(r => r.origine === o);
  const reussites = o => tentatives(o).filter(r => r.ok).length;

  const httpFonctionnel = reussites("http") > 0;
  const blobFonctionnel = reussites("blob") > 0;

  for (const [nom, origine, premiere] of [["HTTP", "http", http], ["BLOB", "blob", blob]]) {
    const n = reussites(origine);
    const m = tentatives(origine).length;
    console.log(`HOST_WORKER_${nom}_FUNCTIONAL ${n > 0 ? "OK" : "FAIL"} successes=${n}/${m}`);
    console.log(`HOST_WORKER_${nom}_FIRST_ATTEMPT ${premiere.ok ? "OK" : "FAIL"} ms=${premiere.ms}`);
    console.log(`HOST_WORKER_${nom}_PERF_FIRST ${dansBudget(premiere) ? "OK" : "FAIL"}`
      + ` ms=${premiere.ms} budget=${BUDGET_WORKER_MS}`);
  }

  // LA PERFORMANCE : la PREMIERE tentative de chaque origine, et non la
  // meilleure. C'est elle que vit un utilisateur qui ouvre une page ; prendre
  // la meilleure effacerait le cout du demarrage a froid qu'on mesure.
  workerHttpOK = dansBudget(http);
  workerBlobOK = dansBudget(blob);

  // LA LECTURE DE LA MATRICE, faite ici pour ne pas avoir a la refaire a la
  // main a chaque execution.
  const rapides = releves.filter(r => r.ms >= 0 && r.ms <= BUDGET_WORKER_MS).length;
  let diagnostic = "indetermine";
  if (releves.length === 4) {
    const [a, b, c, d] = releves;
    const lent = r => r.ms < 0 || r.ms > BUDGET_WORKER_MS;
    if (lent(a) && !lent(b) && !lent(c) && !lent(d)) diagnostic = "demarrage_a_froid";
    else if (!lent(a) && !lent(c) && lent(b) && lent(d)) diagnostic = "origine_http";
    else if (releves.every(r => !lent(r))) diagnostic = "aucun_retard";
    else if (releves.every(lent)) diagnostic = "tous_lents";
  }
  console.log(`HOST_WORKER_DIAGNOSTIC ${diagnostic} rapides=${rapides}/${releves.length}`
    + ` ordre=${releves.map(r => `${r.origine}:${r.ms}`).join(",")}`);

  // LE GLOBAL EST UNE CONJONCTION.
  //
  // « le navigateur sait executer du JavaScript dans un processus WebWorker »
  // n'est pas vrai si une seule des deux facons de lui donner son script
  // marche. Le jalon porte desormais un nom qui ne se confond pas avec les
  // deux autres : `HOST_WORKER_GLOBAL`.
  // LE GLOBAL FONCTIONNEL EST UNE CONJONCTION DE CAPACITES.
  //
  // « le navigateur sait executer du JavaScript dans un processus WebWorker »
  // n'est pas vrai si une seule des deux facons de lui donner son script
  // marche. C'est ce jalon-la que la CI exige.
  const fonctionnelGlobal = httpFonctionnel && blobFonctionnel;
  console.log(`HOST_WORKER_FUNCTIONAL_GLOBAL ${fonctionnelGlobal ? "OK pong" : "FAIL"}`
    + ` http=${httpFonctionnel ? 1 : 0} blob=${blobFonctionnel ? 1 : 0}`);

  // LE DEMARRAGE A FROID A SON PROPRE VERDICT, separe des deux origines.
  //
  // C'est la premiere tentative, quelle qu'en soit l'origine, qui porte le
  // cout du froid : la matrice A/B l'a montre en inversant l'ordre. Le
  // mesurer par origine melangerait deux choses ; il a donc sa ligne.
  const premiereTentative = releves[0] || { ok: false, ms: -1, origine: "?" };
  const froidOK = dansBudget(premiereTentative);
  console.log(`HOST_WORKER_COLD_START_PERF ${froidOK ? "OK" : "FAIL"}`
    + ` ms=${premiereTentative.ms} origine=${premiereTentative.origine}`
    + ` budget=${BUDGET_WORKER_MS}`);

  // `workerOK` alimente le verdict de fumee global : c'est la CAPACITE qui
  // doit y entrer, pas la performance. Une machine lente n'est pas une
  // machine cassee.
  workerOK = fonctionnelGlobal;


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
        if path == "/reutilise.png":
            REQUETES["reutilise.png"] += 1
            octets = MIRE["/mire/rouge.png"]
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(octets)))
            # CACHABLE, contrairement au reste du catalogue : voir l'epreuve
            # de reutilisation dans la page.
            self.send_header("Cache-Control", "max-age=300")
            self.end_headers()
            self.wfile.write(octets)
            print(f"BROWSER_HOST_FIXTURE_REUTILISE requetes={REQUETES['reutilise.png']}", flush=True)
            return
        if path in MIRE:
            octets = MIRE[path]
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(octets)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(octets)
            print(f"BROWSER_HOST_FIXTURE_MIRE path={path}", flush=True)
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
