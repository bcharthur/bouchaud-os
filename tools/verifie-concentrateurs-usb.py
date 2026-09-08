#!/usr/bin/env python3
"""Ce qui fait qu'un clavier branche derriere un concentrateur repond.

# Le defaut que ces regles ferment

La traversee d'un concentrateur est de l'arithmetique de champs de bits. Une
faute n'y produit AUCUN message d'erreur : le controleur adresse un
peripherique qui n'est pas la, la commande reussit -- ou reussit sur un autre
--, et le journal dit « adresse ok ».

C'est la pire forme de defaut : le code a l'air correct, la CI est verte, et
le clavier ne marche pas chez l'utilisateur. Ces regles portent donc sur les
quatre champs dont une valeur fausse ne se voit nulle part, et sur les deux
facons dont la traversee peut faire pire que ne pas marcher : deborder la pile
du noyau, ou ne jamais rendre la main.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
DECODAGE = RACINE / "src/drivers/usb/concentrateur/decodage.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
TEST = RACINE / "tools/platform/test_concentrateur.rs"
CAMPAGNE = RACINE / "tools/ci/run_usb_arbre.sh"
WORKFLOW = RACINE / ".github/workflows/integration.yml"


def sans_commentaires(source):
    sortie = []
    dans_chaine = False
    i = 0
    while i < len(source):
        c = source[i]
        if dans_chaine:
            if c == "\\":
                sortie.append("  ")
                i += 2
                continue
            if c == '"':
                dans_chaine = False
            sortie.append(c)
            i += 1
            continue
        if c == '"':
            dans_chaine = True
            sortie.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < len(source) and source[i + 1] == "/":
            while i < len(source) and source[i] != "\n":
                sortie.append(" ")
                i += 1
            continue
        sortie.append(c)
        i += 1
    return "".join(sortie)


def corps(source, signature):
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        i = trouve
        while i < len(source):
            c = source[i]
            if c in "([<":
                profondeur += 1
            elif c in ")]>":
                if profondeur > 0:
                    profondeur -= 1
            elif profondeur == 0:
                if c == "{":
                    ouvrante = i
                    break
                if c == ";":
                    declaration = i
                    break
            i += 1
        if ouvrante >= 0:
            break
        if declaration < 0:
            return None
        debut = declaration + 1
    profondeur = 0
    i = ouvrante
    while i < len(source):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
        i += 1
    return None


def regle_arithmetique_a_part(decodage, xhci, fautes):
    """Le calcul des champs ne touche aucun registre.

    C'est ce qui permet de l'eprouver sur l'hote, champ par champ. Un seul
    acces au materiel, et l'arithmetique redevient verifiable seulement en
    branchant un vrai concentrateur -- c'est-a-dire jamais.
    """
    for interdit in ("read_volatile", "write_volatile", "serial_println", "wait_ms"):
        if interdit in decodage:
            fautes.append(
                "concentrateur/decodage.rs : « %s » y est apparu. Le calcul "
                "des champs doit rester pur, sinon il redevient intestable."
                % interdit
            )
    # Et le pilote ne doit pas recomposer les champs sur place : deux copies
    # divergent, et l'une des deux n'est relue par personne.
    for fonction in ("fn route_enfant", "fn slot_dw0", "fn slot_dw1", "fn slot_dw2"):
        if fonction in xhci:
            fautes.append(
                "xhci_active.rs : « %s » y a ete recopie ; deux versions du "
                "meme calcul divergeront." % fonction
            )
    for appel in ("concentrateur::slot_dw0(", "concentrateur::slot_dw1(", "concentrateur::slot_dw2("):
        if appel not in xhci:
            fautes.append(
                "xhci_active.rs : le contexte de slot n'est plus construit par "
                "%s ; les champs sont reecrits sur place, hors de portee des "
                "preuves." % appel
            )


def regle_chaine_de_route(decodage, fautes):
    """Les deux pieges de la chaine de route."""
    bloc = corps(decodage, "pub const fn route_enfant(")
    if bloc is None:
        fautes.append("concentrateur/decodage.rs : route_enfant a disparu.")
        return
    # Saturer, et non masquer : masquer designerait le port 1 pour le port 17,
    # sur lequel il y a peut-etre un autre peripherique -- et l'adressage
    # REUSSIRAIT dessus.
    if "port > 15" not in bloc:
        fautes.append(
            "concentrateur/decodage.rs : un numero de port au-dela de quinze "
            "n'est plus sature ; masquer designerait un AUTRE port, et "
            "l'adressage reussirait sur le mauvais peripherique."
        )
    if re.search(r"port\s*&\s*0xf\b", bloc):
        fautes.append(
            "concentrateur/decodage.rs : le numero de port est de nouveau "
            "MASQUE au lieu d'etre sature."
        )
    # Le decalage suit la profondeur du parent : se tromper d'etage designe un
    # autre sous-arbre.
    if "4 * profondeur" not in bloc:
        fautes.append(
            "concentrateur/decodage.rs : l'etage n'est plus choisi par la "
            "profondeur ; la route designerait un autre sous-arbre."
        )
    if "profondeur >= ETAGES_MAX" not in bloc:
        fautes.append(
            "concentrateur/decodage.rs : la profondeur n'est plus bornee ; "
            "un sixieme etage deborderait le champ de vingt bits et ecraserait "
            "la vitesse dans le meme mot."
        )


def regle_bit_hub(decodage, xhci, fautes):
    """Sans le bit `Hub`, la traversee ne sert a rien.

    Le controleur refuse d'adresser quoi que ce soit derriere un slot dont ce
    bit n'est pas pose : il ne sait pas qu'il y a un « derriere ».
    """
    bloc = corps(decodage, "pub const fn slot_dw0(")
    if bloc is None:
        fautes.append("concentrateur/decodage.rs : slot_dw0 a disparu.")
    elif "1 << 26" not in bloc:
        fautes.append(
            "concentrateur/decodage.rs : le bit Hub n'est plus le vingt-"
            "sixieme ; le controleur refuserait d'adresser derriere le "
            "concentrateur, sans nommer la cause."
        )
    declaration = corps(xhci, "fn declare_concentrateur(")
    if declaration is None:
        fautes.append(
            "xhci_active.rs : le concentrateur n'est plus DECLARE au "
            "controleur ; le bit Hub n'est jamais pose, et rien n'est "
            "atteignable derriere lui."
        )
        return
    if "1 << 26" not in declaration:
        fautes.append(
            "xhci_active.rs : declare_concentrateur ne pose plus le bit Hub."
        )
    if "CMD_CONFIGURE_ENDPOINT" not in declaration:
        fautes.append(
            "xhci_active.rs : le bit Hub n'est plus transmis par une commande "
            "Configure Endpoint ; l'ecrire dans le contexte ne suffit pas, le "
            "controleur ne le relit pas de lui-meme."
        )
    # Partir du contexte de SORTIE : les champs qu'on ne change pas doivent
    # rester identiques, sans quoi on ecrase l'etat du slot avec des zeros.
    if "out_ctx_virt" not in declaration or "copy_nonoverlapping" not in declaration:
        fautes.append(
            "xhci_active.rs : declare_concentrateur ne repart plus du contexte "
            "de sortie ; les champs qu'on ne change pas seraient ecrases par "
            "des zeros."
        )


def regle_transactionneur(decodage, xhci, fautes):
    """Un peripherique lent derriere un concentrateur rapide.

    C'est le cas COURANT : la quasi-totalite des claviers filaires sont basse
    vitesse, et tout concentrateur moderne est haute vitesse. Sans ces deux
    champs, le peripherique est adresse et ne repond a rien.
    """
    bloc = corps(decodage, "pub const fn requiert_transactionneur(")
    if bloc is None:
        fautes.append("concentrateur/decodage.rs : requiert_transactionneur a disparu.")
    else:
        if "VITESSE_HAUTE" not in bloc:
            fautes.append(
                "concentrateur/decodage.rs : le besoin de transactionneur ne "
                "regarde plus la vitesse du concentrateur."
            )
        if "VITESSE_BASSE" not in bloc or "VITESSE_PLEINE" not in bloc:
            fautes.append(
                "concentrateur/decodage.rs : le besoin de transactionneur ne "
                "regarde plus la vitesse du peripherique ; un clavier basse "
                "vitesse serait adresse sans traducteur, et resterait muet."
            )
    traversee = corps(xhci, "fn traverse_concentrateur(")
    if traversee is None:
        fautes.append("xhci_active.rs : traverse_concentrateur a disparu.")
        return
    if "requiert_transactionneur(" not in traversee:
        fautes.append(
            "xhci_active.rs : la traversee ne pose plus de transactionneur ; "
            "un clavier basse vitesse derriere un concentrateur haute vitesse "
            "-- le cas courant -- serait adresse et resterait muet."
        )
    # L'HERITAGE : ne le calculer qu'au premier saut donnerait, plus bas, un
    # transactionneur nul.
    if "chemin.tt_slot != 0" not in traversee:
        fautes.append(
            "xhci_active.rs : le transactionneur ne s'herite plus d'un etage a "
            "l'autre ; un concentrateur pleine vitesse derriere un "
            "concentrateur haute vitesse perdrait le sien, et tout ce qui est "
            "derriere deviendrait muet."
        )


def regle_port_racine(xhci, fautes):
    """Le contexte de slot porte le port RACINE, pas le port intermediaire."""
    bloc = corps(xhci, "fn fill_slot_context(")
    if bloc is None:
        fautes.append("xhci_active.rs : fill_slot_context a disparu.")
        return
    if "chemin.port_racine" not in bloc:
        fautes.append(
            "xhci_active.rs : le contexte de slot ne porte plus le port "
            "RACINE ; y mettre le port du concentrateur intermediaire designe "
            "un autre sous-arbre du controleur."
        )
    if "chemin.route" not in bloc:
        fautes.append(
            "xhci_active.rs : le contexte de slot ne porte plus la chaine de "
            "route ; tout serait adresse comme s'il etait branche a la racine."
        )


def regle_traversee_bornee(xhci, fautes):
    """La traversee ne doit ni deborder la pile, ni bloquer le demarrage."""
    # A PLAT, et non en recursion : cinq etages de tampons de descripteurs sur
    # une pile de noyau, c'est un debordement qu'on ne diagnostique pas.
    traversee = corps(xhci, "fn traverse_concentrateur(")
    if traversee is None:
        return
    if "file.pousse(" not in traversee:
        fautes.append(
            "xhci_active.rs : la traversee n'empile plus les peripheriques "
            "trouves dans une file ; si elle s'appelle elle-meme, cinq etages "
            "de tampons de descripteurs deborderaient la pile du noyau -- une "
            "machine qui redemarre sans rien dire."
        )
    if "enumerate_device(" in traversee:
        fautes.append(
            "xhci_active.rs : la traversee enumere de nouveau EN PLACE ; la "
            "recursion revient, et avec elle le debordement de pile."
        )
    file = corps(xhci, "fn pousse(")
    if file is None or "CHEMINS_EN_ATTENTE" not in (file or ""):
        fautes.append(
            "xhci_active.rs : la file des peripheriques a enumerer n'est plus "
            "bornee."
        )
    elif "debordements" not in file:
        fautes.append(
            "xhci_active.rs : un debordement de la file n'est plus compte ; "
            "des peripheriques disparaitraient en silence."
        )
    # L'ATTENTE : un port qui ne sort jamais de reinitialisation -- un cable a
    # moitie enfonce -- ne doit pas suspendre le demarrage.
    reinit = corps(xhci, "fn reinitialise_port_concentrateur(")
    if reinit is None:
        fautes.append("xhci_active.rs : reinitialise_port_concentrateur a disparu.")
    else:
        if "REINITIALISATION_MAX_MS" not in reinit:
            fautes.append(
                "xhci_active.rs : l'attente de reinitialisation d'un port n'est "
                "plus bornee ; un cable a moitie enfonce suspendrait le "
                "demarrage de la machine."
            )
        if "restant" not in reinit:
            fautes.append(
                "xhci_active.rs : la boucle de reinitialisation n'a plus de "
                "compteur decroissant."
            )


def regle_echec_dit(xhci, fautes):
    """Un concentrateur qu'on n'a pas pu traverser SE DIT.

    Un clavier derriere lui est simplement absent : rien ne distingue « le
    concentrateur a echoue » de « il n'y a pas de clavier ».
    """
    for marqueur, quoi in (
        ("BOUCHAUD_USB_CONCENTRATEUR_TRAVERSE", "la traversee reussie"),
        ("BOUCHAUD_USB_CONCENTRATEUR_ECHEC", "une traversee qui echoue"),
        ("BOUCHAUD_USB_CONCENTRATEUR_TROP_PROFOND", "un sixieme etage"),
        ("BOUCHAUD_USB_ARBRE_TRONQUE", "une file pleine"),
        ("BOUCHAUD_HID_ABSENT_DERRIERE_CONCENTRATEUR", "un clavier introuvable"),
    ):
        if marqueur not in xhci:
            fautes.append(
                "xhci_active.rs : « %s » a disparu ; %s ne laisse plus de "
                "trace, et sur une machine sans console il n'y a rien d'autre."
                % (marqueur, quoi)
            )
    # Le compteur rendu au reste du noyau doit compter les ECHECS, et non les
    # concentrateurs trouves : depuis que la traversee marche, un concentrateur
    # trouve n'est plus un probleme.
    bloc = corps(xhci, "pub fn concentrateurs_non_traverses(")
    if bloc is None:
        fautes.append("xhci_active.rs : concentrateurs_non_traverses a disparu.")
    elif "CONCENTRATEURS_ECHOUES" not in bloc:
        fautes.append(
            "xhci_active.rs : « concentrateurs non traverses » compte de "
            "nouveau les concentrateurs TROUVES ; le repli PS/2 se declencherait "
            "sur une traversee qui a pourtant reussi."
        )


def regle_configure_avant_de_traverser(xhci, fautes):
    """Un concentrateur non configure se comporte comme un concentrateur vide.

    Il refuse les requetes de ses ports. Rien ne distingue les deux cas.
    """
    if corps(xhci, "fn valeur_de_configuration(") is None:
        fautes.append(
            "xhci_active.rs : la valeur de configuration n'est plus lue ; un "
            "concentrateur non configure refuse les requetes de ses ports et "
            "parait vide."
        )
    enumeration = corps(xhci, "fn enumerate_device(")
    if enumeration is None:
        fautes.append("xhci_active.rs : enumerate_device a disparu.")
        return
    # La regle doit porter sur la BRANCHE du concentrateur : `enumerate_device`
    # configure aussi les peripheriques ordinaires, et une regle qui regarde
    # toute la fonction resterait verte alors que le concentrateur, lui, ne
    # serait plus configure.
    branche = corps(enumeration, "if classe_peripherique == CLASSE_CONCENTRATEUR")
    if branche is None:
        fautes.append(
            "xhci_active.rs : la branche qui traite un concentrateur a "
            "disparu d'enumerate_device."
        )
        return
    if "set_configuration(" not in branche:
        fautes.append(
            "xhci_active.rs : le concentrateur n'est plus configure avant "
            "d'etre traverse ; il parait vide, et le journal ne distingue pas "
            "ce cas d'un concentrateur sans rien de branche."
        )


def regle_changements_effaces(xhci, fautes):
    """Un changement laisse pose est re-signale sans fin."""
    if corps(xhci, "fn efface_changements(") is None:
        fautes.append(
            "xhci_active.rs : les bits de changement d'un port ne sont plus "
            "effaces ; le concentrateur les repete, et la traversee rebranche "
            "indefiniment le meme peripherique."
        )


def regle_preuves(test, fautes):
    for attendu in (
        "chaque_etage_decale_de_quatre_bits",
        "un_port_au_dela_de_quinze_sature_au_lieu_de_reboucler",
        "au_dela_de_cinq_etages_le_champ_est_plein",
        "le_bit_hub_est_le_vingt_sixieme",
        "un_clavier_basse_vitesse_derriere_un_concentrateur_haute_vitesse_en_a_besoin",
        "un_concentrateur_usb2_code_la_vitesse_par_deux_bits_separes",
        "un_concentrateur_superspeed_place_l_alimentation_ailleurs",
        "la_vitesse_ne_veut_rien_dire_avant_que_le_port_soit_actif",
        "le_delai_d_alimentation_se_compte_en_unites_de_deux_millisecondes",
        "une_requete_de_port_est_adressee_au_port_et_non_au_concentrateur",
    ):
        if attendu not in test:
            fautes.append("test_concentrateur.rs : la preuve « %s » a disparu." % attendu)


def regle_campagne_reelle(fautes):
    """L'arithmetique juste ne prouve pas que l'ensemble atteint un clavier.

    Les preuves hote verifient chaque champ separement. Seule une machine
    complete -- un controleur, un concentrateur, un clavier derriere -- dit que
    l'ensemble marche.
    """
    if not CAMPAGNE.exists():
        fautes.append(
            "tools/ci/run_usb_arbre.sh : la campagne qui branche un clavier "
            "DERRIERE un concentrateur a disparu ; plus rien ne verifie que "
            "l'ensemble atteint un peripherique."
        )
        return
    texte = CAMPAGNE.read_text(encoding="utf-8")
    if "usb-hub" not in texte:
        fautes.append(
            "run_usb_arbre.sh : la topologie n'a plus de concentrateur ; la "
            "campagne ne verifie plus rien de la traversee."
        )
    # RIEN EN DIRECT : c'est ce qui rend le resultat concluant. Un clavier
    # branche a la racine passerait la campagne sans que la traversee marche.
    for peripherique in ("usb-kbd", "usb-mouse"):
        motif = re.search(r"-device %s,bus=([^\s\\]+)" % peripherique, texte)
        if motif is None:
            fautes.append(
                "run_usb_arbre.sh : « %s » a disparu de la topologie."
                % peripherique
            )
        elif "port=1." not in motif.group(1):
            fautes.append(
                "run_usb_arbre.sh : « %s » n'est plus branche DERRIERE le "
                "concentrateur ; branche en direct, il passerait la campagne "
                "sans que la traversee marche." % peripherique
            )
    for attendu in ("profondeur=1", "non_traverses=0", "claviers=[1-9]", "souris=[1-9]"):
        if attendu not in texte:
            fautes.append(
                "run_usb_arbre.sh : la verification « %s » a disparu." % attendu
            )
    if not WORKFLOW.exists():
        return
    workflow = WORKFLOW.read_text(encoding="utf-8")
    if "run_usb_arbre.sh" not in workflow:
        fautes.append(
            "integration.yml : la campagne de l'arbre USB n'est plus lancee ; "
            "elle existe et ne protege rien."
        )
    elif '"$USB"' not in workflow:
        fautes.append(
            "integration.yml : le resultat de l'arbre USB n'entre plus dans la "
            "barriere ; la campagne peut echouer et la fusion passer quand meme."
        )


def main():
    fautes = []
    for chemin in (DECODAGE, XHCI, TEST):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        print("concentrateurs usb : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    decodage = sans_commentaires(DECODAGE.read_text(encoding="utf-8"))
    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    test = TEST.read_text(encoding="utf-8")

    regle_arithmetique_a_part(decodage, xhci, fautes)
    regle_chaine_de_route(decodage, fautes)
    regle_bit_hub(decodage, xhci, fautes)
    regle_transactionneur(decodage, xhci, fautes)
    regle_port_racine(xhci, fautes)
    regle_traversee_bornee(xhci, fautes)
    regle_echec_dit(xhci, fautes)
    regle_configure_avant_de_traverser(xhci, fautes)
    regle_changements_effaces(xhci, fautes)
    regle_preuves(test, fautes)
    regle_campagne_reelle(fautes)

    if fautes:
        print("concentrateurs usb : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "concentrateurs usb : arithmetique pure et eprouvee champ par champ, "
        "route saturee et bornee a cinq etages, bit Hub declare par Configure "
        "Endpoint, transactionneur herite, traversee a plat et bornee, echecs "
        "nommes, et un clavier reellement branche derriere un concentrateur en "
        "integration"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
