#!/usr/bin/env python3
"""Ce qu'un pilote NVMe ne doit jamais redevenir.

Les fautes que ce garde-fou refuse ont toutes la meme forme : elles marchent
sur le materiel qui sert a mettre au point, et sont fausses sur le materiel
qu'on ne possede pas -- ou pire, fausses en ECRITURE, donc silencieuses
jusqu'a ce qu'un systeme de fichiers se corrompe loin de la cause.

Chaque regle a ete mutee : on a casse le code exprès, verifie que la regle
devenait rouge, puis remis le code en etat.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PILOTE = RACINE / "src/drivers/block/nvme.rs"
DECODAGE = RACINE / "src/drivers/block/nvme/decodage.rs"
TEST = RACINE / "tools/platform/test_nvme_decodage.rs"
MINUTERIE = RACINE / "src/kernel/time/timer.rs"


def sans_commentaires(source):
    """Le texte prive de ses commentaires de ligne.

    Une regle satisfaite par une PHRASE et non par du code ne protege rien :
    elle reste verte quand le code part et que le commentaire reste.
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
    """Le corps de la FONCTION dont la signature est donnee.

    Une signature peut apparaitre d'abord comme DECLARATION anticipee. On les
    distingue par le point-virgule qui la termine -- mais SEULEMENT celui qui
    est a profondeur nulle : `-> [u8; 16]` en contient un, et une regle qui le
    prenait pour une fin de declaration cherchait la fonction suivante sans le
    dire, puis rapportait « introuvable » sur une fonction bien presente.
    """
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        # On repart du DEBUT de la signature : certaines « signatures »
        # recherchees incluent deja leur accolade (`struct Champ {`), et
        # repartir apres elle ferait manquer la seule accolade qui compte.
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


def regle_foulee_sonnette(pilote, decodage, fautes):
    """La foulee des sonnettes vient de CAP, jamais d'une constante.

    `DSTRD` vaut zero sur QEMU et sur presque tout le materiel. Un pilote qui
    code la foulee en dur marche partout ou on le teste et sonne dans le vide
    sur le premier controleur qui annonce autre chose : la commande reste dans
    la file, et le disque parait mort.
    """
    if "foulee_sonnette" not in decodage:
        fautes.append(
            "decodage.rs : la foulee des sonnettes n'est plus derivee de CAP."
        )
    corps_decalage = corps(decodage, "pub const fn decalage_sonnette(")
    if corps_decalage is None:
        fautes.append("decodage.rs : decalage_sonnette introuvable.")
    elif "foulee" not in corps_decalage:
        fautes.append(
            "decodage.rs : decalage_sonnette n'utilise plus la foulee annoncee."
        )
    for appel in re.findall(r"decalage_sonnette\([^)]*\)", pilote):
        if "foulee_sonnette" not in appel:
            fautes.append(
                "nvme.rs : une sonnette est calculee sans la foulee de CAP (%s)."
                % appel.strip()
            )


def regle_blocs_decales(decodage, fautes):
    """Le compte de blocs d'un transfert est decale de un.

    Zero veut dire « un bloc ». Ecrire le compte reel lit un bloc de trop --
    jete en lecture, mais en ECRITURE il ecrase le bloc suivant, et la
    corruption apparait ailleurs, plus tard.
    """
    bloc = corps(decodage, "pub const fn commande_transfert(")
    if bloc is None:
        fautes.append("decodage.rs : commande_transfert introuvable.")
        return
    if "saturating_sub(1)" not in bloc and "- 1" not in bloc:
        fautes.append(
            "decodage.rs : commande_transfert n'ecrit plus NLB decale de un ; "
            "chaque ecriture ecrasera le bloc suivant."
        )


def regle_liste_prp(decodage, pilote, fautes):
    """Au-dela de deux pages, PRP2 porte une LISTE et non une page.

    La frontiere est a deux pages exactement, et c'est le DECALAGE du tampon
    qui decide : 8 Kio alignes tiennent en deux pages, les memes 8 Kio decales
    d'un octet en occupent trois.
    """
    bloc = corps(decodage, "pub const fn plan_prp(")
    if bloc is None:
        fautes.append("decodage.rs : plan_prp introuvable.")
        return
    for variante in ("UnePage", "DeuxPages", "Liste"):
        if variante not in bloc:
            fautes.append(
                "decodage.rs : plan_prp ne rend plus le cas %s." % variante
            )
    # Le decalage doit etre DERIVE de l'adresse. Un `let decalage = 0;`
    # garderait le mot et perdrait la propriete : c'est exactement la mutation
    # qu'une regle par simple presence laissait passer.
    derivation = re.search(r"let\s+decalage\s*=\s*([^;]+);", bloc)
    if derivation is None:
        fautes.append("decodage.rs : plan_prp ne calcule plus de decalage.")
    elif "physique" not in derivation.group(1) or "page" not in derivation.group(1):
        fautes.append(
            "decodage.rs : plan_prp ne derive plus le decalage de l'adresse "
            "reelle ; un tampon non aligne basculera dans le mauvais cas."
        )
    prp = corps(pilote, "unsafe fn prp(&self, octets: usize)")
    if prp is None:
        fautes.append("nvme.rs : la preparation des PRP a disparu.")
    elif "Prp::Liste" not in prp or "liste_phys" not in prp:
        fautes.append(
            "nvme.rs : le cas « liste PRP » n'ecrit plus une liste ; un "
            "transfert de plus de deux pages ecrira n'importe ou."
        )


def regle_taille_de_bloc(decodage, pilote, fautes):
    """La taille de bloc vient du namespace, pas d'une supposition.

    512 est vrai sur beaucoup de disques et faux sur beaucoup d'autres. Un
    pilote qui suppose 512 sur un disque en 4096 lit le bon nombre d'octets
    huit fois trop loin, et ne s'en apercoit jamais tout seul.
    """
    bloc = corps(decodage, "pub fn format_bloc(")
    if bloc is None:
        fautes.append("decodage.rs : format_bloc introuvable.")
        return
    if "flbas" not in bloc.lower():
        fautes.append("decodage.rs : format_bloc ne lit plus FLBAS.")
    if "1usize <<" not in bloc:
        fautes.append(
            "decodage.rs : LBADS est un LOGARITHME ; format_bloc ne l'eleve "
            "plus en puissance de deux."
        )
    # Le refus doit porter sur LBADS lui-meme. Chercher `None` ne prouve rien :
    # la fonction en rend deja un pour un tampon tronque, et retirer le
    # controle de validite laissait la regle verte.
    if not re.search(r"lbads\s*\)|\(\s*&\s*lbads|lbads\s*<|lbads\s*>", bloc):
        fautes.append(
            "decodage.rs : format_bloc ne verifie plus que LBADS designe une "
            "taille de bloc utilisable ; un champ non initialise passerait "
            "pour un disque en 512 octets."
        )
    if "nlbaf" not in bloc.lower():
        fautes.append(
            "decodage.rs : format_bloc ne verifie plus que FLBAS designe un "
            "format qui existe."
        )
    if "e.format.taille_bloc" not in pilote:
        fautes.append(
            "nvme.rs : le descripteur du volume n'annonce plus la taille de "
            "bloc lue sur le disque."
        )


def regle_bornes(pilote, fautes):
    """Une requete est bornee par le disque ET par le tampon de l'appelant.

    Hors du disque, le controleur refuserait proprement. Hors du tampon, c'est
    nous qui ecrivons au-dela, et personne ne refuse rien.
    """
    bloc = corps(pilote, "fn bornes_valides(")
    if bloc is None:
        fautes.append("nvme.rs : le controle de bornes a disparu.")
        return
    if "etat.blocs" not in bloc:
        fautes.append("nvme.rs : bornes_valides ne borne plus par le disque.")
    if "octets" not in bloc:
        fautes.append(
            "nvme.rs : bornes_valides ne borne plus par le tampon de "
            "l'appelant ; un depassement ecrira dans la memoire du noyau."
        )
    if "checked_add" not in bloc or "checked_mul" not in bloc:
        fautes.append(
            "nvme.rs : bornes_valides calcule sans garde de debordement ; un "
            "LBA proche du maximum repasserait sous la capacite."
        )
    for nom in ("fn soumet(", "fn soumet_ecriture("):
        corps_soumet = corps(pilote, nom)
        if corps_soumet is None or "bornes_valides" not in corps_soumet:
            fautes.append("nvme.rs : %s ne verifie plus ses bornes." % nom)


def regle_vidange_honnete(pilote, fautes):
    """Le pilote ne declare une barriere que s'il en a une.

    `api::bloc::vidange` croise le resultat de la commande avec
    `vidange_reelle` pour dire a l'appelant s'il a vraiment une barriere. Un
    commit A/B qui croit avoir une barriere qu'il n'a pas est pire qu'un commit
    qui sait qu'il n'en a pas.
    """
    if "vidange_reelle" not in pilote:
        fautes.append(
            "nvme.rs : le pilote n'annonce plus s'il a un cache volatil."
        )
    if "commande_vidange" not in pilote:
        fautes.append("nvme.rs : la commande de vidange n'est plus emise.")
    bloc = corps(pilote, "fn soumet_ecriture(")
    if bloc is not None and "Genre::Vidange" not in bloc:
        fautes.append(
            "nvme.rs : une vidange n'est plus distinguee d'une ecriture."
        )


def regle_attente_hors_irq(pilote, minuterie, fautes):
    """L'attente est BORNEE, et sa borne ne depend d'aucune IRQ.

    Le pilote tourne avec les interruptions masquees. Une attente qui
    compterait des ticks livres par IRQ n'expirerait jamais : le noyau
    tournerait en rond pour toujours au lieu de rapporter un delai.

    Compter du TSC ne suffit pas non plus, parce que le TSC peut ne pas etre
    calibre -- CPUID muet, canal 2 du PIT absent -- et `monotonic_ns` retombe
    alors sur les ticks. La regle exige donc les DEUX : l'attente passe par
    `timer::attente_bornee`, et cette primitive porte un garde-fou en cycles
    `rdtsc` qui ne depend ni d'une IRQ ni d'une calibration reussie.
    """
    bloc = corps(pilote, "fn attend(")
    if bloc is None:
        fautes.append("nvme.rs : la fonction d'attente a disparu.")
        return
    if "attente_bornee" not in bloc:
        fautes.append(
            "nvme.rs : l'attente ne passe plus par timer::attente_bornee ; "
            "rien ne garantit plus qu'elle se termine quand les interruptions "
            "sont masquees."
        )
    if "ticks()" in bloc:
        fautes.append(
            "nvme.rs : l'attente compte des ticks PIT, qui n'arrivent pas "
            "quand les interruptions sont masquees."
        )

    primitive = corps(minuterie, "pub fn attente_bornee(")
    if primitive is None:
        fautes.append(
            "timer.rs : attente_bornee a disparu ; le pilote NVMe n'a plus de "
            "borne independante des IRQ."
        )
        return
    if "rdtsc" not in primitive:
        fautes.append(
            "timer.rs : attente_bornee n'a plus de garde-fou en cycles ; une "
            "horloge non calibree la rendrait infinie."
        )
    if "source_monotone_fiable" not in primitive:
        fautes.append(
            "timer.rs : attente_bornee ne verifie plus que l'horloge est "
            "independante des IRQ avant de s'y fier."
        )


def regle_phase(pilote, decodage, fautes):
    """Un achevement se reconnait a sa PHASE, pas a un contenu non nul.

    La file d'achevement n'est jamais remise a zero : c'est le bit de phase,
    qui alterne a chaque tour, qui distingue une entree neuve d'une ancienne.
    Un pilote qui teste « non nul » relit indefiniment le dernier achevement.
    """
    if "pub phase: bool" not in decodage:
        fautes.append("decodage.rs : l'entree d'achevement n'expose plus sa phase.")
    bloc = corps(pilote, "unsafe fn achevement(&self)")
    if bloc is None:
        fautes.append("nvme.rs : la lecture d'un achevement a disparu.")
        return
    if "self.phase" not in bloc:
        fautes.append(
            "nvme.rs : l'achevement n'est plus filtre par la phase attendue."
        )
    avance = corps(pilote, "fn avance(&mut self)")
    if avance is None or "!self.phase" not in avance:
        fautes.append(
            "nvme.rs : la phase ne bascule plus au tour de la file ; a partir "
            "du deuxieme tour, aucun achevement ne sera reconnu."
        )


def regle_interruption_non_armee(decodage, fautes):
    """La creation de file d'achevement laisse IEN a zero.

    Ce pilote scrute. Armer une interruption qu'aucun vecteur ne recoit ne le
    rendrait pas plus rapide : elle ferait monter un IRQ non gere.
    """
    bloc = corps(decodage, "pub const fn commande_cree_cq(")
    if bloc is None:
        fautes.append("decodage.rs : commande_cree_cq introuvable.")
        return
    mots = re.findall(r"sqe\[11\]\s*=\s*([^;]+);", bloc)
    if not mots:
        fautes.append("decodage.rs : commande_cree_cq n'ecrit plus dw11.")
        return
    # Evaluer la constante plutot que d'y chercher un caractere : `0x3` arme
    # IEN sans jamais contenir le chiffre deux, et une regle textuelle le
    # laissait passer.
    litteral = mots[0].strip()
    try:
        valeur = int(litteral, 0)
    except ValueError:
        fautes.append(
            "decodage.rs : dw11 de commande_cree_cq n'est plus une constante "
            "lisible (%s) ; l'etat de IEN n'est plus verifiable." % litteral
        )
        return
    if valeur & 0x2:
        fautes.append(
            "decodage.rs : commande_cree_cq arme IEN (dw11=%s) alors qu'aucun "
            "vecteur d'interruption n'est installe." % litteral
        )
    if not valeur & 0x1:
        fautes.append(
            "decodage.rs : commande_cree_cq n'annonce plus une file "
            "physiquement contigue."
        )


def regle_tampon_de_rebond(pilote, fautes):
    """Le DMA passe par une region dont le pilote connait l'adresse physique.

    `api::bloc` remet des tranches d'octets dont rien, dans la signature, ne
    dit ou elles vivent physiquement. Deduire une adresse physique d'un
    pointeur quelconque marche tant que tous les appelants passent par le tas,
    et ecrit du DMA a une adresse arbitraire le jour ou l'un d'eux n'y passe
    pas -- ce qui ne produit pas une erreur mais une corruption ailleurs.
    """
    if "rebond_phys" not in pilote or "alloc_dma(REBOND_OCTETS)" not in pilote:
        fautes.append(
            "nvme.rs : le tampon de rebond DMA a disparu ; le pilote deduit "
            "desormais une adresse physique d'un pointeur qu'il n'a pas alloue."
        )
    for nom in ("fn soumet(", "fn soumet_ecriture("):
        bloc = corps(pilote, nom)
        if bloc is None:
            continue
        if "rebond_virt" not in bloc and "Vidange" not in bloc:
            fautes.append("nvme.rs : %s ne passe plus par le tampon de rebond." % nom)


def regle_preuve_hote(test, fautes):
    """L'arithmetique reste mise a l'epreuve sur l'hote."""
    for attendu in (
        "une_foulee_de_sonnette_non_nulle_deplace_toutes_les_sonnettes",
        "nlb_est_decale_de_un",
        "le_decalage_et_non_la_taille_fait_basculer_le_plan",
        "un_disque_en_4096_n_est_pas_lu_en_512",
        "un_format_inutilisable_rend_none_plutot_que_de_supposer_512",
    ):
        if attendu not in test:
            fautes.append(
                "test_nvme_decodage.rs : la preuve « %s » a disparu." % attendu
            )


def main():
    fautes = []
    for chemin in (PILOTE, DECODAGE, TEST, MINUTERIE):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    pilote = sans_commentaires(PILOTE.read_text(encoding="utf-8"))
    decodage = sans_commentaires(DECODAGE.read_text(encoding="utf-8"))
    test = TEST.read_text(encoding="utf-8")
    minuterie = sans_commentaires(MINUTERIE.read_text(encoding="utf-8"))

    regle_foulee_sonnette(pilote, decodage, fautes)
    regle_blocs_decales(decodage, fautes)
    regle_liste_prp(decodage, pilote, fautes)
    regle_taille_de_bloc(decodage, pilote, fautes)
    regle_bornes(pilote, fautes)
    regle_vidange_honnete(pilote, fautes)
    regle_attente_hors_irq(pilote, minuterie, fautes)
    regle_phase(pilote, decodage, fautes)
    regle_interruption_non_armee(decodage, fautes)
    regle_tampon_de_rebond(pilote, fautes)
    regle_preuve_hote(test, fautes)

    if fautes:
        print("nvme : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "nvme : foulee lue dans CAP, NLB decale de un, liste PRP au-dela de "
        "deux pages, taille de bloc lue sur le disque, bornes des deux cotes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
