#!/usr/bin/env python3
"""Garde-fou : le navigateur ne doit pas partir sans resolveur utilisable.

# Le defaut, releve le 16 septembre 2026

Le journal physique, a une seconde d'intervalle :

    17:00:38  BOUCHAUD_NET_LIEN etat=UP ancien_verdict=lien-bas
    17:00:38  net: eth0 lien UP 1000 Mb/s duplex complet
    17:00:39  BOUCHAUD_NAVIGATEUR_RESEAU dns=10.0.2.3 verdict=lien-bas
              lien=0 resolveur=NON-CONFIGURE
    17:00:39  Setting DNS server to 10.0.2.3:53

Deux fautes, et elles se cumulent.

La PREMIERE est le moment : le lancement ne dependait que du temps -- cinq
cents millisecondes apres la premiere trame du bureau. L'autonegociation
cuivre, elle, met environ trois secondes, et le lien n'est monte qu'a 6,5 s.
Le navigateur part a 6,8 s, soit avant tout bail DHCP.

La SECONDE est l'adresse : faute de bail, on transmettait la valeur compilee,
`10.0.2.3`, qui est le resolveur du NAT de QEMU. Sur la machine elle ne mene
nulle part. Le navigateur lit son resolveur UNE FOIS, a l'exec, et le garde
pour la vie : toute la session repond ensuite « Unable to resolve host », et
la panne parait venir du navigateur.

# Ce qui est verifie ici

1. Les deux decisions vivent dans des modules PURS, donc verifiables sur
   machine hote. Sur la machine elles ne se distinguent qu'a une seconde
   pres, une fois par demarrage.
2. La passerelle est un recours AVANT la valeur compilee. Elle existe
   reellement sur le reseau branche ; la valeur compilee, non.
3. Le lancement consulte l'etat du reseau, et pas seulement l'horloge.
4. L'attente reste BORNEE, et ne s'applique pas quand le lien est bas : un
   bail ne peut pas arriver sans cable, et la page d'accueil est locale.
5. Le journal dit D'OU vient l'adresse. « dns=10.0.2.3 » ne distinguait pas
   un bail d'un repli -- toute la difference entre un reseau qui repond et un
   reseau qu'on n'a pas attendu.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

RESOLVEUR = RACINE / "src/net/resolveur.rs"
DEPART = RACINE / "src/gui/demarrage_navigateur.rs"
CLIENT = RACINE / "src/gui/client.rs"
WM = RACINE / "src/gui/window_manager.rs"
TESTS = (
    RACINE / "tools/platform/test_resolveur.rs",
    RACINE / "tools/platform/test_demarrage_navigateur.rs",
)


def code_seul(source):
    """La source privee de ses commentaires.

    Les commentaires de ces fichiers CITENT le defaut, `10.0.2.3` compris.
    Une garde qui lit la prose se declencherait sur le recit de la panne
    qu'elle verifie corrigee.
    """
    sans_blocs = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans_blocs.splitlines())


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    resolveur = lit(RESOLVEUR, fautes)
    depart = lit(DEPART, fautes)
    client = lit(CLIENT, fautes)
    wm = lit(WM, fautes)

    # 1. La purete est ce qui rend les regles verifiables.
    for chemin, source in ((RESOLVEUR, resolveur), (DEPART, depart)):
        if source is None:
            continue
        nom = chemin.name
        code = code_seul(source)
        if "use crate::" in code:
            fautes.append(
                "%s depend du reste du noyau : il ne se compile plus seul avec "
                "`rustc --test`, et sa regle redevient une supposition." % nom
            )
        if "unsafe" in code:
            fautes.append("%s contient du `unsafe` ; cette decision est pure." % nom)

    # 2 et 4. Les regles elles-memes.
    if resolveur is not None:
        code = code_seul(resolveur)
        # LE CORPS DE `choisis`, ET RIEN D'AUTRE.
        #
        # La premiere version cherchait `Source::Passerelle` dans le fichier
        # entier. Or `nom()` contient `Source::Passerelle => "passerelle"` :
        # supprimer la branche de la DECISION laissait le jeton en place, et
        # la garde passait. Pire, l'ordre de preference etait mesure sur
        # l'ordre des bras de `nom()`, qui n'a rien a voir. Les deux mutations
        # correspondantes n'etaient pas attrapees.
        debut = code.find("pub fn choisis(")
        corps = code[debut:] if debut != -1 else ""
        if debut == -1:
            fautes.append("resolveur.rs : `choisis` a disparu.")
        if "Source::Passerelle" not in corps:
            fautes.append(
                "resolveur.rs : la passerelle n'est plus un recours dans "
                "`choisis`. Sans elle, une machine dont le bail tarde repart "
                "sur la valeur compilee -- le resolveur du NAT de QEMU, qui ne "
                "mene nulle part ici."
            )
        rangs = [
            corps.find("Source::Bail"),
            corps.find("Source::Passerelle"),
            corps.find("Source::Compile"),
        ]
        if -1 not in rangs and not (rangs[0] < rangs[1] < rangs[2]):
            fautes.append(
                "resolveur.rs : dans `choisis`, l'ordre de preference n'est "
                "plus bail, passerelle, compile. C'est cet ordre, et lui seul, "
                "qui evite de transmettre une adresse fausse quand une vraie "
                "existe."
            )
        for jeton, pourquoi in (
            ("127", "la boucle locale ne resout rien ici : la pointer fait "
                    "echouer chaque requete APRES un delai au lieu de tout de suite"),
            ("255", "la diffusion n'est pas un interlocuteur"),
            ("224", "le multicast non plus"),
        ):
            if jeton not in code:
                fautes.append("resolveur.rs : `%s` n'est plus ecarte -- %s." % (jeton, pourquoi))

    if depart is not None:
        code = code_seul(depart)
        if "ATTENTE_MAXIMALE_MS" not in code:
            fautes.append(
                "demarrage_navigateur.rs : l'attente n'est plus bornee. Un "
                "reseau sans serveur DHCP retiendrait le bureau indefiniment."
            )
        i_lien = code.find("if !lien")
        # ANCRER SUR LA COMPARAISON, PAS SUR LE NOM. `attente_maximale_ms`
        # apparait d'abord dans la LISTE DE PARAMETRES, donc toujours avant le
        # test du lien : la premiere version de cette garde se declenchait sur
        # un code parfaitement correct.
        i_delai = code.find(">= attente_maximale_ms")
        if i_lien == -1:
            fautes.append(
                "demarrage_navigateur.rs : le lien n'est plus consulte ; une "
                "machine sans cable attendrait un bail qui ne peut pas venir."
            )
        elif i_delai != -1 and i_lien > i_delai:
            fautes.append(
                "demarrage_navigateur.rs : le delai est teste AVANT le lien. "
                "Une machine hors reseau attendrait le delai complet, et le "
                "journal dirait « delai ecoule » la ou il n'y avait pas de cable."
            )

    # 3 et 5. Le cablage.
    if wm is not None and "demarrage_navigateur::decide(" not in code_seul(wm):
        fautes.append(
            "window_manager.rs : le lancement ne consulte plus l'etat du "
            "reseau. Il repartirait sur le seul critere du temps, qui est "
            "exactement ce qui a produit le defaut."
        )

    if client is not None:
        code = code_seul(client)
        if "resolveur::choisis(" not in code:
            fautes.append(
                "client.rs : le resolveur n'est plus choisi ; `dns_server()` "
                "seul rend la valeur compilee quand aucun bail n'est arrive."
            )
        if "BOUCHAUD_NAVIGATEUR_RESOLVEUR" not in code:
            fautes.append(
                "client.rs : le journal ne dit plus D'OU vient l'adresse. "
                "« dns=10.0.2.3 » ne distingue pas un bail d'un repli."
            )

    for chemin in TESTS:
        source = lit(chemin, fautes)
        if source is not None and source.count("#[test]") < 8:
            fautes.append(
                "%s : moins de huit cas pour une decision qui ne se verifie "
                "pas autrement." % chemin.name
            )

    if fautes:
        print("resolveur du navigateur : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "resolveur du navigateur : bail puis passerelle puis compile, depart "
        "conditionne au reseau, attente bornee, source journalisee."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
