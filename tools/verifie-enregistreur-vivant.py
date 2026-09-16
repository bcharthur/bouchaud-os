#!/usr/bin/env python3
"""Garde-fou : l'enregistreur de vol survit a ce qu'il enregistre.

# Le defaut, trois fois de suite

Les trois archives physiques du 12 septembre s'arretent toutes au meme
endroit : l'instant ou le navigateur demarre.

    archive 14:51   84 enregistrements pour 127 numeros, bb_busy_skips=49
    archive 15:53   15,5 s couvertes sur une session de plusieurs minutes
    archive 17:55   arret a 29,028 s, en plein tour de scrutation

La derniere est la plus parlante : `bb_failures=0` jusqu'au dernier
echantillon, le bureau a soixante-deux trames par seconde, et l'utilisateur
s'est servi de la machine deux minutes de plus. L'enregistreur n'avait pas
echoue -- il n'etait plus elu. Le navigateur venait de creer une vingtaine de
taches de priorite Normale, la sienne.

C'est le pire moment possible pour perdre la trace : c'est exactement celui
qu'on cherche a comprendre.

# Ce que V3 change a ce garde-fou

Les deux premieres proprietes defendaient une PARADE : le numero non consomme
et la fenetre rendue existaient parce que `append` pouvait echouer a cause du
peripherique. Depuis le tambour RAM, il ne le peut plus -- et une parade qui
protege d'un defaut supprime protege de rien.

Elles sont remplacees par la propriete qui les rend inutiles, et qui est plus
forte : poser un enregistrement NE PEUT PAS echouer autrement que par une
charge utile trop grande, qui est un defaut de l'appelant. Les deux dernieres,
elles, restent : le fil peut toujours cesser d'etre elu.

# Les quatre proprietes verifiees ici

1. Un numero d'enregistrement reserve est TOUJOURS publie. Le seul refus a
   lieu AVANT la reservation, sur une charge utile trop grande -- donc sans
   trouer la numerotation.
2. La cadence de l'enregistreur ne depend plus d'aucun peripherique : ni
   `poll` ni son fil ne consultent l'etat du support pour decider quand
   repasser.
3. Le fil de l'enregistreur est Interactive : il dort entre deux tours, le
   promouvoir ne coute rien, et cela lui rend la seule chose dont il a besoin.
4. Un filet : si l'enregistreur se tait quand meme, le compositeur produit a
   sa place -- il tourne toujours, et la production est bornee et non
   bloquante.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
BB = RACINE / "src/kernel/debug/blackbox.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
WM = RACINE / "src/gui/window_manager.rs"


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
    for chemin in (BB, XHCI, WM):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    bb = sans_commentaires(BB.read_text(encoding="utf-8"))
    xhci = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))

    # 1. UN NUMERO RESERVE EST TOUJOURS PUBLIE.
    ajout = corps(bb, "fn append(")
    if ajout is None:
        fautes.append("blackbox.rs : `append` a disparu.")
    else:
        if "BOBINE.reserve(" not in ajout or "BOBINE.publie(" not in ajout:
            fautes.append(
                "blackbox.rs : `append` ne reserve plus puis ne publie plus "
                "dans le tambour RAM. Ce sont les deux moities de la "
                "publication atomique."
            )
        # ENTRE LA RESERVATION ET LA PUBLICATION, AUCUNE SORTIE.
        #
        # Un `return` glisse entre les deux consommerait un numero sans jamais
        # le publier -- et ce trou serait indistinguable, a la relecture, d'un
        # enregistrement corrompu. C'est exactement le defaut que la
        # numerotation differee corrigeait en V2 ; il n'a disparu que parce
        # que ce chemin est desormais droit.
        # LA FENETRE COMMENCE APRES LE REFUS, PAS APRES LA RESERVATION.
        #
        # `let Some(r) = reserve(..) else { return false; };` contient un
        # `return` qui est TEXTUELLEMENT apres la reservation et LOGIQUEMENT
        # avant : a cet instant, aucun numero n'a ete consomme. Prendre la
        # reservation pour borne se declenchait donc sur le code correct --
        # une garde qui crie sur ce qu'elle defend ne defend rien.
        refus = ajout.find("};", ajout.find("BOBINE.reserve("))
        entre = ajout[refus : ajout.find("BOBINE.publie(")]
        if "return" in entre or "?" in entre:
            fautes.append(
                "blackbox.rs : `append` peut sortir entre la reservation et la "
                "publication. Le numero serait consomme sans etre publie, et "
                "le trou ne se distinguerait pas d'une corruption."
            )

    # 2. LA CADENCE NE DEPEND PLUS D'AUCUN PERIPHERIQUE.
    scrutation = corps(bb, "pub fn poll()")
    if scrutation is None:
        fautes.append("blackbox.rs : `poll` a disparu.")
    elif "blackbox_storage_ready" in scrutation or "vidange(" in scrutation:
        fautes.append(
            "blackbox.rs : `poll` consulte de nouveau le support. Sa cadence "
            "redeviendrait celle du peripherique qu'il observe -- et c'est "
            "precisement ce qui rendait l'arret des trois archives physiques "
            "indechiffrable."
        )
    fil = corps(xhci, "fn fil_blackbox()")
    if fil is None:
        fautes.append("xhci_active.rs : le fil de l'enregistreur a disparu.")
    elif len(re.findall(r"sleep_ticks\(", fil)) != 1:
        fautes.append(
            "xhci_active.rs : le fil de l'enregistreur a de nouveau deux "
            "cadences. Il n'en a plus besoin : rien ne peut plus le faire "
            "renoncer, et une cadence variable ferait croire le contraire a la "
            "relecture."
        )

    # 3. IL EST INTERACTIVE.
    lancement = corps(xhci, "pub fn demarre_le_fil_blackbox()")
    if lancement is None:
        fautes.append("xhci_active.rs : le lancement du fil a disparu.")
    elif "Priorite::Interactive" not in lancement:
        fautes.append(
            "xhci_active.rs : le fil de l'enregistreur redevient Normale. Le "
            "navigateur cree une vingtaine de taches de cette priorite en "
            "demarrant, et c'est exactement la que les trois archives "
            "physiques se sont arretees."
        )

    # 4. LE FILET.
    filet = corps(bb, "pub fn filet_de_securite(")
    if filet is None:
        fautes.append(
            "blackbox.rs : le filet a disparu. Si l'enregistreur cesse d'etre "
            "elu, plus rien ne l'ecrit."
        )
    else:
        if "silence < seuil_ns" not in filet:
            fautes.append(
                "blackbox.rs : le filet ne COMPARE plus le silence a son "
                "seuil. Il ecrirait a chaque trame, en doublant "
                "l'enregistreur au lieu de le remplacer."
            )
        if "silence_ns()" not in filet:
            fautes.append(
                "blackbox.rs : le filet ne mesure plus le silence de "
                "l'enregistreur ; il ecrirait tout le temps ou jamais."
            )
        if "poll()" not in filet:
            fautes.append("blackbox.rs : le filet n'ecrit rien.")
    if "filet_de_securite(" not in wm:
        fautes.append(
            "window_manager.rs : le compositeur n'arme plus le filet. C'est la "
            "seule boucle qui tourne toujours."
        )
    m = re.search(r"const SILENCE_ENREGISTREUR_NS: u64 = ([0-9_]+);", wm)
    if m is None:
        fautes.append("window_manager.rs : le seuil de silence n'est plus lisible.")
    else:
        seuil = int(m.group(1).replace("_", ""))
        if seuil > 5_000_000_000:
            fautes.append(
                "window_manager.rs : le filet attend plus de cinq secondes. "
                "L'enregistreur ecrit quatre fois par seconde quand tout va "
                "bien ; au-dela de quelques secondes la trace est deja perdue."
            )
        if seuil < 500_000_000:
            fautes.append(
                "window_manager.rs : le filet se declenche avant une demi-"
                "seconde de silence. Il doublerait l'enregistreur au lieu de "
                "le remplacer."
            )

    if fautes:
        print("enregistreur vivant : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "enregistreur vivant : numero toujours publie, cadence independante "
        "du support, fil Interactive, filet arme par le compositeur avec un "
        "seuil raisonnable")
    return 0


if __name__ == "__main__":
    sys.exit(main())
