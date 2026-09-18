#!/usr/bin/env python3
"""Garde-fou : la fenetre Services affiche le REGISTRE, pas une liste de noms.

# Le defaut, releve en photo le 17 septembre 2026

Une passe entiere avait livre un registre de services, sa topologie, ses
tests verts et une commande `services` en ligne. Sur l'ecran de la TRIGKEY,
la fenetre Services montrait toujours exactement ceci :

    BouchaudBrowserHost   En cours   PID ...
    RequestServer         En cours   PID ...
    WebContent            En cours   PID ...
    ImageDecoder          ...
    Compositor            ...
    WebWorker             ...
    Reseau : pret

Six noms de processus Ladybird, codes en dur dans le peintre. Devant une page
qui ne charge pas, cette fenetre ne disait rien de la carte reseau, d'ARP, de
DHCP, de DNS, d'IPv4, de TCP, de TLS ni de l'ordonnanceur -- alors que le
registre, lui, savait.

Le backend n'etait pas le defaut. LE PEINTRE ETAIT LE DEFAUT : il n'ouvrait
jamais le registre.

# Pourquoi une garde et pas un test

Un test de modele visible (`tools/services/test_vue.rs`) prouve que le modele
sait produire l'arbre. Il ne prouve pas que la FENETRE s'en sert : le peintre
peut parfaitement ignorer le modele et repeindre ses six lignes. C'est
exactement ce qui s'est produit. Cette garde relie les deux.

# Les invariants defendus

1. Le peintre visible est `src/gui/apps/services.rs::draw`, et le bureau
   l'appelle par `App::Services`.
2. `draw` prend un INSTANTANE du registre et passe par le modele visible.
   Sans instantane, le rendu tiendrait le verrou du registre pendant la
   peinture et bloquerait toute publication au rythme de l'affichage.
3. `draw` ne contient AUCUN nom de service en dur. La table des noms de
   processus Ladybird vit chez le PUBLICATEUR (`src/gui/services.rs`), qui
   alimente le registre -- jamais chez le peintre, qui le lit.
4. Il n'existe qu'UNE application Services. Pas de `ServicesV2`, pas de
   fenetre d'observabilite parallele, pas de prototype non raccorde.
5. Le peintre dessine les colonnes de la pile, pas seulement un etat : au
   moins l'etat, le CPU, la RAM, le reseau et la latence.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PEINTRE = RACINE / "src/gui/apps/services.rs"
AIGUILLAGE = RACINE / "src/gui/apps/mod.rs"
PUBLICATEUR = RACINE / "src/gui/services.rs"
FENETRE = RACINE / "src/gui/window.rs"
FACADE = RACINE / "src/kernel/services/mod.rs"

SIGNATURE_DRAW = "pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {"
SIGNATURE_ATELIER = "pub fn avec_atelier<R>(travail: impl FnOnce(&mut Atelier) -> R) -> R {"
SIGNATURE_ACTION = "fn action_de(id: &str) -> Option<(&'static str, bool)> {"
SIGNATURE_BARRE = "fn peins_la_barre(bx: usize, by: usize, bw: usize, etat: &mut EtatVue) {"

# Les six lignes de la photo. Un peintre qui les nomme est retombe dans le
# defaut : ces noms appartiennent au publicateur.
NOMS_EN_DUR = [
    "BouchaudBrowserHost",
    "RequestServer",
    "WebContent",
    "ImageDecoder",
    "Compositor",
    "WebWorker",
]

# Les autres applications du bureau. Une Services bis porterait un de ces
# prefixes ; la liste est volontairement large.
SOSIES = ["ServicesV2", "Services2", "ServicesBis", "Observability", "Observabilite",
          "ServicesDebug", "ServicesNouveau"]


def code_seul(source):
    """Le code sans les commentaires.

    L'en-tete du peintre CITE la photo -- les six noms y figurent, c'est tout
    l'interet de l'explication. Une garde qui lit les commentaires
    s'accuserait elle-meme.
    """
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def corps(source, signature):
    """Le corps d'une fonction, jusqu'a la declaration suivante -- INDENTEE
    COMPRISE, sans quoi la regle se satisfait d'un jeton trouve plus bas."""
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:pub\(crate\) )?(?:const |static |fn |struct |impl |enum |mod )", reste)
    return reste[: fin.start()] if fin else reste


def lit(chemin, fautes):
    try:
        return chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        fautes.append("%s est illisible." % chemin.name)
        return None


def main():
    fautes = []

    brut_peintre = lit(PEINTRE, fautes)
    brut_aiguillage = lit(AIGUILLAGE, fautes)
    brut_publicateur = lit(PUBLICATEUR, fautes)
    brut_fenetre = lit(FENETRE, fautes)
    if fautes:
        for f in fautes:
            print("  - %s" % f)
        return 1

    peintre = code_seul(brut_peintre)
    aiguillage = code_seul(brut_aiguillage)
    publicateur = code_seul(brut_publicateur)
    fenetre = code_seul(brut_fenetre)

    # ------------------------------------------------------------------ 1
    #
    # Le bureau appelle CE peintre. Sans cette arete, tout le reste peut etre
    # parfait sans qu'un pixel change a l'ecran.
    if "App::Services" not in aiguillage or "services::draw(" not in aiguillage:
        fautes.append(
            "src/gui/apps/mod.rs n'aiguille plus `App::Services` vers "
            "`services::draw` : la fenetre visible n'est plus ce peintre."
        )
    if "KIND_SERVICES" not in fenetre or "App::Services" not in fenetre:
        fautes.append(
            "src/gui/window.rs ne cree plus de fenetre `App::Services` : "
            "l'icone du bureau n'ouvre plus rien."
        )

    corps_draw = corps(peintre, SIGNATURE_DRAW)
    if corps_draw is None:
        fautes.append(
            "`%s` est introuvable dans src/gui/apps/services.rs : le peintre "
            "visible a change de signature, et cette garde ne protege plus "
            "rien." % SIGNATURE_DRAW
        )
        corps_draw = ""

    # ------------------------------------------------------------------ 2
    #
    # L'instantane est la difference entre lire le registre et le LIRE SANS LE
    # BLOQUER. Le rendu prend des millisecondes ; le verrou tenu pendant ce
    # temps gele la carte reseau, le navigateur et l'ordonnanceur.
    par_atelier = "crate::kernel::services::avec_atelier(" in corps_draw
    if "crate::kernel::services::instantane(" not in corps_draw and not par_atelier:
        fautes.append(
            "`draw` ne prend pas d'instantane du registre "
            "(`instantane` ou `avec_atelier`) : soit il n'affiche pas le "
            "registre, soit il tient son verrou pendant la peinture."
        )

    # L'ATELIER N'EST PAS UN LAISSEZ-PASSER.
    #
    # Accepter `avec_atelier` sur son nom ferait de cette garde un tampon :
    # il suffirait d'appeler ainsi une fonction qui garde le verrou du
    # registre ouvert pendant la peinture. On verifie donc ce que
    # `avec_atelier` FAIT.
    if par_atelier:
        facade = code_seul(lit(FACADE, fautes) or "")
        corps_atelier = corps(facade, SIGNATURE_ATELIER)
        if corps_atelier is None:
            fautes.append(
                "`%s` est introuvable : la fenetre passe par un atelier que "
                "cette garde ne sait plus verifier." % SIGNATURE_ATELIER
            )
        else:
            if "instantane(" not in corps_atelier:
                fautes.append(
                    "`avec_atelier` ne prend pas d'instantane : la fenetre "
                    "peindrait un arbre fige, ou tiendrait le registre."
                )
            if "REGISTRE.lock()" in corps_atelier:
                fautes.append(
                    "`avec_atelier` prend le verrou du registre lui-meme : "
                    "il resterait tenu pendant la peinture, ce que tout ce "
                    "chemin existe pour eviter."
                )
        # Le tampon doit etre STATIQUE. Pose sur la pile, `SERVICES_MAX`
        # entrees et autant de lignes demandaient quatre-vingt-sept
        # kilo-octets a une pile noyau de trente mille : un debordement dans
        # le fil du compositeur, c'est-a-dire un ecran fige sans trace.
        if "static ATELIER" not in facade:
            fautes.append(
                "l'atelier des vues n'est plus un tampon statique : "
                "`SERVICES_MAX` entrees sur la pile noyau la font deborder."
            )
    for pose_sur_la_pile in ["[Entree::vide(); SERVICES_MAX]", "; SERVICES_MAX]"]:
        if pose_sur_la_pile in peintre:
            fautes.append(
                "src/gui/apps/services.rs pose un tableau de `SERVICES_MAX` "
                "elements sur la pile (« %s ») : la pile noyau deborde."
                % pose_sur_la_pile
            )
            break
    if "vue::lignes(" not in corps_draw:
        fautes.append(
            "`draw` ne passe pas par le modele visible (`vue::lignes`) : "
            "l'arbre affiche n'est plus celui que les tests verifient."
        )

    # Le verrou du registre ne se prend pas a la main dans le peintre.
    for interdit in ["REGISTRE.lock()", "registre().lock()", "crate::kernel::services::REGISTRE"]:
        if interdit in peintre:
            fautes.append(
                "src/gui/apps/services.rs prend le verrou du registre "
                "(`%s`) : la peinture doit travailler sur une copie." % interdit
            )

    # ------------------------------------------------------------------ 3
    #
    # LE DEFAUT DE LA PHOTO. Le peintre ne nomme personne.
    for nom in NOMS_EN_DUR:
        if nom in peintre:
            fautes.append(
                "src/gui/apps/services.rs cite « %s » dans son code : la "
                "fenetre est retombee sur une liste de noms en dur. Ces noms "
                "appartiennent au publicateur src/gui/services.rs." % nom
            )

    # Le publicateur, lui, DOIT garder la table : c'est lui qui traduit un
    # processus ring 3 en identifiant de service.
    manquants = [n for n in NOMS_EN_DUR if n not in publicateur]
    if manquants:
        fautes.append(
            "src/gui/services.rs ne publie plus %s vers le registre : "
            "les processus du navigateur disparaitraient de l'arbre."
            % ", ".join(manquants)
        )
    if "crate::kernel::services::" not in publicateur:
        fautes.append(
            "src/gui/services.rs ne publie rien dans le registre : il est "
            "redevenu un observateur prive au lieu d'une source."
        )

    # ------------------------------------------------------------------ 4
    #
    # Une seule application Services dans tout l'arbre.
    for chemin in sorted(RACINE.joinpath("src").rglob("*.rs")):
        try:
            pur = code_seul(chemin.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
        for sosie in SOSIES:
            if sosie in pur:
                fautes.append(
                    "%s introduit « %s » : il ne doit exister qu'UNE fenetre "
                    "Services, celle que l'utilisateur voit." % (chemin.name, sosie)
                )

    variantes = len(re.findall(r"\bServices\b(?:\s*\{)?\s*,", code_seul(brut_fenetre)))
    declarations = re.findall(r"^\s*Services\s*,\s*$", code_seul(brut_fenetre), flags=re.M)
    if len(declarations) > 1:
        fautes.append(
            "src/gui/window.rs declare %d variantes `Services` dans `App` : "
            "une seule fenetre Services doit exister." % len(declarations)
        )

    # ------------------------------------------------------------------ 5
    #
    # Les colonnes de la pile. Une fenetre qui n'affiche qu'un etat ne dit
    # toujours pas pourquoi une page ne charge pas.
    for colonne in ["etat", "cpu", "ram", "disque", "reseau", "latence", "erreurs", "raison"]:
        if ("cols.%s" % colonne) not in peintre:
            fautes.append(
                "src/gui/apps/services.rs n'affiche plus la colonne « %s » : "
                "la fenetre a perdu une mesure de la pile." % colonne
            )

    # ------------------------------------------------------------------ 6
    #
    # LES ACTIONS SONT CONTEXTUELLES.
    #
    # « Demarrer Ladybird / Arreter Ladybird » occupait la premiere ligne
    # d'une fenetre qui surveille l'ordonnanceur, la memoire, l'USB et la pile
    # reseau. Un moniteur systeme dont l'en-tete pilote une application est un
    # panneau d'application deguise -- c'est de la que cette fenetre vient, et
    # c'est la qu'elle retournerait sans une regle.
    corps_action = corps(peintre, SIGNATURE_ACTION)
    if corps_action is None:
        fautes.append(
            "`%s` est introuvable : l'action contextuelle a disparu, et rien "
            "n'empeche plus un bouton fixe en tete de fenetre."
            % SIGNATURE_ACTION
        )
    elif "starts_with" not in corps_action:
        fautes.append(
            "`action_de` ne regarde plus a QUI l'action s'applique : une "
            "action qui ne depend pas de la selection est un bouton fixe."
        )
    corps_barre = corps(peintre, SIGNATURE_BARRE)
    if corps_barre is None:
        fautes.append("`%s` est introuvable." % SIGNATURE_BARRE)
    elif "selection" not in corps_barre:
        fautes.append(
            "la barre de la fenetre Services dessine une action sans "
            "consulter la selection : les boutons Ladybird sont revenus en "
            "en-tete fixe."
        )

    # ------------------------------------------------------------------ 7
    #
    # UNE CELLULE VIDE SE TAIT, ET NE MENT PAS.
    #
    # Ni « N/A » repete cent fois -- une colonne entiere de « N/A » ne se lit
    # plus --, ni un zero, qui est une mesure.
    if '"N/A"' in peintre:
        fautes.append(
            "src/gui/apps/services.rs affiche encore « N/A » : une colonne "
            "entiere de « N/A » ne se lit plus, et le jour ou une vraie "
            "valeur y apparait l'oeil la saute aussi."
        )

    if fautes:
        print("FENETRE SERVICES : %d manquement(s)" % len(fautes))
        for f in fautes:
            print("  - %s" % f)
        return 1

    print("fenetre Services : registre -> instantane -> modele visible -> peinture.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
