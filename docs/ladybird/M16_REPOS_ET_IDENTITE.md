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

## 5. Les sondes M16 (temporaires)

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

Elles s'allument avec `BOUCHAUD_M9`, comme le reste du portage, et disparaitront
avec le defaut qu'elles servent a nommer.

## Ce qui reste

La resolution de nom. Tout le reste de la chaine est mesure et vert :
analyse d'URL, navigation, IPC, entree dans `RequestServer`, emission de la
requete DNS. Voir `M13_DNS.md` pour la couche UDP du noyau, deja corrigee et
prouvee par `tools/net/dns-probe.c`.
