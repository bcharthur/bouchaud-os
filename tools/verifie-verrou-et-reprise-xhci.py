#!/usr/bin/env python3
"""Garde-fou : le verrou du pilote xHCI se rend seul, et le transport se reprend.

# Les trois defauts que ce lot corrige

1. `RUNTIME_BUSY` etait pris et rendu A LA MAIN, sur six chemins. Entre la
   prise et le `store(false)`, chacun contient des `return`. Chaque `return`
   oublie tenait le controleur POUR TOUJOURS -- et la machine paraissait avoir
   perdu son clavier, son stockage et son enregistreur d'un seul coup.

2. Un seul budget d'attente de cinq cents millisecondes servait partout. Au
   runtime, une commande de stockage qui echoue tient le verrou pendant une
   demi-seconde PAR TRANSFERT -- trois par commande --, et le clavier saute
   mille cinq cents tours. C'est la forme exacte de « la souris met trop de
   temps a se deplacer » avec le processeur a 22 % : la machine n'etait pas
   saturee, elle ATTENDAIT.

3. Le transport BOT n'avait pas d'etat. Une commande partait comme si la
   precedente s'etait bien terminee, y compris quand elle avait EXPIRE -- en
   laissant un TD poste, que le peripherique peut achever plus tard et dont
   l'evenement sera pris pour la reponse de la commande suivante. C'est la
   seule classe de defaut de ce pilote qui puisse rendre le contenu d'un autre
   secteur AVEC un statut valide.

Ces trois-la sont lies : raccourcir le budget multiplie les sorties
anticipees (1) et les abandons (3). Aucun des trois correctifs n'est sur sans
les deux autres.

# Ce qui est verifie ici

1. Les modules `proprietaire_runtime.rs` et `reprise_bot.rs` restent PURS.
2. `RUNTIME_BUSY` n'est plus rendu a la main : `store(false)` n'apparait que
   dans `Drop`, qui est pose par le compilateur sur TOUS les chemins de sortie.
3. Les budgets restent SEPARES, et celui du runtime reste tres inferieur a
   celui de l'enumeration.
4. Les transferts Bulk du stockage utilisent le budget du runtime, pas celui
   de l'enumeration.
5. Toute entree-sortie de masse passe par `autorise_es()`.
6. La reprise choisit sa commande selon l'ETAT du point : `Reset Endpoint`
   sur un point arrete, `Stop Endpoint` sur un point en marche. Le banc l'a
   montre : confondre les deux donne trois echecs de reprise, un transport
   hors service, et pas un enregistrement pose.
7. L'injection de panne existe, et chaque bit est CONSOMME -- une injection
   permanente empecherait de verifier la reprise.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

PROPRIETAIRE = RACINE / "src/drivers/usb/proprietaire_runtime.rs"
REPRISE = RACINE / "src/drivers/usb/reprise_bot.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
STOCKAGE = RACINE / "src/drivers/usb/blackbox_storage.rs"


def code_seul(source):
    sans_blocs = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", ligne) for ligne in sans_blocs.splitlines())


def corps(source, signature):
    debut = source.find(signature)
    if debut < 0:
        return None
    reste = source[debut + len(signature):]
    fin = re.search(r"\n(?:pub )?(?:const |static |fn |struct |impl |enum )", reste)
    return reste[: fin.start()] if fin else reste


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    proprietaire = lit(PROPRIETAIRE, fautes)
    reprise = lit(REPRISE, fautes)
    xhci = lit(XHCI, fautes)
    stockage = lit(STOCKAGE, fautes)

    # 1. La purete des deux modules de decision.
    for nom, source in (("proprietaire_runtime.rs", proprietaire), ("reprise_bot.rs", reprise)):
        if source is None:
            continue
        pur = code_seul(source)
        if "use crate::" in pur:
            fautes.append(
                "%s depend du reste du noyau : il ne se compile plus seul avec "
                "`rustc --test`, et ses regles redeviennent invrifiables." % nom
            )
        if "unsafe" in pur:
            fautes.append("%s contient du `unsafe` : cette decision se raisonne sans pointeur." % nom)
        if "monotonic_ns" in pur:
            fautes.append("%s lit l'horloge au lieu de la recevoir en parametre." % nom)

    if xhci is not None:
        pur = code_seul(xhci)

        # 2. Le verrou se rend par Drop, et seulement par Drop.
        rendus = re.findall(r"RUNTIME_BUSY\.store\(false", pur)
        drop_jeton = corps(pur, "impl Drop for Jeton {")
        if drop_jeton is None:
            fautes.append(
                "xhci_active.rs : le `Drop` du jeton a disparu. Le verrou "
                "redevient rendu par une instruction qu'on peut oublier -- et "
                "chaque `return` oublie tient le controleur pour toujours."
            )
        elif "RUNTIME_BUSY.store(false" not in drop_jeton:
            fautes.append("xhci_active.rs : le `Drop` du jeton ne rend plus le verrou.")
        if len(rendus) != 1:
            fautes.append(
                "xhci_active.rs : `RUNTIME_BUSY.store(false)` apparait %d fois. "
                "Il ne doit exister QUE dans `Drop` : ailleurs, c'est un "
                "chemin de sortie qui peut l'oublier." % len(rendus)
            )
        if "RUNTIME_BUSY.swap(true" in pur:
            fautes.append(
                "xhci_active.rs : une prise directe de `RUNTIME_BUSY` a "
                "reapparu. Elle ne passe pas par le jeton, donc rien ne "
                "garantit qu'elle sera rendue."
            )
        if stockage is not None and "RUNTIME_BUSY" in code_seul(stockage):
            fautes.append(
                "blackbox_storage.rs : une prise directe de `RUNTIME_BUSY` a "
                "reapparu. Le vidage doit passer par le jeton."
            )

        # 3. Les budgets restent separes.
        budgets = {}
        for nom in ("BUDGET_ENUMERATION_NS", "BUDGET_SCRUTATION_NS", "BUDGET_RUNTIME_BOT_NS"):
            trouve = re.search(r"const %s: u64 = ([0-9_]+);" % nom, pur)
            if not trouve:
                fautes.append(
                    "xhci_active.rs : `%s` a disparu. Un seul budget pour "
                    "trois chemins, c'est appliquer la patience d'un bring-up "
                    "a un clavier qui attend." % nom
                )
            else:
                budgets[nom] = int(trouve.group(1).replace("_", ""))
        if len(budgets) == 3:
            if budgets["BUDGET_RUNTIME_BOT_NS"] >= budgets["BUDGET_ENUMERATION_NS"]:
                fautes.append(
                    "xhci_active.rs : le budget du transport de masse au "
                    "runtime (%d ns) n'est plus inferieur a celui de "
                    "l'enumeration (%d ns). Cinq cents millisecondes sont "
                    "acceptables au bring-up et nulle part ailleurs."
                    % (budgets["BUDGET_RUNTIME_BOT_NS"], budgets["BUDGET_ENUMERATION_NS"])
                )
            if budgets["BUDGET_SCRUTATION_NS"] >= budgets["BUDGET_RUNTIME_BOT_NS"]:
                fautes.append(
                    "xhci_active.rs : la scrutation HID n'est plus le chemin "
                    "le moins patient. Un peripherique HID qui ne repond pas "
                    "en quelques millisecondes ne repondra pas."
                )

        # 4. Les transferts Bulk paient le budget du runtime.
        bulk = corps(pur, "fn bulk_transfert(")
        if bulk is None:
            fautes.append("xhci_active.rs : `bulk_transfert` est introuvable.")
        elif "budget_bot()" not in bulk:
            fautes.append(
                "xhci_active.rs : `bulk_transfert` n'utilise plus "
                "`budget_bot()`. Trois transferts par commande a une "
                "demi-seconde chacun gelaient l'entree pendant une seconde et "
                "demie."
            )

        # 5. Aucune entree-sortie de masse sans autorisation du transport.
        commande = corps(pur, "fn bot_commande(")
        if commande is None:
            fautes.append("xhci_active.rs : `bot_commande` est introuvable.")
        elif commande.find("autorise_es()") < 0 or commande.find("autorise_es()") > commande.find(
            "bot_commande_une_fois"
        ):
            # ANCRAGE SUR LA PREMIERE COMMANDE, PAS SUR LA PRESENCE DU JETON.
            #
            # `bot_commande` interroge l'etat DEUX fois : avant la premiere
            # tentative, et avant de rejouer apres une reprise. Chercher le
            # jeton n'importe ou se satisferait de la seconde -- c'est-a-dire
            # d'un code qui laisse partir la premiere commande sur un
            # transport casse, ce qui est exactement le defaut.
            fautes.append(
                "xhci_active.rs : `bot_commande` n'interroge plus l'etat du "
                "transport. Une commande qui part apres une echeance prendrait "
                "l'achevement tardif du TD abandonne pour le sien -- et "
                "rendrait le contenu d'un autre secteur avec un CSW valide."
            )

        # 6. La reprise choisit sa commande selon l'etat du point.
        if "const CMD_STOP_ENDPOINT" not in pur:
            fautes.append(
                "xhci_active.rs : `CMD_STOP_ENDPOINT` a disparu. `Reset "
                "Endpoint` ne s'applique qu'a un point ARRETE ; sur un point "
                "en marche -- ce qu'est un point dont le transfert vient "
                "d'expirer -- il repond « Context State Error ». Le banc l'a "
                "montre : trois tentatives, trois echecs, transport hors "
                "service, pas un enregistrement pose."
            )
        for nom, fonction in (
            ("xhci_active.rs", corps(pur, "fn recupere_point_bulk(")),
            (
                "blackbox_storage.rs",
                corps(code_seul(stockage), "fn blackbox_debloque_point(") if stockage else None,
            ),
        ):
            if fonction is None:
                continue
            if "EP_ETAT_RUNNING" not in fonction or "CMD_STOP_ENDPOINT" not in fonction:
                fautes.append(
                    "%s : la reprise n'arrete plus un point EN MARCHE. Le TD "
                    "abandonne resterait poste, et son achevement tardif "
                    "reviendrait sous la commande suivante." % nom
                )
            if "EP_ETAT_HALTED" not in fonction or "CMD_RESET_ENDPOINT" not in fonction:
                fautes.append(
                    "%s : la reprise ne reinitialise plus un point ARRETE." % nom
                )

        # 7. L'injection existe et se consomme.
        injecte = corps(pur, "fn injecte(bit: u32) -> bool {")
        if injecte is None:
            fautes.append(
                "xhci_active.rs : l'injection de panne a disparu. Une echeance "
                "de stockage, un CSW qui n'arrive pas et un verrou deja tenu "
                "ne se fabriquent pas sur commande avec une vraie cle, et sont "
                "pourtant exactement ce qu'il faut prouver."
            )
        elif "fetch_and(!bit" not in injecte:
            fautes.append(
                "xhci_active.rs : l'injection n'est plus CONSOMMEE. Une panne "
                "permanente ne prouve rien de plus et empeche de verifier la "
                "reprise qui doit la suivre."
            )

    if fautes:
        for faute in fautes:
            print("FAUTE: %s" % faute)
        return 1
    print(
        "verrou xHCI : rendu par Drop, budgets separes, transport BOT avec "
        "etat et reprise selon l'etat du point, panne injectable."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
