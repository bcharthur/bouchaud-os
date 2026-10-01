#!/usr/bin/env python3
"""Les invariants de concurrence de la pile inet historique.

Ces controles vivaient dans `verifie-verrouillage.py`, qui verifiait surtout la
table `SANS_BKL` des appels systeme sortis du gros verrou. La table et le
verrou ont ete supprimes (BOUCHAUD_BKL_SUPPRIME_V1 ; les audits sont dans
docs/AUDIT_VERROUILLAGE_SYSCALLS.md). Restent les garanties que la pile inet
s'est donnees pour s'en passer :

  * le port ephemere est un compteur ATOMIQUE monotone, partage par TCP et
    UDP (`net::port_ephemere`, BOUCHAUD_PORT_EPHEMERE_MONOTONE_V1) : c'est ce
    qui rend `bind` et `connect` concurrents sans course sur le port ;
  * le chemin de reception (C8/V2) garde ses points surs reguliers ;
  * la section reseau courte (`section_reseau`) a une seule definition, et le
    `read` sur socket n'y ajoute rien.

Fail-closed : un fichier introuvable est une faute.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
NET = RACINE / "src/compat/linux/net.rs"
PILE = RACINE / "src/net/mod.rs"
FILE = RACINE / "src/compat/linux/file.rs"


def main() -> int:
    fautes = []
    try:
        net = NET.read_text(encoding="utf-8")
        pile = PILE.read_text(encoding="utf-8")
        fichier = FILE.read_text(encoding="utf-8")
    except OSError as e:
        print(f"reseau : lecture impossible : {e}")
        return 1
    for marqueur in (
        "BOUCHAUD_C8_RECV_SANS_BKL_V2",
        "crate::net::port_ephemere()",
        "crate::kernel::scheduler::preempt::safe_point()",
    ):
        if marqueur not in net:
            fautes.append(f"net.rs : marqueur absent `{marqueur}`")
    if "static PROCHAIN_PORT_EPHEMERE: AtomicU16" not in pile:
        fautes.append("net/mod.rs : compteur atomique de port ephemere absent")
    if len(re.findall(r"\bfn section_reseau\b", net)) != 1:
        fautes.append("net.rs : la section reseau doit avoir une seule definition")
    if "BOUCHAUD_C8_READ_SOCKET_SANS_BKL_EXTERNE_V2" not in fichier:
        fautes.append("file.rs : marqueur read socket absent")
    if fautes:
        print("reseau : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    print("RESEAU_CONCURRENCE_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
