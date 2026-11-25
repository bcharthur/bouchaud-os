#!/usr/bin/env python3
"""Le Journal du BUREAU est l'application ring 3 (BOUCHAUD_JOURNAL_RING3_V1).

  1. `BO_ATELIER_OUVRE=7` ouvre le Journal par le meme chemin que le menu ;
  2. le noyau lance `/bin/journal-gui` (BOUCHAUD_APP_LAUNCH), qui lit la trace
     dans /proc/bouchaud/journal (acces=1 : l'application du bureau porte la
     capacite DEBUG) et peint avec la police d'Atelier (police=1) ;
  3. au clavier : F -> filtre AMORCAGE (des lignes retenues), R -> recapture
     (au moins autant de lignes), Fin / Page haut defilent ;
  4. capture d'ecran ; aucune panique.

`--temoin` : la meme sequence SANS /bin/journal-gui dans l'image -- le bureau
retombe sur l'ancien Journal dessine par le noyau (`gui/apps/journal.rs`). La
capture du temoin sert de reference visuelle : la fenetre doit etre la meme.

Sortie : BANC_JOURNAL_OK, ou BANC_JOURNAL_ECHEC (code 1).
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
from banc_bureau_ring3_entrees import FATAL, Moniteur  # noqa: E402,F401
from banc_bureau_utilisable import Banc as BancUtilisable  # noqa: E402

RACINE = Path(__file__).resolve().parents[2]


class Banc(BancUtilisable):
    def construit(self):
        b = self.sortie / "bin"
        b.mkdir(parents=True, exist_ok=True)
        cc = ["musl-gcc", "-O2", "-static-pie", "-fPIE"]
        programmes = ["attends"]
        subprocess.run(cc + ["-o", str(b / "attends"), str(RACINE / "tools/userland/attends.c")], check=True)
        if not self.o.temoin:
            subprocess.run(cc + ["-I", str(RACINE / "userland/libs/police"), "-o", str(b / "journal-gui"),
                                 str(RACINE / "userland/apps/journal/journal-gui.c")], check=True)
            programmes.append("journal-gui")
        scen = self.sortie / "scen"
        (scen / "bin").mkdir(parents=True)
        for nom in programmes:
            shutil.copy(b / nom, scen / "bin" / nom)
            (scen / "bin" / nom).chmod(0o755)
        (scen / "garde-vm-vivante").write_text("")
        (scen / "autorun").write_text("export BO_ATELIER_OUVRE=7\ndesktop\n/bin/attends 600\n")
        self.disque = self.sortie / "disque.img"
        subprocess.run(["./mkdisk.sh", str(scen)], cwd=RACINE / "tools/userland",
                       env=dict(os.environ, IMAGE=str(self.disque)), check=True, stdout=subprocess.DEVNULL)

    def joue(self):
        if self.o.temoin:
            echec = self.attend(r"BOUCHAUD_APP_LAUNCH_ERROR chemin=/bin/journal-gui", self.o.attente_boot)
            self.exige(echec is not None, "temoin : journal-gui absent, le bureau retombe sur le Journal noyau")
            time.sleep(4)
            self.capture("journal-temoin")
            self.exige(not FATAL.search(self.texte()), "aucune panique noyau")
            return
        lance = self.attend(r"BOUCHAUD_APP_LAUNCH chemin=/bin/journal-gui pid=(\d+)", self.o.attente_boot)
        prete = self.attend(r"JOURNAL_GUI_PRETE pid=(\d+) acces=(\d) lignes=(\d+) octets=(\d+) police=(\d)", 60)
        self.exige(lance is not None and prete is not None and lance.group(1) == prete.group(1),
                   "le bureau ouvre le Journal : le processus ring 3 /bin/journal-gui")
        if not prete:
            return
        self.exige(prete.group(2) == "1", "l'application du bureau lit /proc/bouchaud/journal (acces=1)")
        self.exige(int(prete.group(3)) > 20, f"la trace est decoupee en lignes ({prete.group(3)})")
        self.exige(prete.group(5) == "1", "le Journal dessine avec la police d'Atelier (atlas charge)")
        time.sleep(3)
        self.capture("journal-ring3")
        avant = self.taille()
        self.mon.cmd("sendkey f")
        f = self.attend(r"JOURNAL_GUI_FILTRE nom=(\S+) retenues=(\d+)", 20, avant)
        self.exige(f is not None and f.group(1) == "AMORCAGE" and int(f.group(2)) > 0,
                   f"F : filtre AMORCAGE, {f.group(2) if f else '?'} lignes retenues")
        avant = self.taille()
        self.mon.cmd("sendkey r")
        r = self.attend(r"JOURNAL_GUI_RECAPTURE lignes=(\d+) octets=(\d+)", 20, avant)
        self.exige(r is not None and int(r.group(1)) >= int(prete.group(3)),
                   f"R : recapture ({r.group(1) if r else '?'} lignes, >= {prete.group(3)})")
        for k in ("pgup", "end"):
            self.mon.cmd(f"sendkey {k}")
            time.sleep(0.5)
        time.sleep(1)
        self.capture("journal-ring3-filtre")
        self.exige(not FATAL.search(self.texte()), "aucune panique noyau")

    def bilan(self) -> int:
        print(f"BANC_JOURNAL controles_ok={self.oks} echecs={len(self.echecs)}")
        if self.echecs:
            print("BANC_JOURNAL_ECHEC " + " | ".join(self.echecs))
            return 1
        print("BANC_JOURNAL_OK")
        return 0


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", required=True)
    ap.add_argument("--sortie", default="target/journal-bureau")
    ap.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    ap.add_argument("--smp", type=int, default=2)
    ap.add_argument("--attente-boot", type=float, default=240)
    ap.add_argument("--temoin", action="store_true")
    o = ap.parse_args(argv)
    banc = Banc(o)
    try:
        banc.construit()
        banc.demarre()
        banc.joue()
    except Exception as exc:  # noqa: BLE001
        banc.echecs.append(f"exception {exc!r}")
        print(f"BANC_JOURNAL_ECHEC raison={exc!r}")
    finally:
        banc.arrete()
    return banc.bilan()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
