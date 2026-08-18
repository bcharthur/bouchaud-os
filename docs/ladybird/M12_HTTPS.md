# M12 — HTTPS depuis Bouchaud OS

## Ce que M12 ajoute a M9

M9 a prouve la chaine complete jusqu'au reseau :

    LibWeb -> ResourceLoader -> LibRequests -> IPC -> RequestServer
           -> socket Bouchaud -> TCP/IP -> HTTP -> page affichee

M12 n'y ajoute qu'une chose, mais c'est celle qui ouvre l'Internet reel :
**TLS avec verification de la chaine**. Sans elle, aucun site moderne n'est
joignable — pas meme un moteur de recherche.

## Le seul blocage reel, et pourquoi

Trois prerequis conditionnent HTTPS. Deux etaient deja tenus :

| Prerequis | Etat | Pourquoi il compte |
|---|---|---|
| UDP + resolution de noms | present (`SOCK_DGRAM`) | sans DNS, seules les adresses IP sont joignables |
| Horloge murale reelle | presente, ancree sur la RTC | un certificat se valide contre une date ; une horloge a zero les rend tous invalides |
| **Certificats racine** | **absents** | sans autorite de confiance, toute chaine est refusee |

Le troisieme etait le blocage, et il ne se contourne pas : refuser la
verification serait rendre TLS decoratif.

## Le contrat de lancement

`RequestServer` accepte `--certificate <chemin>` et le pose en
`CURLOPT_CAINFO` (`Services/RequestServer/Request.cpp`). Sans cet argument,
`default_certificate_path()` est vide et curl retombe sur le chemin **compile a
la construction**, qui n'existe pas sous Bouchaud : toute connexion TLS echoue
a la verification, meme contre un serveur parfaitement valide.

`tools/ladybird/webcontent-bootstrap.c` passe donc `--certificate` — mais
**seulement si le fichier est lisible** :

    BOUCHAUD_CA_BUNDLE, ou a defaut /etc/ssl/certs/ca-certificates.crt

C'est ce qui permet a M9 de continuer a fonctionner exactement comme avant :
pas de fichier, pas d'argument, comportement inchange. Le lanceur annonce ce
qu'il a trouve :

    [ladybird-bouchaud] M12_CA_BUNDLE /etc/ssl/certs/ca-certificates.crt

## La CI reste hermetique

La regle du projet n'a pas bouge : **aucun test ne depend d'un site externe**.
M12 fabrique donc a chaque execution sa propre autorite et un certificat
serveur, et embarque l'autorite dans l'image Bouchaud.

La verification n'en est pas affaiblie : la chaine est reellement validee, le
nom reellement verifie. Le certificat porte `IP:10.0.2.2` en `subjectAltName`,
parce que c'est l'adresse de l'hote vue depuis QEMU — un certificat pour un nom
ne prouverait rien ici, et la verification du nom fait partie de ce qu'on veut
prouver.

La fixture est d'abord testee **depuis l'hote**, avec `curl --cacert`. Si elle
echoue la, le defaut est dans la fixture et non dans le systeme teste : on evite
de chercher pendant une heure un defaut Bouchaud qui n'existe pas.

## Le temoin est celui de M9, volontairement

M12 rejoue le temoin de M9 avec une URL `https://`. C'est un choix, pas une
economie : si un seul mode de test existe, alors ce que M12 mesure est
**exactement** la difference entre HTTP et HTTPS, et rien d'autre. Les marqueurs
`M9_*` du journal sont donc attendus.

Ce qui distingue M12 tient a trois lignes du verdict :

    [ladybird-bouchaud] M12_CA_BUNDLE /etc/ssl/certs/ca-certificates.crt
    M12_FIXTURE_HTTPS_OK path=/m12.html
    ! M9_FIXTURE_HTTP_OK          (preuve negative : rien n'a transite en clair)

## DNS : Ladybird n'utilise pas `getaddrinfo`

Point important, et contre-intuitif : **Ladybird embarque son propre
resolveur**. `Services/RequestServer/Resolver.cpp` s'appuie sur `LibDNS`, et
`DNSInfo::use_dns_over_tls` y vaut **`true` par defaut**.

Laisse tel quel, cela produit deux cercles :

- le DNS voudrait ouvrir une session **TLS** — donc dependre d'un magasin
  d'autorites — avant meme d'avoir resolu quoi que ce soit ;
- et resoudre le **nom du resolveur** demanderait une resolution.

La sortie est donnee par upstream lui-meme : `set_dns_server(host_or_address,
port, use_tls, dnssec)` accepte une **adresse IP litterale**, sans rien
resoudre. Le port configure donc, a la connexion du `RequestClient` :

    10.0.2.3:53, UDP simple    (le resolveur du NAT de QEMU)

et `BOUCHAUD_DNS_SERVER` prend le relais sur une machine reelle. Le trajet est
trace :

    [ladybird-bouchaud] M12_DNS_SERVER 10.0.2.3 port=53 tls=false

C'est une vraie resolution, pas une correspondance codee en dur : aucun nom
n'est associe a une adresse dans le port.

## Autorites publiques

L'image embarque deux jeux concatenes : l'autorite de test, qui signe la
fixture, et le magasin **public** du paquet `ca-certificates` du runner.

Le magasin est verifie plutot que suppose — nombre de certificats, et presence
d'une racine temoin (`ISRG Root X1`) — parce qu'un fichier tronque passerait
autrement inapercu jusqu'a une poignee de main TLS incomprehensible. Son
empreinte est journalisee.

Epingler une empreinte figee a ete ecarte volontairement : cela casserait a la
premiere mise a jour de securite du paquet, et donnerait l'illusion du controle
plutot que le controle.

**A aucun moment la verification n'est relachee** : ni `-k`, ni
`CURLOPT_SSL_VERIFYPEER=0`, ni exception.

## L'essai Internet reel

Un job separe, **`continue-on-error`**, charge `https://example.com/`. Il est
informatif par construction — une panne d'un site tiers ne doit pas rendre rouge
une CI dont le role est de mesurer Bouchaud.

## Ce qui bloque encore : resoudre un nom

**L'essai Internet ne passe pas**, et le journal du 18 aout dit ou. Deux jobs de
la meme execution, a quatre minutes d'intervalle, avec le meme binaire, le meme
bundle CA et le meme serveur DNS configure :

| | M12 (vert) | Internet (bloque) |
|---|---|---|
| URL | `https://10.0.2.2:18443/m12.html` | `https://example.com/` |
| Forme de l'hote | **adresse IP litterale** | **nom** |
| `M9_NAVIGATION_BEGIN` | 20:56:13 | 20:52:56 |
| `M9_NAVIGATION_COMMITTED` | 20:56:28, soit +15 s | **jamais**, en cinq minutes |
| RequestServer | termine | ~49 % de processeur, en continu |

Cote Internet, le journal s'arrete net apres
`M9_DOCUMENT_BODY_LOCAL_UNPAUSED` : **aucun** des trois marqueurs de retour de
RequestServer (`M9_RS_REQUEST_STARTED`, `M9_RS_HEADERS`,
`M9_RS_REQUEST_FINISHED`) n'apparait. La requete ne ressort donc pas de
RequestServer, et celui-ci consomme du processeur au lieu d'attendre : ce n'est
pas un blocage sur une socket, c'est une boucle.

TLS n'est pas en cause, et c'est M12 qui l'etablit : la meme pile valide une
chaine reelle et verifie un nom d'hote quinze secondes apres le debut de la
navigation. La seule variable qui change est **la forme de l'hote**.

Le suspect est donc la resolution de nom de `LibDNS`. Il reste un suspect : rien
dans ce journal n'instrumente `LibDNS` lui-meme, et cette section sera reecrite
par une mesure, pas completee par une conviction. Les hypotheses de M9 ont ete
refutees trois fois ; celle-ci n'a pas plus de droits.

**Consequence pratique.** `run.ps1 -Ladybird` demarre sur la fixture locale et
non sur un site public : c'est le seul chemin reseau prouve de bout en bout. Une
URL par nom se tape dans la barre d'adresse, et se figera tant que ce point ne
sera pas ferme.

## Ce que M12 ne fait pas encore

- **Aucun site joignable par son nom.** Voir la section precedente : c'est le
  point qui separe « HTTPS fonctionne » de « on peut naviguer sur le Web ».
- **Pas de gestion d'erreur de certificat cote interface.** Un certificat
  invalide fait echouer la requete ; il n'y a pas d'ecran d'avertissement, parce
  qu'il n'y a pas encore d'interface pour le porter (M11).
- **Pas de HSTS, pas de redirections `http` -> `https` eprouvees.**
- **L'URL reste une variable d'environnement.** C'est precisement ce que M11
  doit supprimer.

## Etape suivante

M11 est fait : la barre d'adresse, l'historique et les liens existent
(`M11_NAVIGATEUR.md`). Ce qui manque maintenant n'est plus l'interface, c'est la
resolution de nom — sans elle, la barre d'adresse n'accepte que des adresses IP,
ce qui n'est pas naviguer.
