#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- orchestrateur des parcours E2E de Ladybird.

    e2e.py --boot BOOTIMAGE --disque DISQUE --accel tcg|kvm|whpx [--smp N]
           [--memoire MIO] [--qemu CHEMIN] [--sortie DIR] [--scenarios E2E-01,...]
           [--port-fixture P] [--exige E2E-01,E2E-02]

UN orchestrateur, UNE definition (scenarios.json), plusieurs adaptateurs : le
seul code qui depend de l'environnement est le choix des options QEMU
(`arguments_qemu`). Linux (TCG, KVM) et Windows (WHPX) lancent ce meme fichier
avec la meme definition, les memes assertions, les memes metriques et la meme
structure de rapport.

Gestes REELS : souris PS/2 et clavier par le moniteur QEMU (TCP local, portable
Windows), donc pilote du noyau -> gestionnaire de fenetres -> Atelier ->
Ladybird. Le navigateur est ouvert par le menu Demarrer, comme un humain.

Le disque n'est jamais modifie : QEMU l'ouvre `snapshot=on`, et son empreinte
est verifiee avant et apres.

Statuts : PASS, FAIL, NOT_RUN, BLOCKED_EXTERNAL, NON_MESURABLE. Une mesure
absente reste absente (`null`), jamais zero.

Rapports dans --sortie : manifest.json, summary.json, results.json,
performance.json, failures.json, serial.log, qemu.stderr.log, captures/*.png,
rapport.html. Code 0 : aucun scenario bloquant en FAIL et les scenarios exiges
en PASS ; 1 sinon ; 2 : usage.
"""
from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import json
import os
import platform
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

ICI = Path(__file__).resolve().parent
RACINE = ICI.parents[2]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
PAS_SOURIS = 120
CPU_WHPX = "qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt"  # tools/windows/BouchaudQemu.psm1
PANIQUES = ("KERNEL PANIC", "DOUBLE FAULT", "TRIPLE FAULT", "panicked at")

sys.path.insert(0, str(ICI))
import fixture as _fixture  # noqa: E402  (couleurs des pages de reference)

# Bouchaud OS decode le clavier en AZERTY (src/drivers/input/clavier_decodeur.rs) ;
# `sendkey` nomme des touches PHYSIQUES (QWERTY US). Meme table que
# tools/release/test_livraison_qemu.py.
TOUCHES_AZERTY = {
    "a": "q", "q": "a", "z": "w", "w": "z", "m": "semicolon",
    "-": "6", "_": "8", "=": "equal", " ": "spc",
    ".": "shift-comma", "/": "shift-dot", ":": "dot", ",": "m", ";": "comma",
    **{str(n): f"shift-{n}" for n in range(10)},
}


def touches_de(texte: str) -> list[str]:
    out = []
    for c in texte:
        if c in TOUCHES_AZERTY:
            out.append(TOUCHES_AZERTY[c])
        elif "a" <= c <= "z":
            out.append(c)
        else:
            raise ValueError(f"caractere sans touche AZERTY connue : {c!r}")
    return out


def combinaison(logique: str) -> str:
    """`ctrl-w` (touche LOGIQUE) -> touche physique : W est sur la touche Z en AZERTY."""
    *mods, cle = logique.split("-")
    if len(cle) == 1 and cle in TOUCHES_AZERTY and not cle.isdigit():
        cle = TOUCHES_AZERTY[cle]
    return "-".join(mods + [cle])


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""):
            h.update(b)
    return h.hexdigest()


class Moniteur:
    """Moniteur HMP de QEMU sur TCP local (portable Linux et Windows)."""

    def __init__(self, port: int):
        fin = time.time() + 30
        while True:
            try:
                self.s = socket.create_connection(("127.0.0.1", port), timeout=10)
                break
            except OSError:
                if time.time() > fin:
                    raise
                time.sleep(0.3)
        self._invite()

    def _invite(self) -> bytes:
        recu, fin = b"", time.time() + 15
        while b"(qemu)" not in recu and time.time() < fin:
            try:
                m = self.s.recv(65536)
            except socket.timeout:
                break
            if not m:
                break
            recu += m
        return recu

    def cmd(self, c: str) -> str:
        self.s.sendall(c.encode() + b"\n")
        return self._invite().decode(errors="replace")

    def bouge(self, dx: int, dy: int) -> None:
        while dx or dy:
            px, py = max(-PAS_SOURIS, min(PAS_SOURIS, dx)), max(-PAS_SOURIS, min(PAS_SOURIS, dy))
            self.cmd(f"mouse_move {px} {py}")
            dx, dy = dx - px, dy - py

    def va(self, x: int, y: int) -> None:
        self.bouge(-4000, -4000)
        self.bouge(x, y)

    def clic(self) -> None:
        self.cmd("mouse_button 1")
        time.sleep(0.04)
        self.cmd("mouse_button 0")

    def touche(self, physique: str) -> None:
        self.cmd(f"sendkey {physique}")
        time.sleep(0.05)

    def capture(self, chemin: Path) -> None:
        self.cmd("stop")
        self.cmd(f"screendump {chemin}")
        self.cmd("cont")


def lit_ppm(p: Path):
    d = p.read_bytes()
    if not d.startswith(b"P6"):
        raise ValueError("PPM P6 attendu")
    champs, i = [], 2
    while len(champs) < 3:
        while d[i:i + 1].isspace():
            i += 1
        if d[i:i + 1] == b"#":
            while d[i:i + 1] != b"\n":
                i += 1
            continue
        j = i
        while not d[j:j + 1].isspace():
            j += 1
        champs.append(int(d[i:j]))
        i = j
    i += 1
    l, h, _ = champs
    return l, h, d[i:i + l * h * 3]


def ecrit_png(chemin: Path, l: int, h: int, rgb: bytes) -> None:
    lignes = b"".join(b"\x00" + rgb[y * l * 3:(y + 1) * l * 3] for y in range(h))

    def bloc(t, c):
        return struct.pack(">I", len(c)) + t + c + struct.pack(">I", zlib.crc32(t + c) & 0xFFFFFFFF)
    chemin.write_bytes(b"\x89PNG\r\n\x1a\n" + bloc(b"IHDR", struct.pack(">IIBBBBB", l, h, 8, 2, 0, 0, 0))
                       + bloc(b"IDAT", zlib.compress(lignes, 6)) + bloc(b"IEND", b""))


def pixels_de_couleur(rgb: bytes, couleur, tol: int = 6) -> int:
    r0, g0, b0 = couleur
    n = 0
    for i in range(0, len(rgb), 3):
        if abs(rgb[i] - r0) <= tol and abs(rgb[i + 1] - g0) <= tol and abs(rgb[i + 2] - b0) <= tol:
            n += 1
    return n


def part_differente(a: bytes, b: bytes) -> float:
    if len(a) != len(b) or not a:
        return 1.0
    diff = sum(1 for i in range(0, len(a), 3) if a[i:i + 3] != b[i:i + 3])
    return diff / (len(a) // 3)


def arguments_qemu(o, journal_serie: Path, port_moniteur: int) -> list[str]:
    """LE SEUL point qui depend de l'environnement d'execution."""
    a = ["-machine", "pc", "-m", str(o.memoire), "-smp", str(o.smp),
         "-drive", f"format=raw,file={o.boot}", "-drive", f"format=raw,file={o.disque},snapshot=on",
         "-nic", "user,model=e1000", "-vga", "std", "-display", "none", "-no-reboot",
         "-monitor", f"tcp:127.0.0.1:{port_moniteur},server=on,wait=off",
         "-serial", f"file:{journal_serie}"]
    if o.accel == "tcg":
        a += ["-accel", "tcg", "-cpu", "max"]
    elif o.accel == "kvm":
        a += ["-accel", "kvm", "-cpu", "host"]
    elif o.accel == "whpx":
        a += ["-accel", f"whpx,kernel-irqchip={o.whpx_irqchip}", "-cpu", CPU_WHPX]
    else:
        raise ValueError(o.accel)
    return a


def port_libre() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Banc:
    def __init__(self, o, sortie: Path):
        self.o, self.sortie = o, sortie
        self.serie = sortie / "serial.log"
        self.stderr = sortie / "qemu.stderr.log"
        self.captures = sortie / "captures"
        self.captures.mkdir(parents=True, exist_ok=True)
        self.journal_fixture = sortie / "fixture.jsonl"
        self.t0 = 0.0
        self.qemu = None
        self.fixture = None
        self.mon: Moniteur | None = None
        self.curseur = 0
        self.vars: dict[str, str] = {"FIXTURE": f"http://10.0.2.2:{o.port_fixture}"}
        self.instants: dict[str, float] = {}
        self.captures_rgb: dict[str, tuple] = {}

    # --- environnement -------------------------------------------------------
    def demarre(self) -> None:
        self.fixture = subprocess.Popen([sys.executable, str(ICI / "fixture.py"), str(self.o.port_fixture),
                                         str(self.journal_fixture)],
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        ligne = self.fixture.stdout.readline()
        if "E2E_FIXTURE_PRETE" not in ligne:
            raise RuntimeError(f"fixture : {ligne.strip()}")
        self.serie.write_text("")
        port = port_libre()
        args = [self.o.qemu] + arguments_qemu(self.o, self.serie, port)
        (self.sortie / "qemu.commande.txt").write_text(" ".join(args) + "\n")
        self.t0 = time.monotonic()
        self.instants["lancement_qemu"] = 0.0
        self.qemu = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=self.stderr.open("w"))
        self.mon = Moniteur(port)

    def arrete(self) -> None:
        for p in (self.qemu, self.fixture):
            if p and p.poll() is None:
                p.kill()
                try:
                    p.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    pass

    # --- lecture ------------------------------------------------------------
    def texte(self) -> str:
        try:
            return ANSI.sub("", self.serie.read_text(errors="replace")).replace("\r", "")
        except OSError:
            return ""

    def maintenant_ms(self) -> float:
        return (time.monotonic() - self.t0) * 1000

    def subst(self, motif: str, regex: bool) -> str:
        def r(m):
            v = self.vars.get(m.group(1), m.group(0))
            return re.escape(v) if regex else v
        return re.sub(r"\$\{(\w+)\}", r, motif)

    # --- etapes --------------------------------------------------------------
    def etape(self, e: dict) -> tuple[bool, str]:
        nom = e.get("nom")
        if "attend" in e:
            rx = re.compile(self.subst(e["attend"], True))
            fin = time.time() + float(e.get("delai_s", 60))
            while time.time() < fin:
                m = rx.search(self.texte(), self.curseur)
                if m:
                    self.curseur = m.end()
                    self.vars.update({k: v for k, v in m.groupdict().items() if v is not None})
                    if nom:
                        self.instants[nom] = self.maintenant_ms()
                    return True, ""
                if self.qemu.poll() is not None:
                    return False, f"QEMU arrete (code {self.qemu.returncode}) avant : {rx.pattern}"
                if any(p in self.texte() for p in PANIQUES):
                    return False, f"panique noyau avant : {rx.pattern}"
                time.sleep(0.25)
            return False, f"absent apres {e.get('delai_s', 60)} s : {rx.pattern}"
        if "absent" in e:
            depart = self.curseur
            time.sleep(float(e.get("pendant_s", 3)))
            m = re.compile(self.subst(e["absent"], True)).search(self.texte(), depart)
            return (m is None), (f"present alors qu'interdit : {m.group(0)[:160]}" if m else "")
        # Les actions : le curseur passe a la fin du journal AVANT le geste,
        # pour que l'attente suivante ne trouve pas un marqueur plus ancien.
        if "menu" in e:
            return self.ouvre_par_le_menu(e["menu"], nom)
        if "clic_client" in e:
            return self.clic_client(e["clic_client"], e.get("dy", 0), nom)
        if "touches" in e:
            self.curseur = len(self.texte())
            if nom:
                self.instants[nom] = self.maintenant_ms()
            for t in e["touches"]:
                self.mon.touche(combinaison(t))
            return True, ""
        if "tape" in e:
            self.curseur = len(self.texte())
            for t in touches_de(self.subst(e["tape"], False)):
                self.mon.touche(t)
            if nom:
                self.instants[nom] = self.maintenant_ms()
            if e.get("entree"):
                self.mon.touche("ret")
            return True, ""
        if "pause_s" in e:
            time.sleep(float(e["pause_s"]))
            return True, ""
        if "capture" in e:
            return self.capture(e)
        if "fixture_vu" in e:
            vus = [json.loads(l) for l in self.journal_fixture.read_text().splitlines() if l.strip()]
            ok = any(v["chemin"] == e["fixture_vu"] and (e.get("q") is None or v["q"] == e["q"]) for v in vus)
            return ok, "" if ok else f"le site de reference n'a pas servi {e['fixture_vu']} q={e.get('q')!r}"
        return False, f"etape inconnue : {e}"

    def ouvre_par_le_menu(self, libelle: str, nom: str | None) -> tuple[bool, str]:
        t = self.texte()
        dem = re.findall(r"BOUCHAUD_ATELIER_DEMARRER fenetres=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)", t)
        if not dem:
            return False, "geometrie du bouton Demarrer absente"
        _, x, y, w, h = map(int, dem[-1])
        self.curseur = len(t)
        self.mon.va(x + w // 2, y + h // 2)
        time.sleep(0.3)
        self.mon.clic()
        rx = re.compile(rf"BOUCHAUD_ATELIER_MENU_LIGNE libelle={re.escape(libelle)} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)")
        fin = time.time() + 30
        while time.time() < fin:
            m = rx.search(self.texte(), self.curseur)
            if m:
                lx, ly, lw, lh = map(int, m.groups())
                self.curseur = len(self.texte())
                self.mon.va(lx + lw // 2, ly + lh // 2)
                time.sleep(0.3)
                if nom:
                    self.instants[nom] = self.maintenant_ms()
                self.mon.clic()
                return True, ""
            time.sleep(0.25)
        return False, f"le menu Demarrer ne propose pas « {libelle} »"

    def clic_client(self, titre: str, dy: int, nom: str | None) -> tuple[bool, str]:
        """Clic dans la PAGE d'une fenetre cliente, comme un humain.

        Position : la zone utile publiee par le gestionnaire de fenetres
        (`BOUCHAUD_ATELIER_FENETRE_CLIENT`), centre horizontal, `dy` pixels
        sous son bord haut (la barre d'outils du navigateur comprise).
        """
        z = zone_client(self.texte(), titre)
        if z is None:
            return False, f"geometrie de la fenetre « {titre} » absente"
        x, y, w, h = z
        if not 0 <= dy < h:
            return False, f"dy={dy} hors de la zone utile (h={h})"
        self.curseur = len(self.texte())
        self.mon.va(x + w // 2, y + dy)
        time.sleep(0.3)
        if nom:
            self.instants[nom] = self.maintenant_ms()
        self.mon.clic()
        return True, ""

    def capture(self, e: dict) -> tuple[bool, str]:
        nom = e["capture"]
        ppm = self.captures / f"{nom}.ppm"
        self.mon.capture(ppm)
        fin = time.time() + 10
        while (not ppm.exists() or ppm.stat().st_size == 0) and time.time() < fin:
            time.sleep(0.2)
        if not ppm.exists():
            return False, "capture absente (screendump)"
        try:
            l, h, rgb = lit_ppm(ppm)
        except (ValueError, IndexError) as x:
            return False, f"capture illisible : {x}"
        ecrit_png(self.captures / f"{nom}.png", l, h, rgb)
        ppm.unlink()
        self.captures_rgb[nom] = (l, h, rgb)
        if "couleur" in e:
            n = pixels_de_couleur(rgb, _fixture.COULEURS[e["couleur"]])
            if n < int(e.get("min_px", 20000)):
                return False, f"bloc {e['couleur']} non visible a l'ecran : {n} px (min {e.get('min_px', 20000)})"
        if "different_de" in e:
            ref = self.captures_rgb.get(e["different_de"])
            if ref is None:
                return False, f"capture de reference {e['different_de']} absente"
            part = part_differente(ref[2], rgb)
            if part < 0.02:
                return False, f"ecran quasi identique a {e['different_de']} ({part:.1%} de pixels changes)"
        return True, ""


def statut_externe(banc: Banc, sc: dict, depart: int) -> str | None:
    ext = sc.get("externe")
    if ext and re.search(ext["motif_blocage"], banc.texte()[depart:]):
        return ext["raison"]
    return None


def joue(banc: Banc, sc: dict, resultats: dict) -> dict:
    r = {"id": sc["id"], "titre": sc["titre"], "bloquant": sc.get("bloquant", True), "etapes": [], "metriques": {}}
    if "non_implemente" in sc:
        r.update(statut="NOT_RUN", cause=f"non implemente : {sc['non_implemente']}")
        return r
    manque = [x for x in sc.get("requiert", []) if resultats.get(x, {}).get("statut") != "PASS"]
    if manque:
        r.update(statut="NOT_RUN", cause=f"prerequis non PASS : {', '.join(manque)}")
        return r
    depart = len(banc.texte())
    debut = banc.maintenant_ms()
    # Instants LOCAUX au parcours : des noms comme `requete` ou `resultats`
    # reviennent d'un parcours a l'autre. Sans remise a zero, E2E-04G bloque
    # sur Google rapporterait la latence du site de reference d'E2E-04, ou une
    # latence negative. Seule l'origine commune (lancement de QEMU) est gardee.
    banc.instants = {"lancement_qemu": banc.instants.get("lancement_qemu", 0.0)}
    r["statut"], r["cause"] = "PASS", ""
    for e in sc["etapes"]:
        t = banc.maintenant_ms()
        ok, cause = banc.etape(e)
        r["etapes"].append({"etape": e, "ok": ok, "t_hote_ms": round(t), "cause": cause})
        if not ok:
            ext = statut_externe(banc, sc, depart)
            r["statut"], r["cause"] = ("BLOCKED_EXTERNAL", f"{ext} ; etape : {cause}") if ext else ("FAIL", cause)
            break
    r["duree_ms"] = round(banc.maintenant_ms() - debut)
    for m in sc.get("metriques", []):
        v = None
        if "de" in m and m["de"] in banc.instants and m["a"] in banc.instants:
            v = round(banc.instants[m["a"]] - banc.instants[m["de"]])
        elif "marqueur" in m:
            x = re.search(m["marqueur"], banc.texte())
            v = int(x.group(1)) if x else None
        r["metriques"][m["nom"]] = v
    return r


def hote() -> dict:
    ram = None
    try:
        if sys.platform.startswith("linux"):
            ram = int(re.search(r"MemTotal:\s+(\d+)", Path("/proc/meminfo").read_text()).group(1)) // 1024
        elif os.name == "nt":
            import ctypes

            class M(ctypes.Structure):
                _fields_ = [("l", ctypes.c_ulong), ("c", ctypes.c_ulong), ("t", ctypes.c_ulonglong),
                            ("a", ctypes.c_ulonglong), ("tp", ctypes.c_ulonglong), ("ap", ctypes.c_ulonglong),
                            ("tv", ctypes.c_ulonglong), ("av", ctypes.c_ulonglong), ("e", ctypes.c_ulonglong)]
            m = M()
            m.l = ctypes.sizeof(M)
            ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(m))
            ram = m.t // (1024 * 1024)
    except Exception:  # la mesure absente reste absente
        ram = None
    return {"systeme": platform.platform(), "processeur": platform.processor() or None,
            "coeurs_logiques": os.cpu_count(), "ram_mio": ram, "python": platform.python_version()}


def version_qemu(qemu: str) -> str | None:
    try:
        return subprocess.run([qemu, "--version"], capture_output=True, text=True, timeout=20).stdout.splitlines()[0]
    except (OSError, IndexError, subprocess.TimeoutExpired):
        return None


def commit_git() -> str | None:
    try:
        return subprocess.run(["git", "-C", str(RACINE), "rev-parse", "HEAD"], capture_output=True,
                              text=True, timeout=20).stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def rapport_html(sortie: Path, manifeste: dict, resultats: list[dict]) -> None:
    couleur = {"PASS": "#2e7d32", "FAIL": "#c62828", "NOT_RUN": "#757575",
               "BLOCKED_EXTERNAL": "#ef6c00", "NON_MESURABLE": "#6a1b9a"}
    lignes = "".join(
        f"<tr><td>{r['id']}</td><td>{r['titre']}</td><td style='color:{couleur[r['statut']]}'><b>{r['statut']}</b></td>"
        f"<td>{r.get('cause', '')}</td><td>{', '.join(f'{k}={v}' for k, v in r.get('metriques', {}).items())}</td></tr>"
        for r in resultats)
    imgs = "".join(f"<figure><img src='captures/{p.name}' width='480'><figcaption>{p.stem}</figcaption></figure>"
                   for p in sorted((sortie / "captures").glob("*.png")))
    (sortie / "rapport.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>E2E Ladybird</title>"
        "<style>body{font-family:sans-serif}td{border:1px solid #ccc;padding:4px}figure{display:inline-block}</style>"
        f"<h1>E2E Ladybird -- {manifeste['environnement']['accel_reel']} SMP{manifeste['environnement']['smp']}</h1>"
        f"<pre>{json.dumps(manifeste, indent=1, ensure_ascii=False)}</pre>"
        f"<table><tr><th>id</th><th>parcours</th><th>statut</th><th>cause</th><th>metriques</th></tr>{lignes}</table>{imgs}",
        encoding="utf-8")


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--boot", required=True, type=Path)
    p.add_argument("--disque", required=True, type=Path)
    p.add_argument("--accel", required=True, choices=["tcg", "kvm", "whpx"])
    p.add_argument("--whpx-irqchip", dest="whpx_irqchip", default="off", choices=["off", "on"])
    p.add_argument("--smp", type=int, default=4)
    p.add_argument("--memoire", type=int, default=8192)
    p.add_argument("--qemu", default="qemu-system-x86_64")
    p.add_argument("--sortie", type=Path, default=None)
    p.add_argument("--scenarios", default="")
    p.add_argument("--port-fixture", dest="port_fixture", type=int, default=18300)
    p.add_argument("--exige", default="E2E-01,E2E-02")
    o = p.parse_args(argv[1:])
    for f in (o.boot, o.disque):
        if not f.is_file():
            print(f"E2E_REFUS fichier absent : {f}", file=sys.stderr)
            return 2
    o.boot, o.disque = o.boot.resolve(), o.disque.resolve()
    definition = json.loads((ICI / "scenarios.json").read_text(encoding="utf-8"))
    choisis = [s for s in definition["scenarios"] if not o.scenarios or s["id"] in o.scenarios.split(",")]
    sortie = (o.sortie or Path(tempfile.mkdtemp(prefix="bouchaud-e2e-"))).resolve()
    sortie.mkdir(parents=True, exist_ok=True)

    empreinte_disque = sha256(o.disque)
    manifeste = {
        "schema": "bouchaud-e2e-manifest/1", "definition": definition["schema"],
        "date_utc": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
        "commit": commit_git(), "bootimage_sha256": sha256(o.boot), "disque_sha256": empreinte_disque,
        "qemu": version_qemu(o.qemu), "hote": hote(),
        "environnement": {"accel_demande": o.accel, "accel_reel": None, "signature_cpuid": None, "smp": o.smp,
                          "memoire_mio": o.memoire, "affichage": "vga std, sans fenetre", "gpu": "aucun (VGA standard)"},
        "horloges": "t_hote_ms : horloge monotone de l'HOTE depuis le lancement de QEMU ; *_invite : horloge du noyau. Jamais melangees.",
    }
    banc = Banc(o, sortie)
    resultats: dict[str, dict] = {}
    try:
        banc.demarre()
        for sc in choisis:
            r = joue(banc, sc, resultats)
            resultats[sc["id"]] = r
            print(f"E2E {r['id']} {r['statut']} {r.get('cause', '')}".rstrip(), flush=True)
    except Exception as x:  # un incident du banc n'est jamais un PASS
        print(f"E2E_INCIDENT_BANC {type(x).__name__}: {x}", flush=True)
        for sc in choisis:
            resultats.setdefault(sc["id"], {"id": sc["id"], "titre": sc["titre"], "statut": "NON_MESURABLE",
                                            "cause": f"incident du banc : {x}", "etapes": [], "metriques": {}})
    finally:
        banc.arrete()

    texte = banc.texte()
    sig = re.search(r"BOUCHAUD_CPU_ISA hyperviseur=(\S+)", texte)
    if sig:
        manifeste["environnement"]["signature_cpuid"] = sig.group(1)
        manifeste["environnement"]["accel_reel"] = "TCG" if sig.group(1) == "TCGTCGTCGTCG" else "materiel"
    attendu = "TCG" if o.accel == "tcg" else "materiel"
    accel_ok = manifeste["environnement"]["accel_reel"] == attendu
    panique = [p for p in PANIQUES if p in texte]
    disque_intact = sha256(o.disque) == empreinte_disque

    liste = [resultats[sc["id"]] for sc in choisis]
    echecs = [r for r in liste if r["statut"] == "FAIL" and r.get("bloquant", True)]
    exiges = [x for x in o.exige.split(",") if x]
    manquants = [x for x in exiges if resultats.get(x, {}).get("statut") != "PASS"]
    verdict = "PASS" if not echecs and not manquants and accel_ok and not panique and disque_intact else "FAIL"
    compte = {s: sum(1 for r in liste if r["statut"] == s) for s in definition["statuts"]}
    resume = {"schema": "bouchaud-e2e-summary/1", "verdict": verdict, "compte": compte,
              "exiges_non_pass": manquants, "accelerateur_prouve": accel_ok, "panique": panique,
              "disque_intact": disque_intact}
    (sortie / "manifest.json").write_text(json.dumps(manifeste, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (sortie / "summary.json").write_text(json.dumps(resume, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (sortie / "results.json").write_text(json.dumps(liste, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (sortie / "performance.json").write_text(json.dumps({
        "schema": "bouchaud-e2e-performance/1", "environnement": manifeste["environnement"],
        "avertissement": "valeurs d'UN demarrage ; ne jamais melanger des environnements (TCG, KVM, WHPX, materiel)",
        "metriques": {r["id"]: r.get("metriques", {}) for r in liste}}, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (sortie / "failures.json").write_text(json.dumps([
        {"id": r["id"], "statut": r["statut"], "cause": r.get("cause")} for r in liste
        if r["statut"] in ("FAIL", "BLOCKED_EXTERNAL", "NON_MESURABLE")]
        + ([{"id": "global", "statut": "FAIL", "cause": "accelerateur reel different du demande"}] if not accel_ok else [])
        + ([{"id": "global", "statut": "FAIL", "cause": f"panique : {panique}"}] if panique else [])
        + ([{"id": "global", "statut": "FAIL", "cause": "disque modifie"}] if not disque_intact else []),
        indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    rapport_html(sortie, manifeste, liste)
    if verdict != "PASS":
        for ligne in causes_journal(sortie / "serial.log"):
            print("E2E_CAUSE " + ligne)
        for ligne in extrait_journal(sortie / "serial.log"):
            print("E2E_EXTRAIT " + ligne)
    print(f"E2E_VERDICT {verdict} " + " ".join(f"{k}={v}" for k, v in compte.items())
          + f" accel_reel={manifeste['environnement']['accel_reel']} sortie={sortie}", flush=True)
    return 0 if verdict == "PASS" else 1


def zone_client(texte: str, titre: str):
    """Derniere zone utile publiee pour la fenetre `titre` : (x, y, w, h)."""
    m = re.findall(rf"BOUCHAUD_ATELIER_FENETRE_CLIENT titre={re.escape(titre)} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)", texte)
    return tuple(map(int, m[-1])) if m else None


# Bilans periodiques du noyau : utiles dans serial.log, du bruit dans un extrait.
# Ils commencent par une etiquette entre crochets (`[MEM-NG-HEAP]`,
# `[kernel] [NET-TCP]`) ou portent un suffixe de bilan.
BRUIT = re.compile(r"\] \[(kernel\] \[)?[A-Z0-9][A-Za-z0-9_-]+\] |^\[[A-Z0-9][A-Za-z0-9_-]+\] |\[kernel\] [A-Z_]+_(BREAKDOWN|WAIT|DISK|MEMORY) "
                   r"|PROC-STAT|PERF_TRAMES|BATTEMENT|HEARTBEAT|_BILAN|_SAMPLE")
EXTRAIT_LIGNES = 80
# Ce qui explique un echec du navigateur : morts et fautes de processus,
# marqueurs Ladybird, paniques. Meme motif que le smoke BrowserHost.
CAUSES = re.compile(r"PROCESS_(DEATH|EXIT|FAULT)|FAULT_FATAL|PANIC|panicked|SIGSEGV|SIGILL|\[LB:|LADYBIRD|BROWSER_HOST"
                    r"|bo-navigateur|ladybird/[A-Za-z]+|EXEC_AVORTE|gui: navigateur")
CAUSES_LIGNES = 60


def causes_journal(chemin: Path, n: int = CAUSES_LIGNES) -> list[str]:
    """Les `n` premieres lignes qui expliquent un echec du navigateur."""
    try:
        texte = chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    lignes = (re.sub(r"\x1b\[[0-9;]*m", "", l).rstrip() for l in texte.splitlines())
    return [l[:300] for l in lignes if CAUSES.search(l)][:n]


def extrait_journal(chemin: Path, n: int = EXTRAIT_LIGNES) -> list[str]:
    """Les `n` dernieres lignes SIGNIFICATIVES du journal serie, sans couleurs.

    Un echec en CI doit se lire dans le journal du job lui-meme : les
    artefacts ne sont pas toujours accessibles a qui diagnostique.
    """
    try:
        texte = chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    lignes = [re.sub(r"\x1b\[[0-9;]*m", "", l).rstrip() for l in texte.splitlines()]
    return [l[:300] for l in lignes if l.strip() and not BRUIT.search(l)][-n:]


if __name__ == "__main__":
    sys.exit(main(sys.argv))
