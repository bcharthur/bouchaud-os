// Service de rendu deporte pour le navigateur Nautile de Bouchaud OS.
//
// Bouchaud OS ne peut pas executer les sites modernes (SPA JS, CSS complet,
// SVG, polices web...). Ce service les rend avec un VRAI Chromium headless et
// renvoie une image PNG de la page. Nautile affiche cette image telle quelle.
//   - GET /render?url=<url>&w=<largeur>  -> image/png de la page rendue
//   - GET /healthz                       -> "ok"
//
// L'image est redimensionnee pour rester sous le budget pixels du decodeur PNG
// de l'OS (~1,2 Mpx) et produite en RGB 8 bits NON entrelace (compatible decodeur).
//
// Lancement :  npm run setup   (une fois)   puis   npm start
// Depuis QEMU (reseau SLIRP), l'hote est joignable a 10.0.2.2:8080.

const http = require("http");
const { chromium } = require("playwright");
const sharp = require("sharp");

const PORT = parseInt(process.env.PORT || "8080", 10);
const BUDGET = 1_100_000; // pixels max (sous le plafond 1,2 Mpx du decodeur OS)
const MAXDIM = 4000;      // dimension max (plafond 4096 du decodeur OS)
const RENDER_W = 1280;    // largeur de rendu "desktop"
const RENDER_H = 820;     // hauteur de viewport initiale

let browserPromise = null;
function getBrowser() {
  if (!browserPromise) {
    browserPromise = chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  }
  return browserPromise;
}

// Page Chromium reutilisee entre les requetes (evite de recreer un contexte a
// chaque fois = beaucoup moins de CPU / plus rapide). Recreee si elle plante.
let pagePromise = null;
function getPage() {
  if (!pagePromise) {
    pagePromise = (async () => {
      const b = await getBrowser();
      const ctx = await b.newContext({
        viewport: { width: RENDER_W, height: RENDER_H },
        deviceScaleFactor: 1,
        userAgent:
          "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        locale: "fr-FR",
      });
      return ctx.newPage();
    })();
  }
  return pagePromise;
}
function resetPage() {
  pagePromise = null;
}

// Serialise les rendus : une seule page partagee -> un rendu a la fois.
let renderChain = Promise.resolve();
function queueRender(fn) {
  const run = renderChain.then(fn, fn);
  // la chaine ne doit jamais rester rejetee
  renderChain = run.then(() => {}, () => {});
  return run;
}

function normalizeUrl(u) {
  if (!u) return null;
  if (!/^https?:\/\//i.test(u)) u = "https://" + u;
  try { new URL(u); } catch (_) { return null; }
  return u;
}

async function renderUrl(target) {
  try {
    const page = await getPage();
    // domcontentloaded + court delai = nettement plus rapide que 'load'.
    await page.goto(target, { waitUntil: "domcontentloaded", timeout: 25000 }).catch(() => {});
    await page.waitForTimeout(700);
    // Capture du VIEWPORT seulement (pas fullPage) : bien plus rapide, image
    // plus petite a transferer et a decoder cote OS.
    return await page.screenshot({ type: "png" });
  } catch (e) {
    resetPage(); // page corrompue/fermee -> on la recreera au prochain coup
    throw e;
  }
}

// Redimensionne sous le budget + retire l'alpha + PNG truecolor non entrelace.
async function fitForOs(pngBuffer) {
  const meta = await sharp(pngBuffer).metadata();
  const w = meta.width || 1;
  const h = meta.height || 1;
  let scale = Math.min(1, Math.sqrt(BUDGET / (w * h)), MAXDIM / w, MAXDIM / h);
  let img = sharp(pngBuffer).flatten({ background: "#ffffff" }).removeAlpha();
  if (scale < 1) {
    img = img.resize(Math.max(1, Math.round(w * scale)), Math.max(1, Math.round(h * scale)));
  }
  return img.png({ compressionLevel: 6, palette: false, progressive: false, adaptiveFiltering: false }).toBuffer();
}

const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, "http://localhost");
  if (u.pathname === "/healthz") {
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end("ok");
    return;
  }
  if (u.pathname !== "/render") {
    res.writeHead(404, { "Content-Type": "text/plain" });
    res.end("Nautile render proxy. Utilise /render?url=...");
    return;
  }
  const target = normalizeUrl(u.searchParams.get("url"));
  const width = parseInt(u.searchParams.get("w") || String(RENDER_W), 10) || RENDER_W;
  if (!target) {
    res.writeHead(400, { "Content-Type": "text/plain" });
    res.end("url invalide");
    return;
  }
  const t0 = Date.now();
  try {
    const raw = await queueRender(() => renderUrl(target));
    const png = await fitForOs(raw);
    res.writeHead(200, {
      "Content-Type": "image/png",
      "Content-Length": png.length,
      "Cache-Control": "no-store",
      // L'OS detecte la fin de reponse via la fermeture de connexion.
      "Connection": "close",
    });
    res.end(png);
    console.log(`[render] ${target} -> ${png.length} o en ${Date.now() - t0} ms`);
  } catch (e) {
    console.error(`[erreur] ${target}: ${e && e.message ? e.message : e}`);
    res.writeHead(502, { "Content-Type": "text/plain" });
    res.end("echec du rendu: " + (e && e.message ? e.message : String(e)));
  }
});

server.listen(PORT, "0.0.0.0", () => {
  console.log(`Nautile render proxy a l'ecoute sur http://0.0.0.0:${PORT}`);
  console.log(`Depuis QEMU (SLIRP) : http://10.0.2.2:${PORT}/render?url=https://example.com`);
  // Pre-lance Chromium + la page partagee pour que le premier rendu soit rapide.
  getPage().then(() => console.log("Chromium pret.")).catch((e) => console.error("Chromium:", e.message));
});
