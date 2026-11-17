#!/usr/bin/env python3
"""Banc du bureau ring 3 PILOTE A LA SOURIS ET AU CLAVIER (BOUCHAUD_ENTREES_RING3_V1).

Tranche 3 du bureau hors du noyau. `composited` (ring 3) recoit les entrees
du noyau (`bo_display_entrees`) et les route lui-meme (`bo_fenetre_entree`).
Ce banc le prouve avec de VRAIS gestes, envoyes par le moniteur QEMU au
pilote PS/2 du noyau -- rien n'est simule dans l'OS :

  1. clic dans le contenu de la fenetre du fond -> elle remonte, prend le
     foyer, et SON application (pid verifie) recoit le pointeur, bouton appuye
     puis relache, en coordonnees de son contenu ;
  2. glisser par la barre de titre -> la fenetre suit (position publiee) ;
  3. frappe au clavier -> seule l'application active la recoit (pid) ;
  4. une application plante (faute de page) -> le bureau et les entrees
     continuent : une frappe suivante arrive encore a l'application active ;
  5. clic sur la case de fermeture -> l'application sort d'elle-meme ;
  6. aucune panique, le compositeur survit.

Sortie : BANC_BUREAU_ENTREES_OK, ou BANC_BUREAU_ENTREES_ECHEC (code 1).

Sous Windows (WHPX), ou sans chaine musl : construire le disque une fois
sous Linux/WSL (`--construit-seulement`), puis le rejouer avec `--disque`
(monte en `snapshot=on`, jamais modifie), `--accel whpx --qemu CHEMIN` ; le
moniteur passe en TCP (`--moniteur tcp`, defaut sous Windows). Le disque et
le noyau d'un meme bras vont ensemble : un composited 1.6 appelle
`bo_shm_fill_rect`, qu'un noyau 1.5 ne connait pas.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
FATAL = re.compile(r"KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition")
PAS = 100


class Moniteur:
    def __init__(self, adresse):
        """`adresse` : un chemin (socket UNIX) ou un port TCP local (int)."""
        fin = time.time() + 30
        while True:
            try:
                if isinstance(adresse, int):
                    self.s = socket.create_connection(("127.0.0.1", adresse), timeout=5)
                else:
                    self.s = socket.socket(socket.AF_UNIX)
                    self.s.connect(adresse)
                break
            except OSError:
                if time.time() > fin:
                    raise
                time.sleep(0.2)
        self.s.settimeout(5)
        self._lit()

    def _lit(self):
        donnees = b""
        fin = time.time() + 5
        while b"(qemu)" not in donnees and time.time() < fin:
            try:
                donnees += self.s.recv(4096)
            except socket.timeout:
                break
        return donnees

    def cmd(self, c: str):
        self.s.sendall(c.encode() + b"\n")
        self._lit()

    def bouge(self, dx: int, dy: int):
        while dx or dy:
            px, py = max(-PAS, min(PAS, dx)), max(-PAS, min(PAS, dy))
            self.cmd(f"mouse_move {px} {py}")
            dx, dy = dx - px, dy - py
            time.sleep(0.03)

    def va(self, x: int, y: int):
        self.bouge(-4000, -4000)
        self.bouge(x, y)

    def bouton(self, appui: bool):
        self.cmd(f"mouse_button {1 if appui else 0}")


class Banc:
    def __init__(self, o):
        self.o = o
        self.sortie = Path(o.sortie).resolve()
        if self.sortie.exists():
            shutil.rmtree(self.sortie)
        (self.sortie / "bin").mkdir(parents=True)
        self.log = self.sortie / "serie.log"
        self.echecs: list[str] = []

    # --- construction --------------------------------------------------------
    def construit(self):
        if self.o.disque:
            # Disque deja construit (Windows, ou meme disque pour tous les bras).
            self.disque = Path(self.o.disque).resolve()
            if not self.disque.is_file():
                raise FileNotFoundError(f"disque absent : {self.disque}")
            return
        env = dict(os.environ, OUT=str(self.sortie / "bin"))
        subprocess.run(["bash", "./build.sh"], cwd=RACINE / "userland/services/composited", env=env,
                       check=True, stdout=subprocess.DEVNULL)
        for nom in ("mire-gpu", "attends"):
            subprocess.run(["musl-gcc", "-O2", "-static-pie", "-fPIE", "-o", str(self.sortie / "bin" / nom),
                            str(RACINE / "tools/userland" / f"{nom}.c")], check=True)
        scen = self.sortie / "scen"
        (scen / "bin").mkdir(parents=True)
        shutil.copy(self.sortie / "bin/composited-slice", scen / "bin/composited")
        shutil.copy(self.sortie / "bin/attends", scen / "bin/attends")
        shutil.copy(self.sortie / "bin/mire-gpu", scen / "mire-gpu")
        for f in (scen / "bin/composited", scen / "bin/attends", scen / "mire-gpu"):
            f.chmod(0o755)
        # Trois applications : la troisieme plante d'elle-meme (faute de page).
        (scen / "bureau-ring3").write_text(
            "/mire-gpu\n/mire-gpu\n/mire-gpu BO_MIRE_FAUTE_APRES=%d\n" % self.o.faute_apres)
        (scen / "garde-vm-vivante").write_text("")
        (scen / "autorun").write_text("echo BUREAU_RING3_AUTORUN_BEGIN\ndesktop\n/bin/attends 600\n"
                                      "echo BUREAU_RING3_AUTORUN_END\n")
        self.disque = self.sortie / "disque.img"
        subprocess.run(["./mkdisk.sh", str(scen)], cwd=RACINE / "tools/userland",
                       env=dict(os.environ, IMAGE=str(self.disque)), check=True, stdout=subprocess.DEVNULL)

    # --- QEMU ----------------------------------------------------------------
    def demarre(self):
        if self.o.accel == "kvm":
            accel = ["-enable-kvm", "-cpu", "host"]
        elif self.o.accel == "whpx":
            # BOUCHAUD_WHPX_CPU_V1 (tools/windows/BouchaudQemu.psm1) : jamais
            # `-cpu max` sous WHPX ; qemu64 + SSE4, comme le reste des bancs.
            accel = ["-accel", f"whpx,kernel-irqchip={self.o.whpx_irqchip}",
                     "-cpu", "qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt"]
        else:
            accel = ["-cpu", "max"]
        moniteur = self.o.moniteur or ("tcp" if os.name == "nt" else "unix")
        if moniteur == "tcp":
            with socket.socket() as libre:
                libre.bind(("127.0.0.1", 0))
                port = libre.getsockname()[1]
            arg_moniteur, adresse = f"tcp:127.0.0.1:{port},server=on,wait=off", port
        else:
            self.socket = tempfile.mktemp(prefix="bo-entrees-", suffix=".sock")
            arg_moniteur, adresse = f"unix:{self.socket},server,nowait", self.socket
        disque = f"format=raw,file={self.disque}" + (",snapshot=on" if self.o.disque else "")
        self.qemu = subprocess.Popen(
            [self.o.qemu, *accel, "-drive", f"format=raw,file={self.o.boot}",
             "-drive", disque, "-m", str(self.o.memoire), "-smp", str(self.o.smp),
             "-display", "none", "-no-reboot", "-monitor", arg_moniteur,
             "-serial", f"file:{self.log}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.mon = Moniteur(adresse)

    def arrete(self):
        if getattr(self, "qemu", None) and self.qemu.poll() is None:
            self.qemu.terminate()
            try:
                self.qemu.wait(10)
            except subprocess.TimeoutExpired:
                self.qemu.kill()

    def texte(self) -> str:
        """Le journal serie, sans couleurs ni prefixe d'horodatage."""
        try:
            brut = self.log.read_bytes().decode(errors="replace")
        except OSError:
            return ""
        brut = re.sub(r"\x1b\[[0-9;]*m", "", brut).replace("\r", "")
        return re.sub(r"(?m)^\[[^\]\n]*\]\[[^\]\n]*\]\[FPS:[^\]\n]*\] ?", "", brut)

    def attend(self, motif: str, delai: float, depuis: int = 0):
        rx, fin = re.compile(motif), time.time() + delai
        while time.time() < fin:
            t = self.texte()
            if FATAL.search(t):
                return None
            m = None
            for m in rx.finditer(t, depuis):
                pass
            if m:
                return m
            if self.qemu.poll() is not None:
                return None
            time.sleep(0.2)
        return None

    def taille(self) -> int:
        return len(self.texte())

    def exige(self, cond, libelle: str):
        print(("  ok      " if cond else "  ECHEC   ") + libelle, flush=True)
        if not cond:
            self.echecs.append(libelle)

    # --- etat publie par le compositeur ---------------------------------------
    def fenetres(self) -> dict[int, dict]:
        """Derniere geometrie publiee de chaque fenetre (COMPOSITED_BUREAU_FENETRE)."""
        res, courant, vals = {}, None, {}
        for ligne in self.texte().split("\n"):
            m = re.search(r"COMPOSITED_BUREAU_FENETRE indice=(\d+)", ligne)
            if m:
                courant, vals = int(m.group(1)), {}
                continue
            m = re.search(r"\[COMPOSITED\] (pid|x|y|l|h|rang)=(-?\d+)", ligne)
            if courant is not None and m:
                vals[m.group(1)] = int(m.group(2))
                if len(vals) == 6:
                    res[courant], courant = vals, None
        return res

    def clients(self) -> list[int]:
        return [int(p) for p in re.findall(r"BOUCHAUD_BUREAU_RING3_CLIENT pid=(\d+)", self.texte())]

    def ligne_pid(self, motif: str, depuis: int) -> list[int]:
        """pids des lignes `motif` ecrites par une application apres `depuis`."""
        pids = []
        for ligne in self.texte()[depuis:].split("\n"):
            if motif in ligne:
                m = re.search(r"\bpid[ =:](\d+)|\[(\d+)\]|\bp(\d+)\b", ligne)
                pids.append(int(next(g for g in m.groups() if g)) if m else -1)
        return pids

    # --- scenario --------------------------------------------------------------
    def joue(self):
        o = self.o
        self.exige(self.attend(r"COMPOSITED_BUREAU_FENETRE indice=1", 240) is not None,
                   "deux fenetres composees par composited (ring 3)")
        self.exige(self.attend(r"COMPOSITED_ENTREES_ATTENDUES", 5) is not None,
                   "la capacite d'affichage reveille le bureau (waitset)")
        time.sleep(2)
        f = self.fenetres()
        if 0 not in f or 1 not in f:
            self.exige(False, "geometrie des fenetres publiee")
            return
        f0 = f[0]
        pids = self.clients()
        self.exige(len(pids) >= 2 and len(set(pids)) == len(pids), f"applications distinctes {pids}")

        # 1. Clic dans le contenu de la fenetre 0 (sous la 1).
        cx, cy = f0["x"] + 30, f0["y"] + 30
        avant = self.taille()
        t0 = time.perf_counter()
        self.mon.va(cx, cy)
        time.sleep(0.4)
        self.mon.bouton(True)
        m = self.attend(r"COMPOSITED_ENTREE_CLIC fenetre=0 x=(\d+) y=(\d+)", 15, avant)
        self.exige(m is not None, "clic route : COMPOSITED_ENTREE_CLIC fenetre=0")
        if m:
            self.exige(abs(int(m.group(1)) - 30) <= 2 and abs(int(m.group(2)) - 30) <= 2,
                       f"coordonnees du contenu ({m.group(1)},{m.group(2)}) ~ (30,30)")
        self.exige(self.attend(r"COMPOSITED_ENTREE_FOYER fenetre=0", 10, avant) is not None,
                   "la fenetre cliquee prend le foyer")
        time.sleep(0.2)
        self.mon.bouton(False)
        m = self.attend(r"MIRE_GPU_ENTREE pointeur x=\d+ y=\d+ boutons=0", 15, avant)
        self.exige(self.attend(r"MIRE_GPU_ENTREE pointeur x=(\d+) y=(\d+) boutons=1", 1, avant) is not None
                   and m is not None, "l'APPLICATION recoit l'appui puis le relachement (protocole GUI)")
        self.latence_clic_s = time.perf_counter() - t0

        # 1b. Appui dans le contenu, relachement HORS du contenu (a gauche et
        # au-dessus) : l'application capturee recoit quand meme le relachement,
        # en coordonnees NEGATIVES (revue #335 : un u32 replie etait refuse
        # par le noyau et le relachement perdu).
        avant = self.taille()
        self.mon.bouton(True)
        time.sleep(0.3)
        self.mon.bouge(-60, -50)
        time.sleep(0.4)
        self.mon.bouton(False)
        r = self.attend(r"MIRE_GPU_ENTREE pointeur x=(-?\d+) y=(-?\d+) boutons=0", 15, avant)
        self.exige(r is not None and int(r.group(1)) < 0 and int(r.group(2)) < 0,
                   "relachement hors du contenu recu en coordonnees negatives "
                   f"({r.group(1) + ',' + r.group(2) if r else 'absent'})")

        # 2. Glisser la fenetre 0 par sa barre de titre.
        f0 = self.fenetres()[0]
        bx, by = f0["x"] + 60, f0["y"] - 12
        avant = self.taille()
        self.mon.va(bx, by)
        time.sleep(0.4)
        self.mon.bouton(True)
        prise = self.attend(r"COMPOSITED_ENTREE_PRISE fenetre=0", 15, avant)
        self.exige(prise is not None, "la barre de titre est saisie")
        for _ in range(6):
            self.mon.bouge(25, 15)
            time.sleep(0.15)
        time.sleep(0.5)
        self.mon.bouton(False)
        dep = self.attend(r"COMPOSITED_ENTREE_DEPOSE fenetre=0 x=(\d+) y=(\d+)", 20, avant)
        self.exige(dep is not None, "la fenetre est deposee")
        if dep:
            dx, dy = int(dep.group(1)) - f0["x"], int(dep.group(2)) - f0["y"]
            self.exige(abs(dx - 150) <= 3 and abs(dy - 90) <= 3, f"deplacement suivi ({dx},{dy}) ~ (150,90)")
            m = self.attend(r"\[COMPOSITED\] deplacement_us=(\d+)", 5, avant)
            self.deplacement_us = int(m.group(1)) if m else None
            g = self.attend(r"COMPOSITED_GLISSER_MESURE pas=(\d+) moyen_us=(\d+) max_us=(\d+) "
                            r"appels_region_par_pas=(\d+) copie_us=(\d+) presentation_us=(\d+)", 5, avant)
            if g:
                self.glisser = dict(zip(("pas", "moyen_us", "max_us", "appels_par_pas", "copie_us", "presentation_us"),
                                        (int(v) for v in g.groups())))

        # 3. Clavier : seule l'application active recoit la frappe.
        avant = self.taille()
        for k in ("a", "b"):
            self.mon.cmd(f"sendkey {k}")
            time.sleep(0.3)
        self.exige(self.attend(r"COMPOSITED_ENTREE_TOUCHE fenetre=0 code=0 unicode=98", 15, avant) is not None,
                   "frappe routee a la fenetre active (0)")
        # Le clavier du noyau est AZERTY : la touche `a` du moniteur (scancode
        # US) produit 'q' (113).
        recues = re.findall(r"MIRE_GPU_ENTREE touche code=0 unicode=(\d+)", self.texte()[avant:])
        self.exige(recues[:2] == ["113", "98"], f"l'application recoit 'q' puis 'b' (une seule fois) : {recues}")

        # 4. Une application plante : le bureau et les entrees continuent.
        mort = self.attend(r"COMPOSITED_BUREAU_CLIENT_MORT pid=(\d+)", o.attente_faute)
        self.exige(mort is not None, "une application plante (faute de page) et le compositeur le voit")
        avant = self.taille()
        self.mon.cmd("sendkey c")
        self.exige(self.attend(r"MIRE_GPU_ENTREE touche code=0 unicode=99", 15, avant) is not None,
                   "apres le plantage, la frappe arrive encore a l'application active")
        self.mon.bouge(-20, 10)
        time.sleep(0.5)

        # 5. Case de fermeture de la fenetre 1.
        f = self.fenetres()
        if 1 in f:
            f1 = f[1]
            avant = self.taille()
            self.mon.va(f1["x"] + f1["l"] - 11, f1["y"] - 12)
            time.sleep(0.4)
            self.mon.bouton(True)
            time.sleep(0.2)
            self.mon.bouton(False)
            self.exige(self.attend(r"COMPOSITED_ENTREE_FERME fenetre=1", 15, avant) is not None,
                       "clic sur la case de fermeture")
            self.exige(self.attend(r"COMPOSITED_BUREAU_CLIENT_MORT pid=\d+", 20, avant) is not None,
                       "l'application fermee sort d'elle-meme")
        else:
            self.exige(False, "fenetre 1 encore visible pour la fermeture")
        time.sleep(1)
        t = self.texte()
        self.exige(not FATAL.search(t), "aucune panique noyau")
        self.exige("BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur" in t,
                   "bureau noyau refuse : le bureau est le compositeur ring 3")
        self.exige("BOUCHAUD_ENTREES_RING3_ACTIVES" in t, "entrees confiees au compositeur")

    def bilan(self) -> int:
        print(f"BANC_BUREAU_ENTREES etiquette={self.o.etiquette or '-'} accel={self.o.accel} smp={self.o.smp} "
              f"latence_clic_hote_ms={getattr(self, 'latence_clic_s', 0) * 1000:.0f} "
              f"deplacement_us={getattr(self, 'deplacement_us', None)}")
        g = getattr(self, "glisser", None)
        if g:
            # Mesure, pas un seuil : le banc juge les comportements, le chiffre
            # sert l'A/B (avant/apres) sur la meme machine.
            print(f"BANC_BUREAU_ENTREES_GLISSER pas={g['pas']} moyen_us={g['moyen_us']} "
                  f"max_us={g['max_us']} appels_region_par_pas={g['appels_par_pas']} "
                  f"copie_us={g['copie_us']} presentation_us={g['presentation_us']}")
        if self.echecs:
            print("BANC_BUREAU_ENTREES_ECHEC " + " | ".join(self.echecs))
            return 1
        print("BANC_BUREAU_ENTREES_OK")
        return 0


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", required=True)
    ap.add_argument("--sortie", default="target/bureau-ring3-entrees")
    ap.add_argument("--accel", choices=("tcg", "kvm", "whpx"), default="tcg")
    ap.add_argument("--whpx-irqchip", default="off")
    ap.add_argument("--qemu", default="qemu-system-x86_64")
    ap.add_argument("--memoire", type=int, default=2048)
    ap.add_argument("--moniteur", choices=("unix", "tcp"), default=None,
                    help="defaut : tcp sous Windows, unix ailleurs")
    ap.add_argument("--disque", help="disque deja construit (monte en snapshot=on)")
    ap.add_argument("--construit-seulement", action="store_true",
                    help="construit SORTIE/disque.img et s'arrete (pour --disque ailleurs)")
    ap.add_argument("--etiquette", default="", help="nom du bras, repris dans le bilan")
    ap.add_argument("--smp", type=int, default=2)
    ap.add_argument("--faute-apres", type=int, default=900, help="trames avant la faute de la 3e application")
    ap.add_argument("--attente-faute", type=float, default=180)
    o = ap.parse_args(argv)
    banc = Banc(o)
    if o.construit_seulement:
        banc.construit()
        print(f"BANC_BUREAU_ENTREES_DISQUE {banc.disque}")
        return 0
    try:
        banc.construit()
        banc.demarre()
        banc.joue()
    except Exception as exc:  # noqa: BLE001
        banc.echecs.append(f"exception {exc!r}")
        print(f"BANC_BUREAU_ENTREES_ECHEC raison={exc!r}")
    finally:
        banc.arrete()
    return banc.bilan()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
