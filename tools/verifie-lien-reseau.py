#!/usr/bin/env python3
"""Garde-fou : l'etat du lien se VEILLE, il ne se constate pas une fois.

# Le defaut, lu sur la machine de reference

Le releve du 12 septembre 2026 contient ces trois lignes, et rien apres :

    BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK mac=b0:41:6f:09:70:a1
    BOUCHAUD_TRIGKEY_RTL8168_LINK_DOWN
    net: lo actif ; eth0 initialisee, lien bas

La carte est reconnue et le pilote fonctionne. Le lien est bas AU MOMENT OU ON
REGARDE -- cinq secondes apres la mise sous tension, ce qui est court pour une
autonegociation cuivre gigabit et bien plus court que le temps de brancher un
cable. Apres quoi plus personne ne regardait : `demarre()` etait appele une
fois, son verdict etait definitif, et la machine restait hors ligne pour le
reste de la session.

Cote utilisateur, cela donne un navigateur qui repond « Unable to resolve
host » a toutes les pages, sur une machine dont la carte reseau marche.

# Ce qui est verifie

1. Un fil relit l'etat du lien a cadence bornee.
2. Il retente la configuration quand le lien MONTE.
3. Il redescend le verdict quand le lien TOMBE -- une configuration qui ne mene
   plus nulle part fait attendre chaque requete jusqu'a son echeance.
4. Il ne FABRIQUE aucune adresse : les 10.0.2.x sont une convention QEMU, et
   les inventer sur du materiel reel ferait passer « hors ligne » pour
   « configure ».
5. Il dort entre deux lectures.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
NET = RACINE / "src/net/mod.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
MAIN = RACINE / "src/main.rs"
WIDGETS = RACINE / "src/gui/widgets.rs"
V15 = RACINE / "src/gui/widgets_v15.rs"
RTL = RACINE / "src/drivers/network/rtl8168.rs"
CLIENT = RACINE / "src/gui/client.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


def corps(source, entete):
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


def main():
    fautes = []
    for chemin in (NET, STAGE2, MAIN, WIDGETS, V15, RTL, CLIENT):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    net = sans_commentaires(NET.read_text(encoding="utf-8"))
    veilleur = corps(net, "fn veilleur_de_lien()")
    if veilleur is None:
        fautes.append(
            "net/mod.rs : le veilleur de lien a disparu. Un cable branche "
            "apres le demarrage ne sera plus jamais vu, et la machine restera "
            "hors ligne pour toute la session."
        )
    else:
        if "link_up()" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne relit plus l'etat du lien."
            )
        if "sleep_ticks" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne dort plus entre deux lectures ; "
                "il brulerait un coeur pour lire un registre."
            )
        if "dhcp::negocie_avant(" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne retente plus la configuration "
                "quand le lien monte. Voir le lien monter sans rien en faire "
                "ne sert a rien."
            )
        if "Demarrage::LienBas" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne redescend plus le verdict quand "
                "le lien tombe. Chaque requete partirait alors dans le vide et "
                "attendrait son echeance."
            )
        # La regle qui compte : ne JAMAIS fabriquer une configuration QEMU sur
        # du materiel reel. `SansBail` n'est legitime que pour SLIRP.
        if "using_rtl8168()" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne distingue plus la carte physique "
                "de SLIRP. Sur RTL8168, un DHCP absent deviendrait la fausse "
                "configuration 10.0.2.x, et « hors ligne » passerait pour "
                "« configure »."
            )

    m = re.search(r"const PERIODE_LIEN_MS: u64 = ([0-9_]+);", net)
    if m is None:
        fautes.append("net/mod.rs : la periode de relecture du lien n'est plus lisible.")
    elif int(m.group(1).replace("_", "")) > 5_000:
        fautes.append(
            "net/mod.rs : le lien est relu moins d'une fois toutes les cinq "
            "secondes. Un cable branche se verrait avec un retard sensible."
        )

    if "PLAFOND_DHCP_MS" not in net or "saturating_mul(2)" not in net:
        fautes.append(
            "net/mod.rs : l'attente entre deux reprises DHCP n'augmente plus "
            "apres un echec. Un reseau cable sans serveur DHCP est une "
            "situation durable : la retenter toutes les dix secondes pendant "
            "des heures est du bruit."
        )

    m = re.search(r"const PERIODE_DHCP_MS: u64 = ([0-9_]+);", net)
    if m is None:
        fautes.append("net/mod.rs : la periode de reprise DHCP n'est plus lisible.")
    else:
        periode = int(m.group(1).replace("_", ""))
        if periode < 1_000:
            fautes.append(
                "net/mod.rs : les tentatives DHCP s'enchainent sans pause. "
                "`negocie` attend lui-meme plusieurs secondes : les relancer "
                "sans repit tiendrait le reseau occupe en permanence."
            )
        # ET SURTOUT PAS TROP LONG.
        #
        # Le releve du 13 septembre : lien monte a 23:59:02, configuration
        # obtenue a 00:00:05 -- trente et une secondes, parce que la premiere
        # reprise attendait dix secondes puis doublait. Le navigateur a ete
        # lance entre les deux et a garde le resolveur de QEMU pour toute sa
        # vie. Une premiere requete perdue juste apres une montee de lien est
        # NORMALE : le commutateur en face vient d'allumer son port.
        if periode > 3_000:
            fautes.append(
                "net/mod.rs : la premiere reprise DHCP attend plus de trois "
                "secondes apres une montee de lien. Le navigateur lance "
                "pendant ce creneau garde un resolveur inutilisable pour "
                "toute sa vie -- c'est ce qui est arrive le 13 septembre."
            )

    m = re.search(r"const BUDGET_DHCP_VEILLEUR_MS: u64 = ([0-9_]+);", net)
    if m is None:
        fautes.append(
            "net/mod.rs : le veilleur n'a plus de budget DHCP propre ; il "
            "reprendrait celui du demarrage, taille pour ne pas retarder le "
            "bureau et bien trop court pour un vrai serveur."
        )
    elif int(m.group(1).replace("_", "")) < 2_000:
        fautes.append(
            "net/mod.rs : le budget DHCP du veilleur est retombe sous deux "
            "secondes. Ce fil ne retarde rien : lui refuser le temps de "
            "recevoir une reponse n'economise que des echecs."
        )
    if "negocie_avant(BUDGET_DHCP_VEILLEUR_MS)" not in net:
        fautes.append(
            "net/mod.rs : le veilleur n'utilise plus son budget propre."
        )

    for chemin, nom in ((STAGE2, "stage2.rs"), (MAIN, "main.rs")):
        texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
        if "demarre_le_veilleur_de_lien()" not in texte:
            fautes.append(
                "%s : le veilleur de lien n'est plus lance sur ce chemin de "
                "demarrage." % nom
            )

    # --- L'INDICATEUR, ET CE QU'IL N'A PAS LE DROIT DE MONTRER ---------------
    #
    # Un indicateur reseau qui ment est pire qu'absent : il fait chercher la
    # panne ailleurs. Les trois regles qui suivent disent qu'il lit l'etat
    # COURANT, qu'il ne fabrique pas de nom, et que l'etat se lit autrement
    # que par la couleur.
    widgets = sans_commentaires(WIDGETS.read_text(encoding="utf-8"))
    if "fn dessine_reseau(" not in widgets:
        fautes.append(
            "widgets.rs : l'indicateur reseau a disparu de la barre du haut."
        )
    if "crate::net::connecte()" not in widgets:
        fautes.append(
            "widgets.rs : l'indicateur ne lit plus l'etat COURANT du lien. Le "
            "verdict de demarrage ne dit pas si le cable est branche a cet "
            "instant, et une icone verte sur un cable debranche fait chercher "
            "la panne ailleurs."
        )
    etat = corps(widgets, "pub fn libelle_reseau(")
    if etat is None:
        fautes.append("widgets.rs : le libelle de l'indicateur a disparu.")
    elif "nom_reseau()" not in etat:
        fautes.append(
            "widgets.rs : le libelle n'affiche plus le nom du reseau."
        )
    dessin = corps(widgets, "fn dessine_reseau(")
    if dessin is None or "if etat != EtatReseau::Connecte" not in dessin:
        fautes.append(
            "widgets.rs : l'etat deconnecte ne se distingue plus autrement que "
            "par la couleur. Un ecran mal regle, ou un daltonien, ne verrait "
            "aucune difference -- la barre oblique est ce qui rend l'etat "
            "lisible sans elle."
        )
    nom = corps(net, "pub fn nom_reseau()")
    if nom is None:
        fautes.append("net/mod.rs : `nom_reseau` a disparu.")
    else:
        if "NOM_RESEAU[..NOM_RESEAU_LEN]" not in nom:
            fautes.append(
                "net/mod.rs : `nom_reseau` ne rend plus le nom que le serveur "
                "DHCP a annonce ; il rend autre chose, et l'indicateur nomme "
                "un reseau que personne n'a nomme."
            )
        if 'String::from("' in nom:
            fautes.append(
                "net/mod.rs : `nom_reseau` rend un nom ECRIT DANS LE CODE. Un "
                "nom de reseau vient du serveur DHCP ou du sous-reseau, jamais "
                "d'une constante : une constante nomme un reseau que personne "
                "n'a identifie."
            )
        if "String::new()" not in nom:
            fautes.append(
                "net/mod.rs : `nom_reseau` FABRIQUE un nom quand il n'en "
                "connait aucun. Un nom invente vaut moins que rien : il fait "
                "croire a un reseau qu'on a identifie."
            )
        if "options::longueur_prefixe" not in nom:
            fautes.append(
                "net/mod.rs : le sous-reseau de repli n'est plus calcule par "
                "le module pur du client DHCP -- celui que la suite hote met a "
                "l'epreuve. Une seconde copie serait une seconde a corriger."
            )
    if veilleur is None or "oublie_identite_reseau()" not in veilleur:
        fautes.append(
            "net/mod.rs : le nom du reseau survit a la chute du lien. "
            "L'indicateur nommerait un reseau qu'on ne joint plus."
        )

    # --- LE PHY : LE LIRE NE SUFFIT PAS, IL FAUT LUI PARLER -----------------
    #
    # Le pilote LISAIT `PHYstatus` et n'ecrivait jamais dans le PHY. Sur la
    # machine de reference, brancher le cable APRES le demarrage ne montait
    # rien : le releve du 12 septembre 17:55 ne contient pas une seule ligne
    # `NET_LIEN etat=UP`, alors que l'utilisateur avait branche le RJ45.
    rtl = sans_commentaires(RTL.read_text(encoding="utf-8"))
    if "fn relance_autonegociation(" not in rtl:
        fautes.append(
            "rtl8168.rs : plus personne ne relance l'autonegociation. Le lien "
            "ne monte que si les deux extremites negocient : regarder le bit "
            "de lien sans rien demander au PHY, c'est attendre un evenement "
            "que personne ne declenche."
        )
    negociation = corps(rtl, "unsafe fn relance_autonegociation(")
    if negociation is not None:
        if "BMCR_VEILLE" not in negociation:
            fautes.append(
                "rtl8168.rs : le PHY n'est plus reveille avant la "
                "negociation. Un PHY en veille ne voit pas le cable, quoi "
                "qu'on lui annonce ensuite."
            )
        if "MII_BMCR" not in negociation:
            fautes.append(
                "rtl8168.rs : la relance n'ecrit plus dans BMCR ; rien ne "
                "declenche la negociation."
            )
    for nom, fonction, echec in (
        ("mdio_lit", "unsafe fn mdio_lit(", "None"),
        ("mdio_ecrit", "unsafe fn mdio_ecrit(", "false"),
    ):
        bloc = corps(rtl, fonction)
        if bloc is None:
            fautes.append("rtl8168.rs : `%s` a disparu." % nom)
            continue
        if "MDIO_TOURS" not in bloc:
            fautes.append(
                "rtl8168.rs : `%s` n'est plus bornee. Un controleur muet "
                "figerait le demarrage." % nom
            )
        # L'ECHEANCE DOIT ECHOUER, et non reussir en silence : une ecriture
        # MDIO qui se declare faite sans l'etre ferait croire a une
        # negociation lancee, et le lien ne monterait jamais sans que rien ne
        # le dise.
        dernier = [l.strip() for l in bloc.rstrip().rstrip("}").rstrip().splitlines() if l.strip()]
        if not dernier or dernier[-1] != echec:
            fautes.append(
                "rtl8168.rs : `%s` ne rend plus un echec quand son echeance "
                "expire. Une operation MDIO qui se declare faite sans l'etre "
                "ferait croire a une negociation lancee." % nom
            )
    if "e1000::reveille_le_lien()" not in net:
        fautes.append(
            "net/mod.rs : le veilleur ne relance plus la negociation quand le "
            "lien est bas. Un cable branche apres le demarrage ne serait "
            "jamais vu."
        )

    # --- L'INDICATEUR DIT LA VITESSE, ET DISTINGUE UN LIEN DEGRADE ----------
    if "qualite_lien()" not in widgets:
        fautes.append(
            "widgets.rs : l'indicateur n'affiche plus la qualite du lien. Un "
            "lien a l'alternat, ou negocie a dix megabits sur un port gigabit, "
            "fonctionne MAL et se lirait comme un lien sain."
        )
    if "COLOR_WARNING" not in widgets:
        fautes.append(
            "widgets.rs : un lien degrade est de nouveau peint comme un lien "
            "sain. La lenteur se chercherait ailleurs."
        )
    degrade = None
    for ligne in widgets.splitlines():
        if "let degrade" in ligne:
            degrade = ligne
            break
    bloc_degrade = ""
    if degrade is not None:
        debut = widgets.index(degrade)
        bloc_degrade = widgets[debut:debut + 400]
    if degrade is None \
            or "matches!(etat, EtatReseau::Connecte)" not in degrade \
            or "duplex_complet" not in bloc_degrade \
            or "vitesse_mbps" not in bloc_degrade:
        fautes.append(
            "widgets.rs : la degradation du lien ne se juge plus sur le "
            "duplex ET la vitesse. Ce sont les deux seules choses qu'un lien "
            "cuivre dit de sa qualite."
        )

    # --- UNE SEULE MISE EN PAGE POUR LA BARRE DU HAUT ----------------------
    #
    # La barre etait peinte a deux endroits, la seconde fois a une marge droite
    # ecrite en dur. La photo du 12 septembre montre le resultat :
    # « FPS: 0 necte 17:58:53 » -- la fin de « Deconnecte » sous le compteur.
    v15 = sans_commentaires(V15.read_text(encoding="utf-8"))
    for interdit, quoi in (
        ("fill_rect_rgb", "un rectangle de fond"),
        ("draw_text_prop", "du texte"),
        ("104", "une marge droite ecrite en dur"),
    ):
        if interdit in v15:
            fautes.append(
                "widgets_v15.rs : la facade repeint %s par-dessus la barre. "
                "Deux mises en page pour une seule barre, et tout element "
                "ajoute a droite finit dessous." % quoi
            )
    topbar = corps(widgets, "fn draw_topbar()")
    if topbar is None or "let mut droite" not in topbar \
            or topbar.count("droite = droite.saturating_sub") < 2:
        fautes.append(
            "widgets.rs : la barre du haut ne se remplit plus de la droite "
            "vers la gauche. Sans ce curseur, la largeur de chaque element "
            "redevient une constante, et un nom de reseau un peu long "
            "recouvre son voisin."
        )
    if topbar is not None and "frame_clock::snapshot()" not in topbar:
        fautes.append(
            "widgets.rs : le compteur de trames n'est plus dans la mise en "
            "page unique ; il reviendra se poser par-dessus."
        )

    # --- LE NAVIGATEUR DOIT RECEVOIR LE VRAI RESOLVEUR ---------------------
    #
    # Son hote a une valeur de repli ecrite en dur -- `10.0.2.3`, le resolveur
    # du NAT de QEMU -- et personne ne lui disait jamais autre chose. Chaque
    # releve physique contient « Setting DNS server to 10.0.2.3:53 » suivi de
    # « Unable to resolve host » pour toutes les pages.
    client = sans_commentaires(CLIENT.read_text(encoding="utf-8"))
    if "BOUCHAUD_DNS_SERVER=" not in client:
        fautes.append(
            "client.rs : le navigateur n'apprend plus quel resolveur utiliser. "
            "Il retomberait sur `10.0.2.3`, l'adresse du NAT de QEMU, qui ne "
            "mene nulle part sur une machine reelle."
        )
    else:
        debut = client.index("BOUCHAUD_DNS_SERVER=")
        if "dns_server()" not in client[debut:debut + 200]:
            fautes.append(
                "client.rs : le resolveur passe au navigateur n'est plus celui "
                "du noyau ; une constante y remplacerait le bail DHCP."
            )

    if fautes:
        print("lien reseau : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "lien reseau : veille bornee, reprise DHCP a la montee, verdict "
        "redescendu a la chute, aucune adresse QEMU fabriquee sur materiel "
        "reel, veilleur lance sur les deux chemins de demarrage, "
        "autonegociation relancee et bornee, indicateur lie a l'etat courant, "
        "sans nom invente, qualite du lien dite, et une seule mise en page "
        "pour la barre du haut"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
