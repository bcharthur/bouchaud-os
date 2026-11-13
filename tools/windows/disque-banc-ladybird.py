#!/usr/bin/env python3
"""BOUCHAUD_WHPX_CPU_V1 -- disque de BANC Ladybird, copie de ladybird-browser.img.

    disque-banc-ladybird.py SOURCE.img CIBLE.img

`run.ps1` fabrique ladybird-browser.img pour une session NORMALE : son autorun
ouvre le bureau et attend un double-clic sur "Navigateur" (aucune ligne
`BO_AUTOSTART_BROWSER=1`). Sous `-display none`, personne ne clique : un banc
qui attend la premiere trame de Ladybird sur ce disque attend une application
jamais lancee.

Ce script ecrit une COPIE dont l'autorun exporte `BO_AUTOSTART_BROWSER=1`
juste avant `desktop` -- le mecanisme des regressions de CI
(src/gui/window.rs). Rien d'autre ne change : memes fichiers, memes droits,
meme URL (`BOUCHAUD_M9_URL`), meme alignement sur 512 octets. La SOURCE n'est
jamais ouverte en ecriture, et le dossier scenario-m9 n'est pas touche.

Sortie : `DISQUE_BANC_OK url=... membres=N`, code 0 ; sinon
`DISQUE_BANC_IMPOSSIBLE raison=...`, code 2 (le banc rend alors Ladybird
NON_MESURABLE, avec cette raison).
"""
import io
import re
import sys
import tarfile
from pathlib import Path

LIGNE = "export BO_AUTOSTART_BROWSER=1"


def impossible(raison):
    print(f"DISQUE_BANC_IMPOSSIBLE raison={raison}")
    return 2


def autorun_banc(texte):
    """L'autorun avec le lancement automatique, ou None si sa forme est inconnue."""
    lignes = texte.split("\n")
    if LIGNE in lignes:
        return texte
    if "desktop" not in lignes:
        return None
    i = len(lignes) - 1 - lignes[::-1].index("desktop")
    return "\n".join(lignes[:i] + [LIGNE] + lignes[i:])


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 64
    source, cible = Path(argv[1]), Path(argv[2])
    if not source.is_file():
        return impossible(f"disque-absent:{source}")
    if source.resolve() == cible.resolve():
        return impossible("cible-egale-a-la-source")
    try:
        entree = tarfile.open(source, mode="r:")
    except tarfile.TarError as e:
        return impossible(f"pas-une-archive-ustar:{e}")
    with entree:
        membres = entree.getmembers()
        autoruns = [m for m in membres if m.name in ("./autorun", "autorun") and m.isfile()]
        if len(autoruns) != 1:
            return impossible("autorun-absent")
        texte = entree.extractfile(autoruns[0]).read().decode("utf-8")
        nouveau = autorun_banc(texte)
        if nouveau is None:
            return impossible("autorun-sans-ligne-desktop")
        url = re.search(r"^export BOUCHAUD_M9_URL=(.*)$", texte, re.M)
        tmp = cible.with_name(cible.name + ".tmp")
        with tarfile.open(tmp, mode="w", format=tarfile.USTAR_FORMAT) as sortie:
            for m in membres:
                if m is autoruns[0]:
                    donnees = nouveau.encode("utf-8")
                    info = tarfile.TarInfo(m.name)
                    for champ in ("mode", "uid", "gid", "uname", "gname", "mtime"):
                        setattr(info, champ, getattr(m, champ))
                    info.size = len(donnees)
                    sortie.addfile(info, io.BytesIO(donnees))
                elif m.isfile():
                    sortie.addfile(m, entree.extractfile(m))
                else:
                    sortie.addfile(m)
    taille = tmp.stat().st_size
    with tmp.open("ab") as f:
        f.write(b"\0" * ((-taille) % 512))
    tmp.replace(cible)
    print(f"DISQUE_BANC_OK url={url.group(1) if url else '?'} membres={len(membres)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
