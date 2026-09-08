#!/usr/bin/env python3
"""Ce que l'allocateur compagnon ne doit jamais redevenir.

Un allocateur de memoire a une facon particuliere d'etre faux : il rend une
adresse. L'adresse a l'air bonne. Elle recouvre celle d'un autre, ou elle
designe un bloc dont la seconde moitie n'est pas de la memoire -- et cela se
manifeste ailleurs, plus tard, sous la forme d'une corruption qu'on attribue au
sous-systeme ou elle apparait.

Ces regles protegent les proprietes dont la violation est silencieuse.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
COMPAGNON = RACINE / "src/kernel/memory/compagnon.rs"
DMA = RACINE / "src/kernel/memory/dma_compagnon.rs"
PHYSIQUE = RACINE / "src/kernel/memory/physical.rs"
TEST = RACINE / "tools/platform/test_compagnon.rs"


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


def regle_pur(compagnon, fautes):
    """L'allocateur ne touche pas la memoire physique lui-meme.

    C'est ce qui permet de l'eprouver sur l'hote avec un terrain fait de
    `Vec` -- et donc de verifier ce qu'aucune execution ne montre : que deux
    allocations ne se recouvrent jamais.
    """
    for interdit in ("phys_to_virt", "read_volatile", "write_volatile", "serial_println"):
        if interdit in compagnon:
            fautes.append(
                "compagnon.rs : « %s » y est apparu. L'allocateur doit rester "
                "pur, sinon il redevient intestable." % interdit
            )


def regle_jumeau(compagnon, fautes):
    """Le jumeau est un ou exclusif, pas une recherche."""
    bloc = corps(compagnon, "pub const fn jumeau(")
    if bloc is None:
        fautes.append("compagnon.rs : la fonction jumeau a disparu.")
    elif "^" not in bloc:
        fautes.append(
            "compagnon.rs : le jumeau n'est plus un ou exclusif ; toute "
            "l'economie du compagnon repose sur ce calcul en temps constant."
        )
    pere = corps(compagnon, "pub const fn pere(")
    if pere is None or "&" not in pere:
        fautes.append("compagnon.rs : le calcul du pere a disparu ou a change de nature.")


def regle_fusion(compagnon, fautes):
    """La fusion ne fabrique pas de bloc a moitie imaginaire.

    Sur une zone dont la taille n'est pas une puissance de deux, le jumeau du
    dernier bloc DEBORDE. Le fusionner rendrait un bloc dont la seconde moitie
    n'est pas de la memoire -- et l'appelant y ecrirait.
    """
    bloc = corps(compagnon, "pub fn rend<")
    if bloc is None:
        fautes.append("compagnon.rs : rend() introuvable.")
        return
    if "compagnon + (1usize << ordre) > self.blocs" not in bloc:
        fautes.append(
            "compagnon.rs : la fusion ne verifie plus que le jumeau tient "
            "ENTIEREMENT dans la zone ; sur une zone qui n'est pas une "
            "puissance de deux, elle fabriquerait un bloc a moitie imaginaire."
        )
    if "self.est_libre(bloc, ordre)" not in bloc:
        fautes.append(
            "compagnon.rs : une double liberation n'est plus refusee ; elle "
            "corromprait les listes."
        )
    if "aligne(bloc, ordre)" not in bloc:
        fautes.append(
            "compagnon.rs : un bloc mal aligne est accepte ; il fusionnerait "
            "avec un jumeau qui n'existe pas."
        )
    # Le comptage : ce qu'une liberation ajoute est la taille du bloc RENDU.
    if "let ajout = 1usize << ordre;" not in bloc:
        fautes.append(
            "compagnon.rs : la liberation compte la taille APRES fusion ; les "
            "moities deja rendues seraient comptees deux fois, et "
            "l'allocateur croirait avoir plusieurs fois la memoire qu'il a."
        )


def regle_division(compagnon, fautes):
    bloc = corps(compagnon, "pub fn prend<")
    if bloc is None:
        fautes.append("compagnon.rs : prend() introuvable.")
        return
    if "while source > ordre" not in bloc:
        fautes.append(
            "compagnon.rs : la division a disparu ; une demande d'une page "
            "serait servie par un bloc de quatre mebioctets."
        )
    if "self.empile(terrain, haut, source)" not in bloc:
        fautes.append(
            "compagnon.rs : la moitie haute d'une division n'est plus rendue ; "
            "elle serait perdue a chaque allocation."
        )


def regle_parcours_borne(compagnon, fautes):
    """Aucune boucle sur une liste ne peut tourner indefiniment.

    Une liste circulaire -- memoire abimee, double liberation qui a echappe au
    bitmap -- figerait le noyau au lieu de rapporter une erreur.
    """
    for nom in ("fn retire<", "pub fn compte_a_l_ordre<"):
        bloc = corps(compagnon, nom)
        if bloc is None:
            fautes.append("compagnon.rs : %s introuvable." % nom)
        elif "self.blocs" not in bloc:
            fautes.append(
                "compagnon.rs : %s parcourt sans borne ; une liste circulaire "
                "figerait le noyau." % nom
            )


def regle_bitmap(compagnon, fautes):
    bloc = corps(compagnon, "pub fn neuf(")
    if bloc is None:
        fautes.append("compagnon.rs : neuf() introuvable.")
        return
    if "mots_bitmap(blocs)" not in bloc:
        fautes.append(
            "compagnon.rs : un bitmap trop court n'est plus refuse ; il ferait "
            "lire l'etat d'un bloc hors zone, donc une reponse plausible et "
            "fausse."
        )
    if "libre.fill(0)" not in bloc:
        fautes.append(
            "compagnon.rs : le bitmap n'est plus remis a zero ; d'anciens bits "
            "feraient fusionner avec des blocs qui ne sont pas libres."
        )


def regle_un_seul_allocateur(physique, fautes):
    """L'arene et le compagnon ne sont JAMAIS configures tous les deux.

    Deux allocateurs sur la meme memoire distribuent les memes pages, et aucun
    des deux ne peut le detecter puisque chacun se croit seul.
    """
    bloc = corps(physique, "fn configure_dma(")
    if bloc is None:
        fautes.append(
            "physical.rs : le choix entre le compagnon et l'arene a disparu."
        )
        return
    if "return;" not in bloc:
        fautes.append(
            "physical.rs : configure_dma ne s'arrete plus quand le compagnon "
            "accepte ; l'arene serait configuree SUR LA MEME MEMOIRE, et les "
            "deux distribueraient les memes pages."
        )
    if "ARENE.configure" not in bloc:
        fautes.append(
            "physical.rs : le repli sur l'arene a disparu ; une region trop "
            "petite pour le bitmap n'aurait plus aucun allocateur DMA."
        )
    # Aucune autre configuration de l'arene ailleurs.
    autres = len(re.findall(r"ARENE\.configure", physique))
    if autres != 1:
        fautes.append(
            "physical.rs : l'arene est configuree en %d endroits ; elle ne "
            "doit l'etre que dans le repli de configure_dma." % autres
        )
    # Les trois entrees publiques routent vers le meme allocateur.
    for nom in ("pub fn alloc_dma(", "pub fn free_dma(", "pub fn dma_etat("):
        appelant = corps(physique, nom)
        if appelant is None:
            fautes.append("physical.rs : %s introuvable." % nom)
        elif "dma_compagnon::configure_ok()" not in appelant:
            fautes.append(
                "physical.rs : %s ne demande plus quel allocateur est en "
                "service ; elle en viserait un seul des deux." % nom
            )


def regle_plafond_dit(dma, fautes):
    """Une demande plus grande que le plus gros bloc ECHOUE.

    Rendre une adresse pour une taille qu'on ne couvre pas serait la pire des
    reponses : l'appelant ecrirait au-dela.
    """
    bloc = corps(dma, "pub fn alloue(")
    if bloc is None:
        fautes.append("dma_compagnon.rs : alloue() introuvable.")
        return
    if "(1usize << ordre) < pages" not in bloc:
        fautes.append(
            "dma_compagnon.rs : une demande plus grande que le plus gros bloc "
            "n'est plus refusee ; elle serait servie par un bloc trop petit."
        )
    liberation = corps(dma, "pub fn libere(")
    if liberation is None:
        fautes.append("dma_compagnon.rs : libere() introuvable.")
    else:
        for controle in ("base < etat.base", "bloc >= etat.blocs", "decalage % PAGE"):
            if controle not in liberation:
                fautes.append(
                    "dma_compagnon.rs : libere() ne verifie plus « %s » ; une "
                    "adresse etrangere fabriquerait un bloc qui n'existe pas, "
                    "et les allocations suivantes le distribueraient."
                    % controle
                )


def regle_preuves(test, fautes):
    for attendu in (
        "deux_allocations_ne_se_recouvrent_jamais",
        "une_zone_qui_n_est_pas_une_puissance_de_deux_ne_fabrique_pas_de_bloc_imaginaire",
        "la_fusion_remonte_en_cascade",
        "une_double_liberation_est_refusee_sans_parcourir",
        "rendre_ce_qu_on_a_pris_restaure_l_etat_de_depart",
        "le_bitmap_dit_la_verite_apres_une_longue_serie",
    ):
        if attendu not in test:
            fautes.append("test_compagnon.rs : la preuve « %s » a disparu." % attendu)


def main():
    fautes = []
    for chemin in (COMPAGNON, DMA, PHYSIQUE, TEST):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    compagnon = sans_commentaires(COMPAGNON.read_text(encoding="utf-8"))
    dma = sans_commentaires(DMA.read_text(encoding="utf-8"))
    physique = sans_commentaires(PHYSIQUE.read_text(encoding="utf-8"))
    test = TEST.read_text(encoding="utf-8")

    regle_pur(compagnon, fautes)
    regle_jumeau(compagnon, fautes)
    regle_fusion(compagnon, fautes)
    regle_division(compagnon, fautes)
    regle_parcours_borne(compagnon, fautes)
    regle_bitmap(compagnon, fautes)
    regle_un_seul_allocateur(physique, fautes)
    regle_plafond_dit(dma, fautes)
    regle_preuves(test, fautes)

    if fautes:
        print("compagnon : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "compagnon : jumeau en ou exclusif, fusion bornee par la zone, double "
        "liberation refusee, un SEUL allocateur DMA configure, plafond de "
        "taille qui echoue au lieu de servir trop petit"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
