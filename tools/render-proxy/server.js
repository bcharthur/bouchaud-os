// Service de rendu deporte pour Bouchaud OS.
//
// Deux surfaces :
//
// 1) RENDU STATIQUE (mode compat de Nautile, inchange) :
//    - GET /render?url=<url>&w=<largeur>  -> image/png de la page rendue
//
// 2) WEBVIEW INTERACTIF (app "WebView" du bureau) : une page Chromium
//    persistante pilotee a distance, architecture navigateur "cloud"
//    (Opera Mini / Puffin) -- l'OS envoie clics/molette/clavier, le service
//    renvoie une capture PNG du viewport apres chaque action. Coordonnees
//    1:1 : le viewport Chromium est redimensionne a la taille demandee par
//    la fenetre de l'OS, la capture n'est jamais re-echelonnee.
//    - GET /wv/open?url=&w=&h=   navigation (et reglage du viewport)
//    - GET /wv/shot?w=&h=        capture simple
//    - GET /wv/click?x=&y=&w=&h= clic gauche aux coordonnees viewport
//    - GET /wv/scroll?dy=&w=&h=  molette verticale (px, + = vers le bas)
//    - GET /wv/type?text=&w=&h=  saisie clavier (texte perce-encode)
//    - GET /wv/key?k=&w=&h=      touche speciale (Enter, Backspace, Tab,
//                                Escape, Arrow*, PageUp/Down, Delete)
//    - GET /wv/back /wv/forward /wv/reload (&w=&h=)
//    - GET /wv/url               URL courante (text/plain, pour la barre)
//    - GET /healthz              "ok"
//
// Toutes les captures sont produites en PNG RGB 8 bits NON entrelace
// (compatibles decodeur de l'OS) et bornees au budget pixels (~1,1 Mpx).
//
// Lancement :  npm run setup   (une fois)   puis   npm start
// Depuis QEMU (reseau SLIRP), l'hote est joignable a 10.0.2.2:8080.

const http = require("http");
const { chromium } = require("playwright");
const sharp = require("sharp");

const PORT = parseInt(process.env.PORT || "8080", 10);
const BUDGET = 1_100_000; // pixels max (sous le plafond 1,2 Mpx du decodeur OS)
const MAXDIM = 4000;      // dimension max (plafond 4096 du decodeur OS)
const RENDER_W = 1280;    // largeur de rendu "desktop" (mode /render)
const RENDER_H = 820;     // hauteur de viewport initiale (mode /render)

let browserPromise = null;
function getBrowser() {
  if (!browserPromise) {
    const opts = { args: [
      "--no-sandbox", "--disable-dev-shm-usage",
      // Sans cela une video ne demarre jamais : Chromium exige un geste de
      // l'utilisateur, et le notre arrive par une requete HTTP, pas par un
      // clic qu'il reconnaisse.
      "--autoplay-policy=no-user-gesture-required",
      // Le client leger n'a pas de haut-parleur cote hote : le son sort par
      // la carte de la machine invitee, ou pas du tout. Le couper ici evite
      // qu'un serveur sans peripherique audio refuse de lire.
      "--mute-audio",
    ] };
    // Chromium fourni par l'environnement (CHROMIUM_PATH ou installation
    // Playwright partagée) : évite un `playwright install` si un binaire
    // compatible existe déjà.
    const fs = require("fs");
    const candidates = [
      process.env.CHROMIUM_PATH,
      process.env.PLAYWRIGHT_BROWSERS_PATH &&
        process.env.PLAYWRIGHT_BROWSERS_PATH + "/chromium",
    ].filter(Boolean);
    for (const c of candidates) {
      try {
        const p = fs.statSync(c).isDirectory() ? c + "/chrome-linux/chrome" : c;
        if (fs.existsSync(p)) { opts.executablePath = p; break; }
        // installation playwright : chromium-*/chrome-linux/chrome
        const sub = fs.readdirSync(c).find((d) => /^chromium-\d+$/.test(d));
        if (sub && fs.existsSync(c + "/../" + sub + "/chrome-linux/chrome")) {
          opts.executablePath = c + "/../" + sub + "/chrome-linux/chrome";
          break;
        }
      } catch (_) {}
    }
    // Reseau d'entreprise / environnement avec proxy sortant obligatoire.
    const proxyUrl = process.env.HTTPS_PROXY || process.env.https_proxy;
    if (proxyUrl) {
      opts.proxy = { server: proxyUrl };
      const noProxy = process.env.NO_PROXY || process.env.no_proxy;
      if (noProxy) opts.proxy.bypass = noProxy;
    }
    browserPromise = chromium.launch(opts);
  }
  return browserPromise;
}

// Derriere un proxy d'inspection TLS, les certificats sont re-signes par la
// CA du proxy : on assouplit la verification dans ce cas uniquement.
const CTX_TLS = { ignoreHTTPSErrors: !!(process.env.HTTPS_PROXY || process.env.https_proxy) };

const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

// ── Page partagee du mode /render (Nautile compat) ───────────────────────────
let pagePromise = null;
function getPage() {
  if (!pagePromise) {
    pagePromise = (async () => {
      const b = await getBrowser();
      const ctx = await b.newContext({
        viewport: { width: RENDER_W, height: RENDER_H },
        deviceScaleFactor: 1,
        userAgent: UA,
        locale: "fr-FR",
        ...CTX_TLS,
      });
      return ctx.newPage();
    })();
  }
  return pagePromise;
}
function resetPage() {
  pagePromise = null;
}

// ── Page persistante du WebView interactif ───────────────────────────────────
let wvPromise = null;
function getWv() {
  if (!wvPromise) {
    wvPromise = (async () => {
      const b = await getBrowser();
      const ctx = await b.newContext({
        viewport: { width: 1024, height: 576 },
        deviceScaleFactor: 1,
        userAgent: UA,
        locale: "fr-FR",
        ...CTX_TLS,
      });
      const page = await ctx.newPage();
      // Les liens target=_blank ouvrent un onglet invisible pour l'OS :
      // on rapatrie leur URL dans la page principale puis on les ferme.
      ctx.on("page", async (popup) => {
        try {
          await popup.waitForLoadState("domcontentloaded", { timeout: 8000 }).catch(() => {});
          const u = popup.url();
          if (u && u !== "about:blank") {
            await page.goto(u, { waitUntil: "domcontentloaded", timeout: 25000 }).catch(() => {});
          }
        } finally {
          popup.close().catch(() => {});
        }
      });
      return page;
    })();
  }
  return wvPromise;
}
function resetWv() {
  wvPromise = null;
}

// Serialise les rendus : une action a la fois.
let renderChain = Promise.resolve();
function queue(fn) {
  const run = renderChain.then(fn, fn);
  renderChain = run.then(() => {}, () => {});
  return run;
}

function normalizeUrl(u) {
  if (!u) return null;
  if (!/^https?:\/\//i.test(u)) u = "https://" + u;
  try { new URL(u); } catch (_) { return null; }
  return u;
}

// Ajuste le viewport de la page webview a la taille de fenetre de l'OS
// (bornee au budget pixels), pour des captures et des clics 1:1.
async function wvViewport(page, q) {
  let w = parseInt(q.get("w") || "0", 10) || 0;
  let h = parseInt(q.get("h") || "0", 10) || 0;
  if (w < 160 || h < 120) return;
  w = Math.min(w, MAXDIM);
  h = Math.min(h, MAXDIM);
  if (w * h > BUDGET) {
    const s = Math.sqrt(BUDGET / (w * h));
    w = Math.floor(w * s);
    h = Math.floor(h * s);
  }
  const cur = page.viewportSize();
  if (!cur || cur.width !== w || cur.height !== h) {
    await page.setViewportSize({ width: w, height: h });
  }
}

async function renderUrl(target) {
  try {
    const page = await getPage();
    await page.goto(target, { waitUntil: "domcontentloaded", timeout: 25000 }).catch(() => {});
    await page.waitForTimeout(700);
    return await page.screenshot({ type: "png" });
  } catch (e) {
    resetPage();
    throw e;
  }
}

// Redimensionne sous le budget + retire l'alpha + PNG truecolor non entrelace.
async function fitForOs(pngBuffer, allowResize) {
  const meta = await sharp(pngBuffer).metadata();
  const w = meta.width || 1;
  const h = meta.height || 1;
  let scale = Math.min(1, Math.sqrt(BUDGET / (w * h)), MAXDIM / w, MAXDIM / h);
  let img = sharp(pngBuffer).flatten({ background: "#ffffff" }).removeAlpha();
  if (allowResize && scale < 1) {
    img = img.resize(Math.max(1, Math.round(w * scale)), Math.max(1, Math.round(h * scale)));
  }
  return img.png({ compressionLevel: 6, palette: false, progressive: false, adaptiveFiltering: false }).toBuffer();
}

const WV_KEYS = new Set([
  "Enter", "Backspace", "Tab", "Escape", "Delete",
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "PageUp", "PageDown",
  "Home", "End",
]);

// Signature de la derniere image envoyee, pour ne pas la renvoyer deux fois.
let derniereSignature = "";

const crypto = require("crypto");

function signature(buffer) {
  return crypto.createHash("sha1").update(buffer).digest("hex").slice(0, 16);
}

// Capture le viewport au format demande.
//
// JPEG par defaut, et ce n'est pas un detail : une image de client leger fait
// 40 a 80 ko en JPEG contre 300 ko a 1 Mo en PNG. Sur le lien SLIRP d'une
// machine emulee, c'est la difference entre dix images par seconde et une.
// Le PNG reste disponible pour ce qui doit rester net — du texte fin, une
// capture qu'on veut fidele au pixel.
async function capture(page, q) {
  const format = (q.get("fmt") || "jpeg").toLowerCase();
  if (format === "png") {
    return { buffer: await page.screenshot({ type: "png" }), type: "image/png" };
  }
  const qualite = Math.max(20, Math.min(95, parseInt(q.get("q") || "72", 10)));
  return {
    buffer: await page.screenshot({ type: "jpeg", quality: qualite }),
    type: "image/jpeg",
  };
}

// Execute une action webview et renvoie la capture du viewport.
async function wvAction(pathname, q) {
  const page = await getWv();
  try {
    await wvViewport(page, q);
    switch (pathname) {
      case "/wv/open": {
        const target = normalizeUrl(q.get("url"));
        if (!target) throw new Error("url invalide");
        await page.goto(target, { waitUntil: "domcontentloaded", timeout: 25000 }).catch(() => {});
        await page.waitForTimeout(700);
        break;
      }
      case "/wv/shot":
        break;
      case "/wv/click": {
        const x = parseFloat(q.get("x") || "0");
        const y = parseFloat(q.get("y") || "0");
        await page.mouse.click(x, y).catch(() => {});
        // Laisse une eventuelle navigation/JS reagir avant la capture.
        await page.waitForTimeout(600);
        await page.waitForLoadState("domcontentloaded", { timeout: 4000 }).catch(() => {});
        break;
      }
      case "/wv/scroll": {
        const dy = parseFloat(q.get("dy") || "0");
        await page.mouse.wheel(0, dy).catch(() => {});
        await page.waitForTimeout(150);
        break;
      }
      case "/wv/type": {
        const text = q.get("text") || "";
        if (text) await page.keyboard.type(text, { delay: 15 }).catch(() => {});
        await page.waitForTimeout(80);
        break;
      }
      case "/wv/key": {
        const k = q.get("k") || "";
        if (!WV_KEYS.has(k)) throw new Error("touche non geree: " + k);
        await page.keyboard.press(k).catch(() => {});
        // Enter peut declencher une navigation (formulaires, recherche).
        await page.waitForTimeout(k === "Enter" ? 700 : 100);
        if (k === "Enter") {
          await page.waitForLoadState("domcontentloaded", { timeout: 4000 }).catch(() => {});
        }
        break;
      }
      case "/wv/back":
        await page.goBack({ waitUntil: "domcontentloaded", timeout: 15000 }).catch(() => {});
        await page.waitForTimeout(300);
        break;
      case "/wv/forward":
        await page.goForward({ waitUntil: "domcontentloaded", timeout: 15000 }).catch(() => {});
        await page.waitForTimeout(300);
        break;
      case "/wv/reload":
        await page.reload({ waitUntil: "domcontentloaded", timeout: 25000 }).catch(() => {});
        await page.waitForTimeout(500);
        break;
      default:
        throw new Error("action inconnue");
    }
    return await capture(page, q);
  } catch (e) {
    if (page.isClosed && page.isClosed()) resetWv();
    throw e;
  }
}

const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, "http://localhost");
  const send = (code, type, body) => {
    res.writeHead(code, {
      "Content-Type": type,
      "Content-Length": Buffer.byteLength(body),
      "Cache-Control": "no-store",
    });
    res.end(body);
  };

  if (u.pathname === "/healthz") return send(200, "text/plain", "ok");

  if (u.pathname === "/wv/url") {
    try {
      const page = await getWv();
      return send(200, "text/plain", page.url() || "");
    } catch (e) {
      return send(502, "text/plain", "erreur: " + (e && e.message ? e.message : e));
    }
  }

  // Etat de la page : de quoi tenir une barre d'adresse et des boutons.
  if (u.pathname === "/wv/info") {
    try {
      const page = await getWv();
      const info = {
        url: page.url() || "",
        titre: await page.title().catch(() => ""),
        signature: derniereSignature,
      };
      return send(200, "application/json", JSON.stringify(info));
    } catch (e) {
      return send(502, "text/plain", "erreur: " + (e && e.message ? e.message : e));
    }
  }

  if (u.pathname.startsWith("/wv/")) {
    const t0 = Date.now();
    try {
      const rendu = await queue(() => wvAction(u.pathname, u.searchParams));
      const signee = signature(rendu.buffer);

      // Rien n'a bouge depuis l'image que le client a deja : on le lui dit en
      // trois octets plutot qu'en cinquante kilo-octets. C'est ce qui rend une
      // page immobile gratuite, et laisse la bande passante a celles qui
      // bougent vraiment.
      if (u.searchParams.get("sig") === signee) {
        res.writeHead(304, { "Cache-Control": "no-store" });
        res.end();
        return;
      }
      derniereSignature = signee;

      res.writeHead(200, {
        "Content-Type": rendu.type,
        "Content-Length": rendu.buffer.length,
        "Cache-Control": "no-store",
        "X-Bo-Signature": signee,
      });
      res.end(rendu.buffer);
      console.log(`[wv] ${u.pathname}${u.search} -> ${rendu.buffer.length} o `
                  + `en ${Date.now() - t0} ms`);
    } catch (e) {
      console.error(`[wv erreur] ${u.pathname}:`, e && e.stack ? e.stack : e);
      send(502, "text/plain", "echec webview: " + (e && e.message ? e.message : String(e)));
    }
    return;
  }

  if (u.pathname !== "/render") {
    return send(404, "text/plain", "Nautile render proxy. /render?url=... ou /wv/open?url=...");
  }
  const target = normalizeUrl(u.searchParams.get("url"));
  if (!target) return send(400, "text/plain", "url invalide");
  const t0 = Date.now();
  try {
    const raw = await queue(() => renderUrl(target));
    const png = await fitForOs(raw, true);
    send(200, "image/png", png);
    console.log(`[render] ${target} -> ${png.length} o en ${Date.now() - t0} ms`);
  } catch (e) {
    console.error(`[erreur] ${target}: ${e && e.message ? e.message : e}`);
    send(502, "text/plain", "echec du rendu: " + (e && e.message ? e.message : String(e)));
  }
});

// Le client leger demande une image plusieurs fois par seconde : sans
// connexion persistante, chaque image coute une poignee de main TCP, ce qui sur
// le lien d'une machine emulee pese plus que l'image elle-meme.
server.keepAliveTimeout = 30_000;
server.headersTimeout = 35_000;

server.listen(PORT, "0.0.0.0", () => {
  console.log(`Nautile render proxy a l'ecoute sur http://0.0.0.0:${PORT}`);
  console.log(`Depuis QEMU (SLIRP) : http://10.0.2.2:${PORT}/render?url=https://example.com`);
  getPage().then(() => console.log("Chromium pret.")).catch((e) => console.error("Chromium:", e.message));
});
