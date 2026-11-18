#!/usr/bin/env python3
"""Banc E2E du BUREAU RING 3 UTILISABLE (BOUCHAUD_BUREAU_UTILISABLE_V1).

Une image de test complete ou le bureau noyau ne tourne pas : `composited`
(ring 3) affiche le bureau et son dock ; aucune application n'est lancee au
demarrage. Le banc, avec de VRAIS gestes (moniteur QEMU -> pilotes PS/2 du
noyau -> compositeur ring 3 -> application), joue :

  1. le bureau s'affiche (dock, deux applications), bureau noyau refuse ;
  2. clic sur le dock -> la calculatrice (application BOGU ring 3) s'ouvre ;
  3. clics sur 7 + 5 = -> l'APPLICATION calcule 12 ;
  4. clavier (AZERTY) 9 * 3 Entree -> 27 ;
  5. la fenetre se deplace par sa barre ;
  6. la poignee la redimensionne -> l'application recoit sa nouvelle taille,
     et ses touches (nouvelle geometrie) repondent : 1 + 1 = -> 2 ;
  7. une seconde application, lancee du dock, plante (faute de page) : le
     bureau et la calculatrice continuent (frappe recue apres le plantage) ;
  8. la case de fermeture ferme la calculatrice ; le dock la rouvre (nouveau
     processus) et elle calcule encore (4 * 2 = 8) ;
  9. le dock minimise puis restaure la fenetre active ;
 10. le compositeur lui-meme plante (faute volontaire sur F12) : le noyau le
     RELANCE, le bureau revient, la calculatrice se rouvre et calcule ;
 11. aucune panique.

Captures d'ecran (PPM + PNG) a chaque etape cle, dans --sortie.
Sortie : BANC_BUREAU_UTILISABLE_OK, ou BANC_BUREAU_UTILISABLE_ECHEC (code 1).
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from banc_bureau_ring3_entrees import FATAL, Moniteur  # noqa: E402

RACINE = Path(__file__).resolve().parents[2]
DOCK_H, DOCK_BTN_L = 40, 170


def png_depuis_ppm(ppm: Path, png: Path) -> None:
    """PPM binaire (P6) -> PNG, sans dependance."""
    data = ppm.read_bytes()
    entete = re.match(rb"P6\s+(\d+)\s+(\d+)\s+(\d+)\s", data)
    if not entete:
        return
    l, h = int(entete.group(1)), int(entete.group(2))
    pix = data[entete.end():]
    brut = b"".join(b"\x00" + pix[y * l * 3:(y + 1) * l * 3] for y in range(h))

    def morceau(t: bytes, d: bytes) -> bytes:
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)

    png.write_bytes(b"\x89PNG\r\n\x1a\n" + morceau(b"IHDR", struct.pack(">IIBBBBB", l, h, 8, 2, 0, 0, 0))
                    + morceau(b"IDAT", zlib.compress(brut, 6)) + morceau(b"IEND", b""))


def eprouve_message_coupe(binaire: Path) -> bool:
    """Revue #336 : un message GUI coupe entre deux lectures du canal (flux
    d'octets) ne doit pas etre perdu. La calculatrice tourne ICI, sur l'hote,
    branchee a une socketpair ; une touche « 7 » lui arrive en deux ecritures."""
    import socket
    a, b = socket.socketpair()
    fd = os.memfd_create("surface")
    os.ftruncate(fd, 640 * 480 * 4)
    env = dict(os.environ, BO_GUI_FD=str(b.fileno()), BO_SURFACE_FD=str(fd), BO_SURFACE_WIDTH="640",
               BO_SURFACE_HEIGHT="480", BO_SURFACE_STRIDE=str(640 * 4))
    p = subprocess.Popen([str(binaire)], env=env, pass_fds=(b.fileno(), fd), stderr=subprocess.PIPE, text=True)

    def msg(genre: int, charge: bytes) -> bytes:
        return struct.pack("<IHHII", 0x55474F42, 1, genre, len(charge), 1) + charge
    try:
        a.sendall(msg(0x101, struct.pack("<IIII", 0, 320, 420, 1)))
        time.sleep(0.5)
        touche = msg(0x103, struct.pack("<IIIII", 0, 0, 0, ord("7"), 1))
        a.sendall(touche[:10])
        time.sleep(0.4)
        a.sendall(touche[10:])
        time.sleep(0.6)
        a.sendall(msg(0x106, b""))
        _, err = p.communicate(timeout=5)
    except Exception:  # noqa: BLE001
        p.kill()
        _, err = p.communicate()
    finally:
        a.close(); b.close(); os.close(fd)
    return "CALC_GUI_TOUCHE label=7 source=clavier" in err


class Banc:
    def __init__(self, o):
        self.o = o
        self.sortie = Path(o.sortie).resolve()
        if self.sortie.exists():
            shutil.rmtree(self.sortie)
        (self.sortie / "bin").mkdir(parents=True)
        self.log = self.sortie / "serie.log"
        self.echecs: list[str] = []
        self.oks = 0

    # --- construction --------------------------------------------------------
    def construit(self):
        env = dict(os.environ, OUT=str(self.sortie / "bin"))
        subprocess.run(["bash", "./build.sh"], cwd=RACINE / "userland/services/composited", env=env,
                       check=True, stdout=subprocess.DEVNULL)
        b = self.sortie / "bin"
        cc = ["musl-gcc", "-O2", "-static-pie", "-fPIE"]
        subprocess.run(cc + ["-I", str(RACINE / "userland/libs/police"), "-o", str(b / "calculatrice-gui"),
                             str(RACINE / "userland/apps/calculatrice/calculatrice-gui.c")], check=True)
        for nom in ("mire-gpu", "attends"):
            subprocess.run(cc + ["-o", str(b / nom), str(RACINE / "tools/userland" / f"{nom}.c")], check=True)
        self.message_coupe = eprouve_message_coupe(b / "calculatrice-gui")
        scen = self.sortie / "scen"
        (scen / "bin").mkdir(parents=True)
        shutil.copy(b / "composited-slice", scen / "bin/composited")
        for nom in ("calculatrice-gui", "attends"):
            shutil.copy(b / nom, scen / "bin" / nom)
        shutil.copy(b / "mire-gpu", scen / "bin/mire-gpu")
        for f in (scen / "bin").iterdir():
            f.chmod(0o755)
        # Rien n'est lance au demarrage : tout s'ouvre depuis le dock.
        (scen / "bureau-ring3").write_text(
            "@dock\n"
            "@faute-compositeur\n"
            "/bin/calculatrice-gui 320x420 differe\n"
            f"/bin/mire-gpu 480x300 differe BO_MIRE_FAUTE_APRES={self.o.faute_apres}\n")
        (scen / "garde-vm-vivante").write_text("")
        (scen / "autorun").write_text("echo BUREAU_UTILISABLE_AUTORUN_BEGIN\ndesktop\n/bin/attends 900\n")
        self.disque = self.sortie / "disque.img"
        subprocess.run(["./mkdisk.sh", str(scen)], cwd=RACINE / "tools/userland",
                       env=dict(os.environ, IMAGE=str(self.disque)), check=True, stdout=subprocess.DEVNULL)

    # --- QEMU ----------------------------------------------------------------
    def demarre(self):
        # Un chemin court : AF_UNIX borne le chemin d'une socket a ~108 octets.
        self.socket = tempfile.mktemp(prefix="bo-util-", suffix=".sock")
        accel = ["-enable-kvm", "-cpu", "host"] if self.o.accel == "kvm" else ["-cpu", "max"]
        self.qemu = subprocess.Popen(
            ["qemu-system-x86_64", *accel, "-drive", f"format=raw,file={self.o.boot}",
             "-drive", f"format=raw,file={self.disque}", "-m", "2048", "-smp", str(self.o.smp),
             "-display", "none", "-no-reboot", "-monitor", f"unix:{self.socket},server,nowait",
             "-serial", f"file:{self.log}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.mon = Moniteur(self.socket)

    def arrete(self):
        if getattr(self, "qemu", None) and self.qemu.poll() is None:
            self.qemu.terminate()
            try:
                self.qemu.wait(10)
            except subprocess.TimeoutExpired:
                self.qemu.kill()

    def texte(self) -> str:
        try:
            brut = self.log.read_bytes().decode(errors="replace")
        except FileNotFoundError:
            return ""
        brut = re.sub(r"\x1b\[[0-9;]*m", "", brut)
        return re.sub(r"(?m)^\[\d\d:\d\d:\d\d\]\[[^\]]*\]\[FPS:[^\]]*\] ", "", brut)

    def taille(self) -> int:
        return len(self.texte())

    def attend(self, motif: str, delai: float, depuis: int = 0):
        rx, fin = re.compile(motif), time.time() + delai
        while time.time() < fin:
            m = rx.search(self.texte(), depuis)
            if m:
                return m
            time.sleep(0.2)
        return None

    def exige(self, ok: bool, nom: str):
        if ok:
            self.oks += 1
            print(f"  ok      {nom}")
        else:
            self.echecs.append(nom)
            print(f"  ECHEC   {nom}")

    def capture(self, nom: str):
        ppm = self.sortie / f"{nom}.ppm"
        self.mon.cmd(f"screendump {ppm}")
        time.sleep(0.8)
        if ppm.exists():
            png_depuis_ppm(ppm, self.sortie / f"{nom}.png")
            print(f"  capture {nom}.png")

    # --- geometrie -----------------------------------------------------------
    def fenetre(self, indice: int) -> dict | None:
        """Derniere geometrie publiee de la fenetre `indice`."""
        res, courant, vals = None, None, {}
        for ligne in self.texte().split("\n"):
            m = re.search(r"COMPOSITED_BUREAU_(?:FENETRE|RESTAURE) (?:indice|fenetre)=(\d+)", ligne)
            if m:
                courant, vals = int(m.group(1)), {}
                continue
            m = re.search(r"\[COMPOSITED\] (pid|x|y|l|h|rang)=(-?\d+)", ligne)
            if courant is not None and m:
                vals[m.group(1)] = int(m.group(2))
                if len(vals) == 6:
                    if courant == indice:
                        res = dict(vals)
                    courant = None
        return res

    def touches(self, depuis: int) -> dict[str, tuple[int, int, int, int]]:
        """Geometrie des touches publiee par la calculatrice (la plus recente)."""
        res = {}
        for m in re.finditer(r"CALC_GUI_TOUCHE_RECT label=(\S+) x=(\d+) y=(\d+) l=(\d+) h=(\d+)",
                             self.texte()[depuis:]):
            res[m.group(1)] = tuple(int(v) for v in m.groups()[1:])
        return res

    def clique(self, x: int, y: int):
        self.mon.va(x, y)
        time.sleep(0.3)
        self.mon.bouton(True)
        time.sleep(0.15)
        self.mon.bouton(False)
        time.sleep(0.35)

    def clique_dock(self, k: int):
        self.clique(8 + k * (DOCK_BTN_L + 8) + DOCK_BTN_L // 2, self.ecran_h - DOCK_H // 2)

    def tape_calcul(self, labels: list[str], f: dict, geo: dict) -> None:
        for lab in labels:
            x, y, l, h = geo[lab]
            self.clique(f["x"] + x + l // 2, f["y"] + y + h // 2)

    # --- scenario ------------------------------------------------------------
    def joue(self):
        o = self.o
        self.exige(getattr(self, "message_coupe", False),
                   "hote : un message GUI coupe en deux lectures n'est pas perdu (touche recue)")
        pret = self.attend(r"COMPOSITED_DOCK_PRET applications=(\d+)", o.attente_boot)
        self.exige(pret is not None and pret.group(1) == "2", "le bureau ring 3 s'affiche avec son dock (2 applications)")
        if not pret:
            return
        m = self.attend(r"\[COMPOSITED\] ecran_h=(\d+)", 5)
        self.ecran_h = int(m.group(1)) if m else 720
        t = self.texte()
        self.exige("BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur" in t,
                   "le bureau noyau ne demarre pas (ecran au compositeur)")
        self.exige("CALC_GUI_PRETE" not in t, "aucune application lancee au demarrage")
        time.sleep(1)
        self.capture("01-bureau-dock")

        # 2. Ouvrir la calculatrice depuis le dock.
        avant = self.taille()
        self.clique_dock(0)
        lance = self.attend(r"COMPOSITED_DOCK_LANCE app=calculatrice\b", 15, avant)
        prete = self.attend(r"CALC_GUI_PRETE pid=(\d+)", 30, avant)
        self.exige(lance is not None and prete is not None, "clic sur le dock : la calculatrice s'ouvre (processus ring 3)")
        if not prete:
            return
        pid1 = int(prete.group(1))
        self.attend(r"COMPOSITED_BUREAU_FENETRE indice=0", 15, avant)
        time.sleep(0.5)
        f = self.fenetre(0)
        geo = self.touches(avant)
        self.exige(f is not None and len(geo) == 20, f"fenetre et touches publiees ({f}, {len(geo)} touches)")
        if not f or len(geo) != 20:
            return

        # 3. A la souris.
        avant = self.taille()
        self.tape_calcul(["7", "+", "5", "="], f, geo)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 15, avant)
        self.exige(r is not None and r.group(1) == "7+5" and r.group(2) == "12",
                   f"souris : 7 + 5 = -> {r.group(2) if r else '?'} (12 attendu), calcule PAR l'application")
        self.capture("02-calcul-souris")

        # 4. Au clavier (AZERTY : chiffres avec Maj, '*' sur la touche a droite de 'u-grave').
        avant = self.taille()
        for k in ("c", "shift-9", "backslash", "shift-3", "ret"):
            self.mon.cmd(f"sendkey {k}")
            time.sleep(0.35)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 15, avant)
        self.exige(r is not None and r.group(1) == "9*3" and r.group(2) == "27",
                   f"clavier : 9 * 3 Entree -> {r.group(2) if r else '?'} (27 attendu)")

        # 5. Deplacer par la barre de titre.
        f = self.fenetre(0)
        avant = self.taille()
        self.mon.va(f["x"] + 60, f["y"] - 12)
        time.sleep(0.3)
        self.mon.bouton(True)
        time.sleep(0.2)
        for _ in range(4):
            self.mon.bouge(25, 15)
            time.sleep(0.15)
        self.mon.bouton(False)
        dep = self.attend(r"COMPOSITED_ENTREE_DEPOSE fenetre=0 x=(\d+) y=(\d+)", 15, avant)
        self.exige(dep is not None and abs(int(dep.group(1)) - f["x"] - 100) <= 3
                   and abs(int(dep.group(2)) - f["y"] - 60) <= 3, "la fenetre se deplace par sa barre (100,60)")
        time.sleep(0.5)

        # 6. Redimensionner par la poignee.
        f = self.fenetre(0)
        avant = self.taille()
        gx, gy = f["x"] + f["l"] - 5, f["y"] + f["h"] - 5
        self.mon.va(gx, gy)
        time.sleep(0.3)
        self.mon.bouton(True)
        time.sleep(0.2)
        self.mon.bouge(80, 60)
        time.sleep(0.3)
        self.mon.bouton(False)
        z = self.attend(r"COMPOSITED_ENTREE_REDIM fenetre=0 x=(\d+) y=(\d+)", 15, avant)
        tl = self.attend(r"CALC_GUI_TAILLE l=(\d+) h=(\d+)", 15, avant)
        self.exige(z is not None and tl is not None and (int(tl.group(1)), int(tl.group(2))) == (400, 480),
                   f"poignee : l'application recoit sa nouvelle taille ({tl.group(1) + 'x' + tl.group(2) if tl else '?'}, 400x480 attendu)")
        nf = self.attend(r"COMPOSITED_BUREAU_FENETRE indice=0", 15, avant)
        time.sleep(1.0)
        f = self.fenetre(0)
        self.exige(nf is not None and f is not None and (f["l"], f["h"]) == (400, 480),
                   f"le compositeur recompose la fenetre a sa nouvelle taille ({f})")
        geo = self.touches(avant)
        avant = self.taille()
        if f and len(geo) == 20:
            self.tape_calcul(["C", "1", "+", "1", "="], f, geo)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 15, avant)
        self.exige(r is not None and r.group(2) == "2", "apres redimensionnement, les touches repondent : 1 + 1 = 2")
        self.capture("03-redimensionnee")

        # 7. Une autre application plante : le bureau continue.
        avant = self.taille()
        self.clique_dock(1)
        mire = self.attend(r"MIRE_GPU_PRETE", 30, avant)
        self.exige(mire is not None, "le dock ouvre une seconde application (mire-gpu)")
        mort = self.attend(r"COMPOSITED_BUREAU_CLIENT_MORT pid=(\d+)", o.attente_faute, avant)
        self.exige(mort is not None and int(mort.group(1)) != pid1 and "MIRE_GPU_FAUTE_VOLONTAIRE" in self.texte()[avant:],
                   "la seconde application plante (faute de page), le compositeur le voit")
        f = self.fenetre(0)
        avant = self.taille()
        self.clique(f["x"] + f["l"] // 2, f["y"] - 12)   # remonter la calculatrice
        self.mon.cmd("sendkey shift-8")
        r = self.attend(r"CALC_GUI_TOUCHE label=8 source=clavier", 15, avant)
        self.exige(r is not None, "apres le plantage, la calculatrice recoit encore le clavier")
        self.capture("04-apres-plantage")

        # 8. Fermer, puis rouvrir depuis le dock.
        f = self.fenetre(0)
        avant = self.taille()
        self.clique(f["x"] + f["l"] - 11, f["y"] - 12)
        fermee = self.attend(r"CALC_GUI_FERMEE", 15, avant)
        partie = self.attend(rf"COMPOSITED_BUREAU_CLIENT_MORT pid={pid1}\b", 15, avant)
        self.exige(fermee is not None and partie is not None, "case de fermeture : la calculatrice sort d'elle-meme")
        avant = self.taille()
        self.clique_dock(0)
        prete = self.attend(r"CALC_GUI_PRETE pid=(\d+)", 30, avant)
        self.exige(prete is not None and int(prete.group(1)) != pid1, "le dock la rouvre : un NOUVEAU processus")
        self.attend(r"COMPOSITED_BUREAU_FENETRE indice=0", 15, avant)
        time.sleep(0.5)
        f, geo = self.fenetre(0), self.touches(avant)
        avant = self.taille()
        if f and len(geo) == 20:
            self.tape_calcul(["4", "*", "2", "="], f, geo)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 15, avant)
        self.exige(r is not None and r.group(2) == "8", "rouverte, elle calcule : 4 * 2 = 8")

        # 9. Minimiser puis restaurer depuis le dock.
        avant = self.taille()
        self.clique_dock(0)
        mini = self.attend(r"COMPOSITED_DOCK_MINIMISE app=calculatrice\b", 10, avant)
        self.clique_dock(0)
        rest = self.attend(r"COMPOSITED_DOCK_RESTAURE app=calculatrice\b", 10, avant)
        self.exige(mini is not None and rest is not None, "le dock minimise puis restaure la fenetre active")
        time.sleep(1)
        self.capture("05-final")

        # 10. Le compositeur plante : le noyau le relance.
        avant = self.taille()
        self.mon.cmd("sendkey f12")
        faute = self.attend(r"COMPOSITED_FAUTE_VOLONTAIRE", 10, avant)
        mort = self.attend(r"BOUCHAUD_COMPOSITEUR_MORT pid=(\d+)", 20, avant)
        rep = self.attend(r"BOUCHAUD_COMPOSITEUR_REPRISE n=1 ancien_pid=(\d+) nouveau_pid=(\d+)", 30, avant)
        self.exige(faute is not None and mort is not None and rep is not None and rep.group(1) != rep.group(2),
                   "le compositeur plante (faute de page) : le noyau le relance (nouveau processus)")
        pret = self.attend(r"COMPOSITED_DOCK_PRET applications=2", 60, avant)
        self.exige(pret is not None, "apres la reprise, le bureau et son dock reviennent")
        time.sleep(1)
        avant = self.taille()
        self.clique_dock(0)
        prete = self.attend(r"CALC_GUI_PRETE pid=(\d+)", 30, avant)
        self.attend(r"COMPOSITED_BUREAU_FENETRE indice=0", 15, avant)
        time.sleep(0.5)
        f, geo = self.fenetre(0), self.touches(avant)
        avant2 = self.taille()
        if prete and f and len(geo) == 20:
            self.tape_calcul(["C", "6", "/", "3", "="], f, geo)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 15, avant2)
        self.exige(r is not None and r.group(2) == "2", "apres la reprise, la calculatrice se rouvre et calcule : 6 / 3 = 2")
        self.capture("06-apres-reprise")

        t = self.texte()
        self.exige(not FATAL.search(t), "aucune panique noyau")
        self.exige("COMPOSITED_BUREAU_FAIL" not in t and "COMPOSITED_BUREAU_SCENARIO_ECHEC" not in t,
                   "le compositeur n'a signale aucun echec")

    def bilan(self) -> int:
        print(f"BANC_BUREAU_UTILISABLE controles_ok={self.oks} echecs={len(self.echecs)}")
        if self.echecs:
            print("BANC_BUREAU_UTILISABLE_ECHEC " + " | ".join(self.echecs))
            return 1
        print("BANC_BUREAU_UTILISABLE_OK")
        return 0


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", required=True)
    ap.add_argument("--sortie", default="target/bureau-utilisable")
    ap.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    ap.add_argument("--smp", type=int, default=2)
    ap.add_argument("--faute-apres", type=int, default=240)
    ap.add_argument("--attente-boot", type=float, default=240)
    ap.add_argument("--attente-faute", type=float, default=120)
    o = ap.parse_args(argv)
    banc = Banc(o)
    try:
        banc.construit()
        banc.demarre()
        banc.joue()
    except Exception as exc:  # noqa: BLE001
        banc.echecs.append(f"exception {exc!r}")
        print(f"BANC_BUREAU_UTILISABLE_ECHEC raison={exc!r}")
    finally:
        banc.arrete()
    return banc.bilan()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
