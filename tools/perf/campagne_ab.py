#!/usr/bin/env python3
"""BOUCHAUD_BANC_PERF_V2 -- campagne A/B reproductible du navigateur reel.

  campagne_ab.py --bras main=boot-main.bin --bras cand=boot-cand.bin \\
                 --ladybird DIR --paires 3 --accel tcg --sortie DOSSIER \\
                 [--io] [--scenario browser-host]

Chaque passage est un demarrage A FROID du smoke BrowserHost
(tools/ci/run_ladybird_browser_host.sh) : meme artefact Ladybird, meme page,
seule l'image de boot change. Les bras ALTERNENT (A B, B A, A B...) pour que
la derive de la machine hote ne favorise aucun bras.

Mesures, par passage :
  * hote (temps mural, horloge de l'hote, resolution ~20 ms) : lancement de
    QEMU -> premiere ligne du noyau, -> bureau, -> premiere trame Ladybird,
    -> document charge, -> verdict de la page ;
  * invite (horloge du noyau) : celles de tools/ci/bench_ladybird_ab.py
    (demarrage -> bureau, lancement -> premiere image, presentation, dessin
    p50/p95/p99, fork, execve, RSS...) ;
  * volume serie (octets) et, avec --io, les acces MMIO/port par peripherique
    comptes par la trace QEMU (`memory_region_ops_*`) : sous KVM/WHPX chacun
    est une sortie de VM. --io RALENTIT l'invite : ses passages ne servent
    qu'au comptage, jamais aux temps.

Sorties : SORTIE/campagne.json (metadonnees + passages), SORTIE/campagne.csv
(une ligne par passage), SORTIE/resume.md (mediane, min-max, ecart).
Aucun seuil : le banc MESURE. TCG, KVM, WHPX et materiel ne se comparent
jamais entre eux en absolu.
"""
import argparse
import csv
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(RACINE / "tools" / "ci"))
import bench_ladybird_ab as bab  # noqa: E402

JALONS_HOTE = [
    ("hote_noyau_ms", "BOUCHAUD_BOOT_POINT kernel-main"),
    ("hote_bureau_ms", "BOUCHAUD_BOOT_POINT bureau-premiere-trame"),
    ("hote_premiere_trame_ms", "BOUCHAUD_UI_FIRST_FRAME onglet=1"),
    ("hote_document_ms", "[LB:NAV] onglet=1 document_charge"),
    ("hote_smoke_ok_ms", "HOST_SMOKE_OK"),
]


def sha256(chemin):
    h = hashlib.sha256()
    with open(chemin, "rb") as f:
        for bloc in iter(lambda: f.read(1 << 20), b""):
            h.update(bloc)
    return h.hexdigest()


def commande(*args):
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception:
        return ""


def metadonnees_hote(args):
    cpu = ""
    try:
        for l in open("/proc/cpuinfo"):
            if l.startswith("model name"):
                cpu = l.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    manifeste = Path(args.ladybird) / "BOUCHAUD_ARTIFACT_MANIFEST.json"
    lb = {}
    if manifeste.exists():
        try:
            lb = {f["path"]: f["sha256"] for f in json.load(open(manifeste))["files"]}
        except Exception:
            lb = {}
    return {
        "date": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "depot_sha": commande("git", "-C", str(RACINE), "rev-parse", "HEAD"),
        "hote_cpu": cpu,
        "hote_coeurs": os.cpu_count(),
        "hote_noyau": platform.release(),
        "qemu": commande("qemu-system-x86_64", "--version").splitlines()[0] if shutil.which("qemu-system-x86_64") else "",
        "accel": args.accel,
        "vcpu": 4,
        "memoire_mio": 8192,
        "affichage": os.environ.get("BO_SMOKE_AFFICHAGE", "bga"),
        "scenario": "browser-host (tools/health/browser_host_fixture.py, page locale deterministe)",
        "ladybird_dossier": str(Path(args.ladybird).resolve()),
        "ladybird_sha256": lb,
    }


class Guetteur(threading.Thread):
    """Date l'apparition des jalons dans le journal serie, cote hote."""

    def __init__(self, journal):
        super().__init__(daemon=True)
        self.journal = journal
        self.t0 = None
        self.vus = {}
        self.arret = threading.Event()

    def run(self):
        taille_vue = -1
        position = 0
        reste = b""
        while not self.arret.is_set():
            try:
                st = os.stat(self.journal)
            except FileNotFoundError:
                time.sleep(0.01)
                continue
            if self.t0 is None:
                # Le banc tronque le journal juste avant de lancer QEMU.
                if st.st_size == 0 and taille_vue != 0:
                    self.t0 = time.monotonic()
                taille_vue = st.st_size
                time.sleep(0.005)
                continue
            if st.st_size > position:
                with open(self.journal, "rb") as f:
                    f.seek(position)
                    nouveau = f.read()
                position += len(nouveau)
                texte = (reste + nouveau).decode(errors="replace")
                maintenant = time.monotonic()
                for cle, marque in JALONS_HOTE:
                    if cle not in self.vus and marque in texte:
                        self.vus[cle] = round((maintenant - self.t0) * 1000)
                reste = (reste + nouveau)[-512:]
            time.sleep(0.02)


def compte_io(trace):
    """Acces MMIO/port avant et apres le premier octet ecrit par le NOYAU
    dans le registre de donnees de COM1 (0x3f8) : ni SeaBIOS (port 0x402) ni
    le chargeur (ecran VGA) n'y ecrivent, l'initialisation de l'UART par le
    noyau est donc la frontiere."""
    pre, noyau = {}, {}
    derniere = 0
    with open(trace, errors="replace") as f:
        for i, l in enumerate(f):
            if l.startswith("memory_region_ops_write") and " addr 0x3f8 " in l and l.rstrip().endswith("'serial'"):
                derniere = i - 1
                break
    with open(trace, errors="replace") as f:
        for i, l in enumerate(f):
            if not l.startswith("memory_region_ops"):
                continue
            nom = l.rsplit("name ", 1)[-1].strip().strip("'")
            cible = pre if i <= derniere else noyau
            cible[nom] = cible.get(nom, 0) + 1
    return {"io_prenoyau_total": sum(pre.values()), "io_noyau_total": sum(noyau.values()),
            "io_noyau_par_peripherique": dict(sorted(noyau.items(), key=lambda kv: -kv[1])),
            "io_prenoyau_par_peripherique": dict(sorted(pre.items(), key=lambda kv: -kv[1]))}


def un_passage(bras, image, args, numero, dossier):
    dossier.mkdir(parents=True, exist_ok=True)
    journal = RACINE / "serie-browser-host.log"
    env = dict(os.environ)
    env["BO_SMOKE_ACCEL"] = args.accel
    extra = env.get("BO_SMOKE_QEMU_ARGS", "")
    trace = dossier / "io.trace"
    if args.io:
        extra += f" -trace memory_region_ops_read,file={trace} -trace memory_region_ops_write,file={trace}"
    env["BO_SMOKE_QEMU_ARGS"] = extra.strip()
    try:
        journal.unlink()
    except FileNotFoundError:
        pass
    g = Guetteur(str(journal))
    g.start()
    debut = time.monotonic()
    with open(dossier / "smoke.out", "w") as sortie:
        rc = subprocess.run(["tools/ci/run_ladybird_browser_host.sh", str(image), args.ladybird],
                            cwd=RACINE, env=env, stdout=sortie, stderr=subprocess.STDOUT).returncode
    g.arret.set()
    g.join(timeout=2)
    duree = round(time.monotonic() - debut, 1)
    shutil.copy(journal, dossier / "serie.log")
    texte = (dossier / "serie.log").read_text(errors="replace")
    smoke = (dossier / "smoke.out").read_text(errors="replace")
    m = {k: v for k, v in bab.mesures(texte).items()}
    m.update(g.vus)
    m["octets_serie"] = (dossier / "serie.log").stat().st_size
    m["lignes_serie"] = texte.count("\n")
    m["smoke_rc"] = rc
    m["smoke_fonctionnel"] = int("LADYBIRD_FUNCTIONAL_SMOKE ok" in smoke)
    m["duree_passage_s"] = duree
    commit = re.search(r"BOUCHAUD_BUILD commit=(\w+)", texte)
    m["noyau_commit"] = commit.group(1) if commit else None
    m["mode_graphique"] = (re.findall(r"GPU=(\w+):", texte) or ["?"])[-1]
    if args.io and trace.exists():
        m.update(compte_io(trace))
        trace.unlink()
    return {"bras": bras, "numero": numero, "image_sha256": sha256(image), **m}


def resume(passages, bras):
    cles = [k for k in passages[0] if isinstance(passages[0].get(k), (int, float))
            and k not in ("numero", "smoke_rc")]
    lignes = ["| Mesure | " + " | ".join(f"{b} (médiane [min–max], n)" for b in bras) + " |",
              "|---|" + "---|" * len(bras)]
    for c in cles:
        cellules = []
        for b in bras:
            v = [p[c] for p in passages if p["bras"] == b and isinstance(p.get(c), (int, float))]
            if not v:
                cellules.append("n/m")
                continue
            med = statistics.median(v)
            cellules.append(f"{med:g} [{min(v):g}–{max(v):g}], n={len(v)}")
        lignes.append(f"| {c} | " + " | ".join(cellules) + " |")
    return "\n".join(lignes)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--bras", action="append", required=True, help="NOM=BOOTIMAGE")
    p.add_argument("--ladybird", required=True)
    p.add_argument("--paires", type=int, default=3)
    p.add_argument("--accel", default="tcg", choices=["tcg", "kvm"])
    p.add_argument("--sortie", required=True)
    p.add_argument("--io", action="store_true")
    args = p.parse_args()
    bras = [b.split("=", 1) for b in args.bras]
    sortie = Path(args.sortie)
    sortie.mkdir(parents=True, exist_ok=True)
    meta = metadonnees_hote(args)
    meta["bras"] = {n: {"image": str(Path(i).resolve()), "image_sha256": sha256(i)} for n, i in bras}
    meta["io_trace"] = args.io
    passages = []
    for i in range(args.paires):
        ordre = bras if i % 2 == 0 else list(reversed(bras))
        for nom, image in ordre:
            print(f"[campagne] paire {i + 1}/{args.paires} bras={nom}", flush=True)
            r = un_passage(nom, image, args, i + 1, sortie / f"{nom}-{i + 1}")
            print(f"[campagne]   fonctionnel={r['smoke_fonctionnel']} bureau_hote={r.get('hote_bureau_ms')} "
                  f"premiere_trame_hote={r.get('hote_premiere_trame_ms')} serie={r['octets_serie']}", flush=True)
            passages.append(r)
            json.dump({"metadonnees": meta, "passages": passages}, open(sortie / "campagne.json", "w"),
                      indent=1, ensure_ascii=False)
    champs = sorted({k for p_ in passages for k, v in p_.items() if not isinstance(v, dict)})
    with open(sortie / "campagne.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=champs, extrasaction="ignore")
        w.writeheader()
        for p_ in passages:
            w.writerow(p_)
    md = resume(passages, [n for n, _ in bras])
    (sortie / "resume.md").write_text(
        f"# Campagne A/B ({meta['accel'].upper()}, {meta['date']})\n\n"
        f"Hôte : {meta['hote_cpu']} ({meta['hote_coeurs']} cœurs), {meta['qemu']}\n\n{md}\n")
    print(md)


if __name__ == "__main__":
    main()
