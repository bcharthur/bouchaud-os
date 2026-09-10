#!/usr/bin/env python3
"""Garde-fou : l'entree n'est pas lue a la cadence du rendu.

# Le defaut, mesure sur la machine de reference

`xhci_active::poll()` etait appele depuis la boucle de trames du compositeur,
et de nulle part ailleurs. L'entree etait donc lue A LA CADENCE DU RENDU : une
trame longue ne ralentissait pas le pointeur, elle l'ARRETAIT.

L'enregistreur de vol du TRIGKEY le montre sans interpretation :

    [22:23:11][FPS:  0]  BOUCHAUD_USB_HID_RECOMPTE claviers=2 souris=3
    [22:23:15][FPS:  1]  BOUCHAUD_HID_REPORT ...
    [22:23:16][FPS: 55]  BOUCHAUD_STAGE2_CLICK_DISPATCHED x=200 y=57

Cinq secondes a zero ou une trame par seconde -- le chargement des polices --
pendant lesquelles la souris etait lue une fois par seconde. Vu de la machine,
c'est un gel : rien ne repond, et rien n'a plante.

# Ce qui est verifie

Que le fil d'entree existe, qu'il soit lance a l'amorcage AVANT le bureau, que
sa cadence vienne d'un sommeil et non d'un tour de boucle, et que l'appel
restant dans le compositeur soit un REPLI conditionnel -- pas le chemin normal.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
USB = RACINE / "src/drivers/usb/xhci_active.rs"
WM = RACINE / "src/gui/window_manager.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
MAIN = RACINE / "src/main.rs"


def sans_commentaires(texte):
    return "\n".join(
        ligne for ligne in texte.splitlines()
        if not ligne.lstrip().startswith("//")
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
    for chemin in (USB, WM, STAGE2, MAIN):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
            return 1

    usb = sans_commentaires(USB.read_text(encoding="utf-8"))
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))
    stage2 = sans_commentaires(STAGE2.read_text(encoding="utf-8"))
    principal = sans_commentaires(MAIN.read_text(encoding="utf-8"))

    # 1. Le fil existe, et sa cadence vient d'un sommeil.
    fil = corps(usb, "fn fil_hid()")
    if fil is None:
        fautes.append(
            "xhci_active.rs : le fil d'entree a disparu ; la scrutation "
            "retombe dans la boucle de trames, et l'entree redevient tributaire "
            "du rendu."
        )
    else:
        if "poll()" not in fil:
            fautes.append("xhci_active.rs : le fil d'entree ne scrute plus.")
        if "sleep_ticks" not in fil:
            fautes.append(
                "xhci_active.rs : la cadence du fil d'entree ne vient plus d'un "
                "sommeil. Une boucle sans sommeil brulerait le coeur zero, qui "
                "est le seul a recevoir le tick sur la machine de reference."
            )

    # 2. Il est lance a l'amorcage, sur LES DEUX chemins.
    for nom, source in (("stage2.rs", stage2), ("main.rs", principal)):
        if "demarre_le_fil_hid()" not in source:
            fautes.append(
                "%s : le fil d'entree n'est plus lance a l'amorcage ; le bureau "
                "demarrerait sans souris." % nom
            )

    lancement = corps(usb, "pub fn demarre_le_fil_hid()")
    if lancement is None:
        fautes.append("xhci_active.rs : le lancement du fil d'entree a disparu.")
    else:
        if "Priorite::Interactive" not in lancement:
            fautes.append(
                "xhci_active.rs : le fil d'entree n'est plus interactif. Ce qui "
                "repond a l'utilisateur passe avant le travail de fond, ou "
                "l'etiquette ne sert a rien."
            )
        if "REFUSE" not in lancement:
            fautes.append(
                "xhci_active.rs : un fil d'entree non cree ne se dit plus ; le "
                "bureau serait lent sans que rien ne l'explique."
            )

    # 3. L'appel du compositeur est un REPLI, pas le chemin normal.
    #
    # La regle ne demande pas de le supprimer : `poll()` est borne, ne bloque
    # pas et se protege de la reentrance -- contrairement a un montage de
    # disque, le laisser en repli ne fige rien. Elle demande qu'il soit
    # CONDITIONNEL, sinon la scrutation se ferait deux fois et le decouplage
    # serait une illusion.
    tours = corps(wm, "crate::drivers::xhci_active::poll()")
    if "xhci_active::poll()" in wm and "fil_hid_actif()" not in wm:
        fautes.append(
            "window_manager.rs : le compositeur scrute l'entree sans demander "
            "si le fil s'en charge deja. La scrutation se ferait deux fois, et "
            "le decouplage annonce serait une illusion."
        )

    if fautes:
        print("entree hors rendu : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "entree : fil dedie, cadence par sommeil, priorite interactive, lance "
        "a l'amorcage sur les deux chemins, appel du compositeur en repli "
        "conditionnel"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
