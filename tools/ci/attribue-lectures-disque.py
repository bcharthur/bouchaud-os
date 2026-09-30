#!/usr/bin/env python3
"""Qui lit le disque : le dernier releve `BACKING_DISK_ATTRIB`, par service.

    attribue-lectures-disque.py JOURNAL_SERIE

Joint trois familles emises par le noyau, sans rien estimer :

  * `BACKING_DISK_ATTRIB releve=N pid= node= ...` -- dernier releve ENTIER ;
  * `PERF_EXEC_PRET image= pid=` -- l'image de chaque pid ;
  * `BACKING_PROBE path= node=` -- le chemin de chaque noeud connu.

# Deux totaux qui ne se confondent pas

`service` est le temps pendant lequel le controleur ATA -- une ressource
SERIE, protegee par un verrou -- travaillait pour ce couple. Sa somme sur tous
les couples est l'occupation reelle du disque, bornee par l'ecoule.

`attente` est le temps passe a attendre ce verrou pendant qu'il servait un
AUTRE lecteur. Elle allonge la latence vue par le service, mais ce n'est pas
du travail disque : l'additionner au service compterait deux fois la meme
seconde de disque. C'etait le cas de `BACKING_DISK_GLOBAL total_us`.
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
CHAMP = re.compile(r"(\w+)=(\S+)")


def champs(ligne: str) -> dict[str, str]:
    return dict(CHAMP.findall(ligne))


def analyse(texte: str) -> tuple[list[str], int]:
    texte = ANSI.sub("", texte).replace("\r", "")
    images: dict[str, str] = {}
    chemins: dict[str, str] = {}
    attrib: dict[int, list[dict[str, str]]] = defaultdict(list)
    decomp: dict[int, dict[str, str]] = {}
    for ligne in texte.splitlines():
        if "PERF_EXEC_PRET" in ligne:
            c = champs(ligne[ligne.index("PERF_EXEC_PRET"):])
            if "pid" in c and "image" in c:
                images[c["pid"]] = c["image"].rsplit("/", 1)[-1]
        elif "BACKING_PROBE" in ligne:
            c = champs(ligne[ligne.index("BACKING_PROBE"):])
            if c.get("node", "-") != "-":
                chemins[c["node"]] = c["path"].rsplit("/", 1)[-1]
        elif "BACKING_DISK_ATTRIB" in ligne:
            c = champs(ligne[ligne.index("BACKING_DISK_ATTRIB"):])
            attrib[int(c["releve"])].append(c)
        elif "BACKING_DISK_DECOMP" in ligne:
            c = champs(ligne[ligne.index("BACKING_DISK_DECOMP"):])
            decomp[int(c["releve"])] = c

    if not decomp:
        return ["BACKING_DISK_DECOMP absent : ce noyau n'emet pas l'attribution"], 1
    releve = max(decomp)
    g = decomp[releve]
    lignes = [
        f"releve {releve} t={g['t']} ms : {int(g['lectures'])} lectures, "
        f"{int(g['octets']) / 2**20:.1f} Mio",
        f"  occupation du controleur (somme du service) : {int(g['service_us']) / 1e3:.0f} ms",
        f"  attente du verrou (somme, NON additive)     : {int(g['attente_us']) / 1e3:.0f} ms",
        f"  pages neuves {g['pages_neuves']}  relues {g['pages_relues']}  "
        f"hors_bitmap {g['hors_bitmap']}  debordements {g['debordements']}",
        f"  tailles {g['tailles']}",
        "",
        f"  {'pid':>4} {'image':<16} {'fichier':<16} {'Mio':>7} {'lect':>6} "
        f"{'seq%':>5} {'service_ms':>10} {'attente_ms':>10} {'pire_ms':>8} {'relu_Mio':>8}",
    ]
    par_service: dict[str, list[float]] = defaultdict(lambda: [0.0, 0, 0.0, 0.0, 0.0])
    couples = sorted(attrib.get(releve, []), key=lambda c: -int(c["service_us"]))
    for c in couples:
        image = images.get(c["pid"], "noyau" if c["pid"] == "0" else "?")
        fichier = chemins.get(c["node"], f"node{c['node']}")
        lectures = int(c["lectures"])
        seq = 100 * int(c["sequentielles"]) / lectures if lectures else 0
        lignes.append(
            f"  {c['pid']:>4} {image[:16]:<16} {fichier[:16]:<16} "
            f"{int(c['octets']) / 2**20:7.1f} {lectures:6d} {seq:5.0f} "
            f"{int(c['service_us']) / 1e3:10.0f} {int(c['attente_us']) / 1e3:10.0f} "
            f"{int(c['pire_service_us']) / 1e3:8.0f} {int(c['pages_relues']) * 4096 / 2**20:8.1f}"
        )
        s = par_service[f"{image} (pid {c['pid']})"]
        s[0] += int(c["octets"]) / 2**20
        s[1] += lectures
        s[2] += int(c["service_us"]) / 1e3
        s[3] += int(c["attente_us"]) / 1e3
        s[4] += int(c["pages_relues"]) * 4096 / 2**20
    lignes.append("")
    lignes.append("  par service : Mio / lectures / service ms / attente ms / relu Mio")
    for nom, (mio, n, service, attente, relu) in sorted(par_service.items(), key=lambda kv: -kv[1][2]):
        lignes.append(f"    {nom:<28} {mio:7.1f} {int(n):6d} {service:9.0f} {attente:9.0f} {relu:7.1f}")
    somme = sum(v[2] for v in par_service.values())
    lignes.append(
        f"  controle : somme des couples {somme:.0f} ms / global {int(g['service_us']) / 1e3:.0f} ms"
    )
    return lignes, 0


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__.splitlines()[2], file=sys.stderr)
        return 2
    lignes, rc = analyse(Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"))
    print("\n".join(lignes))
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
