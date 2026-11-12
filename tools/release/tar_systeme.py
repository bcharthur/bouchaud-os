#!/usr/bin/env python3
"""BOUCHAUD_LIVRAISON_V1 -- archive USTAR DETERMINISTE d'un systeme assemble.

    tar_systeme.py SYSTEME SORTIE.tar [SORTIE2.tar ...]

Ordre trie, mtime 0, uid/gid 0, droits conserves : deux constructions du meme
commit donnent le meme ramdisk. Utilise par build_trigkey_ab_image.sh (slots
A/B) et par la cle USB de validation (Prepare-TrigkeyUSB.ps1) : le MEME
systeme dans les deux.
"""
import sys
import tarfile
from pathlib import Path


def archive(racine: Path, sortie: Path) -> None:
    chemins = sorted(racine.rglob("*"))
    with tarfile.open(sortie, "w", format=tarfile.USTAR_FORMAT) as a:
        for p in chemins:
            info = a.gettarinfo(str(p), arcname=str(p.relative_to(racine)))
            info.mtime = 0
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            if p.is_file():
                with p.open("rb") as f:
                    a.addfile(info, f)
            else:
                a.addfile(info)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    for s in sys.argv[2:]:
        archive(Path(sys.argv[1]), Path(s))
