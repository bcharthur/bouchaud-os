#!/usr/bin/env python3
"""Ce qui fait qu'une machine physique a, ou n'a pas, de clavier.

# Le defaut que ces regles ferment

Sur une machine reelle, « pas de clavier » ne se debogue pas : il n'y a ni
console serie, ni moyen de taper quoi que ce soit. Ce fichier protege les
quatre decisions qui, prises a l'envers, donnent exactement ce symptome -- et
qu'aucun test sous QEMU ne peut retrouver, parce que QEMU rend la main
poliment, n'arme aucune interruption de gestion systeme, et expose toujours
ses peripheriques.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
WM = RACINE / "src/gui/window_manager.rs"
PS2_SOURIS = RACINE / "src/drivers/input/mouse/ps2.rs"
FICHE = RACINE / "docs/reference/TRIGKEY_SPEED_S5.md"


def sans_commentaires(source):
    """Retire les commentaires de ligne, en preservant les chaines.

    Une regle qui compte les commentaires est verte parce que quelqu'un a
    DECRIT le comportement, pas parce qu'il l'a ecrit.
    """
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
    """Le corps `{...}` qui suit une signature, accolades equilibrees.

    On ne s'arrete sur `;` ou `{` qu'a profondeur nulle : un type de retour
    comme `-> [u8; 16]` contient un `;` qui n'est pas la fin d'une declaration.
    """
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


def regle_smi_desarmes(xhci, fautes):
    """Prendre le controleur ne suffit pas : il faut desarmer ses SMI.

    Tant que les autorisations de SMI de `USBLEGCTLSTS` restent posees, le
    micrologiciel continue d'etre APPELE sur chaque evenement USB, par une
    interruption que le noyau ne voit pas et ne peut pas masquer. Il traite
    l'evenement pour son emulation PS/2, et le noyau ne recoit rien.

    Le symptome est celui qu'on cherche justement a fermer : un clavier qui
    marche dans le BIOS et pas dans le systeme. Sur une machine AMD dont le
    BIOS propose « USB legacy emulation », c'est le cas NORMAL.
    """
    bloc = corps(xhci, "unsafe fn legacy_handoff(")
    if bloc is None:
        fautes.append(
            "xhci_active.rs : la reprise au micrologiciel a disparu ; le "
            "controleur resterait au BIOS et aucun evenement n'arriverait."
        )
        return

    # L'ECRITURE, et non la simple presence de la constante : declarer
    # `LEGACY_CONTROLE` sans jamais y ecrire laisse les SMI armes.
    ecriture = re.search(r"w32\(\s*base\s*,\s*off\s*\+\s*LEGACY_CONTROLE\s*,", bloc)
    if ecriture is None:
        fautes.append(
            "xhci_active.rs : USBLEGCTLSTS n'est plus ECRIT dans "
            "legacy_handoff ; les SMI du micrologiciel restent armes, il "
            "continue d'intercepter chaque evenement USB, et le clavier "
            "marche dans le BIOS mais pas dans le systeme."
        )
        return

    # La valeur ecrite doit PRESERVER les bits reserves. Ecrire zero partout
    # marcherait sur beaucoup de machines et corromprait les autres, sans que
    # rien ne le dise.
    if "LEGACY_BITS_RESERVES" not in bloc:
        fautes.append(
            "xhci_active.rs : les bits reserves de USBLEGCTLSTS ne sont plus "
            "preserves ; on ecrit alors dans des bits dont le sens appartient "
            "au controleur."
        )

    # Et EFFACER les evenements en attente : un evenement laisse pose est
    # re-signale des qu'une autorisation revient.
    if "LEGACY_EVENEMENTS_SMI" not in bloc:
        fautes.append(
            "xhci_active.rs : les evenements SMI en attente ne sont plus "
            "effaces ; ils restent poses et reviennent."
        )

    # L'ORDRE : desarmer avant d'avoir pris la main ne sert a rien, le
    # micrologiciel les reposerait.
    semaphore = bloc.find("1 << 24")
    if semaphore < 0 or semaphore > ecriture.start():
        fautes.append(
            "xhci_active.rs : les SMI sont desarmes AVANT la prise du "
            "semaphore ; le micrologiciel, encore proprietaire, les repose."
        )


def regle_reprise_forcee(xhci, fautes):
    """Un micrologiciel qui ne rend jamais la main.

    Certains ne baissent jamais « BIOS Owned ». Attendre l'expiration puis
    continuer comme si de rien n'etait donne une machine sans clavier et sans
    rien pour le dire.
    """
    bloc = corps(xhci, "unsafe fn legacy_handoff(")
    if bloc is None:
        return
    if "BOUCHAUD_XHCI_HANDOFF_FORCE" not in bloc:
        fautes.append(
            "xhci_active.rs : la reprise forcee a disparu ; un micrologiciel "
            "qui ne rend pas la main laisserait la machine sans clavier, et "
            "le journal ne dirait pas pourquoi."
        )
    # Le bit doit etre EFFACE, pas seulement journalise.
    if not re.search(r"w32\(\s*base\s*,\s*off\s*,[^;]*!\(1 << 16\)", bloc):
        fautes.append(
            "xhci_active.rs : « BIOS Owned » n'est plus efface de force ; "
            "annoncer la reprise sans la faire est pire que ne rien faire."
        )
    if "BOUCHAUD_XHCI_SMI_DESARMES" not in bloc:
        fautes.append(
            "xhci_active.rs : le desarmement des SMI ne se dit plus dans le "
            "journal ; c'est la seule trace qu'on ait sur une machine sans "
            "console."
        )


def _decision(stage2):
    """Le bloc ou le repli PS/2 est decide."""
    return corps(stage2, "fn stage2_main(") or stage2


def regle_repli_sur_ce_qui_est_trouve(stage2, fautes):
    """Le repli PS/2 se decide sur les PERIPHERIQUES, pas sur le CONTROLEUR.

    Un xHCI existe sur toute machine moderne. S'en servir comme critere veut
    dire : « il y a un controleur, donc il y a un clavier » -- et si la mise en
    route HID echoue, la machine n'a plus AUCUNE entree. Ce qu'il faut
    regarder, c'est ce qui a repondu.
    """
    if "hid_keyboards()" not in stage2 or "hid_mice()" not in stage2:
        fautes.append(
            "stage2.rs : la decision ne consulte plus les peripheriques "
            "TROUVES ; un echec de la mise en route USB laisserait la machine "
            "sans aucune entree."
        )
        return

    for drapeau, source, genre in (
        ("LEGACY_PS2_CLAVIER", "claviers_usb", "clavier"),
        ("LEGACY_PS2_SOURIS", "souris_usb", "souris"),
    ):
        pose = re.search(
            r"%s\.store\(\s*([^,]*),"
            % drapeau,
            stage2,
        )
        if pose is None:
            fautes.append(
                "stage2.rs : le drapeau %s n'est plus pose ; le repli %s "
                "n'est plus decide." % (drapeau, genre)
            )
            continue
        condition = pose.group(1)
        if "%s == 0" % source not in condition:
            fautes.append(
                "stage2.rs : %s ne suit plus « aucun %s USB n'a repondu » "
                "(%s == 0) mais « %s » ; c'est le controleur qu'on regarde, "
                "pas ce qui a repondu."
                % (drapeau, genre, source, condition.strip())
            )
        # Le piege exact qu'on a corrige : decider sur la presence du
        # controleur.
        if "xhci_present" in condition:
            fautes.append(
                "stage2.rs : %s se decide de nouveau sur la PRESENCE du "
                "controleur ; si la mise en route HID echoue, la machine n'a "
                "plus d'entree du tout." % drapeau
            )


def regle_deux_genres_distincts(stage2, wm, fautes):
    """Clavier et souris se decident SEPAREMENT.

    Un drapeau unique force a choisir entre « tout le PS/2 » et « rien » : une
    machine dont le clavier USB repond et la souris non se retrouve sans
    pointeur -- un bureau ou l'on tape et ou l'on ne clique rien.

    Et l'inverse compte autant : initialiser le PS/2 alors qu'un peripherique
    USB du meme genre repond deja fait arriver chaque frappe DEUX FOIS quand le
    micrologiciel emule encore un 8042.
    """
    for fonction in ("pub fn legacy_ps2_clavier(", "pub fn legacy_ps2_souris("):
        if corps(stage2, fonction) is None:
            fautes.append(
                "stage2.rs : %s a disparu ; les deux genres d'entree "
                "redeviennent une seule decision." % fonction
            )

    # Le clavier PS/2 ne s'initialise que si aucun clavier USB n'a repondu.
    garde = re.search(
        r"if claviers_usb == 0 \{(.*?)\n    \}",
        stage2,
        re.S,
    )
    if garde is None or "keyboard::init()" not in garde.group(1):
        fautes.append(
            "stage2.rs : le clavier PS/2 n'est plus garde par « aucun clavier "
            "USB n'a repondu » ; chaque frappe arriverait deux fois si le "
            "micrologiciel emule encore un 8042."
        )

    # Et la souris suit SA propre decision, pas celle du clavier.
    boucle = corps(wm, "fn boucle(")
    if boucle is None:
        fautes.append("window_manager.rs : la boucle du bureau est introuvable.")
        return
    if "legacy_ps2_souris()" not in boucle:
        fautes.append(
            "window_manager.rs : la souris PS/2 ne suit plus sa propre "
            "decision ; une machine avec un clavier USB et une souris PS/2 "
            "n'aurait pas de pointeur."
        )
    if "legacy_ps2_allowed()" in boucle:
        fautes.append(
            "window_manager.rs : la souris est de nouveau decidee par le "
            "drapeau commun ; elle serait initialisee parce que le CLAVIER "
            "est en PS/2."
        )
    if re.search(r"^\s*mouse::init\(\);", boucle, re.M) is None:
        fautes.append("window_manager.rs : mouse::init() a disparu de la boucle.")


def regle_decision_journalisee(stage2, wm, fautes):
    """Sur une machine sans console, le journal est tout ce qu'on aura.

    Le Trigkey n'a pas de port serie : ces lignes ne se lisent qu'apres coup,
    sous QEMU ou via un adaptateur. Elles doivent donc dire l'ETAT COMPLET, pas
    « prete ».
    """
    if "BOUCHAUD_STAGE2_ENTREE_DECIDEE" not in stage2:
        fautes.append(
            "stage2.rs : la decision d'entree ne se dit plus ; on ne pourrait "
            "plus savoir si le clavier vient de l'USB ou du PS/2."
        )
    else:
        ligne = stage2[stage2.find("BOUCHAUD_STAGE2_ENTREE_DECIDEE") :][:400]
        for champ in ("claviers_usb=", "souris_usb=", "ps2_clavier=", "ps2_souris="):
            if champ not in ligne:
                fautes.append(
                    "stage2.rs : « %s » a disparu de la ligne de decision ; "
                    "l'etat n'est plus lisible en entier." % champ
                )
    if "BOUCHAUD_TRIGKEY_REPLI_PS2" not in stage2:
        fautes.append(
            "stage2.rs : le repli ne se nomme plus ; un controleur present "
            "sans clavier redeviendrait un silence."
        )
    if "BOUCHAUD_STAGE2_INPUT_READY" not in wm:
        fautes.append(
            "window_manager.rs : le bureau ne dit plus d'ou vient chaque "
            "genre d'entree."
        )


def regle_repli_borne(ps2, fautes):
    """Le repli ne doit pas pouvoir suspendre la machine.

    Sonder un 8042 qui n'existe pas est le cas NORMAL sur une machine qui n'en
    a pas. Une attente non bornee y transformerait un repli de securite en
    gel au demarrage -- et un gel est pire qu'une absence de souris.
    """
    for signature in ("fn wait_write", "fn wait_read"):
        bloc = corps(ps2, signature)
        if bloc is None:
            continue
        if re.search(r"\bloop\s*\{", bloc) or "while " in bloc:
            fautes.append(
                "input/mouse/ps2.rs : %s attend sans borne ; sur une machine "
                "sans 8042 le repli gelerait le demarrage." % signature
            )


def regle_fiche_materielle(fautes):
    """Ce qui a ete compris sur la machine doit rester ecrit.

    Ces quatre pieges ne se retrouvent pas en relisant le code : ils se
    retrouvent en perdant une journee sur une machine muette.
    """
    if not FICHE.exists():
        fautes.append(
            "docs/reference/TRIGKEY_SPEED_S5.md : la fiche de la machine de "
            "reference a disparu."
        )
        return
    texte = FICHE.read_text(encoding="utf-8")
    for marqueur, quoi in (
        ("USBLEGCTLSTS", "le desarmement des SMI du micrologiciel"),
        ("WPR", "la difference entre reinitialisation USB2 et USB3"),
        ("BOUCHAUD_STAGE2_ENTREE_DECIDEE", "la ligne de journal a chercher"),
        ("concentrateur", "les concentrateurs non traverses"),
    ):
        if marqueur not in texte:
            fautes.append(
                "TRIGKEY_SPEED_S5.md : « %s » n'y figure plus (%s)."
                % (marqueur, quoi)
            )


def main():
    fautes = []
    for chemin in (XHCI, STAGE2, WM, PS2_SOURIS):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        print("entree trigkey : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    stage2 = sans_commentaires(STAGE2.read_text(encoding="utf-8"))
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))
    ps2 = sans_commentaires(PS2_SOURIS.read_text(encoding="utf-8"))

    regle_smi_desarmes(xhci, fautes)
    regle_reprise_forcee(xhci, fautes)
    regle_repli_sur_ce_qui_est_trouve(stage2, fautes)
    regle_deux_genres_distincts(stage2, wm, fautes)
    regle_decision_journalisee(stage2, wm, fautes)
    regle_repli_borne(ps2, fautes)
    regle_fiche_materielle(fautes)

    if fautes:
        print("entree trigkey : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "entree trigkey : SMI du micrologiciel desarmes apres la prise du "
        "semaphore, reprise forcee si le BIOS ne rend pas la main, repli PS/2 "
        "decide genre par genre sur ce qui a REPONDU, attentes 8042 bornees, "
        "decision journalisee"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
