#!/usr/bin/env python3
"""On ne balaye pas le cache de pages pour ne rien trouver.

BOUCHAUD_C70_NE_PAS_BALAYER_POUR_NE_RIEN_TROUVER

`acquire` appelle `retire_un_candidat` a CHAQUE defaut de cache des que la
table atteint `MAX_RECLAIMABLE_PAGES`. Quand la file de candidats est vide --
le cas normal tant que les pages restent mappees -- le repli parcourait toute
la table, en prenant le verrou d'etat de chaque entree, sous le verrou GLOBAL
du cache.

Le modele « un balayage par defaut de cache une fois la table pleine » se
verifie au chiffre pres sur deux charges independantes :

    banc local      20 480 miss - 16 384 = 4 096 attendus,  4 096 observes
    Ladybird #357   52 534 miss - 16 384 = 36 150 attendus, 35 495 observes

Cout mesure au run 35955074619 : 277 s, soit 57 % des 490 s de temps noyau.

Correction : `RECUPERABLES` a zero, le balayage est GARANTI de ne rien
trouver. On sort avant.

Avant / apres, banc `run_faute_fichier.sh` a 80 Mio, trois executions :

    variante   duree_ms min/med/max      noyau_ms min/med/max    balayage
    avant      53014 / 54422 / 55130     49722 / 51399 / 51842   31 384 ms
    apres      21905 / 22607 / 22817     18677 / 19344 / 19565        0 ms

Temoins IDENTIQUES dans les deux bras -- `entrees=20480`, `recuperees=0` --
donc la table n'a pas deborde et aucune recuperation n'a ete perdue.

Ce fichier empeche la regression de revenir en silence.
"""
from pathlib import Path
import re
import tempfile

RACINE = Path(__file__).resolve().parents[2]
CACHE = "src/kernel/memory/page_cache.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", l) for l in texte.splitlines())


def corps(texte: str) -> str | None:
    debut = texte.find("fn retire_un_candidat(")
    if debut < 0:
        return None
    fin = texte.find("\n}\n", debut)
    return sans_commentaires(texte[debut:fin]) if fin > 0 else None


def controle(racine: Path) -> list:
    erreurs = []
    txt = (racine / CACHE).read_text()
    c = corps(txt)
    if c is None:
        return ["retire_un_candidat introuvable dans " + CACHE]

    garde = re.search(r"if\s+RECUPERABLES\.load\([^)]*\)\s*==\s*0\s*\{", c)
    if not garde:
        erreurs.append(
            "retire_un_candidat ne sort plus quand RECUPERABLES vaut zero : "
            "chaque defaut de cache reparcourrait toute la table"
        )
    else:
        # La garde doit PRECEDER le parcours, sinon elle ne protege rien.
        parcours = c.find(".entrees.iter()")
        if parcours >= 0 and garde.start() > parcours:
            erreurs.append(
                "la garde RECUPERABLES == 0 est posee APRES le parcours de la "
                "table : elle ne supprime aucun balayage"
            )
        if "return None" not in c[garde.start(): garde.start() + 200]:
            erreurs.append("la garde RECUPERABLES == 0 ne sort pas de la fonction")

    if "BALAYAGE_EVITES" not in c:
        erreurs.append(
            "aucun compteur de balayages evites : la correction ne serait pas "
            "mesurable, donc pas verifiable"
        )
    # Les temoins doivent rester publies : sans eux, « plus rapide » et « ne
    # fait plus son travail » se confondent.
    if "pub fn balayage_temoins" not in txt:
        erreurs.append(
            "balayage_temoins a disparu : sans `entrees` et `recuperees`, rien "
            "ne distingue une eviction supprimee d'une eviction cassee"
        )
    return erreurs


def test_negatif() -> list:
    """Le controle doit REFUSER chacune des trois regressions connues."""
    txt = (RACINE / CACHE).read_text()
    mutations = [
        ("garde retiree", lambda t: t.replace(
            "    if RECUPERABLES.load(Ordering::Relaxed) == 0 {\n"
            "        BALAYAGE_EVITES.fetch_add(1, Ordering::Relaxed);\n"
            "        return None;\n    }\n", "", 1)),
        ("garde neutralisee", lambda t: t.replace(
            "if RECUPERABLES.load(Ordering::Relaxed) == 0 {",
            "if false && RECUPERABLES.load(Ordering::Relaxed) == 0 {", 1)),
        ("temoins retires", lambda t: t.replace(
            "pub fn balayage_temoins", "fn _balayage_temoins_retire", 1)),
    ]
    inertes = []
    for nom, muter in mutations:
        with tempfile.TemporaryDirectory() as tmp:
            faux = Path(tmp)
            (faux / CACHE).parent.mkdir(parents=True, exist_ok=True)
            (faux / CACHE).write_text(muter(txt))
            if not controle(faux):
                inertes.append(f"test negatif inerte : « {nom} » n'a pas ete detecte")
    return inertes


def main() -> int:
    erreurs = controle(RACINE) + test_negatif()
    if erreurs:
        for e in erreurs:
            print(f"BALAYAGE_CACHE_FAIL {e}")
        print(f"BALAYAGE_CACHE_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("BALAYAGE_CACHE_VERDICT ok garde=posee temoins=publies negatifs=3")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
