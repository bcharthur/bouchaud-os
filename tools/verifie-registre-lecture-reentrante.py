#!/usr/bin/env python3
"""Garde-fou : une lecture du registre des taches peut en contenir une autre.

# Le defaut, observe au moniteur QEMU

Le rendez-vous de quiescence du registre est un verrou lecteur/ecrivain a
PRIORITE ECRIVAIN. L'ecrivain publie son drapeau puis attend que le compte des
lecteurs retombe a zero ; un lecteur qui voit le drapeau attend sans se
compter. C'est correct tant qu'un lecteur n'en prend qu'un a la fois.

Un lecteur qui en prend un SECOND en tenant deja le premier referme la
discipline sur elle-meme : son propre garde exterieur maintient le compte a un,
l'ecrivain ne repart donc jamais, et le garde interieur attend un drapeau qui
ne tombera plus.

Ce n'est pas une hypothese. Le scenario `nvme-parallele` gelait environ une
fois sur deux, au troisieme passage, a l'instant precis ou `registre_ajoute`
recyclait l'emplacement du fil de montage qui venait de mourir. Les registres
pris au moniteur au moment du gel nommaient les quatre coeurs :

    CPU#0  RIP=...  RegistreEcriture::acquire   registre.rs:171   IF=0
    CPU#1  RIP=...  RegistreLecture::acquire    registre.rs:136
    CPU#2  RIP=...  RegistreLecture::acquire    registre.rs:136
    CPU#3  RIP=...  RegistreLecture::acquire    registre.rs:136

Un ecrivain qui attend des lecteurs, trois lecteurs qui attendent l'ecrivain.
La machine se taisait, et rien d'autre ne le disait.

# Pourquoi la regle vit dans la structure, et pas au site d'appel

L'imbrication n'etait pas une maladresse isolee : `wake_sleepers` tient une vue
du registre et appelle `publish_ready`, qui en reprend une ; `preempt_from_irq`
tient une vue et appelle `registre_pointeur_ordonnanceur`, qui en reprend une.
Interdire l'imbrication site par site serait a refaire a chaque nouvel
appelant, et le prochain oubli redonnerait le meme silence.

Le garde de lecture est donc REENTRANT par coeur : un garde imbrique ne
retouche pas le compte global -- le coeur y figure deja pour un -- et ne
consulte pas le drapeau, puisque son propre compte prouve qu'aucun ecrivain
n'est ENTRE dans sa section critique.

# Ce qui est verifie

Que la profondeur par coeur existe, que le niveau imbrique court-circuite bien
l'attente ET le compte global, que la comptabilite se fasse IRQ masquees -- un
handler tombant entre les deux compteurs se croirait au premier niveau --, que
le compte global ne retombe qu'au dernier garde du coeur, que la priorite
ecrivain soit intacte, et que le recycleur refuse de partir en tenant lui-meme
une lecture.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
REGISTRE = RACINE / "src/kernel/process/thread/registre.rs"


def sans_commentaires(texte):
    return "\n".join(
        ligne for ligne in texte.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def corps(source, entete):
    """Le corps `{...}` qui suit `entete`, accolades equilibrees."""
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def condition_constante(bloc):
    """Une condition figee neutralise une regle sans la supprimer."""
    return re.search(r"if\s+(?:true|false)\b", bloc) is not None


def main():
    if not REGISTRE.exists():
        print("  - fichier absent : src/kernel/process/thread/registre.rs")
        return 1
    source = sans_commentaires(REGISTRE.read_text(encoding="utf-8"))
    fautes = []

    if "static PROFONDEUR_LECTURE" not in source:
        fautes.append(
            "registre.rs : la profondeur de lecture par coeur a disparu. Sans "
            "elle, un garde imbrique reprend le protocole complet et attend un "
            "drapeau que son propre garde exterieur empeche de tomber."
        )
    else:
        # La declaration tient sur deux lignes et contient deux `;` : on prend
        # la fenetre, pas le premier point-virgule venu.
        debut = source.find("static PROFONDEUR_LECTURE")
        declaration = source[debut:debut + 200]
        if declaration.count("MAX_CPUS") < 2:
            fautes.append(
                "registre.rs : la profondeur de lecture n'est plus dimensionnee "
                "par coeur. Une profondeur partagee entre coeurs ferait passer "
                "un coeur pour imbrique parce qu'un AUTRE tient une lecture, et "
                "l'exclusion vis-a-vis du recycleur tomberait."
            )

    # --- le garde de lecture -------------------------------------------------
    acquisition = corps(source, "impl RegistreLecture {")
    if acquisition is None:
        fautes.append("registre.rs : RegistreLecture::acquire a disparu.")
    else:
        if "PROFONDEUR_LECTURE" not in acquisition:
            fautes.append(
                "registre.rs : l'acquisition ne consulte plus la profondeur "
                "locale ; l'imbrication redevient un interblocage."
            )
        if not re.search(r"if\s+profondeur\s*>\s*0", acquisition):
            fautes.append(
                "registre.rs : le court-circuit du niveau imbrique a disparu. "
                "C'est LUI la correction : sans lui, un second garde attend un "
                "drapeau que le premier retient."
            )
        # Le niveau imbrique ne doit toucher NI le compte global NI le drapeau.
        niveau_imbrique = corps(acquisition, "if profondeur > 0")
        if niveau_imbrique is None:
            fautes.append(
                "registre.rs : la branche imbriquee n'est plus identifiable."
            )
        else:
            if "LECTEURS" in niveau_imbrique:
                fautes.append(
                    "registre.rs : la branche imbriquee touche au compte "
                    "global. Le coeur y figure deja pour un : l'y compter deux "
                    "fois rendrait la quiescence inatteignable a la sortie."
                )
            if "ECRIVAIN" in niveau_imbrique:
                fautes.append(
                    "registre.rs : la branche imbriquee attend de nouveau le "
                    "drapeau d'ecriture. C'est exactement l'attente que le "
                    "garde exterieur rend eternelle."
                )
        # La priorite ecrivain reste intacte au premier niveau.
        if "ECRIVAIN.load" not in acquisition or "LECTEURS.fetch_sub" not in acquisition:
            fautes.append(
                "registre.rs : le protocole du premier niveau -- se compter, "
                "relire le drapeau, se decompter s'il est leve -- a disparu. "
                "La reentrance ne remplace pas l'exclusion."
            )
        if "interrupts::disable()" not in acquisition:
            fautes.append(
                "registre.rs : la comptabilite d'acquisition n'est plus faite "
                "IRQ masquees. Un handler tombant entre le compte global et la "
                "profondeur locale se croirait au premier niveau, et "
                "reprendrait le protocole complet sous le garde interrompu : "
                "le meme interblocage, reduit a quelques instructions."
            )
        if condition_constante(acquisition):
            fautes.append(
                "registre.rs : une condition figee dans l'acquisition. Une "
                "regle qu'on neutralise sans la retirer est pire que son "
                "absence : elle a l'air d'etre la."
            )

    relachement = corps(source, "impl Drop for RegistreLecture {")
    if relachement is None:
        fautes.append("registre.rs : le relachement du garde de lecture a disparu.")
    else:
        if "PROFONDEUR_LECTURE" not in relachement:
            fautes.append(
                "registre.rs : le relachement ne decompte plus la profondeur "
                "locale ; le coeur resterait imbrique a vie et ne contribuerait "
                "plus jamais au compte global."
            )
        if not re.search(r"if\s+profondeur\s*<=\s*1", relachement):
            fautes.append(
                "registre.rs : le compte global retombe sans condition de "
                "profondeur. Un garde imbrique rendrait alors la quiescence "
                "alors que le garde exterieur tient encore une reference, et "
                "le recycleur ecraserait une tache sous un lecteur."
            )
        if "interrupts::disable()" not in relachement:
            fautes.append(
                "registre.rs : le relachement n'est plus fait IRQ masquees ; "
                "les deux compteurs cesseraient de bouger ensemble."
            )
        if condition_constante(relachement):
            fautes.append(
                "registre.rs : une condition figee dans le relachement."
            )

    # --- le rendez-vous d'ecriture ------------------------------------------
    ecrivain = corps(source, "impl RegistreEcriture {")
    if ecrivain is None:
        fautes.append("registre.rs : RegistreEcriture::acquire a disparu.")
    else:
        if "LECTEURS.load" not in ecrivain:
            fautes.append(
                "registre.rs : le rendez-vous d'ecriture n'attend plus la "
                "quiescence des lecteurs. Il ecraserait une tache sous une "
                "reference vivante."
            )
        if "profondeur_lecture_locale()" not in ecrivain:
            fautes.append(
                "registre.rs : le rendez-vous d'ecriture ne verifie plus qu'il "
                "ne tient pas lui-meme une lecture. Il attend un compte global "
                "dont il ferait partie : l'interblocage serait certain, et "
                "muet."
            )
        if condition_constante(ecrivain):
            fautes.append(
                "registre.rs : une condition figee dans le rendez-vous "
                "d'ecriture."
            )

    if fautes:
        print("registre lecture reentrante : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "registre lecture reentrante : profondeur par coeur, niveau imbrique "
        "sans attente ni recompte, comptabilite IRQ masquees, quiescence rendue "
        "au dernier garde, recycleur qui refuse de s'attendre lui-meme"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
