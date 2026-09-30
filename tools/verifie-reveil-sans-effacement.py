#!/usr/bin/env python3
"""Un reveilleur n'efface ni la cle d'attente ni l'echeance de la tache qu'il reveille.

BOUCHAUD_REVEIL_SANS_EFFACER_LA_CLE_V1

# Le defaut

`wake_wait_queue` faisait : transition `Blocked -> Ready` (compare_exchange),
PUIS `wait_queue_key = 0`, puis mise en file. Entre la transition et
l'effacement, la tache reveillee -- encore sur son coeur, entre la publication
de `Blocked` et `schedule()` -- voyait `Ready`, sortait de sa boucle, n'avait
pas son tour, et SE REPARQUAIT avec une nouvelle cle. L'effacement tardif
tombait alors sur cette NOUVELLE attente : tache `Blocked`, cle 0, introuvable
par tout reveil (la recherche se fait par cle), et la mise en file suivante
jetee (tache non eligible).

Observe en endurance SMP4 (verrou du controleur ATA) : le lecteur dont
c'etait le tour `Blocked cle_attente=0`, les cinq autres parques derriere lui,
neuf minutes de machine figee. Le meme motif existait dans `wake_for_signal`
(cle et echeance) et dans le reveil par echeance `wake_sleepers` (echeance :
une attente bornee devenait infinie).

# La regle

Dans ces trois reveilleurs, AUCUNE ecriture de `wait_queue_key`, de
`wake_deadline_ns` ni de `waiting_for_child` sur la tache reveillee.
(`waiting_for_child` : meme motif dans `wake_for_signal` -- un parent revenu
dans `wait4` et reparque perdait le drapeau que la sortie du fils exige.) Chaque attente efface ses propres
champs en reprenant la main ; une cle perimee sur une tache `Ready` est sans
effet, puisque tout reveil exige `Blocked -> Ready`.

Fail-closed : une fonction introuvable est une faute (un renommage ne doit pas
rendre le garde muet). Un test negatif reintroduit l'ecriture exacte.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CIBLES = [
    ("src/kernel/process/thread/blocage.rs", "pub(crate) fn wake_wait_queue("),
    ("src/kernel/process/thread/blocage.rs", "pub fn wake_for_signal("),
    ("src/kernel/process/thread/sommeil.rs", "fn wake_sleepers("),
]
INTERDIT = re.compile(
    r"\b(?!current\(\))\w+\s*\.\s*(wait_queue_key|wake_deadline_ns|waiting_for_child)\s*\.\s*range\s*\("
)


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", ligne) for ligne in texte.splitlines())


def corps(texte: str, entete: str) -> str | None:
    debut = texte.find(entete)
    if debut < 0:
        return None
    ouverture = texte.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture : i + 1]
    return None


def verifie(racine: Path) -> list[str]:
    fautes = []
    for fichier, entete in CIBLES:
        chemin = racine / fichier
        if not chemin.is_file():
            fautes.append(f"{fichier} introuvable")
            continue
        texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
        bloc = corps(texte, entete)
        if bloc is None:
            fautes.append(f"{fichier} : `{entete}` introuvable -- le garde ne lit plus rien")
            continue
        for m in INTERDIT.finditer(bloc):
            fautes.append(
                f"{fichier} : `{entete.strip('(')}` ecrit `{m.group(1)}` de la tache reveillee"
            )
    return fautes


def test_negatif() -> bool:
    """Reintroduit l'effacement fautif dans une copie : le garde doit rougir."""
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for fichier, _ in CIBLES:
            dest = copie / fichier
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / fichier).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / "src/kernel/process/thread/blocage.rs"
        texte = cible.read_text(encoding="utf-8")
        texte = texte.replace("        publish_ready(index);\n        woke += 1;",
                              "        tache.wait_queue_key.range(0);\n        publish_ready(index);\n        woke += 1;", 1)
        cible.write_text(texte, encoding="utf-8")
        if not verifie(copie):
            return False
        # Second negatif : l'effacement du drapeau d'attente de fils.
        texte = texte.replace("        tache.wait_queue_key.range(0);\n", "", 1)
        texte = texte.replace("            task.futex_key.range(0);\n",
                              "            task.futex_key.range(0);\n            task.waiting_for_child.range(false);\n", 1)
        cible.write_text(texte, encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("reveil sans effacement : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    if not test_negatif():
        print("reveil sans effacement : le test negatif ne rougit pas -- garde inoperant")
        return 1
    print("REVEIL_SANS_EFFACEMENT_OK cibles=3 negatifs=2")
    return 0


if __name__ == "__main__":
    sys.exit(main())
