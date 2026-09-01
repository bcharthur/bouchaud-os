#!/usr/bin/env python3
"""L'ecriture incrementale de `/persist` ne peut pas sauter une ecriture reelle.

# Ce que ce garde-fou protege

`synchronise` ne reecrit plus la zone entiere a chaque `fsync` : il garde
l'empreinte de ce que le dernier `sync` REUSSI a laisse sur le disque, et saute
les fichiers dont le chemin, le secteur, la longueur et le sceau coincident.

Cette optimisation repose sur UNE regle : l'empreinte doit etre oubliee des
qu'une ecriture echoue. Sinon le `sync` suivant croirait sur le disque des
octets qui n'y sont jamais arrives, et les sauterait -- une perte de donnees
silencieuse, que rien dans le journal ne signalerait.

Le compilateur ne peut pas la verifier : `return -1` est un chemin ordinaire.
Ce script le fait, en exigeant que chaque sortie en erreur de `synchronise`
soit precedee de `oublie_le_disque()`.

# Et la deuxieme regle

L'empreinte vit dans un `static mut`. Elle n'est sure que parce que `fsync`,
`fdatasync` et `sync` s'executent sous le gros verrou du noyau. Les liberer
sans reprendre ce probleme les ferait courir en parallele sur quatre coeurs.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

# La ligne qui adopte l'empreinte, une fois le disque entierement ecrit.
ADOPTION = "*DISQUE.lock() = nouveau;"
PERSISTANCE = RACINE / "src" / "fs" / "persistance.rs"
PERSISTANCE_TREE = RACINE / "src" / "fs" / "persistance"

def lignes_de(*chemins) -> list[tuple[str, int, str]]:
    """Les lignes d'un sous-systeme, avec leur VRAIE origine.

    Concatener un arbre pour l'analyser est commode, mais rend les numeros de
    ligne faux -- et un garde-fou qui designe la mauvaise ligne fait perdre
    plus de temps qu'il n'en fait gagner. On garde donc, pour chaque ligne, le
    fichier et le numero d'ou elle vient.
    """
    sortie = []
    for chemin in chemins:
        fichiers = sorted(chemin.rglob("*.rs")) if chemin.is_dir() else (
            [chemin] if chemin.exists() else [])
        for fichier in fichiers:
            relatif = fichier.relative_to(RACINE).as_posix()
            for numero, texte in enumerate(
                    fichier.read_text(encoding="utf-8").splitlines(), start=1):
                sortie.append((relatif, numero, texte))
    return sortie



def source_de(*chemins) -> str:
    """Le code d'un sous-systeme, quel que soit son decoupage en fichiers.

    Ce garde-fou lisait `persistance.rs`. Sa fragmentation en `persistance/**`
    l'a fait s'arreter sur « introuvable » -- et comme rien ne l'executait, la
    regle a cesse de proteger quoi que ce soit sans que personne le voie.
    """
    morceaux = []
    for chemin in chemins:
        if chemin.is_dir():
            for fichier in sorted(chemin.rglob("*.rs")):
                morceaux.append(fichier.read_text(encoding="utf-8"))
        elif chemin.exists():
            morceaux.append(chemin.read_text(encoding="utf-8"))
    return "\n".join(morceaux)
BKL = RACINE / "src" / "compat" / "linux" / "bkl.rs"

# Combien de lignes en arriere on accepte de chercher l'oubli. Un `return -1`
# est toujours precede de son `log_fmt`, qui tient sur quelques lignes.
PORTEE = 12


def corps(source: str, signature: str) -> tuple[int, list[str]]:
    """Lignes du corps de la fonction, et le numero de sa premiere ligne."""
    lignes = source.splitlines()
    for numero, ligne in enumerate(lignes):
        if ligne.startswith(signature):
            profondeur = 0
            corps_lignes = []
            for suite in lignes[numero:]:
                profondeur += suite.count("{") - suite.count("}")
                corps_lignes.append(suite)
                if profondeur == 0 and corps_lignes[0].count("{"):
                    break
            return numero + 1, corps_lignes
    raise SystemExit(f"introuvable : {signature}")


def verifie_oublis() -> list[str]:
    source = source_de(PERSISTANCE, PERSISTANCE_TREE)
    premiere, lignes = corps(source, "pub fn synchronise()")
    fautes = []
    for decalage, ligne in enumerate(lignes):
        if ligne.strip() != "return -1;":
            continue
        avant = lignes[max(0, decalage - PORTEE) : decalage]
        if not any("oublie_le_disque()" in l for l in avant):
            fautes.append(
                f"  persistance.rs:{premiere + decalage}  `return -1` sans "
                f"`oublie_le_disque()` : l'empreinte survivrait a un echec "
                f"d'ecriture, et le sync suivant sauterait des secteurs qui "
                f"n'ont jamais ete ecrits"
            )
    adoptions = source.count(ADOPTION)
    if adoptions == 0:
        fautes.append(
            "  persistance  l'empreinte n'est adoptee nulle part : "
            "l'optimisation ne peut pas fonctionner"
        )
    elif adoptions != 1:
        # Une seule adoption, et apres l'en-tete : c'est ce qui garantit que
        # l'empreinte ne decrit JAMAIS un disque a moitie ecrit.
        fautes.append(
            f"  persistance  l'empreinte est adoptee {adoptions} fois ; elle "
            f"ne doit l'etre qu'apres un sync completement reussi"
        )
    return fautes


def verifie_verrou() -> list[str]:
    """L'empreinte doit avoir son PROPRE verrou.

    # Ce que cette regle disait avant, et pourquoi elle a change

    L'empreinte etait un `static mut`. Elle n'etait alors sure que parce que
    `fsync` gardait le gros verrou, et cette regle interdisait donc de faire
    figurer `FSYNC`/`FDATASYNC`/`SYNC` dans la liste des appels sans gros
    verrou. C'etait une contrainte SUBIE : elle attachait un chemin
    d'entree/sortie lent -- des centaines de millisecondes mesurees -- au
    verrou global du noyau.

    P0-NG1 a donne a l'empreinte son propre `SpinLock`. La contrainte tombe,
    et la regle avec elle. Ce qu'il faut proteger maintenant, c'est
    l'acquis : que l'empreinte ne redevienne JAMAIS un `static mut`. Si elle
    le redevenait, elle cesserait d'etre sure sans que rien ne le signale --
    le gros verrou n'etant plus la pour la couvrir.
    """
    source = source_de(PERSISTANCE, PERSISTANCE_TREE)
    fautes = []
    if not re.search(r"static\s+DISQUE\s*:\s*SpinLock<", source):
        fautes.append(
            "  persistance  l'empreinte du disque n'est plus derriere un "
            "SpinLock : elle n'etait sure sous `static mut` que grace au gros "
            "verrou, dont ce chemin est desormais sorti"
        )
    if re.search(r"static\s+mut\s+DISQUE", source):
        fautes.append(
            "  persistance  l'empreinte est redevenue un `static mut` : plus "
            "rien ne la protege, le gros verrou ayant ete retire de ce chemin"
        )
    return fautes


def main() -> int:
    fautes = verifie_oublis() + verifie_verrou()
    if fautes:
        print("ecriture incrementale de /persist : regle violee")
        print("\n".join(fautes))
        return 1
    print(
        "ok  src/fs/persistance : tout echec oublie l'empreinte, elle n'est "
        "adoptee qu'apres un sync complet, et son verrou est le sien"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
