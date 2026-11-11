#!/usr/bin/env python3
"""BOUCHAUD_FIND_PARCOURS_V1 / BOUCHAUD_RUN_SANS_VERROU_V1 : aucun garde du RAMFS
n'est tenu pendant un appel qui le reprend.

Le verrou du RAMFS (`ramfs::fs()`) refuse la reprise par le meme coeur :
lockdep panique (`LOCKDEP inversion ... vfs(50)`), et sans lockdep le coeur
boucle pour toujours. Deux commandes du shell le faisaient -- `find` (via
`find_rec`) et `run` (via `run_line_journalisee`) --, toutes deux reproduites
sous QEMU (tools/ci/unix_fiabilite.py).

Regle : dans chaque fonction de `src/`, un garde `let X = ramfs::fs();` encore
vivant (meme bloc, pas de `drop(X)` avant) ne doit etre suivi, dans ce bloc,
ni d'une nouvelle prise `ramfs::fs()`, ni d'un appel a une fonction du depot
qui en prend une (premier niveau, par nom).

Limites, assumees : l'analyse est textuelle (pas d'appel indirect par
pointeur ni de prise au deuxieme niveau d'appel) ; elle couvre le RAMFS, pas
les autres verrous. Le banc QEMU reste le juge des chemins reels.

Fail-closed ; chaque regle a son temoin negatif.
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
IGNORES = {"fs", "init", "new", "main"}


CHAINE = re.compile(r'"(?:[^"\\\n]|\\.)*"')


def sans_commentaires(t: str) -> str:
    """Ni commentaires ni contenu des chaines : `"monte (volatil)"` n'est pas un
    appel a `monte`."""
    t = CHAINE.sub('""', t)
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps_fonctions(t: str):
    for m in re.finditer(r"\bfn\s+(\w+)\s*(<[^>]*>)?\s*\(", t):
        j = t.find("{", m.end())
        if j < 0:
            continue
        # Une declaration de trait (`;` avant `{`) n'a pas de corps.
        if ";" in t[m.end():j]:
            continue
        d, k = 0, j
        while k < len(t):
            d += {"{": 1, "}": -1}.get(t[k], 0)
            k += 1
            if d == 0:
                break
        yield m.group(1), t[j:k]


def analyse(textes: dict[str, str]) -> list[str]:
    propres = {f: sans_commentaires(t) for f, t in textes.items()}
    prenantes = set()
    for t in propres.values():
        for nom, corps in corps_fonctions(t):
            if "ramfs::fs()" in corps or re.search(r"(?<![\w:.])fs\(\)", corps):
                prenantes.add(nom)
    prenantes -= IGNORES
    fautes = []
    for f, s in propres.items():
        for m in re.finditer(r"let\s+(mut\s+)?(\w+)\s*=\s*(crate::fs::)?ramfs::fs\(\);", s):
            i, d, j = m.end(), 0, m.end()
            while j < len(s):
                c = s[j]
                if c == "{":
                    d += 1
                elif c == "}":
                    if d == 0:
                        break
                    d -= 1
                j += 1
            reste = s[i:j]
            k = reste.find(f"drop({m.group(2)})")
            if k >= 0:
                reste = reste[:k]
            touches = []
            if "ramfs::fs()" in reste:
                touches.append("ramfs::fs()")
            for nom in prenantes:
                if re.search(rf"(?<![\w.]){nom}\s*\(", reste):
                    touches.append(nom)
            if touches:
                ligne = s[: m.start()].count("\n") + 1
                fautes.append(f"{f}:{ligne} : garde ramfs::fs() tenu pendant {', '.join(sorted(touches)[:4])}")
    return fautes


def lit_src(racine: Path) -> dict[str, str]:
    return {str(p.relative_to(racine)): p.read_text(encoding="utf-8", errors="replace")
            for p in (racine / "src").rglob("*.rs")}


# Temoins : les deux defauts reproduits, remis tels quels.
NEGATIFS = [
    ("src/shell/commands.rs", '''
pub fn find_temoin(path: &str, cwd: usize) {
    let fs = ramfs::fs();
    let start = fs.resolve_checked(path, cwd).unwrap_or(0);
    find_rec_temoin(start);
}

fn find_rec_temoin(idx: usize) {
    let fs = ramfs::fs();
    let _ = fs.nodes[idx].used;
}
'''),
    ("src/shell/mod.rs", '''
fn run_script_temoin(cwd: &mut usize) -> i32 {
    let fs = ramfs::fs();
    let content = fs.nodes[0].content_str();
    for l in content.lines() {
        run_line_journalisee("script", l, cwd);
    }
    0
}
'''),
    ("src/shell/commands.rs", '''
fn double_temoin() {
    let fs = ramfs::fs();
    let autre = ramfs::fs();
    let _ = (fs.nodes[0].used, autre.nodes[0].used);
}
'''),
]


def main() -> int:
    textes = lit_src(RACINE)
    fautes = analyse(textes)
    for f in fautes:
        print("verrou vfs :", f)
    if fautes:
        return 1
    for n, (fichier, ajout) in enumerate(NEGATIFS, 1):
        mutes = dict(textes)
        mutes[fichier] = mutes[fichier] + ajout
        if not analyse(mutes):
            print(f"verrou vfs : temoin negatif {n} ne rougit pas -- garde inoperant")
            return 1
    # Le temoin d'un `drop` explicite : relacher avant de rappeler est permis.
    mutes = dict(textes)
    mutes["src/shell/commands.rs"] += '''
fn relache_temoin() {
    let fs = ramfs::fs();
    let _ = fs.nodes[0].used;
    drop(fs);
    let _ = ramfs::fs().nodes[0].used;
}
'''
    if analyse(mutes):
        print("verrou vfs : temoin positif (drop explicite) rejete a tort")
        return 1
    print(f"VERROU_VFS_OK negatifs={len(NEGATIFS)} positifs=1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
