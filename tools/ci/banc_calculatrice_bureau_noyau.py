#!/usr/bin/env python3
"""La Calculatrice du BUREAU NOYAU est l'application ring 3 (BOUCHAUD_CALCULATRICE_RING3_V1).

`src/gui/apps/calculator.rs` (dessin et evaluation dans le noyau) est
supprime. Ce banc prouve que le bureau noyau -- le chemin par defaut des
images livrees -- ouvre toujours une Calculatrice qui fonctionne, et que
c'est bien le processus `/bin/calculatrice-gui` :

  1. `BO_ATELIER_OUVRE=4` ouvre la Calculatrice par `make_app`'s appelant,
     le meme chemin que l'icone et le menu Demarrer ;
  2. le noyau lance `/bin/calculatrice-gui` (BOUCHAUD_APP_LAUNCH), qui peint ;
  3. au clavier (AZERTY) 7 + 2 Entree -> l'APPLICATION calcule 9 ;
  4. aucune panique.

Sortie : BANC_CALCULATRICE_NOYAU_OK, ou BANC_CALCULATRICE_NOYAU_ECHEC (code 1).
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
        subprocess.run(cc + ["-I", str(RACINE / "userland/libs/police"), "-o", str(b / "calculatrice-gui"),
                             str(RACINE / "userland/apps/calculatrice/calculatrice-gui.c")], check=True)
        subprocess.run(cc + ["-o", str(b / "attends"), str(RACINE / "tools/userland/attends.c")], check=True)
        scen = self.sortie / "scen"
        (scen / "bin").mkdir(parents=True)
        for nom in ("calculatrice-gui", "attends"):
            shutil.copy(b / nom, scen / "bin" / nom)
            (scen / "bin" / nom).chmod(0o755)
        (scen / "garde-vm-vivante").write_text("")
        # Pas de /bureau-ring3 ni de /bin/composited : le bureau NOYAU.
        (scen / "autorun").write_text("export BO_ATELIER_OUVRE=4\ndesktop\n/bin/attends 600\n")
        self.disque = self.sortie / "disque.img"
        subprocess.run(["./mkdisk.sh", str(scen)], cwd=RACINE / "tools/userland",
                       env=dict(os.environ, IMAGE=str(self.disque)), check=True, stdout=subprocess.DEVNULL)

    def joue(self):
        lance = self.attend(r"BOUCHAUD_APP_LAUNCH chemin=/bin/calculatrice-gui pid=(\d+)", self.o.attente_boot)
        prete = self.attend(r"CALC_GUI_PRETE pid=(\d+)", 60)
        self.exige(lance is not None and prete is not None and lance.group(1) == prete.group(1),
                   "le bureau noyau ouvre la Calculatrice : le processus ring 3 /bin/calculatrice-gui")
        if not prete:
            return
        t = self.texte()
        self.exige("BOUCHAUD_BUREAU_NOYAU_REFUSE" not in t, "c'est bien le bureau noyau qui tient l'ecran")
        time.sleep(1.5)
        avant = self.taille()
        for k in ("shift-7", "shift-equal", "shift-2", "ret"):
            self.mon.cmd(f"sendkey {k}")
            time.sleep(0.4)
        r = self.attend(r"CALC_GUI_RESULTAT expr=(\S+) valeur=(\S+)", 20, avant)
        self.exige(r is not None and r.group(1) == "7+2" and r.group(2) == "9",
                   f"clavier sous le bureau noyau : 7 + 2 Entree -> {r.group(2) if r else '?'} (9 attendu)")
        time.sleep(1)
        self.capture("calculatrice-bureau-noyau")
        self.exige(not FATAL.search(self.texte()), "aucune panique noyau")

    def bilan(self) -> int:
        print(f"BANC_CALCULATRICE_NOYAU controles_ok={self.oks} echecs={len(self.echecs)}")
        if self.echecs:
            print("BANC_CALCULATRICE_NOYAU_ECHEC " + " | ".join(self.echecs))
            return 1
        print("BANC_CALCULATRICE_NOYAU_OK")
        return 0


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", required=True)
    ap.add_argument("--sortie", default="target/calculatrice-bureau-noyau")
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
        print(f"BANC_CALCULATRICE_NOYAU_ECHEC raison={exc!r}")
    finally:
        banc.arrete()
    return banc.bilan()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
