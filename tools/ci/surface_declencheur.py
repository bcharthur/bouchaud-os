#!/usr/bin/env python3
"""Quand faut-il capturer l'ecran pour prouver que la mire est affichee ?

BOUCHAUD_C41_CAPTURE_VIVANTE

La preuve visuelle a une fenetre de validite : elle ne peut etre prise que
pendant que QEMU vit. Le run #347 a pose sa mire vers 211 s et QEMU a vecu
jusqu'a 342 s -- cent trente secondes pendant lesquelles la capture etait
possible -- mais le banc ne la tentait qu'apres sa boucle de surveillance,
donc apres la mort de la machine. Le verdict rendu etait `moniteur_muet` :
une panne du banc presentee comme une panne du produit.

La decision est ici, et pas dans le script shell, pour UNE raison : elle doit
pouvoir etre mise en echec sur un journal synthetique, y compris le cas qui
importe -- la mire est posee, deux trames suivent, PUIS la machine meurt.

## La regle

Au moment ou `HOST_SURFACE_MIRE_INSEREE` parait, on retient le numero de la
derniere trame composee. Toute trame de numero STRICTEMENT superieur a ete
composee apres l'insertion, donc la contient. La marge de deux couvre la trame
qui pouvait etre en cours de composition a cet instant.

Le numero d'insertion est fige a l'instant de l'insertion. Le recalculer plus
tard -- ce que faisait l'ancienne version -- donnait le dernier numero du
journal entier, c'est-a-dire une cible qui recule a mesure qu'on l'approche.
"""
import re
import sys

MIRE = "HOST_SURFACE_MIRE_INSEREE"
TRAME = re.compile(r"BROWSER_HOST_M11_TRAME page=\d+ seq=(\d+)")
MARGE = 2


def etat(texte, seq_insertion=None):
    """Rend (inseree, seq_insertion, seq_voulue, seq_vue, pret).

    `seq_insertion` fige la cible : passe-le a chaque tour une fois connu.
    """
    trames = [int(m.group(1)) for m in TRAME.finditer(texte)]
    seq_vue = max(trames) if trames else 0

    position = texte.find(MIRE)
    if position < 0:
        return (False, None, None, seq_vue, False)

    if seq_insertion is None:
        # La derniere trame connue A L'INSTANT de l'insertion : on ne regarde
        # que ce qui precede le marqueur dans le journal.
        avant = [int(m.group(1)) for m in TRAME.finditer(texte, 0, position)]
        seq_insertion = max(avant) if avant else 0

    seq_voulue = seq_insertion + MARGE
    return (True, seq_insertion, seq_voulue, seq_vue, seq_vue >= seq_voulue)


def _cas(nom, texte, seq_insertion, attendu):
    obtenu = etat(texte, seq_insertion)
    if obtenu != attendu:
        print(f"  ECHEC {nom}\n    attendu {attendu}\n    obtenu  {obtenu}",
              file=sys.stderr)
        return 1
    print(f"  ok    {nom}")
    return 0


def autotest():
    """Journaux synthetiques. Chaque cas est une situation deja rencontree."""
    def trame(n):
        return f"[serie] BROWSER_HOST_M11_TRAME page=1 seq={n}\n"

    echecs = 0
    print("surface/declencheur : cas synthetiques")

    # 1. Rien encore : ni mire, ni trame.
    echecs += _cas("journal vide", "", None, (False, None, None, 0, False))

    # 2. Des trames, mais la mire n'est pas posee : ne rien capturer.
    echecs += _cas("mire absente", trame(3) + trame(4), None,
                   (False, None, None, 4, False))

    # 3. La mire vient d'etre posee : la cible est figee, rien n'est pret.
    journal = trame(5) + MIRE + "\n"
    echecs += _cas("insertion a la trame 5", journal, None,
                   (True, 5, 7, 5, False))

    # 4. UNE seule trame apres : la marge de deux n'est pas atteinte.
    echecs += _cas("une trame apres", journal + trame(6), 5,
                   (True, 5, 7, 6, False))

    # 5. LE CAS QUI COMPTE : deux trames apres, puis la machine meurt.
    #    Le journal s'arrete net -- c'est exactement #347. La capture doit
    #    avoir ete declenchee AVANT, donc `pret` vaut vrai ici.
    mort = journal + trame(6) + trame(7) + "[kernel] extinction demandee\n"
    echecs += _cas("deux trames puis mort de QEMU", mort, 5,
                   (True, 5, 7, 7, True))

    # 6. La cible NE DOIT PAS reculer. Sans figement, `seq_insertion` serait
    #    recalcule a 7 et la cible passerait a 9 : une cible qui fuit.
    echecs += _cas("cible figee malgre des trames tardives",
                   mort + trame(8) + trame(9), 5, (True, 5, 7, 9, True))

    # 7. Aucune trame avant l'insertion : la cible part de zero, pas d'erreur.
    echecs += _cas("insertion sans trame prealable", MIRE + "\n" + trame(1),
                   None, (True, 0, 2, 1, False))

    if echecs:
        print(f"surface/declencheur : {echecs} cas en echec", file=sys.stderr)
        return 1
    print("SURFACE_DECLENCHEUR_OK")
    return 0


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--test":
        return autotest()
    if len(sys.argv) < 2:
        print("usage: surface_declencheur.py <journal> [seq_insertion]",
              file=sys.stderr)
        return 2
    try:
        with open(sys.argv[1], "rb") as flux:
            texte = flux.read().decode("utf-8", "replace")
    except OSError as exc:
        print(f"journal illisible : {exc}", file=sys.stderr)
        return 2
    fige = None
    if len(sys.argv) >= 3 and sys.argv[2] not in ("", "-"):
        fige = int(sys.argv[2])
    inseree, seq_insertion, seq_voulue, seq_vue, pret = etat(texte, fige)
    print(
        f"inseree={int(inseree)}"
        f" seq_insertion={seq_insertion if seq_insertion is not None else -1}"
        f" seq_voulue={seq_voulue if seq_voulue is not None else -1}"
        f" seq_vue={seq_vue}"
        f" pret={int(pret)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
