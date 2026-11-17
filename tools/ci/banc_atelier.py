#!/usr/bin/env python3
"""Banc de reactivite de l'Atelier (BOUCHAUD_BANC_ATELIER_V1).

    banc_atelier.py --boot BOOTIMAGE --sortie DOSSIER [--accel tcg|kvm|whpx]
                    [--smp N] [--memoire MIO] [--qemu CHEMIN] [--cycles N]
                    [--duree S] [--etiquette NOM]

Demarre le bureau Atelier dans QEMU et le pilote comme un utilisateur, par le
moniteur (souris et clavier PS/2), sans rien simuler dans l'OS. Phases :

    bureau_vide        le curseur parcourt le bureau, aucune fenetre
    ouverture          clic sur Insights dans le menu Demarrer, puis 1,5 s
    fermeture          clic sur le bouton Fermer, puis 1 s
    survol             le curseur parcourt la fenetre ouverte
    deplacement        la fenetre est tiree par sa barre de titre
    redimensionnement  le coin bas droit est tire, aller et retour

Le noyau ecrit une ligne `BOUCHAUD_PERF_TRAME` par trame composee
(`BO_ATELIER_PERF_TRAME=1`, exporte par l'autorun du banc). Chaque phase est
delimitee par la position dans le journal serie au debut et a la fin du geste ;
on en tire, par phase : trames, p50/p95/p99/max (us), FPS observes (trames
composees par seconde de geste), part des fenetres, E/S PCI et CMOS par trame,
`cpu_pct` du fil `desktop` ([PROC-SAMPLE]) et pics `HID_LATENCY_SPIKE`.

Portable : moniteur en TCP local, disque construit en Python (meme format que
tools/userland/mkdisk.sh). Sous Windows : `--accel whpx --qemu
"C:/Program Files/qemu/qemu-system-x86_64.exe"`.

Sorties : SORTIE/serie.log, SORTIE/resultat.json et des lignes
`BANC_ATELIER phase=... ` sur la sortie standard. Code 1 sur panique, phase sans
trame ou geste impossible -- jamais un vert sans mesure.
"""
import argparse
import io
import json
import os
import re
import socket
import subprocess
import sys
import tarfile
import time
from pathlib import Path

PAS = 120  # pas maximal d'un mouse_move (un paquet PS/2)


# --- disque de scenario (format de tools/userland/mkdisk.sh) ----------------

def disque_scenario(chemin: Path, autorun: str) -> None:
    tampon = io.BytesIO()
    with tarfile.open(fileobj=tampon, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        racine = tarfile.TarInfo("./")
        racine.type = tarfile.DIRTYPE
        racine.mode = 0o755
        tar.addfile(racine)
        donnees = autorun.encode()
        info = tarfile.TarInfo("./autorun")
        info.size = len(donnees)
        info.mode = 0o644
        tar.addfile(info, io.BytesIO(donnees))
    image = bytearray(tampon.getvalue())
    image += b"\0" * ((-len(image)) % 512)
    plancher = (1025 + 1) * 512  # SECTEUR_CONTENU + 1, comme mkdisk.sh
    if len(image) < plancher:
        image += b"\0" * (plancher - len(image))
    with open(chemin, "wb") as f:
        f.write(image)
        f.truncate(len(image) + 262144 * 512)  # zone persistante de mkdisk.sh


# --- moniteur QEMU ----------------------------------------------------------

class Moniteur:
    def __init__(self, port: int):
        fin = time.time() + 60
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

    def cmd(self, c: str) -> None:
        self.s.sendall(c.encode() + b"\n")
        self._invite()

    def bouge(self, dx: int, dy: int) -> None:
        while dx or dy:
            px, py = max(-PAS, min(PAS, dx)), max(-PAS, min(PAS, dy))
            self.cmd(f"mouse_move {px} {py}")
            dx, dy = dx - px, dy - py

    def va(self, x: int, y: int) -> None:
        self.bouge(-4000, -4000)
        self.bouge(x, y)

    def bouton(self, appui: bool) -> None:
        self.cmd(f"mouse_button {1 if appui else 0}")

    def clic(self) -> None:
        self.bouton(True)
        time.sleep(0.04)
        self.bouton(False)


# --- banc -------------------------------------------------------------------

def port_libre() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Banc:
    def __init__(self, o, sortie: Path):
        self.o, self.sortie = o, sortie
        self.log = sortie / "serie.log"
        self.phases = []
        self.position = (0, 0)

    def texte(self, depuis: int = 0) -> str:
        try:
            with open(self.log, "rb") as f:
                f.seek(depuis)
                return f.read().decode(errors="replace")
        except OSError:
            return ""

    def taille(self) -> int:
        try:
            return self.log.stat().st_size
        except OSError:
            return 0

    def attend(self, motif: str, delai: float, depuis: int = 0):
        rx, fin = re.compile(motif), time.time() + delai
        while time.time() < fin:
            m = None
            for m in rx.finditer(self.texte(depuis)):
                pass
            if m:
                return m
            if self.qemu.poll() is not None:
                return None
            time.sleep(0.05)
        return None

    def demarre(self) -> None:
        disque = self.sortie / "scenario.img"
        disque_scenario(disque, "export BO_ATELIER_PERF_TRAME=1\n"
                        f"export BO_ATELIER_ANIMATIONS={self.o.animations}\ndesktop\n")
        port = port_libre()
        a = [self.o.qemu, "-machine", "pc", "-m", str(self.o.memoire), "-smp", str(self.o.smp),
             "-drive", f"format=raw,file={self.o.boot},snapshot=on",
             "-drive", f"format=raw,file={disque},snapshot=on",
             "-vga", "std", "-nic", "none", "-display", "none", "-no-reboot",
             "-monitor", f"tcp:127.0.0.1:{port},server=on,wait=off",
             "-serial", f"file:{self.log}"]
        if self.o.accel == "tcg":
            a += ["-accel", "tcg", "-cpu", "max"]
        elif self.o.accel == "kvm":
            a += ["-accel", "kvm", "-cpu", "host"]
        else:
            a += ["-accel", f"whpx,kernel-irqchip={self.o.whpx_irqchip}", "-cpu", "qemu64,+rdrand"]
        self.qemu = subprocess.Popen(a, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.mon = Moniteur(port)

    def arrete(self) -> None:
        try:
            self.mon.cmd("quit")
        except Exception:
            pass
        try:
            self.qemu.wait(10)
        except subprocess.TimeoutExpired:
            self.qemu.kill()

    # Une phase : un geste, borne par la position dans le journal.
    def phase(self, nom: str, geste) -> None:
        debut_octet, debut = self.taille(), time.perf_counter()
        geste()
        duree = time.perf_counter() - debut
        time.sleep(0.3)  # les dernieres trames du geste
        self.phases.append((nom, debut_octet, self.taille(), duree))

    def parcours(self, x0: int, y0: int, largeur: int, hauteur: int, secondes: float,
                 pas: int = 24) -> None:
        """Le curseur fait des allers-retours en zigzag dans le rectangle."""
        self.mon.va(x0, y0)
        x, y, sens, fin = x0, y0, 1, time.perf_counter() + secondes
        while time.perf_counter() < fin:
            nx = x + sens * pas
            if nx < x0 or nx > x0 + largeur:
                sens = -sens
                ny = y + pas
                if ny > y0 + hauteur:
                    ny = y0
                self.mon.bouge(0, ny - y)
                y = ny
                continue
            self.mon.bouge(nx - x, 0)
            x = nx
            time.sleep(0.008)

    def geometrie(self, depuis: int = 0):
        m = self.attend(r"BOUCHAUD_ATELIER_FENETRE_GEOMETRIE x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) "
                        r"fermer_x=(-?\d+) fermer_y=(-?\d+) titre=([^\r\n]*)", 30, depuis)
        if not m:
            raise RuntimeError("geometrie de fenetre non publiee")
        return tuple(map(int, m.groups()[:6]))

    def ouvre_insights(self) -> tuple:
        t = self.texte()
        dem = re.findall(r"BOUCHAUD_ATELIER_DEMARRER fenetres=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)", t)
        if not dem:
            raise RuntimeError("geometrie du bouton Demarrer absente")
        _, x, y, w, h = map(int, dem[-1])
        depart = self.taille()
        self.mon.va(x + w // 2, y + h // 2)
        time.sleep(0.2)
        self.mon.clic()
        m = self.attend(r"BOUCHAUD_ATELIER_MENU_LIGNE libelle=Insights x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)",
                        20, depart)
        if not m:
            raise RuntimeError("le menu Demarrer ne propose pas Insights")
        lx, ly, lw, lh = map(int, m.groups())
        self.mon.va(lx + lw // 2, ly + lh // 2)
        time.sleep(0.2)
        avant = self.taille()
        t0 = time.perf_counter()
        self.mon.clic()
        if not self.attend(r"BOUCHAUD_ATELIER_FENETRE_OUVERTE titre=", 20, avant):
            raise RuntimeError("Insights ne s'ouvre pas")
        self.latences_ouverture.append(round((time.perf_counter() - t0) * 1000, 1))
        return self.geometrie(avant)

    def joue(self) -> None:
        o = self.o
        self.latences_ouverture = []
        if not self.attend(r"BOUCHAUD_BOOT_POINT bureau-premiere-trame", o.demarrage):
            raise RuntimeError("le bureau n'a pas compose sa premiere trame")
        time.sleep(o.repos)
        self.phase("bureau_vide", lambda: self.parcours(200, 200, 700, 300, o.duree))
        geo = None
        for _ in range(o.cycles):
            def ouverture():
                nonlocal geo
                geo = self.ouvre_insights()
                time.sleep(1.5)
            self.phase("ouverture", ouverture)
            x, y, w, h, fx, fy = geo

            def fermeture():
                self.mon.va(fx, fy)
                time.sleep(0.15)
                avant = self.taille()
                self.mon.clic()
                time.sleep(1.0)
                if "BOUCHAUD_ATELIER_FENETRE_FERMEE titre=" not in self.texte(avant):
                    raise RuntimeError("la fenetre ne s'est pas fermee")
            self.phase("fermeture", fermeture)
        geo = self.ouvre_insights()
        time.sleep(1.5)
        x, y, w, h, fx, fy = geo
        self.phase("survol", lambda: self.parcours(x + 20, y + 60, max(40, w - 60),
                                                   max(40, h - 100), o.duree))

        def deplacement():
            self.mon.va(x + 60, y + 12)
            time.sleep(0.15)
            self.mon.bouton(True)
            fin = time.perf_counter() + o.duree
            k = 0
            while time.perf_counter() < fin:
                dx, dy = [(16, 0), (0, 12), (-16, 0), (0, -12)][(k // 10) % 4]
                self.mon.bouge(dx, dy)
                k += 1
                time.sleep(0.008)
            self.mon.bouton(False)
        avant = self.taille()
        self.phase("deplacement", deplacement)
        x, y, w, h, fx, fy = self.geometrie(avant)

        def redimensionnement():
            self.mon.va(x + w - 3, y + h - 3)
            time.sleep(0.15)
            self.mon.bouton(True)
            fin = time.perf_counter() + o.duree
            k = 0
            while time.perf_counter() < fin:
                d = 12 if (k // 12) % 2 == 0 else -12
                self.mon.bouge(d, d)
                k += 1
                time.sleep(0.008)
            self.mon.bouton(False)
        self.phase("redimensionnement", redimensionnement)

    def analyse(self) -> dict:
        resultat = {"accel": self.o.accel, "smp": self.o.smp, "memoire_mio": self.o.memoire,
                    "etiquette": self.o.etiquette, "latences_ouverture_ms": self.latences_ouverture,
                    "phases": {}}
        rx = re.compile(r"BOUCHAUD_PERF_TRAME t_ms=(\d+) us=(\d+) fenetres_us=(\d+) "
                        r"presentation_us=(\d+) pci_acces=(\d+) cmos_lectures=(\d+)")
        cumul = {}
        for nom, a, b, duree in self.phases:
            t = self.texte(a)[: max(0, b - a)]
            c = cumul.setdefault(nom, {"trames": [], "duree": 0.0, "cpu": [], "hid": []})
            c["trames"] += [tuple(map(int, m.groups())) for m in rx.finditer(t)]
            c["duree"] += duree
            c["cpu"] += [int(v) for v in re.findall(r"\[PROC-SAMPLE\][^\n]* name=desktop cpu_pct=(\d+)", t)]
            c["hid"] += [int(v) for v in re.findall(r"HID_LATENCY_SPIKE delta_us=(\d+)", t)]
        for nom, c in cumul.items():
            us = sorted(f[1] for f in c["trames"])
            n = len(us)
            q = (lambda p: us[min(n, max(1, -(-p * n // 100))) - 1]) if n else (lambda p: None)
            moy = (lambda i: round(sum(f[i] for f in c["trames"]) / n, 1)) if n else (lambda i: None)
            resultat["phases"][nom] = {
                "trames": n, "duree_s": round(c["duree"], 2),
                "fps": round(n / c["duree"], 1) if c["duree"] else None,
                "p50_us": q(50), "p95_us": q(95), "p99_us": q(99), "max_us": us[-1] if n else None,
                "moy_us": moy(1), "fenetres_moy_us": moy(2), "presentation_moy_us": moy(3),
                "pci_acces_par_trame": moy(4), "cmos_lectures_par_trame": moy(5),
                "desktop_cpu_pct_max": max(c["cpu"]) if c["cpu"] else None,
                "hid_latence_pics": len(c["hid"]), "hid_latence_max_us": max(c["hid"]) if c["hid"] else None,
            }
        return resultat


def main(argv) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--boot", required=True)
    p.add_argument("--sortie", required=True)
    p.add_argument("--accel", choices=["tcg", "kvm", "whpx"], default="tcg")
    p.add_argument("--whpx-irqchip", default="off")
    p.add_argument("--smp", type=int, default=2)
    p.add_argument("--memoire", type=int, default=2048)
    p.add_argument("--qemu", default="qemu-system-x86_64")
    p.add_argument("--cycles", type=int, default=3)
    p.add_argument("--duree", type=float, default=6.0)
    p.add_argument("--repos", type=float, default=5.0)
    p.add_argument("--demarrage", type=float, default=300.0)
    p.add_argument("--animations", default="normales")
    p.add_argument("--etiquette", default="")
    o = p.parse_args(argv)
    sortie = Path(o.sortie)
    sortie.mkdir(parents=True, exist_ok=True)
    banc = Banc(o, sortie)
    code = 0
    try:
        banc.demarre()
        banc.joue()
    except Exception as exc:  # le journal dira pourquoi
        print(f"BANC_ATELIER_ECHEC raison={exc}")
        code = 1
    finally:
        banc.arrete()
    texte = banc.texte()
    if re.search(r"KERNEL PANIC|panicked at", texte):
        print("BANC_ATELIER_ECHEC raison=panique_noyau")
        code = 1
    resultat = banc.analyse()
    (sortie / "resultat.json").write_text(json.dumps(resultat, indent=2))
    for nom, r in resultat["phases"].items():
        print("BANC_ATELIER " + " ".join([f"phase={nom}"] + [f"{k}={v}" for k, v in r.items()]))
        if not r["trames"]:
            print(f"BANC_ATELIER_ECHEC raison=aucune_trame_mesuree phase={nom}")
            code = 1
    print(f"BANC_ATELIER latences_ouverture_ms={resultat['latences_ouverture_ms']}")
    print("BANC_ATELIER_OK" if code == 0 else "BANC_ATELIER_ECHEC")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
