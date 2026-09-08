#!/usr/bin/env python3
"""Ce que le clavier et la souris USB ne doivent jamais redevenir.

La table des touches est cent lignes de correspondances. Une entree fausse ne
fait rien planter : elle fait qu'UNE touche ne marche pas, sur une machine
physique, ou l'on ne peut la decouvrir qu'en appuyant dessus. Ces regles
protegent ce qui rend une telle faute possible : un decodage melange au
materiel, une table qu'aucun test ne relit, un manque qui ne se dit pas.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
DECODAGE = RACINE / "src/drivers/usb/hid/decodage.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
TEST = RACINE / "tools/platform/test_hid.rs"
WM = RACINE / "src/gui/window_manager.rs"


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


def regle_decodage_sans_materiel(decodage, fautes):
    """Le decodage ne touche rien.

    C'est ce qui permet de l'eprouver sur l'hote. Un seul acces au materiel, et
    la table redevient intestable -- donc invisible jusqu'a ce qu'un
    utilisateur appuie sur la touche qui manque.
    """
    for interdit in (
        "read_volatile",
        "write_volatile",
        "ring_doorbell",
        "crate::drivers::keyboard",
        "crate::drivers::mouse",
        "serial_println",
    ):
        if interdit in decodage:
            fautes.append(
                "hid/decodage.rs : « %s » y est apparu. Le decodage doit "
                "rester pur, sinon il redevient intestable." % interdit
            )


def regle_table_complete(decodage, fautes):
    """Les touches qu'un clavier francais a et qu'un clavier americain n'a pas.

    L'usage 0x64 est la touche « inferieur / superieur », a gauche du W. Sans
    elle, elle ne fait RIEN -- et c'est un manque qu'on ne decouvre qu'en
    appuyant dessus, apres la livraison.
    """
    bloc = corps(decodage, "pub fn usage_ps2(")
    if bloc is None:
        fautes.append("hid/decodage.rs : la table des touches a disparu.")
        return
    for usage, quoi in (
        ("0x64", "la touche « inferieur / superieur » d'un clavier francais"),
        ("0x65", "la touche « menu contextuel »"),
        ("0x39", "le verrouillage des majuscules"),
        ("0x4f", "la fleche droite"),
    ):
        if usage not in bloc.lower():
            fautes.append(
                "hid/decodage.rs : l'usage %s (%s) n'est plus traduit." % (usage, quoi)
            )
    # Les touches qui demandent une SEQUENCE ne doivent pas etre traduites par
    # un code unique : ce serait faux, et faux en silence.
    for usage in ("0x46", "0x48"):
        if re.search(r"\b%s\s*=>" % usage, bloc):
            fautes.append(
                "hid/decodage.rs : l'usage %s demande une SEQUENCE de codes ; "
                "le traduire par un seul est faux." % usage
            )
    modificateurs = corps(decodage, "pub fn modificateur_ps2(")
    if modificateurs is None:
        fautes.append("hid/decodage.rs : la table des modificateurs a disparu.")
    else:
        # AltGr et Alt gauche partagent 0x38 : seul le prefixe les separe, et
        # sur un clavier francais AltGr sert a taper @, #, [, ], { et }.
        if "(0x38, true)" not in modificateurs:
            fautes.append(
                "hid/decodage.rs : AltGr ne porte plus son prefixe etendu ; il "
                "serait confondu avec Alt gauche, et @ deviendrait "
                "intapable sur un clavier francais."
            )


def regle_difference_de_rapports(decodage, fautes):
    """Un rapport dit ce qui est enfonce, pas ce qui vient d'etre appuye."""
    bloc = corps(decodage, "pub fn evenements_clavier(")
    if bloc is None:
        fautes.append("hid/decodage.rs : evenements_clavier introuvable.")
        return
    if "etat.touches.contains" not in bloc:
        fautes.append(
            "hid/decodage.rs : les appuis ne sont plus la DIFFERENCE avec le "
            "rapport precedent ; chaque caractere serait double."
        )
    if "!touches.contains" not in bloc:
        fautes.append(
            "hid/decodage.rs : une touche qui disparait n'est plus relachee ; "
            "un modificateur reste enfonce et transforme tout ce qui suit."
        )
    # Les relachements avant les appuis.
    relachement = bloc.find("appui: false }")
    appui = bloc.find("appui: true }", bloc.find("for touche in touches"))
    if relachement < 0 or appui < 0 or relachement > appui:
        fautes.append(
            "hid/decodage.rs : les appuis sortent avant les relachements ; "
            "quand une touche en remplace une autre, l'application verrait "
            "les deux enfoncees en meme temps."
        )


def regle_identifiant_de_rapport(decodage, fautes):
    """Un identifiant de rapport decale tout d'un octet."""
    bloc = corps(decodage, "pub fn charge_utile")
    if bloc is None:
        fautes.append("hid/decodage.rs : le retrait de l'identifiant a disparu.")
        return
    if "donnees[1..]" not in bloc:
        fautes.append(
            "hid/decodage.rs : l'identifiant de rapport n'est plus retire ; "
            "les modificateurs deviendraient le premier code de touche."
        )
    if "return None" not in bloc and "None" not in bloc:
        fautes.append(
            "hid/decodage.rs : un rapport destine a un autre identifiant n'est "
            "plus ignore ; il serait decode comme des touches au hasard."
        )
    for nom in ("pub fn evenements_clavier(", "pub fn decode_souris("):
        appelant = corps(decodage, nom)
        if appelant is None or "charge_utile(" not in appelant:
            fautes.append("hid/decodage.rs : %s ignore l'identifiant de rapport." % nom)


def regle_souris(decodage, fautes):
    bloc = corps(decodage, "pub fn decode_souris(")
    if bloc is None:
        fautes.append("hid/decodage.rs : decode_souris introuvable.")
        return
    if "as i8" not in bloc:
        fautes.append(
            "hid/decodage.rs : les deplacements de souris ne sont plus signes ; "
            "le pointeur ne partirait que dans un sens, et vite."
        )
    if "donnees.len() >= 4" not in bloc:
        fautes.append(
            "hid/decodage.rs : la molette n'est plus optionnelle ; lire un "
            "quatrieme octet qui n'existe pas ferait defiler au hasard."
        )
    if "& 0x07" not in bloc:
        fautes.append(
            "hid/decodage.rs : les boutons lateraux ne sont plus masques ; ils "
            "seraient pris pour un clic du milieu."
        )


def regle_xhci_delegue(xhci, fautes):
    """Le pilote ne refait pas le decodage."""
    for nom in ("fn process_keyboard_report(", "fn process_mouse_report("):
        bloc = corps(xhci, nom)
        if bloc is None:
            fautes.append("xhci_active.rs : %s introuvable." % nom)
            continue
        # L'appel NOMME, et pas la simple mention du module : `hid::Evenement`
        # suffisait a satisfaire la regle pendant que le decodage etait
        # recopie sur place.
        attendu = (
            "hid::evenements_clavier("
            if "keyboard" in nom
            else "hid::decode_souris("
        )
        if attendu not in bloc:
            fautes.append(
                "xhci_active.rs : %s n'appelle plus %s ; le decodage a ete "
                "repris sur place, et il redevient intestable."
                % (nom, attendu)
            )
    if "fn usage_ps2" in xhci:
        fautes.append(
            "xhci_active.rs : une seconde table des touches y est reapparue. "
            "Deux tables divergent, et l'une des deux n'est relue par personne."
        )


def regle_concentrateur_dit(xhci, fautes):
    """Un concentrateur non traverse se DIT.

    Un clavier branche derriere ne repond pas. Sans ce message, le journal ne
    dit rien et on cherche le defaut dans le code du clavier, qui marche.
    """
    if "CLASSE_CONCENTRATEUR" not in xhci:
        fautes.append(
            "xhci_active.rs : un concentrateur n'est plus reconnu ; les "
            "peripheriques qui sont derriere disparaissent en silence."
        )
    if "BOUCHAUD_USB_CONCENTRATEUR" not in xhci:
        fautes.append(
            "xhci_active.rs : le journal ne signale plus un concentrateur non "
            "traverse."
        )
    if "BOUCHAUD_HID_ABSENT_DERRIERE_CONCENTRATEUR" not in xhci:
        fautes.append(
            "xhci_active.rs : un clavier absent alors qu'un concentrateur est "
            "branche ne nomme plus sa cause."
        )


def regle_reveil_pendant_la_scrutation(wm, fautes):
    """Le bureau ne dort pas indefiniment quand l'entree est scrutee.

    Sans interruption xHCI, une frappe ne signale rien : c'est la scrutation
    qui la decouvre. Un bureau endormi sans echeance ne se reveillerait donc
    jamais, et le clavier paraitrait mort.
    """
    if "xhci_active::poll()" not in wm:
        fautes.append(
            "window_manager.rs : la scrutation HID a disparu de la boucle du "
            "bureau ; plus aucune touche USB n'arriverait."
        )
    # La regle doit porter sur le CALCUL DE L'ECHEANCE, pas sur la simple
    # presence du mot ailleurs dans le fichier : `hid_polling` peut y figurer
    # pour tout autre chose, et une regle par presence resterait verte.
    echeance = re.search(
        r"match politique::prochaine_echeance\(&etat\) \{(.*?)\n        \}",
        wm,
        re.S,
    )
    if echeance is None:
        fautes.append(
            "window_manager.rs : la decision de sommeil du bureau est "
            "introuvable ; la regle ne peut plus rien dire."
        )
    elif "hid_polling()" not in echeance.group(1):
        fautes.append(
            "window_manager.rs : le sommeil du bureau n'est plus borne quand "
            "l'entree est scrutee ; une frappe ne reveillerait rien, et le "
            "clavier paraitrait mort."
        )


def regle_preuves(test, fautes):
    for attendu in (
        "les_vingt_six_lettres_sont_justes",
        "la_touche_a_gauche_du_w_d_un_clavier_francais_existe",
        "altgr_se_distingue_de_alt_gauche",
        "le_pave_de_navigation_est_etendu",
        "un_rapport_dit_ce_qui_est_enfonce_pas_ce_qui_vient_d_etre_appuye",
        "les_relachements_sortent_avant_les_appuis",
        "un_identifiant_de_rapport_decale_tout_d_un_octet",
    ):
        if attendu not in test:
            fautes.append("test_hid.rs : la preuve « %s » a disparu." % attendu)


def main():
    fautes = []
    for chemin in (DECODAGE, XHCI, TEST, WM):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    decodage = sans_commentaires(DECODAGE.read_text(encoding="utf-8"))
    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    test = TEST.read_text(encoding="utf-8")
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))

    regle_decodage_sans_materiel(decodage, fautes)
    regle_table_complete(decodage, fautes)
    regle_difference_de_rapports(decodage, fautes)
    regle_identifiant_de_rapport(decodage, fautes)
    regle_souris(decodage, fautes)
    regle_xhci_delegue(xhci, fautes)
    regle_concentrateur_dit(xhci, fautes)
    regle_reveil_pendant_la_scrutation(wm, fautes)
    regle_preuves(test, fautes)

    if fautes:
        print("hid usb : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "hid usb : decodage pur et eprouve touche par touche, AltGr distinct "
        "d'Alt, relachements avant appuis, identifiant de rapport retire, "
        "concentrateur non traverse annonce"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
