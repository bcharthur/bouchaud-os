#!/usr/bin/env python3
"""Garde-fou : un point de terminaison qui s'est TU n'est pas un point au repos.

# Le defaut, lu sur la machine de reference

Releve du 13 septembre 2026, 11:24. La souris produit ses rapports normalement
puis s'arrete net :

    t= 18.81 s  polls=3296  events=692  mouse=692  rearms=695
    t= 37.87 s  polls=8060  events=692  mouse=692  rearms=695

Dix-neuf secondes sans un evenement, et quatre minutes d'utilisation ensuite
pendant lesquelles le pointeur n'a plus bouge du tout -- l'utilisateur n'a meme
pas pu cliquer sur « eteindre ». La boucle de scrutation tournait pourtant
(`polls` monte de 250 par seconde) et la cloche de relance etait tiree toutes
les cent vingt-huit scrutations (`kicks` monte). Une sonnette ne reveille pas
un point de terminaison ARRETE.

Personne ne venait a son secours :

* le pont EP0 ne s'occupe que des points qui n'ont JAMAIS parle
  (`evenements == 0`), donc jamais de celui-la ;
* rien ne regardait l'etat du point dans le contexte que le controleur tient
  a jour.

# Ce qui rendait le defaut invisible

Vu du seul compteur d'evenements, une souris immobile et une souris morte se
ressemblent trait pour trait. Le contexte du controleur les separe sans
ambiguite : une souris au repos est `Running` AVEC un TD en attente ; un point
casse est arrete, ou `Running` avec un anneau vide -- c'est-a-dire qu'un
achevement s'est perdu.

# Ce qui est verifie

1. Chaque point retient la date de son dernier evenement.
2. Un chien de garde regarde l'ETAT du point, et pas seulement son silence.
3. Il distingue le repos de la panne par l'etat ET par le pointeur de
   defilement -- sans quoi il relancerait une souris immobile sans fin.
4. Il est borne en nombre de reprises.
5. Sa patience est plus courte que celle de l'enumeration : il tourne le
   verrou du pilote tenu.
6. Le pont EP0 accepte de reprendre un point qui a parle puis s'est tu.
7. La date et le drapeau sont remis a zero des que le point reparle.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"


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


def _bloc(source, ouvrante):
    """Le texte de `{` a son `}` correspondant, accolades comprises."""
    profondeur = 0
    for i in range(ouvrante, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
    return None


def chaines_de_continue(gate, corps_gate):
    """Les conditions qui gouvernent chaque `continue` d'une branche.

    Rend une liste de chaines de caracteres : pour chaque `continue` trouve,
    la concatenation de la condition de la porte et de celles de tous les `if`
    encore ouverts au-dessus de lui.

    On lit les accolades plutot qu'une expression reguliere. C'est ce qui
    permet de distinguer un `continue` qui appartient a un test interne -- donc
    conditionne par lui -- d'un `continue` pose a plat dans la branche, qui
    epargne TOUT ce qui entre dedans. Les deux se ressemblent dans un journal
    et n'ont pas du tout le meme effet sur un transport mort.
    """
    chaines = []
    pile = [gate]
    attente = None
    i = 0
    while i < len(corps_gate):
        reste = corps_gate[i:]
        m = re.match(r"\bif\b([^{;]*)\{", reste)
        if m:
            attente = m.group(1)
            pile.append(attente)
            i += m.end()
            continue
        m = re.match(r"\belse\b\s*\{", reste)
        if m:
            # La branche `else` depend de la meme condition, niee. Le NOM y
            # figure : c'est ce qui compte ici.
            pile.append("else " + (attente or ""))
            i += m.end()
            continue
        c = corps_gate[i]
        if c == "{":
            pile.append("")
            i += 1
            continue
        if c == "}":
            if len(pile) > 1:
                pile.pop()
            i += 1
            continue
        if corps_gate.startswith("continue", i):
            chaines.append(" ".join(pile))
            i += len("continue")
            continue
        i += 1
    return chaines


def conditions_if(source):
    """Les conditions de tous les `if` du texte donne.

    La chaine de format d'une macro contient des accolades et son nom un point
    d'exclamation : lire les conditions plutot que le texte est ce qui empeche
    de prendre une ligne de journal pour une decision.
    """
    return [m.group(1) for m in re.finditer(r'\bif\b([^{;"]*)\{', source)]


PREUVES = ("interrupt_in_casse", "sentinelle_activite", "activite", "file_vide")


def portes_sur_running(source):
    """Les portes `if ... EP_ETAT_RUNNING ... {` du texte donne.

    Rend, pour chacune, son corps et les chaines de conditions qui gouvernent
    ses `continue`.
    """
    portes = []
    for m in re.finditer(r"\bif\b([^{;]*EP_ETAT_RUNNING[^{;]*)\{", source):
        corps_porte = _bloc(source, m.end() - 1)
        if corps_porte is None:
            continue
        portes.append((corps_porte, chaines_de_continue(m.group(1), corps_porte)))
    return portes

def main():
    if not XHCI.exists():
        print("  - fichier absent : %s" % XHCI)
        return 1
    source = XHCI.read_text(encoding="utf-8")
    fautes = []

    # --- 1. la date du dernier evenement existe et est POSEE ----------------
    if "dernier_evenement_ns" not in source:
        fautes.append(
            "xhci_active.rs : les points de terminaison ne retiennent plus la "
            "date de leur dernier evenement. Sans elle, un point mort et un "
            "point au repos sont indiscernables -- c'est exactement ce qui a "
            "laisse la souris morte pendant quatre minutes le 13 septembre."
        )
    else:
        evenement = corps(source, "fn process_hid_event(")
        if evenement is None:
            fautes.append("xhci_active.rs : process_hid_event a disparu.")
        elif "dernier_evenement_ns =" not in evenement:
            fautes.append(
                "xhci_active.rs : la date du dernier evenement n'est plus "
                "posee a la reception. Le chien de garde croirait le point "
                "muet depuis toujours et le relancerait sans fin."
            )
        elif "interrupt_in_casse = false" not in evenement:
            fautes.append(
                "xhci_active.rs : un point qui REPARLE ne reprend plus son "
                "transport Interrupt-IN ; il resterait sur le pont EP0 pour "
                "le reste de la session."
            )

    # --- 2, 3, 4. le chien de garde -----------------------------------------
    veille = corps(source, "fn veille_points_hid(")
    if veille is None:
        fautes.append(
            "xhci_active.rs : le chien de garde des points HID a disparu. Un "
            "point de terminaison qui s'arrete ne serait plus jamais remis en "
            "marche, et le pointeur resterait mort."
        )
    else:
        if "etat_point_hid(" not in veille:
            fautes.append(
                "xhci_active.rs : le chien de garde ne lit plus l'ETAT du "
                "point. Le silence seul ne dit rien : une souris immobile se "
                "tait aussi."
            )
        # LA CONDITION QUI DISTINGUE LE REPOS DE LA PANNE.
        #
        # Sans elle, chaque souris immobile serait rearmee toutes les trois
        # cents millisecondes, et chaque rearmement pose un TD de plus.
        #
        # LA PROPRIETE, ET PAS LA FORME QUI LA PORTAIT.
        #
        # Deux versions de cette regle ont echoue, chacune un cran plus loin.
        # La premiere cherchait la PRESENCE de `EP_ETAT_RUNNING` et de
        # `file_vide` dans la fonction : les deux noms survivent dans la ligne
        # de journal, donc la suppression du test passait. La seconde exigeait
        # l'expression litterale `etat == EP_ETAT_RUNNING && !file_vide {
        # continue` -- et elle a casse le 19 septembre sur un changement qui
        # RENFORCE le pilote.
        #
        # Ce changement vaut d'etre lu, parce qu'il explique la forme de la
        # regle ci-dessous. Distinguer une souris immobile d'un transport mort
        # par le seul `file_vide` est une DEDUCTION : « il reste un TD en
        # attente, donc le materiel a encore du travail, donc il est vivant ».
        # Elle est juste la plupart du temps et fausse exactement dans le cas
        # qui nous interesse -- un achevement perdu laisse aussi un TD en
        # attente. Le pilote demande maintenant une PREUVE : il interroge le
        # meme peripherique par EP0, et ce n'est que si EP0 rend une entree
        # reelle -- le peripherique parle, mais plus par son transport
        # Interrupt-IN -- qu'il declare le transport casse et bascule dessus.
        # Tant que cette preuve n'est pas venue, le point est epargne.
        #
        # La regle verifie donc ce qui doit rester vrai sous n'importe quelle
        # forme : AVANT toute reprise, il existe une porte qui epargne un point
        # `Running` dont le transport n'est pas prouve casse.
        recuperations = [
            veille.find("recupere_point_hid("),
            veille.find("reprises_silence = ep.reprises_silence"),
            veille.find("reprises_silence.saturating_add"),
        ]
        recuperations = [i for i in recuperations if i >= 0]
        amont = veille[: min(recuperations)] if recuperations else veille
        if not recuperations:
            fautes.append(
                "xhci_active.rs : le chien de garde ne reprend plus aucun "
                "point. Un transport mort le resterait."
            )

        portes = portes_sur_running(amont)
        corps_portes = [corps_porte for corps_porte, _ in portes]
        chaines = [chaine for _, liste in portes for chaine in liste]
        epargne_conditionnelle = any(
            any(preuve in chaine for preuve in PREUVES) for chaine in chaines
        )
        epargne_aveugle = any(
            not any(preuve in chaine for preuve in PREUVES) for chaine in chaines
        )

        if not epargne_conditionnelle:
            fautes.append(
                "xhci_active.rs : le chien de garde ne distingue plus le repos "
                "de la panne. Une souris immobile est `Running` avec un TD en "
                "attente : la relancer remplirait son anneau de TD jamais "
                "consommes."
            )
        if epargne_aveugle:
            fautes.append(
                "xhci_active.rs : un point `Running` est epargne SANS condition. "
                "Un transport Interrupt-IN mort est `Running` lui aussi : il ne "
                "serait alors jamais repris, ni bascule sur le pont EP0, et le "
                "pointeur resterait mort -- le defaut du 13 septembre."
            )

        # L'EPARGNE DOIT POUVOIR PRENDRE FIN.
        #
        # Une porte qui epargne un point suspect n'est utile que si le soupcon
        # peut se changer en verdict. Sans cette regle, la garde acceptait une
        # version ou le point suspect etait epargne a CHAQUE tour et ou rien ne
        # posait jamais `interrupt_in_casse` : la condition de la porte nommait
        # bien la preuve, mais la preuve n'arrivait jamais. Le transport mort
        # n'etait alors ni repris ni bascule sur EP0 -- c'est-a-dire exactement
        # la souris morte du 13 septembre, avec une condition en plus.
        #
        # La confirmation doit vivre DANS une porte sur `Running` : c'est la
        # que le point est epargne, et c'est donc la seule place d'ou l'epargne
        # peut cesser.
        if not any("interrupt_in_casse = true" in corps_porte for corps_porte in corps_portes):
            fautes.append(
                "xhci_active.rs : aucune porte sur `Running` ne CONFIRME la "
                "panne d'un transport. Un point suspect serait epargne a chaque "
                "tour, pour toujours : le pointeur resterait mort et le pont "
                "EP0 ne prendrait jamais le relais."
            )

        # `file_vide` DECIDE, il n'est pas seulement imprime.
        #
        # Un point `Running` dont l'anneau est vide n'est pas au repos : il a
        # perdu un achevement, et celui-la doit etre repris tout de suite. Sans
        # ce test, il serait confondu avec la souris immobile et epargne.
        #
        # On lit les CONDITIONS, et non le texte brut. Une version de cette
        # regle cherchait `file_vide` dans toute expression suivie d'une
        # accolade : elle trouvait la ligne de journal, ou `serial_println!`
        # fournit le point d'exclamation et le format fournit l'accolade. Elle
        # affirmait alors « file_vide decide » d'un fichier ou il ne decidait
        # plus rien.
        if not any("file_vide" in condition for condition in conditions_if(amont)):
            fautes.append(
                "xhci_active.rs : `file_vide` ne decide plus de rien. Un point "
                "`Running` dont l'anneau est vide a perdu un achevement ; sans "
                "ce test il serait confondu avec un point au repos et jamais "
                "repris."
            )

        if "REPRISES_SILENCE_MAX" not in veille:
            fautes.append(
                "xhci_active.rs : les reprises du chien de garde ne sont plus "
                "bornees."
            )

    m = re.search(r"const SILENCE_HID_NS: u64 = ([0-9_]+);", source)
    if m is None:
        fautes.append("xhci_active.rs : le seuil de silence n'est plus lisible.")
    else:
        silence = int(m.group(1).replace("_", ""))
        if silence < 50_000_000:
            fautes.append(
                "xhci_active.rs : le seuil de silence est descendu sous "
                "cinquante millisecondes ; il declencherait sur le creux "
                "normal entre deux rapports."
            )
        if silence > 2_000_000_000:
            fautes.append(
                "xhci_active.rs : le seuil de silence depasse deux secondes. "
                "Un pointeur mort pendant deux secondes est un pointeur mort."
            )

    # --- 5. la patience du chien de garde -----------------------------------
    reprise = corps(source, "fn recupere_point_hid(")
    if reprise is None:
        fautes.append("xhci_active.rs : recupere_point_hid a disparu.")
    elif "BUDGET_REPRISE_NS" not in reprise:
        fautes.append(
            "xhci_active.rs : la reprise d'un point reprend la patience de "
            "l'enumeration. Elle tourne le verrou du pilote tenu : une demi-"
            "seconde par tentative ratee, c'est l'entree gelee -- le mal "
            "qu'elle est censee guerir."
        )
    m = re.search(r"const BUDGET_REPRISE_NS: u64 = ([0-9_]+);", source)
    if m is not None and int(m.group(1).replace("_", "")) > 100_000_000:
        fautes.append(
            "xhci_active.rs : la patience d'une reprise depasse cent "
            "millisecondes, verrou du pilote tenu."
        )

    # --- 6. le pont EP0 reprend un point casse ------------------------------
    pont = corps(source, "fn repli_ep0_un_point(")
    if pont is None:
        fautes.append("xhci_active.rs : repli_ep0_un_point a disparu.")
    elif re.search(r"!\s*controller\.hids\[[^\]]+\]\.interrupt_in_casse", pont) is None:
        fautes.append(
            "xhci_active.rs : le pont EP0 refuse de nouveau d'aider un point "
            "qui a parle puis s'est tu -- c'est-a-dire precisement le "
            "peripherique qui avait PROUVE qu'il fonctionnait."
        )

    if fautes:
        print("veille des points HID : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "veille des points HID : silence date, etat verifie, repos distingue "
        "de la panne, reprises bornees et courtes, pont EP0 en dernier recours"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
