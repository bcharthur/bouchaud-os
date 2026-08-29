#!/usr/bin/env python3
"""La comptabilite du gros verrou ne doit plus pouvoir annoncer l'impossible.

# Ce que ce garde-fou protege

Un releve du runtime portait `hold_pct=183 %` pour un verrou EXCLUSIF. Le
chiffre n'est pas imprecis, il est impossible : a tout instant il y a au plus
un proprietaire, donc la somme des tenues est majoree par le temps qui passe.
Et un chiffre impossible ne se corrige pas -- il retire toute valeur aux autres
chiffres du meme releve, y compris a la pointe de 29 secondes qui, elle,
designait un vrai figement.

Deux fautes le produisaient, et toutes deux se rattrapent a la lecture :

  1. l'horodatage d'acquisition vivait dans un tableau INDEXE PAR CPU, alors
     que l'intervalle appartient au VERROU. Une case laissee en place par une
     acquisition dont la liberation n'a pas eu lieu sur le meme coeur etait
     consommee bien plus tard par une liberation sans rapport ;
  2. le cumul etait incremente HORS du test qui verifiait qu'un intervalle
     etait bien ouvert. `maintenant - 0` vaut alors le temps depuis le
     demarrage, ajoute d'un coup.

# La troisieme regle, moins evidente

Un chemin qui capture son index de CPU AVANT de masquer les interruptions peut
etre commute entre les deux : la pile noyau reprend sur un autre coeur, et
l'index designe un etranger. `OWNER` recoit alors le jeton d'un coeur pendant
que `DEPTH` est pose sur un autre -- et le bit publie dans `PARKED` avant de
s'arreter n'est pas celui du dormeur, ce qui est un reveil perdu par
construction. L'index se lit donc TOUJOURS apres le masquage.

Les trois regles sont verifiees sur la source, parce qu'aucune ne produit
d'erreur : elles produisent des chiffres, et des chiffres faux se lisent
exactement comme des vrais.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
BKL = RACINE / "src" / "kernel" / "sync" / "bkl.rs"
COMPTE = RACINE / "src" / "kernel" / "sync" / "bkl_compte.rs"

# Chaque chemin qui touche l'etat du verrou, avec la signature qui l'ouvre.
CHEMINS_SOUS_MASQUE = [
    "pub fn enter(",
    "pub fn try_enter(",
    "pub fn try_enter_depuis_zero(",
    "pub fn profondeur_locale(",
    "pub fn held_by_current_cpu(",
    "pub fn suspend_for_schedule(",
    "pub fn resume_after_schedule(",
]


def sans_commentaires(source: str) -> str:
    """Le code seul.

    Les regles ci-dessous cherchent des IDENTIFIANTS. Les commentaires de ce
    fichier citent volontiers l'ancien nom pour expliquer ce qui a change --
    et une explication ne doit pas declencher la regle qu'elle explique.
    """
    return re.sub(r"//[^\n]*", "", source)


def corps(source: str, signature: str) -> str:
    debut = source.index(signature)
    profondeur = 0
    for position in range(debut, len(source)):
        if source[position] == "{":
            profondeur += 1
        elif source[position] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[debut : position + 1]
    raise SystemExit(f"corps introuvable : {signature}")


def main() -> int:
    source = BKL.read_text(encoding="utf-8")
    code = sans_commentaires(source)
    fautes = []

    # --- 1. l'intervalle appartient au verrou, pas a un CPU -----------------
    if not COMPTE.exists():
        fautes.append(f"  {COMPTE.name} a disparu : la comptabilite n'est plus "
                      f"testable sur l'hote")
    if "ACQUIRED_AT_NS" in code:
        fautes.append(
            "  bkl.rs  l'horodatage d'acquisition est revenu dans un tableau "
            "indexe par CPU : une tenue appartient au VERROU, et une case "
            "orpheline sera consommee par une liberation sans rapport"
        )
    if "COMPTES.ouvre(" not in code or "COMPTES.ferme(" not in code:
        fautes.append(
            "  bkl.rs  les sondes ne passent plus par `Comptes` : l'invariant "
            "`somme des tenues <= temps ecoule` n'est plus garanti par "
            "construction"
        )

    # --- 2. rien n'est facture sans intervalle ouvert ------------------------
    release = corps(code, "fn probe_note_release(")
    ajouts = [l for l in release.splitlines()
              if "TOTAL_HOLD_NS.fetch_add" in l]
    if not ajouts:
        fautes.append("  probe_note_release ne cumule plus rien")
    for ligne in ajouts:
        # L'addition doit vivre DANS le bloc que `ferme` n'ouvre qu'en cas
        # d'intervalle reel. Une addition a l'indentation de la fonction est
        # exactement la faute d'origine.
        indentation = len(ligne) - len(ligne.lstrip())
        if indentation <= 4:
            fautes.append(
                "  probe_note_release  `TOTAL_HOLD_NS.fetch_add` est hors du "
                "bloc conditionnel : une liberation orpheline ajouterait tout "
                "le temps ecoule depuis le demarrage"
            )
    if "if let Some(" not in release:
        fautes.append(
            "  probe_note_release  ne teste plus que la fermeture a bien rendu "
            "une duree"
        )

    # --- 3. l'index du CPU se lit APRES le masquage --------------------------
    for signature in CHEMINS_SOUS_MASQUE:
        texte = corps(code, signature)
        lecture = re.search(r"=\s*cpu\(\);", texte)
        if not lecture:
            continue
        masque = texte.find("LocalIrqGuard::acquire()")
        if masque == -1 or masque > lecture.start():
            nom = signature.split("fn ")[1].rstrip("(")
            fautes.append(
                f"  {nom}  lit son index de CPU avant de masquer les "
                f"interruptions : une commutation entre les deux ferait "
                f"travailler ce chemin au nom d'un autre coeur"
            )

    # Le dormeur, lui, doit lire le sien apres le `cli` de `prepare_lock_park`,
    # sans jamais faire confiance a celui de son appelant : c'est le bit pose
    # dans PARKED qui decide qui sera reveille.
    attente = corps(code, "fn wait_for_owner_change(")
    if re.search(r"fn wait_for_owner_change\(\s*cpu\s*:", attente):
        fautes.append(
            "  wait_for_owner_change  recoit encore un index de CPU de son "
            "appelant : cet index peut dater d'avant une commutation, et le "
            "bit pose dans PARKED serait celui d'un autre coeur -- un reveil "
            "perdu par construction"
        )
    ordre_prepare = attente.find("prepare_lock_park()")
    ordre_lecture = attente.find("= cpu();")
    if ordre_prepare == -1 or ordre_lecture == -1 or ordre_lecture < ordre_prepare:
        fautes.append(
            "  wait_for_owner_change  ne lit pas son index de CPU apres le "
            "`cli` de `prepare_lock_park`"
        )

    # --- 4. la liberation qui deverrouille doit rester ordonnee --------------
    #
    # `ferme` avant `OWNER <- FREE` : c'est CE qui rend les intervalles
    # disjoints. Inverse, un autre coeur pourrait ouvrir le sien avant que
    # celui-ci ne soit clos, et les deux se recouvriraient.
    for fonction, nom in [("fn release_one(", "release_one"),
                          ("pub fn suspend_for_schedule(", "suspend_for_schedule")]:
        texte = corps(code, fonction)
        sonde = texte.find("probe_note_release(")
        libere = texte.find("OWNER.store(FREE")
        if sonde == -1 or libere == -1 or sonde > libere:
            fautes.append(
                f"  {nom}  ferme l'intervalle APRES avoir rendu le verrou : "
                f"deux tenues pourraient alors se recouvrir, et leur somme "
                f"depasser le temps ecoule"
            )

    if fautes:
        print("comptabilite du gros verrou : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  src/kernel/sync/bkl.rs : l'intervalle appartient au verrou, rien "
        "n'est facture sans acquisition, et l'index du CPU se lit sous masque"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
