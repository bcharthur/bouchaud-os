#!/usr/bin/env python3
"""Une sonde appelee depuis `exit_current` ne prend pas de verrou.

BOUCHAUD_C70_UNE_SONDE_NE_PREND_PAS_DE_VERROU

`exit_current` tient `process.lifecycle` sur toute la fenetre ou les sondes
de mort de processus sont publiees. Une sonde qui y prend un autre verrou
global -- `CACHE`, une table du noyau -- cree une arete d'ordre de verrous
depuis un chemin de sortie, et l'ordre inverse existe ailleurs.

La regle etait deja ecrite dans `page_cache.rs`, sur `log_ng_stats` :

    Reporting must stay lock-free: browser_report can run while
    compatibility code still owns the BKL. Taking CACHE here would turn
    observability into another lock-order edge.

Je l'ai enfreinte quand meme. `balayage_temoins` prenait `CACHE.lock()` et
`exit_current` l'appelait : l'integration #256 s'est figee juste apres
`SESSION_PERE_SORT fils=4`, machine vivante, scenario bloque, 10 minutes
jusqu'a l'echeance. Le commentaire juste au-dessus du site d'appel mettait
en garde contre exactement ca (« l'ordre inverse existe ailleurs »).

Une regle qu'on doit se rappeler tout seul n'est pas une regle. Celle-ci est
desormais verifiee.
"""
from pathlib import Path
import re
import tempfile

RACINE = Path(__file__).resolve().parents[2]
LIFECYCLE = "src/kernel/process/thread/lifecycle.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", l) for l in texte.splitlines())


def corps_de(texte: str, entete: str) -> str | None:
    """Corps d'une fonction, des accolades equilibrees."""
    debut = texte.find(entete)
    if debut < 0:
        return None
    i = texte.find("{", debut)
    if i < 0:
        return None
    profondeur, j = 0, i
    while j < len(texte):
        if texte[j] == "{":
            profondeur += 1
        elif texte[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[i: j + 1]
        j += 1
    return None


def controle(racine: Path) -> list:
    erreurs = []
    lc = racine / LIFECYCLE
    if not lc.exists():
        return [f"{LIFECYCLE} introuvable"]
    exit_corps = corps_de(sans_commentaires(lc.read_text()), "pub fn exit_current(")
    if exit_corps is None:
        return ["exit_current introuvable : le garde-fou ne controle plus rien"]

    # Les sondes appelees sur ce chemin, sous la forme crate::kernel::MODULE::fn()
    appels = set(re.findall(
        r"crate::kernel::([a-z_]+(?:::[a-z_]+)*)::([a-z_0-9]+)\s*\(\s*\)", exit_corps))
    if not appels:
        erreurs.append("aucune sonde reconnue dans exit_current : motif a revoir")

    for module, fonction in sorted(appels):
        # `clean_page_cache` est un alias de module : le chemin d'appel ne
        # donne PAS le fichier. On cherche donc la definition partout, et on
        # controle CHAQUE homonyme -- fail-closed : si deux fichiers
        # definissent le meme nom, les deux doivent etre sans verrou.
        trouvee = False
        for src in sorted((racine / "src").rglob("*.rs")):
            corps = corps_de(sans_commentaires(src.read_text()), f"pub fn {fonction}(")
            if corps is None:
                continue
            trouvee = True
            if ".lock()" in corps:
                erreurs.append(
                    f"{fonction} ({src.relative_to(racine)}) prend un verrou et "
                    f"est appelee depuis exit_current, qui tient deja "
                    f"process.lifecycle : arete d'ordre de verrous sur un "
                    f"chemin de sortie"
                )
        if not trouvee:
            erreurs.append(
                f"{module}::{fonction} appelee depuis exit_current mais "
                f"introuvable : le garde-fou ne peut pas conclure"
            )
    return erreurs


def test_negatif() -> list:
    """Le controle doit REFUSER la regression exacte de l'integration #256."""
    pc = RACINE / "src/kernel/memory/page_cache.rs"
    txt = pc.read_text()
    mute = txt.replace("        EN_TABLE.load(Ordering::Relaxed),",
                       "        CACHE.lock().entrees.len(),", 1)
    if mute == txt:
        return ["test negatif impossible : balayage_temoins a change de forme"]
    with tempfile.TemporaryDirectory() as tmp:
        faux = Path(tmp)
        for rel in (LIFECYCLE, "src/kernel/memory/page_cache.rs"):
            (faux / rel).parent.mkdir(parents=True, exist_ok=True)
        (faux / LIFECYCLE).write_text((RACINE / LIFECYCLE).read_text())
        (faux / "src/kernel/memory/page_cache.rs").write_text(mute)
        if not controle(faux):
            return ["test negatif inerte : le CACHE.lock() reintroduit n'est pas detecte"]
    return []


def main() -> int:
    erreurs = controle(RACINE) + test_negatif()
    if erreurs:
        for e in erreurs:
            print(f"SONDES_SANS_VERROU_FAIL {e}")
        print(f"SONDES_SANS_VERROU_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("SONDES_SANS_VERROU_VERDICT ok chemin=exit_current negatif=1")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
