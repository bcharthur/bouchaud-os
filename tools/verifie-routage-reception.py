#!/usr/bin/env python3
"""Garde-fou : une seule carte, un seul lecteur, et rien qui se jette.

# Le defaut, lu sur la machine de reference

Releve du 13 septembre 2026, 00:36. Le lien est a 1000 Mb/s duplex complet, le
bail DHCP est pose (`192.168.1.97`, passerelle et resolveur `192.168.1.254`),
et le navigateur ne charge rien :

    M17_UDP_TX dst=192.168.1.254:53 src_port=49185 octets=29 parti=false
    M9_RS_STATE id=0 DNSLookup -> Error
    WebContent: Failed load of "https://example.com/", Unable to resolve host

`parti=false` remonte de `hop_mac`, qui remonte de `arp_resolve`, qui n'a
jamais vu la reponse de la passerelle. DHCP fonctionnait parce qu'il est en
DIFFUSION et ne demande aucune resolution ; TOUT l'unicast echouait.

Deux causes, toutes deux structurelles :

1. **Cinq lecteurs concurrents de la meme carte**, chacun jetant ce qui ne
   l'interessait pas. `arp_resolve` detruisait les reponses DNS ;
   `dhcp::recv_avant` detruisait les reponses ARP -- et le veilleur de lien
   tient cette boucle quatre secondes d'affilee. Une reponse ARP est unique et
   jamais retransmise : il suffit qu'un autre lecteur la sorte de l'anneau une
   fois pour que la resolution echoue, et l'echec est mis en cache.

2. **Aucun bourrage des trames courtes.** Une requete ARP fait 42 octets ;
   802.3 en exige 60. C'est la seule trame de moins de soixante octets que la
   pile emette -- et c'est precisement celle qui n'obtenait jamais de reponse.

# Ce qui est verifie

1. `e1000::receive` n'est appele que depuis le routage commun.
2. Le routage donne chaque trame a qui de droit : ARP, DHCP, IP en attente.
3. `arp_resolve` et le client DHCP ne lisent plus la carte eux-memes.
4. Le verdict de `e1000::send` est REGARDE quand on emet une requete ARP.
5. Les anneaux de la carte sont serialises.
6. Les trames courtes sont bourrees a soixante octets, dans les deux pilotes.
7. `same_subnet` utilise le masque du bail, pas un /24 code en dur.
8. Un echec de resolution n'ecrase jamais une reussite concurrente.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
NET = RACINE / "src/net/mod.rs"
DHCP = RACINE / "src/net/application/dhcp.rs"
E1000 = RACINE / "src/drivers/network/e1000.rs"
RTL = RACINE / "src/drivers/network/rtl8168.rs"
SMOL = RACINE / "src/net/transport/smol_device.rs"


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
    for chemin in (NET, DHCP, E1000, RTL):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    net = sans_commentaires(NET.read_text(encoding="utf-8"))
    dhcp = sans_commentaires(DHCP.read_text(encoding="utf-8"))
    e1000 = sans_commentaires(E1000.read_text(encoding="utf-8"))
    rtl = sans_commentaires(RTL.read_text(encoding="utf-8"))

    # --- 1. UN SEUL LECTEUR DE LA CARTE ------------------------------------
    lectures = len(re.findall(r"e1000::receive\(", net))
    if lectures != 1:
        fautes.append(
            "net/mod.rs : %d appels a `e1000::receive` au lieu d'un seul. "
            "Chaque lecteur supplementaire est un lecteur qui jette le "
            "courrier des autres -- c'est ce qui detruisait les reponses ARP "
            "et rendait toute page injoignable le 13 septembre." % lectures
        )
    else:
        drain = corps(net, "fn draine_verrouille(")
        if drain is None or "e1000::receive(" not in drain:
            fautes.append(
                "net/mod.rs : l'unique lecture de la carte n'est plus dans le "
                "routage commun. Elle appartient au seul endroit qui sait "
                "donner une trame a qui de droit."
            )

    # --- 2. LE ROUTAGE NE JETTE RIEN DE CE QU'ON ATTEND ---------------------
    route = corps(net, "fn route_trame(")
    if route is None:
        fautes.append("net/mod.rs : le routage des trames a disparu.")
    else:
        if "traite_arp(" not in route:
            fautes.append(
                "net/mod.rs : le routage ne traite plus l'ARP. Une reponse ARP "
                "n'est jamais retransmise : la perdre une fois suffit a rendre "
                "un voisin injoignable."
            )
        if "route_ipv4(" not in route:
            fautes.append("net/mod.rs : le routage ne traite plus l'IPv4.")

    route_ip = corps(net, "fn route_ipv4(")
    if route_ip is None:
        fautes.append("net/mod.rs : le routage IPv4 a disparu.")
    else:
        if "depose_dhcp_verrouille(" not in route_ip:
            fautes.append(
                "net/mod.rs : les reponses DHCP n'ont plus de boite. Elles "
                "arrivent avant qu'on ait une adresse, en diffusion : aucun "
                "appelant de `poll_ip` ne les reclamera jamais, et le bail "
                "serait perdu."
            )
        if "depose_en_attente_verrouille(" not in route_ip:
            fautes.append(
                "net/mod.rs : les paquets IP ne sont plus mis de cote. Celui "
                "qui les attend ne les verrait jamais."
            )

    poll = corps(net, "pub(crate) fn poll_ip(")
    if poll is None:
        fautes.append("net/mod.rs : poll_ip a disparu.")
    elif "e1000::receive(" in poll:
        fautes.append(
            "net/mod.rs : poll_ip relit la carte directement. Il redeviendrait "
            "un lecteur concurrent de plus."
        )

    # --- 3. NI ARP NI DHCP NE LISENT LA CARTE EUX-MEMES ---------------------
    arp = corps(net, "fn arp_resolve(")
    if arp is None:
        fautes.append("net/mod.rs : arp_resolve a disparu.")
    else:
        if "e1000::receive(" in arp:
            fautes.append(
                "net/mod.rs : arp_resolve relit la carte lui-meme. Il jetterait "
                "de nouveau la reponse DNS que le navigateur attend au meme "
                "instant -- et raterait la sienne des qu'un autre fil la sort "
                "de l'anneau avant lui."
            )
        if "draine_anneau()" not in arp:
            fautes.append(
                "net/mod.rs : arp_resolve ne fait plus tourner le routage "
                "commun ; il attendrait une reponse que personne ne va "
                "chercher."
            )
        # --- 4. LE VERDICT DE L'EMISSION EST REGARDE ------------------------
        if not re.search(r"if\s*!\s*arp_demande_sans_attendre\(", arp):
            fautes.append(
                "net/mod.rs : arp_resolve ne verifie plus que sa requete est "
                "PARTIE. Attendre cinq cents millisecondes la reponse a une "
                "question restee dans un anneau plein est un faux negatif."
            )

    demande = corps(net, "fn arp_demande_sans_attendre(")
    if demande is None:
        fautes.append("net/mod.rs : arp_demande_sans_attendre a disparu.")
    elif "-> bool" not in net[net.find("fn arp_demande_sans_attendre("):
                              net.find("fn arp_demande_sans_attendre(") + 120]:
        fautes.append(
            "net/mod.rs : arp_demande_sans_attendre ne rend plus de verdict. "
            "Une requete jamais emise redeviendrait indiscernable d'un voisin "
            "qui ne repond pas."
        )

    # `recv_avant` peut deleguer sa boucle a un corps interne : la trace par
    # etage a besoin d'encadrer l'attente, pas de vivre dedans. La garde suit
    # donc la delegation au lieu de l'interdire -- mais elle verifie
    # l'invariant sur la REUNION des deux corps, si bien qu'aucun des deux ne
    # peut relire la carte, et que la boite doit etre relevee dans l'un d'eux.
    recv = corps(dhcp, "fn recv_avant(")
    interne = corps(dhcp, "fn recv_avant_interne(")
    if recv is not None and interne is not None:
        recv = recv + "\n" + interne
    if recv is None:
        fautes.append("dhcp.rs : recv_avant a disparu.")
    else:
        if "e1000::receive(" in recv:
            fautes.append(
                "dhcp.rs : le client DHCP relit la carte lui-meme. Le veilleur "
                "de lien tient cette boucle quatre secondes d'affilee : il "
                "detruirait de nouveau les reponses ARP que le reste du "
                "systeme attend."
            )
        if "prend_dhcp(" not in recv:
            fautes.append(
                "dhcp.rs : le client DHCP ne releve plus sa boite."
            )

    # --- 5. LES ANNEAUX SONT SERIALISES -------------------------------------
    for nom, entete in (("send", "pub fn send("), ("receive", "pub fn receive(")):
        bloc = corps(e1000, entete)
        if bloc is None:
            fautes.append("e1000.rs : %s a disparu." % nom)
        elif ".lock()" not in bloc.split("\n")[1] and ".lock()" not in bloc[:200]:
            fautes.append(
                "e1000.rs : `%s` ne prend plus le verrou de l'anneau. Deux "
                "appelants concurrents partageraient le meme descripteur : le "
                "premier copie le tampon pendant que le second le rend a la "
                "carte." % nom
            )

    # --- 6. LES TRAMES COURTES SONT BOURREES --------------------------------
    for nom, texte, constante in (
        ("rtl8168.rs", rtl, "TRAME_MIN"),
        ("e1000.rs", e1000, "TRAME_MIN_ETHERNET"),
    ):
        m = re.search(r"const %s: usize = ([0-9_]+);" % constante, texte)
        if m is None:
            fautes.append(
                "%s : la longueur minimale d'une trame n'est plus declaree. "
                "Une requete ARP fait quarante-deux octets : sans bourrage, "
                "c'est un « runt » que le commutateur d'en face jette, et "
                "c'est la SEULE trame courte que la pile emette." % nom
            )
            continue
        if int(m.group(1).replace("_", "")) < 60:
            fautes.append(
                "%s : la longueur minimale d'une trame est passee sous les "
                "soixante octets exiges par 802.3." % nom
            )
        envoi = corps(texte, "pub fn send(")
        if envoi is None or ".max(%s" % constante not in envoi:
            fautes.append(
                "%s : l'emission ne bourre plus les trames courtes. La requete "
                "ARP repartirait en runt, et tout l'unicast echouerait derriere "
                "elle." % nom
            )
        if envoi is not None and "write_bytes(" not in envoi:
            fautes.append(
                "%s : le bourrage n'efface pas le tampon DMA. Le remplissage "
                "laisserait filtrer la trame precedente." % nom
            )

    # --- 7. LE MASQUE DU BAIL, PAS UN /24 CODE EN DUR -----------------------
    sous_reseau = corps(net, "fn same_subnet(")
    if sous_reseau is None:
        fautes.append("net/mod.rs : same_subnet a disparu.")
    elif "MASQUE" not in sous_reseau:
        fautes.append(
            "net/mod.rs : same_subnet ignore de nouveau le masque du bail. Sur "
            "un /16 ou un /22, chaque voisin hors des 254 premieres adresses "
            "serait envoye a la passerelle."
        )

    # --- 8. UN ECHEC N'ECRASE PAS UNE REUSSITE ------------------------------
    echec = corps(net, "fn arp_cache_pose_echec(")
    if echec is None:
        fautes.append(
            "net/mod.rs : l'entree negative du cache ARP se pose de nouveau "
            "sans regarder. Le routage peut avoir appris la bonne adresse "
            "pendant la fenetre d'ecoute : l'ecraser transformerait une "
            "resolution reussie en voisin muet pour deux secondes."
        )
    elif "Some(Some(" not in echec:
        fautes.append(
            "net/mod.rs : arp_cache_pose_echec ne verifie plus l'absence d'une "
            "reussite concurrente."
        )

    if fautes:
        print("routage de reception : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "routage de reception : un seul lecteur, rien de jete, trames courtes "
        "bourrees, masque du bail respecte"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
