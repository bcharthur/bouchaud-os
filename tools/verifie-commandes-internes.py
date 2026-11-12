#!/usr/bin/env python3
"""BOUCHAUD_ARBORESCENCE_COHERENTE_V1 : `INTERNES` est exactement l'ensemble des
commandes que `dispatch` traite lui-meme.

`type`/`which` dit qu'un nom est une commande interne s'il est dans `INTERNES`.
Si un motif de `dispatch` y manquait, `type` dirait « introuvable » (ou
designerait un fichier de PATH) pour une commande que le shell execute en
interne -- c'etait le cas des 14 commandes absentes de `COMMANDS` (revue #302).
Si `INTERNES` portait un nom sans motif, `type` mentirait dans l'autre sens.

Les motifs sont ceux du `match argv[0]` de `dispatch`, au niveau 8 espaces
(les `match` imbriques des bras sont plus indentes), jusqu'au repli
`_ => c::execute_programme(...)`.

Fail-closed ; temoins negatifs dans les deux sens.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
MOD = "src/shell/mod.rs"
DEBUT = "fn dispatch(line: &str, cwd: &mut usize) -> i32 {"
REPLI = "        _ => c::execute_programme(line, argc, &argv, *cwd),"


def motifs(s: str) -> set[str] | None:
    i = s.find(DEBUT)
    j = s.find(REPLI, i)
    if i < 0 or j < 0:
        return None
    noms = set()
    for l in s[i:j].splitlines():
        m = re.match(r'^ {8}((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*(if\b[^=]*)?=>', l)
        if m:
            noms |= set(re.findall(r'"([^"]+)"', m.group(1)))
    return noms


def internes(s: str) -> set[str] | None:
    m = re.search(r"pub const INTERNES: &\[&str\] = &\[(.*?)\];", s, re.S)
    return set(re.findall(r'"([^"]+)"', m.group(1))) if m else None


def verifie(s: str) -> list[str]:
    a, b = motifs(s), internes(s)
    if a is None:
        return ["dispatch ou son repli introuvable"]
    if b is None:
        return ["INTERNES introuvable"]
    if len(a) < 100:
        return [f"seulement {len(a)} motifs lus dans dispatch : extraction cassee"]
    f = []
    if a - b:
        f.append("motifs de dispatch absents de INTERNES : " + ", ".join(sorted(a - b)))
    if b - a:
        f.append("INTERNES sans motif dans dispatch : " + ", ".join(sorted(b - a)))
    if "INTERNES.contains(nom)" not in s:
        f.append("type_cmd ne consulte pas INTERNES")
    return f


def main() -> int:
    s = (RACINE / MOD).read_text(encoding="utf-8")
    fautes = verifie(s)
    for x in fautes:
        print("commandes internes :", x)
    if fautes:
        return 1
    temoins = [
        # Un nouveau motif sans entree dans INTERNES.
        s.replace(REPLI, '        "nouvelle-commande" => 0,\n' + REPLI, 1),
        # Une entree de INTERNES sans motif.
        s.replace("pub const INTERNES: &[&str] = &[", 'pub const INTERNES: &[&str] = &[ "fantome",', 1),
        # Un motif a plusieurs noms dont un oublie.
        s.replace(REPLI, '        "alias-a" | "alias-b" => 0,\n' + REPLI, 1)
         .replace("pub const INTERNES: &[&str] = &[", 'pub const INTERNES: &[&str] = &[ "alias-a",', 1),
        # type revenu a la liste de completion.
        s.replace("INTERNES.contains(nom)", "COMMANDS.contains(nom)", 1),
    ]
    for n, t in enumerate(temoins, 1):
        if t == s or not verifie(t):
            print(f"commandes internes : temoin negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"COMMANDES_INTERNES_OK internes={len(motifs(s))} negatifs={len(temoins)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
