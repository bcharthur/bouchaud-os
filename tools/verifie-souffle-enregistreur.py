#!/usr/bin/env python3
"""Garde-fou : l'enregistreur de vol doit pouvoir rapporter sa propre mort.

# Le defaut, mesure le 16 septembre 2026

L'archive physique couvre 1,07 s a 7,30 s. La machine a tourne vingt minutes.
Cinquante-trois enregistrements sur huit mille cent quatre-vingt-douze
emplacements, `fatal.log` vide, aucune marque de FIN. Et au dernier
echantillon reellement ecrit :

    bb_writes=47 bb_failures=0 bb_busy_skips=0 bb_fenetres_rendues=0 bb_filets=0

Ces zeros ne disent pas que tout allait bien. Ils disent qu'a 7,047 s tout
allait ENCORE bien. Le clavier qui lache, la charge, l'extinction -- tout cela
devait etre raconte par des compteurs qui ne voyagent que dans un
enregistrement, qu'il fallait justement pouvoir ecrire.

C'est un piege circulaire, et il a deja coute trois corrections : le fil
dedie, la promotion en Interactive, le filet de securite. Aucune n'etait
verifiable, parce qu'un echec produit exactement le meme silence qu'avant.

# Ce qui est verifie ici

1. `src/kernel/debug/souffle.rs` reste PUR -- aucun `use crate::`, aucun
   `unsafe`, l'horloge en parametre. C'est ce qui permet de le tester sur
   machine hote, la ou la machine cible ne montre rien.
2. `blackbox::append` note les DEUX issues. Ne noter que les echecs perdrait
   la date du dernier succes, qui est ce qui date le debut du silence.
3. `vide_avant_extinction` rend l'etape atteinte, et non un booleen : « la
   sauvegarde a echoue » ne dit pas si le journal n'a pas pu etre draine, si
   la marque de fin n'est pas passee, ou si seule la synchronisation a
   manque -- trois couches, trois remedes.
4. L'extinction AFFICHE le detail. Le message precedent renvoyait vers « les
   logs », c'est-a-dire vers le fichier dont il annoncait l'absence.
5. L'echantillon par coeur porte `quantums=`. Sans lui, `enters=0` sur les
   quinze coeurs secondaires -- vrai en permanence, puisque seul le BSP
   recoit IRQ0 -- se lit comme quinze coeurs morts. C'est la lecture qui en a
   ete faite le 16 septembre, a tort.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

SOUFFLE = RACINE / "src/kernel/debug/souffle.rs"
BLACKBOX = RACINE / "src/kernel/debug/blackbox.rs"
POWER = RACINE / "src/kernel/power.rs"
ECRAN = RACINE / "src/gui/power_screen.rs"
TEST = RACINE / "tools/platform/test_souffle.rs"


def code_seul(source):
    """La source privee de ses commentaires.

    La premiere version de cette garde lisait le fichier entier et se
    declenchait sur SA PROPRE documentation : le commentaire de souffle.rs
    dit « aucun `use crate::`, aucun `unsafe` », et la garde y voyait les deux
    fautes qu'il promettait d'eviter. Une garde qui lit la prose verifie ce
    que le code PRETEND, pas ce qu'il fait.
    """
    sans_blocs = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(
        re.sub(r"//.*$", "", ligne)
        for ligne in sans_blocs.splitlines()
    )


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    souffle = lit(SOUFFLE, fautes)
    blackbox = lit(BLACKBOX, fautes)
    power = lit(POWER, fautes)
    ecran = lit(ECRAN, fautes)
    test = lit(TEST, fautes)

    # 1. La purete, qui est ce qui rend la regle verifiable.
    if souffle is not None:
        souffle_code = code_seul(souffle)
        if "use crate::" in souffle_code:
            fautes.append(
                "souffle.rs depend du reste du noyau. Il ne se compile plus "
                "seul avec `rustc --test`, et ses regles redeviennent "
                "invrifiables -- or c'est precisement le module dont on ne "
                "peut rien observer sur la machine."
            )
        if "unsafe" in souffle_code:
            fautes.append(
                "souffle.rs contient du `unsafe`. Cet etat doit survivre a la "
                "perte du support : il ne peut pas dependre d'un pointeur."
            )
        if "monotonic_ns" in souffle_code:
            fautes.append(
                "souffle.rs lit l'horloge lui-meme au lieu de la recevoir. "
                "Les tests hote ne pourraient plus fabriquer de silence."
            )
        for jeton, pourquoi in (
            ("fetch_max", "la pire serie doit etre un maximum, sinon une "
                          "accalmie efface la panne qui l'a precedee"),
            ("compare_exchange", "seul le PREMIER echec d'une serie date le "
                                 "debut du silence"),
            ("saturating_sub", "une horloge qui recule ne doit pas produire "
                               "un silence absurde"),
        ):
            if jeton not in souffle_code:
                fautes.append("souffle.rs : `%s` a disparu -- %s." % (jeton, pourquoi))

    # 2. Les deux issues sont notees, au seul point d'ecriture.
    if blackbox is not None:
        if not re.search(r"note_souffle\(ok,", code_seul(blackbox)):
            fautes.append(
                "blackbox.rs : `append` ne note plus les deux issues. Ne "
                "compter que les echecs perdrait la date du dernier succes, "
                "qui est ce qui date le debut du silence."
            )
        if "pub fn souffle()" not in blackbox:
            fautes.append("blackbox.rs : l'etat de survie n'est plus lisible de l'exterieur.")
        # 5. L'echantillon par coeur ne doit plus pouvoir se lire a l'envers.
        if "quantums={}" not in blackbox or "quantums_recus(cpu)" not in blackbox:
            fautes.append(
                "blackbox.rs : la ligne `cpus` ne porte plus `quantums=`. "
                "`enters=0` vaut zero en permanence sur les quinze coeurs "
                "secondaires -- seul le BSP recoit IRQ0 -- et se lit alors "
                "comme quinze coeurs morts."
            )
        # 3. L'etape atteinte survit au retour.
        #
        # LA BORNE DE MOT N'EST PAS UN DETAIL. `"pub struct Vidage"` est une
        # SOUS-CHAINE de `"pub struct VidageAncien"` : renommer le type
        # passait la garde sans rien casser de son texte. C'est la mutation
        # M5, et elle n'etait pas attrapee.
        blackbox_code = code_seul(blackbox)
        if not re.search(r"\bpub struct Vidage\b", blackbox_code):
            fautes.append(
                "blackbox.rs : `vide_avant_extinction` a reperdu son etat "
                "detaille. Un booleen ne distingue pas un journal non draine "
                "d'une marque de fin absente ou d'une synchronisation ratee."
            )
        if not re.search(r"->\s*Vidage\b", blackbox_code):
            fautes.append(
                "blackbox.rs : `vide_avant_extinction` ne rend plus un "
                "`Vidage` ; l'etape atteinte se perd de nouveau au retour."
            )
        for champ in ("support", "draine", "marque", "synchronise"):
            if not re.search(r"\bpub %s: bool\b" % champ, blackbox_code):
                fautes.append("blackbox.rs : Vidage a perdu son champ `%s`." % champ)

    # 4. L'extinction dit ce qui a echoue, la ou on peut encore le lire.
    if power is not None:
        if "fn rapporte_echec" not in power or "rapporte_echec(&vidage" not in power:
            fautes.append(
                "power.rs : l'extinction n'affiche plus le detail de l'echec. "
                "C'est le seul endroit qui reste quand le journal est "
                "precisement ce qu'on n'a pas pu sauver."
            )
        if power.count("rapporte_echec(&vidage") < 2:
            fautes.append(
                "power.rs : une seule des deux sorties rapporte. L'extinction "
                "et le redemarrage echouent pour les memes raisons."
            )
        if "alloc::" in code_seul(power).split("fn rapporte_echec")[-1][:3000]:
            fautes.append(
                "power.rs : le rapport d'extinction alloue. L'allocateur est "
                "un des sous-systemes qui peuvent avoir lache, et un rapport "
                "de panne qui a besoin du tas ne sort pas quand le tas est le "
                "probleme."
            )

    if ecran is not None:
        if "pub fn detail_echec" not in ecran:
            fautes.append("power_screen.rs : `detail_echec` a disparu.")
        if "consulter les logs" in code_seul(ecran):
            fautes.append(
                "power_screen.rs : l'ecran renvoie de nouveau vers « les "
                "logs ». Ce sont exactement ceux qui viennent de ne pas etre "
                "sauves : le message designe le fichier dont il annonce "
                "l'absence."
            )

    # BOUCHAUD_ARCHIVE_SUFFISANTE_V1
    #
    # L'archive du 16 septembre ne portait ni l'etat du lien, ni le bail, ni
    # les trames par seconde, ni le pire a-coup, ni l'etat du disque. Tout
    # cela existait dans le noyau, dispersé dans des compteurs qu'aucun
    # enregistrement ne transportait : il a fallu recouper la console serie
    # pour retrouver la montee du lien Ethernet.
    if blackbox is not None:
        code = code_seul(blackbox)
        if "fn etat_systeme" not in code:
            fautes.append(
                "blackbox.rs : la ligne `etat` a disparu. L'archive ne dirait "
                "plus si le lien etait monte, si le bail etait la, a combien "
                "de trames tournait le bureau, ni si le disque repondait."
            )
        for champ, pourquoi in (
            ("reseau={}", "l'etat du demarrage reseau"),
            ("lien={}", "l'etat du lien Ethernet"),
            ("dns=", "le resolveur reellement en vigueur"),
            ("fps={}", "les trames par seconde"),
            ("ecart_max_ms={}", "le PIRE a-coup -- une moyenne de soixante "
                                "trames avec un trou d'une seconde et demie se "
                                "lit « fluide », et c'est pourtant le trou "
                                "qu'on voit a l'ecran"),
            ("disque_stalls={}", "les blocages du disque"),
            ("equite_hid_sauts={}", "la famine du clavier, qui est ce qui le "
                                    "fait passer pour deconnecte"),
        ):
            if champ not in code:
                fautes.append(
                    "blackbox.rs : la ligne `etat` ne porte plus `%s` -- %s."
                    % (champ, pourquoi)
                )
        # Le genre doit rester celui que l'extracteur connait.
        #
        # LA FENETRE DOIT S'ARRETER A LA FONCTION. Une borne fixe de quatre
        # mille caracteres debordait sur `echantillon_par_cpu`, qui appelle
        # elle aussi `append(KIND_SAMPLE, ...)` : changer le genre DANS
        # `etat_systeme` passait la garde grace au voisin.
        i_etat = code.find("fn etat_systeme")
        corps_etat = ""
        if i_etat != -1:
            suite = code[i_etat:]
            fin = suite.find("\nfn ", 1)
            corps_etat = suite if fin == -1 else suite[:fin]
        if i_etat != -1 and "KIND_SAMPLE" not in corps_etat:
            fautes.append(
                "blackbox.rs : la ligne `etat` n'est plus un KIND_SAMPLE. "
                "L'extracteur qui produit samples.log ne connait que les "
                "genres existants ; un genre neuf verrait sa charge utile "
                "perdue et n'apparaitrait que comme un numero dans "
                "records.json."
            )

    if test is not None and test.count("#[test]") < 8:
        fautes.append(
            "test_souffle.rs : moins de huit cas. Les regles couvertes -- "
            "accalmie, machine au repos, horloge qui recule -- sont celles qui "
            "produisent des faux verdicts."
        )

    if fautes:
        print("souffle de l'enregistreur : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "souffle de l'enregistreur : etat pur et resident, deux issues notees, "
        "etape detaillee, echec affiche, coeurs non equivoques."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
