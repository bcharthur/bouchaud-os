#!/usr/bin/env python3
# Une seule question par requete DNS.
#
# ## Le defaut, et comment il a ete nomme
#
# Une navigation vers `https://example.com/` emettait sa requete DNS, et rien
# ne revenait jamais. Cinq campagnes de mesure ont ferme les hypotheses une par
# une :
#
#     M16_DNS_SOCKET_OK                       la socket vers 10.0.2.3:53 existe
#     M17_UDP_TX dst=10.0.2.3:53 ... parti=true   la carte a pris la trame
#     M16_DNS_TX id=58667 octets=46           LibDNS a bien ecrit 46 octets
#     M16_DNS_REPEAT id=58667 tentative=1     la boucle d'evenements vit
#     M17_RING appels=240000 trames=0         on interroge la carte, elle ne
#                                             rend rien, pendant cinq minutes
#
# `trames=0` sur deux cent quarante mille interrogations elimine tout ce qui se
# trouve **apres** la carte : ni routage, ni notificateur, ni analyse. La
# reponse n'arrive pas.
#
# Reste `octets=46`, et c'est de l'arithmetique :
#
#     en-tete DNS                                     12
#     QNAME "example.com" = 1+7 + 1+3 + 1             13
#     QTYPE + QCLASS                                 + 4
#                                             une question = 17
#
#     12 + 1 x 17 = 29 octets
#     12 + 2 x 17 = 46 octets   <- ce qui est parti
#
# LibDNS empile donc **A et AAAA dans un seul message**, avec `QDCOUNT=2` :
#
#     query.header.question_count = max(1u, desired_types.size());
#     for (auto const& type : desired_types)
#         query.questions.append(...);
#
# La RFC 1035 autorise plusieurs questions ; aucun resolveur reel ne les
# implemente, et le resolveur integre de SLIRP — celui de QEMU, a 10.0.2.3 —
# les ignore en silence. Pas d'erreur, pas de `FormatError` : rien. Ce qui est
# exactement le silence observe.
#
# ## Ce qui le prouve, et non le suggere
#
# `tools/userland/dns-probe.c` emet des requetes DNS **a une question** vers le
# meme 10.0.2.3, depuis le meme noyau, dans la meme execution d'integration
# continue — et recoit ses reponses. Son CAS5 rejoue meme la sequence exacte de
# `Core::UDPSocket` : `socket(SOCK_DGRAM|SOCK_CLOEXEC)`, `connect`, emission
# sans adresse, double interrogation de lisibilite, `FIONREAD`. C'est une
# barriere **bloquante** de `ci.yml`, et elle est verte sur le meme commit.
#
# Meme machine, meme pile, meme resolveur : ce qui distingue les deux requetes
# est leur nombre de questions.
#
# ## La correction
#
# Ce portage n'a pas d'IPv6. `sys_socket` rend `EAFNOSUPPORT` pour `AF_INET6`,
# et c'est deliberé — le dire franchement fait retomber `getaddrinfo` sur IPv4
# au lieu de le faire echouer. Demander un enregistrement AAAA revient donc a
# demander une adresse que rien dans la machine ne saura utiliser.
#
# On ne demande plus que `A`. La requete redevient la forme standard a une
# question, que tout resolveur au monde traite, et elle perd du meme coup la
# moitie qui n'aurait servi a rien.
#
# Ce n'est pas un contournement pour `example.com` : c'est la meme requete pour
# `google.com`, `github.com`, `wikipedia.org` et n'importe quel autre nom.
#
# ## Ce que cela ne corrige pas
#
# Le jour ou Bouchaud aura une pile IPv6, il faudra emettre **deux messages
# separes** plutot que de remettre deux questions dans un seul — c'est ce que
# font Chrome et Firefox. Le present script devra alors etre remplace, pas
# etendu.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-dns-une-question.py <ladybird-worktree>")

root = Path(sys.argv[1])

DEUX_QUESTIONS = "{ Messages::ResourceType::A, Messages::ResourceType::AAAA }"
UNE_QUESTION = "{ Messages::ResourceType::A }"
DEUX_QUALIFIE = "{ DNS::Messages::ResourceType::A, DNS::Messages::ResourceType::AAAA }"
UNE_QUALIFIEE = "{ DNS::Messages::ResourceType::A }"

sites = [
    # La surcharge de commodite : tout appelant qui ne precise pas ses types
    # passe par elle, y compris la resolution synchrone du proxy.
    ("Libraries/LibDNS/Resolver.h", DEUX_QUESTIONS, UNE_QUESTION),
    # Le chemin d'une navigation.
    ("Services/RequestServer/Request.cpp", DEUX_QUALIFIE, UNE_QUALIFIEE),
    # Le chemin d'une connexion WebSocket.
    ("Services/RequestServer/ConnectionFromClient.cpp", DEUX_QUALIFIE, UNE_QUALIFIEE),
]

total = 0
for chemin, ancien, nouveau in sites:
    fichier = root / chemin
    data = fichier.read_text()
    if ancien not in data:
        if nouveau in data:
            continue  # deja applique
        raise SystemExit(
            f"DNS une question : motif introuvable dans {chemin} — "
            "les types demandes ont change en amont"
        )
    compte = data.count(ancien)
    fichier.write_text(data.replace(ancien, nouveau))
    total += compte

print(f"DNS : une seule question par requete ({total} site(s) ajuste(s))")
