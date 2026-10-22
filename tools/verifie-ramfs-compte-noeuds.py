#!/usr/bin/env python3
"""Un nœud du RAMFS ne se libere que par `FileSystem::libere_noeud` (ou
`libere_anonyme`) : c'est la que le miroir `USED_NODES_RELAXED` decremente.

BOUCHAUD_RAMFS_COMPTE_NOEUDS_V1

os-primitives 38442948204 (4401bcec, TCG) : KERNEL PANIC `ramfs.rs:279`
« used-node accounting overflow ». Sept sites (unlink, renommage qui ecrase,
`rm`/`rmdir` du shell, WASI) ecrivaient `nodes[i].used = false` eux-memes :
le miroir, incremente a chaque allocation, ne faisait que monter jusqu'a
l'assertion d'allocation, apres ~4 096 creations cumulees. Reproduit
(noeuds-churn-probe : panique avant, 8 000 cycles apres).

Ce garde verifie :
  1. aucune ecriture `nodes[...].used = false` hors de `libere_noeud` ;
  2. `alloc_node` incremente, `libere_noeud` et `libere_anonyme` decrementent.

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
RAMFS = "src/fs/ramfs.rs"
ECRITURE = re.compile(r"nodes\s*\[[^\]]*\]\s*\.\s*used\s*=\s*false")


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    fautes = []
    for f in sorted((racine / "src").rglob("*.rs")):
        rel = str(f.relative_to(racine))
        t = sans_commentaires(f.read_text(encoding="utf-8", errors="replace"))
        if rel == RAMFS:
            continue
        if ECRITURE.search(t):
            fautes.append(f"{rel} : libere un nœud RAMFS sans `libere_noeud` (le miroir de comptage fuit)")
    try:
        r = sans_commentaires((racine / RAMFS).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]
    hors = ECRITURE.sub("", corps(r, "libere_noeud"), count=1)
    reste = r.replace(corps(r, "libere_noeud"), hors, 1)
    if ECRITURE.search(reste):
        fautes.append(f"{RAMFS} : un nœud est libere hors de `libere_noeud`")
    if "USED_NODES_RELAXED.fetch_add(1" not in corps(r, "alloc_node"):
        fautes.append(f"{RAMFS} : alloc_node ne compte plus le nœud alloue")
    for f in ("libere_noeud", "libere_anonyme"):
        if "USED_NODES_RELAXED.fetch_sub(1" not in corps(r, f):
            fautes.append(f"{RAMFS} : {f} ne decompte plus le nœud rendu")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in sorted((RACINE / "src").rglob("*.rs")):
            rel = str(f.relative_to(RACINE))
            d = Path(tmp) / rel
            d.parent.mkdir(parents=True, exist_ok=True)
            t = f.read_text(encoding="utf-8", errors="replace")
            if rel == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("ramfs : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        ("src/compat/linux/file.rs", "            fs.libere_noeud(node);\n",
         "            fs.nodes[node].used = false;\n"),
        (RAMFS, "        let old = USED_NODES_RELAXED.fetch_sub(1, Ordering::Relaxed);\n        assert!(old != 0, \"ramfs: used-node accounting underflow\");\n    }\n\n    pub fn touch_at",
         "    }\n\n    pub fn touch_at"),
        (RAMFS, "    pub fn touche(&mut self, idx: usize) {\n",
         "    pub fn touche(&mut self, idx: usize) {\n        self.nodes[idx].used = false;\n"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"ramfs : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"RAMFS_COMPTE_NOEUDS_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
