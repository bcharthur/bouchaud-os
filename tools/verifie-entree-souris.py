#!/usr/bin/env python3
"""Garde-fou : UNE source de boutons ne parle QUE pour elle.

# Le defaut, observe sur la machine de reference

Le releve de vol d'une session de dix-sept secondes contient QUATRE-VINGT-QUATRE
`CLICK_DISPATCHED`, avec des positions qui marchent regulierement :

    CLICK_DISPATCHED x=318 y=54 buttons=0x1
    CLICK_DISPATCHED x=321 y=60 buttons=0x1
    CLICK_DISPATCHED x=323 y=63 buttons=0x1

Ce n'est pas quelqu'un qui clique quatre-vingt-quatre fois : c'est quelqu'un qui
TIRE une barre de titre. La detection de front (`left && !prev_left`) etait
pourtant juste. C'est donc l'ETAT du bouton qui vacillait.

Le recomptage HID de cette machine dit pourquoi : `claviers=2 souris=3`. Trois
« souris », parce qu'un recepteur sans fil expose une interface par
peripherique associe plus la sienne. Chacune ecrivait DIRECTEMENT l'etat global
des boutons ; une interface au repos publie « aucun bouton », et effacait donc
le bouton maintenu sur la vraie souris.

Consequences, toutes rapportees par l'utilisateur : impossible de deplacer une
fenetre (« ca met le plein ecran soit ca l'enleve »), impossible de la
redimensionner par le coin bas-droit, et le curseur qui se teleporte quand une
interface VENDEUR est relue comme une souris.

# Ce qui est verifie

1. `inject_usb_report` prend une identite de source et passe par l'union.
2. Aucun rapport n'ecrit `BTN` directement, ni cote PS/2 ni cote USB.
3. La source est RENDUE au debranchement -- sinon un bouton reste enfonce pour
   toujours et le tableau s'epuise en quelques rebranchements.
4. Le repli aveugle « interface non classee = souris » ne contredit plus le
   descripteur de rapport quand celui-ci a parle.
5. Le bureau ne prend pas un front montant pendant un glissement pour un clic.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
ETAT = RACINE / "src/drivers/input/mouse/etat.rs"
PAQUET = RACINE / "src/drivers/input/mouse/paquet.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
BUREAU = RACINE / "src/gui/window_manager.rs"
DETECTEUR = RACINE / "src/gui/windowing/hit_test.rs"
PS2 = RACINE / "src/drivers/input/mouse/ps2.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


def main():
    fautes = []

    etat = sans_commentaires(ETAT.read_text(encoding="utf-8"))
    if "BOUTONS_PAR_SOURCE" not in etat:
        fautes.append(
            "etat.rs : le tableau des boutons PAR SOURCE a disparu. L'etat "
            "global redevient ce que la derniere interface a dit, et une "
            "interface au repos efface le bouton maintenu sur une autre."
        )
    if "SOURCES_OCCUPEES" not in etat:
        fautes.append(
            "etat.rs : plus de reservation de source. Deux peripheriques "
            "partageraient la meme case et s'effaceraient l'un l'autre."
        )

    paquet = sans_commentaires(PAQUET.read_text(encoding="utf-8"))
    if "fn recompose_boutons(" not in paquet:
        fautes.append(
            "paquet.rs : `recompose_boutons` a disparu. C'est elle qui fait de "
            "l'etat global l'UNION des sources au lieu du dernier rapport recu."
        )
    else:
        corps = paquet.split("fn recompose_boutons(", 1)[1].split("\n}", 1)[0]
        if "|=" not in corps:
            fautes.append(
                "paquet.rs : `recompose_boutons` ne fait plus l'union des "
                "sources. Sans le OU, elle rend le dernier rapport et le "
                "defaut revient a l'identique."
            )
    if "pub fn inject_usb_report(source: usize," not in paquet:
        fautes.append(
            "paquet.rs : `inject_usb_report` ne prend plus d'identite de "
            "source. Sans elle, tous les points de terminaison ecrivent le "
            "meme etat de boutons."
        )
    if "pub fn libere_source_souris(" not in paquet:
        fautes.append(
            "paquet.rs : une source debranchee n'est plus rendue. Un "
            "debranchement bouton enfonce laisserait le bureau croire le "
            "bouton maintenu pour toujours."
        )
    # Aucun chemin de rapport ne doit ecrire l'etat global sans passer par
    # l'union : c'est exactement le defaut d'origine.
    for bloc, nom in (
        (paquet.split("unsafe fn apply_packet(", 1)[-1].split("\n}", 1)[0], "PS/2"),
        (paquet.split("pub fn inject_usb_report(", 1)[-1].split("\n}", 1)[0], "USB"),
    ):
        if "recompose_boutons(" not in bloc:
            fautes.append(
                "paquet.rs : le chemin %s n'appelle plus `recompose_boutons`. "
                "Il ecrit donc l'etat global pour tout le monde." % nom
            )

    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    if "source_souris" not in xhci:
        fautes.append(
            "xhci_active.rs : un point de terminaison souris n'a plus de "
            "source propre."
        )
    if "libere_source_souris(" not in xhci:
        fautes.append(
            "xhci_active.rs : `retire_slot` ne rend plus la source de boutons "
            "du peripherique debranche."
        )
    if "BOUCHAUD_HID_HEURISTIC_MOUSE" in xhci:
        # Le repli existe encore : il doit s'effacer devant le descripteur.
        avant = xhci.split("BOUCHAUD_HID_HEURISTIC_MOUSE", 1)[0]
        condition = avant.rsplit("let mut kind = descriptor.kind;", 1)[-1]
        if "!descriptor.classe_par_rapport" not in condition:
            fautes.append(
                "xhci_active.rs : le repli aveugle promeut de nouveau en "
                "souris une interface dont le descripteur de rapport a DEJA "
                "dit qu'elle n'en etait pas. C'est ainsi que l'interface "
                "vendeur d'un recepteur sans fil injectait des boutons et des "
                "deplacements tires de ses notifications."
            )
    if "classe_par_rapport = true" not in xhci:
        fautes.append(
            "xhci_active.rs : le verdict du descripteur de rapport n'est plus "
            "retenu ; le repli aveugle reprend la main sur tout."
        )

    bureau = sans_commentaires(BUREAU.read_text(encoding="utf-8"))
    if "glissement_en_cours" not in bureau:
        fautes.append(
            "window_manager.rs : un front montant pendant un glissement "
            "repasse pour un clic. Le detecteur de double-clic se rearme a "
            "chaque tour et la fenetre bascule en plein ecran au lieu de "
            "suivre le curseur."
        )
    else:
        ligne = [l for l in bureau.splitlines() if "let click = left" in l]
        if not ligne or "!glissement_en_cours" not in ligne[0]:
            fautes.append(
                "window_manager.rs : `click` ne retranche plus le glissement "
                "en cours."
            )
    if "title_clicks.oublie()" not in bureau:
        fautes.append(
            "window_manager.rs : un appui qui a servi a TIRER ouvre de nouveau "
            "un double-clic. Relacher une fenetre deplacee puis reappuyer sur "
            "sa barre de titre la basculerait en plein ecran."
        )

    detecteur = sans_commentaires(DETECTEUR.read_text(encoding="utf-8"))
    if "pub fn oublie(" not in detecteur:
        fautes.append(
            "hit_test.rs : le detecteur de double-clic ne sait plus oublier "
            "l'appui precedent."
        )

    # --- ON N'ARME PAS L'IRQ D'UN PERIPHERIQUE QUI N'EST PAS LA --------------
    #
    # Une souris PS/2 repond a la demande d'identite par 0x00, 0x03 ou 0x04.
    # La machine de reference a repondu 0xFE -- « RESEND » : un 8042 emule par
    # le micrologiciel, sans souris derriere. IRQ12 etait demasquee quand meme,
    # et la machine est morte sur une double faute dans la seconde qui a suivi
    # (releve du 12 septembre 18:40, vecteur 0x8, tache `desktop`).
    #
    # Aucun demarrage precedent n'etait passe par la : ce repli ne s'arme que
    # lorsqu'aucune souris USB n'a ete reconnue.
    ps2 = sans_commentaires(PS2.read_text(encoding="utf-8"))
    init = None
    if "pub fn init()" in ps2:
        debut = ps2.index("pub fn init()")
        init = ps2[debut:]
    if init is None:
        fautes.append("ps2.rs : `init` a disparu.")
    else:
        if "matches!(id, ID_STANDARD | ID_MOLETTE | ID_CINQ_BOUTONS)" not in init:
            fautes.append(
                "ps2.rs : l'identite rendue par le 8042 n'est plus verifiee. "
                "Une souris PS/2 repond 0x00, 0x03 ou 0x04 ; tout le reste dit "
                "qu'il n'y a pas de souris."
            )
        # LA REGLE : IRQ12 sous condition, IRQ1 sans condition.
        ligne12 = [l for l in init.splitlines() if "unmask_irq(12)" in l]
        if not ligne12:
            fautes.append("ps2.rs : IRQ12 n'est plus demasquee du tout.")
        else:
            avant = init[: init.index(ligne12[0])]
            queue = avant.rstrip().splitlines()
            if not queue or "if presente" not in queue[-1]:
                fautes.append(
                    "ps2.rs : IRQ12 est de nouveau demasquee sans condition. "
                    "Armer une ligne d'interruption pour un peripherique absent "
                    "n'apporte rien, et sur la machine de reference cela a tue "
                    "le noyau."
                )
        if "unmask_irq(1)" not in init:
            fautes.append(
                "ps2.rs : IRQ1 n'est plus demasquee. Le clavier n'a rien a voir "
                "avec l'identite de la souris : sa ligne doit etre armee quoi "
                "qu'il arrive."
            )

    if "BOUCHAUD_HID_REPORT_OCTETS" not in xhci:
        fautes.append(
            "xhci_active.rs : les octets du descripteur de rapport ne sont "
            "plus journalises. Sans eux, un mauvais classement ne peut pas "
            "etre distingue d'un peripherique qui ment."
        )

    if fautes:
        print("entree souris : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "entree souris : boutons par source et unis, source rendue au "
        "debranchement, repli aveugle soumis au descripteur de rapport, front "
        "montant ignore pendant un glissement, double-clic oublie apres un "
        "deplacement, IRQ12 armee seulement pour une souris qui repond, et les "
        "octets du descripteur de rapport journalises"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
