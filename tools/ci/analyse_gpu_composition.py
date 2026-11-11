#!/usr/bin/env python3
"""BOUCHAUD_GPU_COMPOSITION_V2 : verdicts du banc du compositeur GPU.

  analyse_gpu_composition.py mire   SERIE CAPTURE.png
  analyse_gpu_composition.py ab     SERIE_GPU SERIE_CPU
  analyse_gpu_composition.py repli  SERIE_2D
  analyse_gpu_composition.py occlusion SERIE_SUPERPOSEES SERIE_CPU
  analyse_gpu_composition.py panne  SERIE CAPTURE.png

Contre le faux vert : chaque verdict exige ses lignes, une capture absente ou
un releve manquant est un ECHEC, jamais un succes par defaut.
"""
import re
import statistics
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def lignes(chemin):
    with open(chemin, "rb") as f:
        return [ANSI.sub("", l.decode("utf-8", "replace")) for l in f]


def champs(ligne):
    return {k: v for k, v in re.findall(r"(\w+)=([^\s]+)", ligne)}


def releves(serie):
    out = []
    for l in lignes(serie):
        if "[GPU-COMPOSITION]" in l:
            c = champs(l.split("[GPU-COMPOSITION]", 1)[1])
            out.append({k: int(v) for k, v in c.items() if v.isdigit()})
    return out


def decode_mire(capture, ox, oy, zw, zh):
    """Pixels de la zone qui montrent EXACTEMENT le pixel de surface attendu."""
    from PIL import Image
    im = Image.open(capture).convert("RGB")
    bb = im.getbbox()
    if bb is None:
        return None
    im = im.crop(bb)
    px = im.load()
    W, H = im.size
    exacts = faux = blancs = 0
    for y in range(oy, min(oy + zh, H)):
        for x in range(ox, min(ox + zw, W)):
            r, g, b = px[x, y]
            if (r, g, b) == (255, 255, 255):
                blancs += 1
                continue
            sx, sy = x - ox, y - oy
            attendu = (sy & 0xFF, sx & 0xFF, 0x80 | ((sy >> 8) << 3) | (sx >> 8))
            if (r, g, b) == attendu:
                exacts += 1
            else:
                faux += 1
    return exacts, faux, blancs


def mire(serie, capture):
    texte = "".join(lignes(serie))
    if "BOUCHAUD_GPU_COMPOSITION_AUTOTEST_OK" not in texte:
        print("MIRE_ECHEC autotest de convention absent")
        return 1
    m = re.search(r"GPU_COUCHE noeud=\d+ surface=\d+x\d+ origine=(\d+),(\d+)", texte)
    v = re.search(r"MIRE_GPU_PRETE surface=\d+x\d+ vue=(\d+)x(\d+)", texte)
    if not m or not v:
        print("MIRE_ECHEC couche GPU ou mire absente du journal")
        return 1
    ox, oy, zw, zh = int(m[1]), int(m[2]), int(v[1]), int(v[2])
    try:
        res = decode_mire(capture, ox, oy, zw, zh)
    except FileNotFoundError:
        res = None
    if res is None:
        print("MIRE_ECHEC capture absente ou noire")
        return 1
    exacts, faux, blancs = res
    total = zw * zh
    # Le carre mobile (32x32) et le curseur sont blancs ; tout le reste doit
    # etre le pixel EXACT de la surface.
    ok = faux == 0 and exacts >= total - 2 * 32 * 32 - 22 * 14
    print(f"MIRE_{'OK' if ok else 'ECHEC'} zone={zw}x{zh}@{ox},{oy} exacts={exacts} faux={faux} blancs={blancs} total={total}")
    return 0 if ok else 1


def mediane(rs, cle):
    vals = [r[cle] for r in rs if cle in r]
    return statistics.median(vals) if vals else None


def ab(serie_gpu, serie_cpu):
    g = releves(serie_gpu)[1:]  # le premier releve contient le demarrage
    c = releves(serie_cpu)[1:]
    if len(g) < 2 or len(c) < 2:
        print(f"AB_ECHEC releves insuffisants gpu={len(g)} cpu={len(c)}")
        return 1
    if any(r.get("disponible") != 1 or r.get("trames_gpu", 0) == 0 for r in g):
        print("AB_ECHEC le bras GPU n'a pas compose par le GPU")
        return 1
    if any(r.get("trames_gpu", 0) != 0 for r in c):
        print("AB_ECHEC le bras processeur a compose par le GPU")
        return 1
    dg, dc = mediane(g, "us_dessin_cpu_par_trame"), mediane(c, "us_dessin_cpu_par_trame")
    pg, pc = mediane(g, "px_client_copies_cpu"), mediane(c, "px_client_copies_cpu")
    tg, tc = mediane(g, "trames_bureau"), mediane(c, "trames_bureau")
    xg, xc = mediane(g, "us_presentation_par_trame"), mediane(c, "us_presentation_par_trame")
    criteres = {
        "dessin_cpu_divise_par_2": dg <= 0.5 * dc,
        "pixels_clients_cpu_moins_1pct": pg <= 0.01 * max(pc, 1),
        "cadence_conservee_90pct": tg >= 0.9 * tc,
    }
    ok = all(criteres.values())
    print(
        f"AB_{'CONVAINCANT' if ok else 'NON_CONVAINCANT'} "
        f"us_dessin_cpu_par_trame gpu={dg} cpu={dc} (x{dc / max(dg, 1):.1f}) "
        f"px_client_copies_cpu_30s gpu={pg} cpu={pc} "
        f"trames_bureau_30s gpu={tg} cpu={tc} "
        f"us_presentation_par_trame gpu={xg} cpu={xc} "
        + " ".join(f"{k}={int(v)}" for k, v in criteres.items())
    )
    return 0 if ok else 1


def repli(serie):
    texte = "".join(lignes(serie))
    rs = releves(serie)
    if "GPU_COMPOSITION_ECHEC" in texte or "panic" in texte.lower():
        print("REPLI_ECHEC echec ou panique sur l'hote sans VIRGL")
        return 1
    if "BOUCHAUD_DOOM_ATELIER_LAUNCH" not in texte or not rs:
        print("REPLI_ECHEC DOOM ou releve absent")
        return 1
    ok = all(r.get("disponible") == 0 and r.get("trames_gpu") == 0 for r in rs) \
        and any(r.get("px_client_copies_cpu", 0) > 0 for r in rs)
    print(f"REPLI_{'OK' if ok else 'ECHEC'} releves={len(rs)} (virtio-gpu 2D, composition processeur)")
    return 0 if ok else 1


def occlusion(serie, serie_cpu):
    """DOOM CACHE sous la mire : ses pixels ne se voient (presque) pas ; le
    processeur ne doit pas les recopier. Borne : 10 % de ce que recopie le
    bras processeur de l'A/B (DOOM seul, visible)."""
    o = releves(serie)[1:]
    c = releves(serie_cpu)[1:]
    if len(o) < 1 or len(c) < 1:
        print(f"OCCLUSION_ECHEC releves insuffisants occlusion={len(o)} cpu={len(c)}")
        return 1
    po, pc = mediane(o, "px_client_copies_cpu"), mediane(c, "px_client_copies_cpu")
    gpu = all(r.get("fenetres_gpu", 0) >= 1 for r in o)
    ok = gpu and po <= 0.1 * pc
    print(f"OCCLUSION_{'OK' if ok else 'ECHEC'} px_client_copies_cpu_30s superposees={po} doom_seul_cpu={pc} fenetre_du_dessus_gpu={int(gpu)}")
    return 0 if ok else 1


def panne(serie, capture):
    """Une trame GPU refusee (file intacte) : repli processeur, reprise du
    GPU apres 5 s, et la mire de nouveau EXACTE apres la reprise."""
    texte = "".join(lignes(serie))
    exige = {
        "injection": "GPU_COMPOSITION_INJECTION refus_trame=300",
        "repli": "GPU_COMPOSITION_ECHEC raison=injection-banc file_cassee=0 repli=composition-processeur reprise=dans-5s",
        "reprise": "BOUCHAUD_GPU_COMPOSITION_REPRISE tentative=1",
    }
    manque = [k for k, m in exige.items() if m not in texte]
    if manque or "panic" in texte.lower() or "VIRTIO_GPU_SCANOUT_ABANDON" in texte:
        print(f"PANNE_ECHEC manque={manque} panique={'panic' in texte.lower()}")
        return 1
    apres = texte.split(exige["reprise"], 1)[1]
    rs = [r for r in releves_texte(apres) if r.get("trames_gpu", 0) > 0]
    if not rs:
        print("PANNE_ECHEC aucun releve avec trames GPU apres la reprise")
        return 1
    if mire(serie, capture) != 0:
        print("PANNE_ECHEC mire inexacte apres la reprise")
        return 1
    print(f"PANNE_OK repli puis reprise ; trames_gpu_apres={rs[-1]['trames_gpu']}")
    return 0


def releves_texte(t):
    out = []
    for l in t.splitlines():
        if "[GPU-COMPOSITION]" in l:
            out.append({k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", l.split("[GPU-COMPOSITION]", 1)[1])})
    return out


if __name__ == "__main__":
    if sys.argv[1] == "panne":
        sys.exit(panne(sys.argv[2], sys.argv[3]))
    if sys.argv[1] == "occlusion":
        sys.exit(occlusion(sys.argv[2], sys.argv[3]))
    mode = sys.argv[1]
    if mode == "mire":
        sys.exit(mire(sys.argv[2], sys.argv[3]))
    if mode == "ab":
        sys.exit(ab(sys.argv[2], sys.argv[3]))
    if mode == "repli":
        sys.exit(repli(sys.argv[2]))
    sys.exit("mode inconnu")
