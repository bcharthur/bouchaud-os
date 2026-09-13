#!/usr/bin/env python3
"""Brancher le clavier APRES avoir allume la machine.

# Le defaut que ces regles ferment

C'est le premier geste de quiconque allume un ordinateur : il demarre, puis on
branche le clavier. Toutes les campagnes du depot branchent leurs
peripheriques AVANT le demarrage, donc aucune ne couvrait ce cas -- et une
enumeration qui n'a lieu qu'au demarrage rend l'OS inutilisable sans que rien
ne le signale.

Trois choses le rendent possible, et chacune peut etre defaite sans que rien
ne devienne rouge : surveiller les ports meme sans peripherique, ne pas perdre
les rapports HID pendant l'enumeration, et rendre ce qu'un debranchement
libere.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
WM = RACINE / "src/gui/window_manager.rs"
CAMPAGNE = RACINE / "tools/ci/run_usb_branchement.sh"
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


def regle_surveillance_sans_hid(xhci, wm, fautes):
    """La surveillance ne peut pas dependre de la presence d'un HID.

    « Un clavier USB a repondu » est exactement l'INVERSE du cas qui compte :
    demarrer sans clavier, puis en brancher un. Gater la surveillance sur
    `hid_polling()` la rend inoperante precisement quand on en a besoin.
    """
    bloc = corps(xhci, "pub fn poll()")
    if bloc is None:
        fautes.append("xhci_active.rs : poll() a disparu.")
        return
    # LA CONDITION, PAS LA MISE EN PAGE.
    #
    # La forme precedente exigeait `{ return;` colle a l'accolade. Elle est
    # devenue rouge le jour ou une ligne utile -- la scrutation de
    # l'enregistreur de vol -- s'est glissee avant le `return`, alors que la
    # propriete verifiee, elle, n'avait pas bouge d'un caractere.
    #
    # Un garde-fou qui accuse une mise en page apprend surtout a etre ignore.
    # On cherche donc le premier `if` dont le BLOC contient un `return;`, et on
    # regarde sa CONDITION.
    sortie = None
    for candidat in re.finditer(r"if\s+([^{\n]*?)\s*\{", bloc):
        debut = bloc.index("{", candidat.start())
        profondeur = 0
        for i in range(debut, len(bloc)):
            if bloc[i] == "{":
                profondeur += 1
            elif bloc[i] == "}":
                profondeur -= 1
                if profondeur == 0:
                    if "return;" in bloc[debut:i]:
                        sortie = candidat
                    break
        if sortie is not None:
            break
    if sortie is None:
        fautes.append(
            "xhci_active.rs : la condition de sortie de poll() est "
            "introuvable ; la regle ne peut plus rien dire."
        )
    elif "surveille_branchements()" not in sortie.group(1):
        fautes.append(
            "xhci_active.rs : poll() abandonne de nouveau des qu'aucun HID ne "
            "repond ; brancher un clavier apres le demarrage ne serait jamais "
            "remarque -- le cas meme qu'on veut couvrir."
        )
    if corps(xhci, "pub fn surveille_branchements()") is None:
        fautes.append(
            "xhci_active.rs : surveille_branchements() a disparu."
        )
    # Et le bureau doit BORNER son sommeil, sinon personne n'appelle poll().
    boucle = corps(wm, "fn boucle(")
    if boucle is None or "surveille_branchements()" not in boucle:
        fautes.append(
            "window_manager.rs : le sommeil du bureau n'est plus borne quand "
            "un controleur attend un peripherique ; il dormirait jusqu'a "
            "trente secondes et le clavier paraitrait mort tout ce temps."
        )


def regle_rapports_non_perdus(xhci, fautes):
    """Un rapport HID qui arrive pendant un transfert de controle est une FRAPPE.

    L'anneau d'evenements est unique. `wait_event` attend un evenement precis ;
    tout ce qu'il jette est perdu, et personne ne le voit. Au demarrage le
    croisement etait evite en armant les extremites plus tard -- le branchement
    a chaud rend le croisement NORMAL, et cette parade ne marche plus.
    """
    # LA REGLE SUIT LE CORPS, PAS LE NOM.
    #
    # `wait_event` n'est plus qu'une enveloppe qui fixe le budget par defaut :
    # l'attente elle-meme vit dans `wait_event_budget`, depuis que le chemin de
    # scrutation a recu son propre budget court. Chercher l'invariant dans
    # l'enveloppe le declarait absent alors qu'il avait seulement demenage --
    # et une regle qui accuse a tort finit par etre eteinte.
    bloc = corps(xhci, "fn wait_event_budget(")
    if bloc is None:
        bloc = corps(xhci, "fn wait_event(")
    if bloc is None:
        fautes.append("xhci_active.rs : l'attente d'evenement a disparu.")
        return
    if "differe(controller, event)" not in bloc:
        fautes.append(
            "xhci_active.rs : wait_event jette de nouveau les evenements qui "
            "ne lui etaient pas destines ; un rapport de clavier arrivant "
            "pendant un transfert de controle est une frappe perdue, en "
            "silence."
        )
    if "EVT_TRANSFER" not in bloc:
        fautes.append(
            "xhci_active.rs : wait_event ne distingue plus un achevement de "
            "transfert du reste ; il mettrait de cote n'importe quoi."
        )
    differe = corps(xhci, "fn differe(")
    if differe is None:
        fautes.append("xhci_active.rs : le tampon d'evenements differes a disparu.")
    else:
        if "EVENEMENTS_DIFFERES" not in differe:
            fautes.append(
                "xhci_active.rs : le tampon d'evenements differes n'est plus "
                "borne."
            )
        if "EVENEMENTS_PERDUS" not in differe:
            fautes.append(
                "xhci_active.rs : un debordement du tampon ne se compte plus ; "
                "des frappes disparaitraient sans laisser de trace."
            )
    poll = corps(xhci, "pub fn poll()")
    if poll is None:
        return
    vidage = corps(xhci, "fn traite_differes(")
    if vidage is None:
        fautes.append(
            "xhci_active.rs : `traite_differes` a disparu. Les evenements mis "
            "de cote s'accumuleraient jusqu'a saturer le tampon, et toutes "
            "les frappes suivantes seraient perdues."
        )
    else:
        if "differes_len = 0" not in vidage:
            fautes.append(
                "xhci_active.rs : `traite_differes` ne VIDE plus la file ; "
                "chaque tour rejouerait les memes rapports."
            )
        if "process_hid_event" not in vidage:
            fautes.append(
                "xhci_active.rs : `traite_differes` ne traite plus les "
                "evenements mis de cote, il se contente de les jeter."
            )
        # L'ORDRE D'ARRIVEE, dans le vidage lui-meme : deux frappes traitees a
        # l'envers, c'est une touche relachee avant d'etre appuyee, donc une
        # touche qui reste enfoncee.
        if "for index in 0..differes" not in vidage:
            fautes.append(
                "xhci_active.rs : `traite_differes` ne parcourt plus la file "
                "du plus ancien au plus recent ; l'ordre des frappes "
                "s'inverse, et une touche relachee avant d'etre appuyee reste "
                "enfoncee."
            )
    if "traite_differes(controller)" not in poll:
        fautes.append(
            "xhci_active.rs : poll() ne vide plus les evenements mis de cote ; "
            "ils s'accumuleraient jusqu'a saturer le tampon, et toutes les "
            "frappes suivantes seraient perdues."
        )
    else:
        # L'ORDRE : les differes sont ARRIVES AVANT ceux de l'anneau. Les
        # traiter apres inverserait les frappes.
        avant = poll.find("traite_differes(controller)")
        apres = poll.find("next_event(controller)")
        if avant < 0 or apres < 0 or avant > apres:
            fautes.append(
                "xhci_active.rs : les evenements mis de cote sont traites "
                "APRES ceux de l'anneau ; l'ordre des frappes s'inverse, et "
                "une touche relachee avant d'etre appuyee reste enfoncee."
            )


def regle_enumeration_hors_boucle(xhci, fautes):
    """Enumerer depuis la boucle des evenements reentrerait dedans.

    Enumerer emet des transferts de controle, qui attendent leurs propres
    evenements sur l'anneau qu'on est en train de drainer.
    """
    poll = corps(xhci, "pub fn poll()")
    if poll is None:
        return
    if "ports_a_traiter" not in poll:
        fautes.append(
            "xhci_active.rs : poll() ne traite plus les changements de port ; "
            "le branchement a chaud n'a plus lieu."
        )
    if "BRANCHEMENTS_PAR_TOUR" not in poll:
        fautes.append(
            "xhci_active.rs : le nombre de branchements traites par tour n'est "
            "plus borne ; le bureau appelle poll() depuis sa boucle de dessin, "
            "et enumerer plusieurs peripheriques d'affilee ferait sauter "
            "l'image."
        )
    # La notation pendant le drainage, le traitement apres.
    drainage = poll.find("next_event(controller)")
    traitement = poll.find("traite_port_change(")
    if drainage < 0 or traitement < 0 or traitement < drainage:
        fautes.append(
            "xhci_active.rs : l'enumeration a chaud a lieu PENDANT le drainage "
            "de l'anneau ; les transferts de controle qu'elle emet "
            "reentreraient dans la boucle qui les draine."
        )
    if corps(xhci, "fn traite_port_change(") is None:
        fautes.append("xhci_active.rs : traite_port_change a disparu.")


def regle_scrutation_en_plus_des_evenements(xhci, fautes):
    """Ne pas dependre des seuls evenements de l'anneau.

    Les interruptions sont desactivees, et un controleur avare peut ne jamais
    poster l'evenement. Relire les bits de changement coute une lecture par
    port.
    """
    releve = corps(xhci, "fn releve_ports_changes(")
    if releve is None:
        fautes.append(
            "xhci_active.rs : les ports ne sont plus relus directement ; on "
            "dependrait des seuls evenements de l'anneau, qu'un controleur "
            "avare peut ne jamais poster."
        )
    elif "PORTSC_CHANGE_BITS" not in releve:
        fautes.append(
            "xhci_active.rs : le releve des ports ne regarde plus les bits de "
            "changement."
        )
    if "PERIODE_SCRUTATION_PORTS_MS" not in xhci:
        fautes.append(
            "xhci_active.rs : la periode de relecture des ports a disparu ; "
            "relire a chaque tour de scrutation HID couterait huit lectures de "
            "registre toutes les deux millisecondes."
        )


def regle_debranchement_rend_tout(xhci, fautes):
    """Un debranchement rend le slot ET la memoire.

    Tant que l'enumeration n'avait lieu qu'au demarrage, ne rien rendre ne
    coutait rien. Brancher et debrancher fait de la meme omission une fuite :
    quatre pages plus deux anneaux par branchement, et une arene DMA epuisee ne
    se manifeste que par des enumerations qui echouent.
    """
    retire = corps(xhci, "fn retire_slot(")
    if retire is None:
        fautes.append(
            "xhci_active.rs : rien ne retire un peripherique debranche ; son "
            "extremite reste ARMEE, sa cloche sonne pour un peripherique "
            "absent, et son slot n'est jamais rendu."
        )
        return
    if "disable_slot(" not in retire:
        fautes.append(
            "xhci_active.rs : le slot d'un peripherique debranche n'est plus "
            "rendu au controleur ; une dizaine de branchements et il n'y a "
            "plus de slot libre."
        )
    if "libere_device(" not in retire:
        fautes.append(
            "xhci_active.rs : la memoire DMA d'un peripherique debranche n'est "
            "plus rendue ; l'arene se vide branchement apres branchement."
        )
    if "free_dma(" not in retire:
        fautes.append(
            "xhci_active.rs : les anneaux et tampons des extremites HID ne "
            "sont plus rendus ; ils sont alloues PAR EXTREMITE, et un clavier "
            "composite en a deux."
        )
    # L'ORDRE : rendre la memoire avant le slot laisserait le controleur ecrire
    # dans des contextes rendus a l'arene.
    slot = retire.find("disable_slot(")
    memoire = retire.find("libere_device(")
    if slot >= 0 and memoire >= 0 and memoire < slot:
        fautes.append(
            "xhci_active.rs : la memoire est rendue AVANT que le slot ne soit "
            "desactive ; le controleur peut encore ecrire dans des contextes "
            "qui appartiennent deja a quelqu'un d'autre."
        )
    # Le tableau doit etre COMPACTE : `hid_count` borne les boucles, et une
    # entree morte au milieu ferait sauter celles d'apres.
    if "hid_count = dernier" not in retire:
        fautes.append(
            "xhci_active.rs : le tableau des extremites n'est plus compacte ; "
            "une entree morte au milieu ferait sauter toutes celles d'apres."
        )
    # Un concentrateur debranche emporte ce qui etait derriere lui, et le
    # controleur ne signale que le port racine.
    sous_arbre = corps(xhci, "fn retire_sous_arbre(")
    if sous_arbre is None:
        fautes.append(
            "xhci_active.rs : debrancher un concentrateur ne retire plus ce "
            "qui etait branche dessus ; le controleur ne signale que le port "
            "racine, et ses enfants resteraient armes sur un chemin qui "
            "n'existe plus."
        )
    elif "port_racine == port" not in sous_arbre:
        fautes.append(
            "xhci_active.rs : le retrait d'un sous-arbre ne suit plus le port "
            "racine."
        )


def regle_campagne_reelle(fautes):
    """Une campagne qui branche AVANT le demarrage ne prouve rien d'ici."""
    if not CAMPAGNE.exists():
        fautes.append(
            "tools/ci/run_usb_branchement.sh : la campagne du branchement a "
            "chaud a disparu."
        )
        return
    texte = CAMPAGNE.read_text(encoding="utf-8")
    # La COMMANDE, et non la simple mention du mot : un message d'erreur qui
    # nomme `device_add` suffisait a satisfaire la regle pendant que la
    # commande envoyee etait autre chose.
    commande = re.search(r'"execute"\s*:\s*"device_add"', texte)
    if commande is None or "qmp" not in texte.lower():
        fautes.append(
            "run_usb_branchement.sh : le peripherique n'est plus ajoute A "
            "CHAUD par le moniteur ; la campagne prouverait l'enumeration au "
            "demarrage, qui est deja couverte ailleurs."
        )
    # RIEN AU DEMARRAGE : c'est ce qui rend le resultat concluant.
    for peripherique in ("usb-kbd", "usb-mouse"):
        if re.search(r"-device %s" % peripherique, texte):
            fautes.append(
                "run_usb_branchement.sh : « %s » est de nouveau branche au "
                "DEMARRAGE ; la campagne passerait sans que le branchement a "
                "chaud marche." % peripherique
            )
    if "peripheriques=0" not in texte:
        fautes.append(
            "run_usb_branchement.sh : la campagne ne verifie plus que la "
            "machine a demarre sans aucun peripherique USB."
        )
    for attendu in ("BOUCHAUD_USB_BRANCHEMENT ", "claviers=[1-9]", "souris=[1-9]", "evenements_perdus"):
        if attendu not in texte:
            fautes.append(
                "run_usb_branchement.sh : la verification « %s » a disparu."
                % attendu
            )
    if not WORKFLOW.exists():
        return
    workflow = WORKFLOW.read_text(encoding="utf-8")
    if "run_usb_branchement.sh" not in workflow:
        fautes.append(
            "integration.yml : la campagne du branchement a chaud n'est plus "
            "lancee ; elle existe et ne protege rien."
        )
    elif '"$PLUG"' not in workflow:
        fautes.append(
            "integration.yml : le resultat du branchement a chaud n'entre plus "
            "dans la barriere ; la campagne peut echouer et la fusion passer."
        )


def main():
    fautes = []
    for chemin in (XHCI, WM):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        print("branchement usb : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))

    regle_surveillance_sans_hid(xhci, wm, fautes)
    regle_rapports_non_perdus(xhci, fautes)
    regle_enumeration_hors_boucle(xhci, fautes)
    regle_scrutation_en_plus_des_evenements(xhci, fautes)
    regle_debranchement_rend_tout(xhci, fautes)
    regle_campagne_reelle(fautes)

    if fautes:
        print("branchement usb : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "branchement usb : ports surveilles meme sans HID, rapports mis de "
        "cote et rejoues dans l'ordre, enumeration hors du drainage et bornee "
        "par tour, slot et memoire rendus au debranchement, et un clavier "
        "reellement branche apres le demarrage en integration"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
