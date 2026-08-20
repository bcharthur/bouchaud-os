# M16 — le processeur au repos, le navigateur nomme

## Ce que ce jalon corrige

M15 a leve le verrou de l'analyse d'URL : la navigation atteint enfin
`RequestServer::DNSLookup`. Trois choses restaient entre ce point et « un
navigateur qu'on peut utiliser », et aucune des trois n'est du DNS.

| Constat | Cause | Correction |
|---|---|---|
| 100 % de processeur des le demarrage | quatre taches `Ready` qui attendent toutes dans `poll` | l'attente bloque la tache au lieu de la garder prete |
| bureau a 76 % devant une image immobile | recomposition « aveugle » a chaque tour de boucle | cadence liee aux entrees |
| icone « Navigateur », logo nautile, fenetre « Bouchaud Navigateur » | trois noms ecrits a trois endroits | un seul nom, `Ladybird`, et son logo |

## 1. Le processeur ne s'arretait jamais

### Ce qu'on voyait

```text
[21:34:48][100%:  2%:  3%] [ps] desktop pid=4 cpu 76% | /bo-navigateur pid=5 cpu 0%
                                | RequestServer pid=6 cpu 10% | WebContent pid=7 cpu 13%
```

100 % dès le demarrage, avant qu'une page soit demandee. La somme des parts
valait le cœur entier alors qu'aucun des quatre processus ne progressait.

### La cause

Les trois boucles d'attente de descripteurs — `poll`, `select`, `epoll_wait` —
faisaient chacune :

```rust
if task::schedule() { continue; }   // une autre tache est prete
cpu::wait_for_interrupt();          // personne d'autre : on dort
```

Le raisonnement est juste tache par tache, et faux pour la machine. Un bureau
avec le navigateur ouvert compte quatre processus, et quand ils attendent, ils
attendent **tous dans `poll`**. Chacun reste `Ready` du point de vue de
l'ordonnanceur : `schedule()` trouve toujours un autre candidat, rend `true`,
et personne n'atteint jamais le `hlt`.

La mesure de charge, elle, ne compte comme repos que les ticks ou le processeur
etait dans un `hlt`. Elle avait raison : la machine ne dormait jamais.

### La correction

`task::attends_un_tick()` marque la tache **bloquee** pour un tick — ce que
`sleep_ticks` savait deja faire. Les taches reellement pretes continuent d'etre
elues ; quand plus aucune ne l'est, `schedule()` tombe sur son propre
`wait_for_interrupt()` et le processeur s'arrete.

La latence ne change pas : le tick vaut une milliseconde, exactement le delai
qu'imposait deja le `hlt` reveille par l'horloge. Le reveil logiciel n'est pas
perdu : une tache qui rend un descripteur pret pendant notre tick le trouvera
pret a notre reveil.

Cinq autres attentes faisaient `yield_now()` puis `wait_for_interrupt()` :
elles arretaient le processeur alors que d'autres taches avaient du travail —
l'inverse exact du defaut precedent, et le meme remede.

## 2. Le compositeur recopiait une image immobile

Un client qui n'annonce pas ses trames est recompose « a l'aveugle » : le
compositeur recopie sa surface sans savoir si elle a change. Il le faisait a
chaque tour de boucle — 1100x604 pixels recopies puis presentes jusqu'a
soixante fois par seconde, y compris pendant les cinq minutes ou WebContent
attendait une reponse DNS sans peindre un pixel.

La cadence suit desormais ce qui peut faire changer l'image, c'est-a-dire une
entree :

- pleine cadence pendant `REACTIVITE_MUETTE_MS` (600 ms) apres une touche, un
  clic ou un mouvement transmis au client — le temps qu'une page defile,
  reagisse, affiche un curseur ;
- `REPOS_MUET_MS` (200 ms, soit cinq trames par seconde) au repos, comme filet
  pour une page qui s'animerait d'elle-meme.

Le cout du repos est divise par douze.

## 3. Un seul nom : Ladybird

Le moteur execute est Ladybird. L'interface disait trois choses differentes :
`Navigateur` sur l'icone du bureau et dans le menu Demarrer, `Nautile` dans le
registre d'applications, `Bouchaud Navigateur` dans la barre de titre — et le
logo etait une coquille de nautile, dessinee du temps ou le moteur etait maison.

- `window::TITRE_NAVIGATEUR` est desormais l'unique endroit ou le nom est ecrit
  cote noyau ; le bureau, le menu, la barre de titre et la barre des taches le
  lisent la.
- L'icone est une coccinelle, dessinee avec les seules primitives du
  compositeur (disques et rectangles clippes), lisible a 48 px comme en menu.
- Le chrome annonce `Titre de la page - Ladybird`, la convention de tous les
  navigateurs. La troncature a 96 caracteres coupe la page, jamais le suffixe :
  l'inverse mangeait le nom du navigateur precisement sur les titres longs.

## 4. Le reseau : router au lieu de jeter

M13 avait corrige, au niveau des prises UDP, le fait qu'un datagramme sorti de
l'anneau pour une prise et destine a une autre etait jete. La meme faute
existait un etage plus bas, dans `poll_ip`, et pour **tous** les protocoles.

Une seule carte alimente toute la machine. Des qu'un navigateur a une connexion
TCP ouverte **et** une resolution DNS en cours — c'est-a-dire des la premiere
page qui charge une ressource d'un autre hote — le `poll` de la prise TCP
mangeait la reponse DNS.

Ce qui sort de l'anneau sans etre pour l'appelant est donc mis de cote pour
celui qui l'attend. La file est bornee en nombre (256, le plus ancien part) et
en age (2 s) : un datagramme rendu dix secondes trop tard est pire qu'un
datagramme perdu, le protocole a deja retransmis.

`TcpConn::pump` epuisait par ailleurs son budget entier sur un anneau vide —
vingt millions de lectures par seconde et par connexion, appelees depuis `poll`
a chaque tick. C'est le troisieme endroit ou une attente etait comptee en tours
de boucle au lieu d'etre comptee en temps ; les deux premiers sont dans M13.

## 5. La resolution de nom : la cause, et comment elle a ete nommee

Une navigation vers `https://example.com/` emettait sa requete DNS et rien ne
revenait. Cinq campagnes de mesure ont ferme les hypotheses une par une, et
chacune a elimine plutot que devine.

| Mesure | Ce qu'elle a ferme |
|---|---|
| `M16_DNS_LOOKUP path=async-query` | pas de bascule muette vers le resolveur systeme |
| `M16_DNS_SOCKET_OK` | la socket vers 10.0.2.3:53 existe |
| `M16_DNS_TX id=… octets=46` | LibDNS a bien ecrit sa requete |
| `M16_DNS_REPEAT tentative=1` | la boucle d'evenements et ses minuteurs vivent |
| `M17_UDP_TX … parti=true` | la carte a pris la trame |
| `M17_RING appels=240000 trames=0` | on interroge la carte deux cent quarante mille fois, elle ne rend rien |

`trames=0` elimine tout ce qui se trouve **apres** la carte : ni routage, ni
notificateur, ni analyse. La reponse n'arrivait pas.

Restait `octets=46`, et c'est de l'arithmetique :

```text
en-tete DNS                                  12
QNAME "example.com" = 1+7 + 1+3 + 1          13
QTYPE + QCLASS                              + 4
                                    une question = 17

12 + 1 x 17 = 29 octets
12 + 2 x 17 = 46 octets   <- ce qui partait
```

LibDNS empile **A et AAAA dans un seul message**, avec `QDCOUNT=2`. La RFC 1035
l'autorise ; aucun resolveur reel ne l'implemente, et celui qu'integre SLIRP —
le 10.0.2.3 de QEMU — l'ignore en silence. Pas d'erreur, pas de `FormatError` :
rien.

Ce qui le prouve et ne le suggere pas : `tools/userland/dns-probe.c` emet des
requetes a **une** question vers le meme 10.0.2.3, depuis le meme noyau, dans la
meme execution d'integration continue, et recoit ses reponses. Son CAS5 rejoue
la sequence exacte de `Core::UDPSocket`. C'est une barriere bloquante de
`ci.yml`, verte sur le meme commit. Meme machine, meme pile, meme resolveur : ce
qui distingue les deux requetes est leur nombre de questions.

### La correction

`tools/ladybird/prepare-dns-une-question.py` ne demande plus que `A`. Ce portage
n'a pas d'IPv6 — `sys_socket` rend `EAFNOSUPPORT` pour `AF_INET6`, deliberement —
donc un enregistrement AAAA designerait une adresse que rien dans la machine ne
saurait utiliser.

Ce n'est pas un contournement pour `example.com` : c'est la meme requete pour
`google.com`, `github.com` ou n'importe quel nom. Le jour ou Bouchaud aura une
pile IPv6, il faudra emettre **deux messages separes** — ce que font Chrome et
Firefox — et remplacer ce script, pas l'etendre.

### Ce que la mesure montre apres

```text
M17_UDP_TX dst=10.0.2.3:53 src_port=52463 octets=29 parti=true
M16_DNS_TX id=39241 octets=29
M17_UDP_LIVRE src=10.0.2.3:53 vers_port=52463 octets=61 connecte=true
M16_DNS_READY
M16_DNS_RX id=39241 rcode=0 reponses=2
M9_RS_DNS_RESOLU id=0 host=example.com adresses=2
M9_RS_STATE id=0 DNSLookup -> RetrieveCookie
M9_RS_STATE id=0 RetrieveCookie -> Fetch
M9_RS_HEADERS id=0 statut=200 nb=9
M9_RS_STATE id=0 Fetch -> Complete
M9_RS_REQUEST_FINISHED id=0 taille=559
M9_NAVIGATION_COMMITTED page=1 url=https://example.com/
```

Deux secondes entre la requete et le document commite : nom resolu par DNS,
chaine TLS **publique** validee, HTTP 200, 559 octets. C'est la premiere fois
qu'un site public est charge par son nom de bout en bout depuis Bouchaud OS.

## 6. Les sondes (temporaires)

`tools/ladybird/prepare-m16-dns.py` instrumente `LibDNS/Resolver.h`, et rien
d'autre. Le silence apres `Init -> DNSLookup` a trois causes incompatibles
qu'aucune mesure ne separait :

1. `has_connection()` echoue — LibDNS bascule **sans le dire** sur le resolveur
   systeme (`getaddrinfo` dans un `ThreadPool`), un chemin sans delai ni
   retransmission ;
2. la requete part, la reponse ne revient pas ;
3. la reponse revient et ne s'analyse pas.

| Sonde | Ce qu'elle elimine |
|---|---|
| `M16_DNS_LOOKUP name= path=` | (1) — `lookup_path` est deja tenu a jour par upstream, on le rend visible |
| `M16_DNS_SOCKET_OK` / `_ECHEC` | (1) — la socket vers le resolveur a-t-elle pu etre creee |
| `M16_DNS_TX id= octets=` | (2) — la requete est-elle reellement partie |
| `M16_DNS_REPEAT id= tentative=` | (2) — le minuteur de retransmission tire-t-il |
| `M16_DNS_READY` | (2) — la socket se declare-t-elle lisible |
| `M16_DNS_RX id= rcode= reponses=` | (3) — le message recu s'analyse-t-il |

Elles s'allument avec `BOUCHAUD_M9`, comme le reste du portage.

Deux sondes cote noyau les completaient. `M17_UDP_TX` / `M17_UDP_LIVRE`, bornees
au port 53, disent ce qui sort et ce qui rentre ; elles restent, parce que deux a
quatre lignes par resolution sont le prix juste d'une preuve permanente que la
chaine tient. `M17_RING` (compteur d'interrogations de la carte) et `M18_POLL`
(classement des descripteurs scrutes) ont ete **retirees** : leur question est
close, et `M18_POLL` vivait sur le chemin le plus chaud du noyau.

### Ce que ces sondes ont coute, et appris

Deux fois, un motif de `grep` ecrit a la main a produit une conclusion fausse.
`M17_RING` ne figurait pas dans la liste du verdict — seul `M17_UDP_` y etait —
et son absence a l'ecran a ete lue comme l'absence de la mesure : « `poll_ip`
n'est jamais appele », alors qu'il l'etait deux cent quarante mille fois. Le
verdict imprime desormais **tous** les marqueurs, sans choisir. Le journal serie
pese six kilooctets : il n'y a rien a economiser en filtrant, et tout a perdre en
se trompant de filtre.

Une mesure qu'on ne peut pas voir ne vaut pas mieux qu'une mesure qu'on n'a pas
prise — elle est pire, parce qu'elle donne la meme assurance.

## Ce qui reste

`M9_DOCUMENT_LOADED` n'apparait pas, et WebContent a quitte la liste des
processus apres le commit de la navigation. Le document distant est arrive
entier ; ce qui suit — analyse, mise en page, peinture, capture — reste a
mesurer. C'est le defaut suivant, et il est d'une autre nature que celui-ci.

Une seconde retransmission DNS n'a jamais lieu non plus : le minuteur n'est
redemarre qu'apres l'ecriture, et la deuxieme recherche rejoint la recherche en
attente au lieu de reemettre. Sans consequence maintenant que la premiere
requete aboutit, mais cela veut dire qu'une perte de datagramme ne serait
jamais rattrapee.
