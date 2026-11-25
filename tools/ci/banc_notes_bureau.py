#!/usr/bin/env python3
"""Bouchaud Notes du BUREAU est l'application ring 3 (BOUCHAUD_NOTES_RING3_V1).

  1. `BO_ATELIER_OUVRE=9` ouvre Notes par le meme chemin que l'icone et le
     menu Demarrer ;
  2. le noyau publie l'atlas de police d'Atelier et lance `/bin/notes-gui`
     (BOUCHAUD_APP_LAUNCH), qui charge la police (police=1) et peint ;
  3. au clavier : `ert`, Entree, `yu` puis F2 -> l'APPLICATION ecrit 6 octets ;
  4. F3 recharge le fichier -> 6 octets relus ;
  5. capture d'ecran ; aucune panique.

Sortie : BANC_NOTES_OK, ou BANC_NOTES_ECHEC (code 1).
"""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from banc_bureau_ring3_entrees import FATAL, Moniteur  # noqa: E402
from banc_bureau_utilisable import Banc as BancUtilisable, png_depuis_ppm  # noqa: E402

RACINE = Path(__file__).resolve().parents[2]


class Banc(BancUtilisable):
    def construit(self):
        b = self.sortie / "bin"
        cc = ["musl-gcc", "-O2", "-static-pie", "-fPIE"]
        subprocess.run(cc + ["-I", str(RACINE / "userland/libs/police"), "-o", str(b / "notes-gui"),
                             str(RACINE / "userland/apps/notes/notes-gui.c")], check=True)
        subprocess.run(cc + ["-o", str(b / "attends"), str(RACINE / "tools/userland/attends.c")], check=True)
        scen = self.sortie / "scen"
        (scen / "bin").mkdir(parents=True)
        for nom in ("notes-gui", "attends"):
            shutil.copy(b / nom, scen / "bin" / nom)
            (scen / "bin" / nom).chmod(0o755)
        (scen / "garde-vm-vivante").write_text("")
        # Pas de /bureau-ring3 ni de /bin/composited : le bureau NOYAU.
        (scen / "autorun").write_text("export BO_ATELIER_OUVRE=9\ndesktop\n/bin/attends 600\n")
        self.disque = self.sortie / "disque.img"
        subprocess.run(["./mkdisk.sh", str(scen)], cwd=RACINE / "tools/userland",
                       env=dict(os.environ, IMAGE=str(self.disque)), check=True, stdout=subprocess.DEVNULL)

    def joue(self):
        atlas = self.attend(r"BOUCHAUD_ATLAS_POLICE tailles=(\d+)", self.o.attente_boot)
        lance = self.attend(r"BOUCHAUD_APP_LAUNCH chemin=/bin/notes-gui pid=(\d+)", 60)
        prete = self.attend(r"NOTES_GUI_PRETE pid=(\d+) fichier=(\S+) durable=(\d) octets=(\d+) police=(\d)", 60)
        self.exige(atlas is not None and int(atlas.group(1)) == 4, "le noyau publie l'atlas de police d'Atelier (4 tailles)")
        self.exige(lance is not None and prete is not None and lance.group(1) == prete.group(1),
                   "le bureau ouvre Notes : le processus ring 3 /bin/notes-gui")
        if not prete:
            return
        self.exige(prete.group(5) == "1", "Notes dessine avec la police d'Atelier (atlas charge)")
        time.sleep(1.5)
        avant = self.taille()
        for k in ("e", "r", "t", "ret", "y", "u", "f2"):
            self.mon.cmd(f"sendkey {k}")
            time.sleep(0.4)
        s = self.attend(r"NOTES_GUI_SAUVE fichier=(\S+) octets=(\d+) durable=(\d) fsync=(-?\d+)", 20, avant)
        self.exige(s is not None and s.group(2) == "6", f"F2 enregistre 'ert\\nyu' : {s.group(2) if s else '?'} octets (6 attendus)")
        time.sleep(1)
        self.capture("notes-bureau")
        avant = self.taille()
        self.mon.cmd("sendkey f3")
        r = self.attend(r"NOTES_GUI_RECHARGE octets=(\d+)", 20, avant)
        self.exige(r is not None and r.group(1) == "6", f"F3 relit le fichier : {r.group(1) if r else '?'} octets (6)")
        self.exige(not FATAL.search(self.texte()), "aucune panique noyau")

    def bilan(self) -> int:
        print(f"BANC_NOTES controles_ok={self.oks} echecs={len(self.echecs)}")
        if self.echecs:
            print("BANC_NOTES_ECHEC " + " | ".join(self.echecs))
            return 1
        print("BANC_NOTES_OK")
        return 0


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", required=True)
    ap.add_argument("--sortie", default="target/notes-bureau")
    ap.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    ap.add_argument("--smp", type=int, default=2)
    ap.add_argument("--attente-boot", type=float, default=240)
    o = ap.parse_args(argv)
    banc = Banc(o)
    try:
        banc.construit()
        banc.demarre()
        banc.joue()
    except Exception as exc:  # noqa: BLE001
        banc.echecs.append(f"exception {exc!r}")
        print(f"BANC_NOTES_ECHEC raison={exc!r}")
    finally:
        banc.arrete()
    return banc.bilan()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
