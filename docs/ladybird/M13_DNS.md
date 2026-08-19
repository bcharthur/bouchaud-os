# M13 — resoudre un nom, et le prouver

## Ce qui bloquait

M12 a rendu HTTPS possible : chaine validee, nom d'hote verifie, document
distant affiche. Mais uniquement contre un hote designe par son **adresse**.
Demande par son **nom**, la meme pile ne commitait jamais la navigation, et
`RequestServer` consommait la moitie d'un cœur pendant cinq minutes.

La cause etait dans le noyau, dans `pump_udp` — la fonction qui lit l'anneau de
reception de la carte pour le compte d'un socket. Deux defauts distincts s'y
superposaient.

### 1. Les datagrammes des autres sockets etaient jetes

Une seule carte alimente tous les sockets. `poll_ip` sort une trame de l'anneau.
Si son port de destination n'etait pas celui du socket interroge, le code
faisait `continue` : le datagramme, deja hors de l'anneau, etait **perdu**.

Un resolveur emet plusieurs requetes en parallele — A et AAAA, ou plusieurs
noms. Le premier socket servi mangeait donc la reponse du second. Les deux
attendaient, et la page ne se chargeait jamais.

### 2. L'attente etait une boucle a plein processeur

`poll_ip` est **non bloquant**. Quand il rend `None`, l'anneau est vide a cet
instant, et le rappeler ne peut rien faire arriver. Le code le rappelait
pourtant — jusqu'a trois millions de fois pour un `recvfrom` bloquant. C'est la
que passait le processeur.

## La preuve, avant la correction

`tools/userland/dns-probe.c` est un ELF de treize kilooctets, sans libc, qui
n'utilise que l'instruction `syscall`. Il ouvre de vrais sockets UDP et emet de
vraies requetes DNS vers le resolveur du NAT de QEMU. Aucune simulation.

Sur le noyau **avant** correction :

```text
[dns-probe] CAS2 socketB recu=-110 duree_ms=4963
[dns-probe] CAS2_ECHEC demultiplexage : la reponse de B a ete jetee
charge processeur pendant l'attente : 100 %
```

Apres :

```text
[dns-probe] CAS2 socketB recu=61 id=0x3333 duree_ms=0
[dns-probe] CAS2_OK
charge processeur pendant l'attente : 0 %
```

### La premiere version du test ne prouvait rien

Elle envoyait deux requetes puis lisait les deux reponses. La panne dependait
alors de l'ordre d'arrivee : si la reponse de B arrivait apres que A ait fini,
B la trouvait. **Le test passait une fois sur deux sur un noyau pourtant
casse.**

Le CAS 2 a donc ete refait pour etre deterministe :

1. B emet sa requete ;
2. on laisse passer 300 ms — les reponses arrivent en ~15 ms ;
3. A fait une lecture **non bloquante**, qui vide l'anneau ;
4. B lit a son tour.

Une couche qui route rend la reponse a B. Une couche qui jette l'a perdue. Le
test a ete verifie dans les deux sens : rouge sur le noyau non corrige, vert sur
le corrige.

### 3. Le meme defaut ailleurs : la resolution ARP

La CI a trouve ce que la machine de developpement ne pouvait pas montrer. Le
tout premier `sendto` y echouait en `ENETUNREACH` :

```text
[dns-probe] envoye=-101
```

`arp_resolve` ecoutait la reponse pendant **1 500 000 iterations**. Ce n'est pas
un delai, c'est une quantite de travail : le temps qu'elle represente depend de
la vitesse du processeur. Sur un runner rapide, les trois tentatives
s'epuisaient avant qu'un aller-retour ARP ait eu le temps de se faire ; sur la
machine de developpement, jamais. **Un defaut qui ne se voit que sur certaines
machines est la signature de ce genre de mesure.**

L'ecoute compte desormais des millisecondes — quatre tentatives de 500 ms — et
le cout n'est paye qu'a la premiere sortie vers un voisin inconnu.

## La correction

C'est la primitive POSIX qui est reparee, pas un cas particulier Ladybird.

| Avant | Apres |
|---|---|
| datagramme d'un autre port : jete | route sur son port de destination |
| anneau vide : rappeler `poll_ip` | s'arreter |
| `recvfrom` bloquant : 3 000 000 de tours | delai nomme, avec `schedule()` puis `hlt` |
| `poll` : 20 000 tours | un seul passage |
| ARP : 1 500 000 iterations d'ecoute | 4 x 500 ms, comptes a l'horloge |

`livre_datagramme` prefere un socket **connecte** a la source a un socket
simplement lie, comme le veut la specification des sockets. Ce qui n'est adresse
a personne est ecarte, comme sur toute machine.

Le delai d'un `recvfrom` bloquant s'appelle `RECV_UDP_DELAI_MS` et vaut cinq
secondes, l'ordre de grandeur des resolveurs. Un delai compte en tours de boucle
variait avec la vitesse de la machine : ce n'etait pas un delai.

## Le test

```bash
tools/net/verifie-dns.sh
```

### Mesurer le processeur, pas la montre

Le CAS 3 comparait une duree ecoulee a un seuil. Cela ne prouvait rien : la
boucle a plein processeur rendait la main au bout de ~4,9 s, donc elle aurait
passe le meme seuil qu'une attente qui dort. **Une montre ne dit pas si le
processeur travaille.**

Le noyau n'exposait pas de quoi le savoir : `clock_gettime` renvoyait la base
monotone pour *toutes* les horloges, y compris `CLOCK_PROCESS_CPUTIME_ID`. Il
comptait pourtant deja — le profileur par echantillonnage incremente
`ticks_cpu` a chaque IRQ0, et le PIT bat a 1000 Hz, donc un tick vaut une
milliseconde de processeur. Il suffisait de le dire a l'espace utilisateur.
`CLOCK_PROCESS_CPUTIME_ID` et `CLOCK_THREAD_CPUTIME_ID` sont donc implementees.

Le CAS 3 compare maintenant les deux horloges :

| | mur | processeur |
|---|---|---|
| noyau corrige | 5000 ms | **0 ms** |
| ancienne boucle | 2926 ms | **2926 ms** |

Entre les deux il n'y a rien a discuter.

Il construit le noyau, la sonde et le disque, lance QEMU, et verifie quatre
marqueurs — dont le CAS 4, qui rejoue le chemin reel du resolveur par `poll()`. Il est **bloquant en CI** (`DNS-UDP-ring3`) et ne demande aucun
artefact Ladybird : quelques minutes, la ou le portage complet en demande
quatre-vingt-dix.

## Memoire et processeurs

Mesure sur ce noyau, avec la sonde reseau comme charge :

| RAM | demarrage + scenario | sonde |
|---|---|---|
| 2048 Mio | 20 s | vert |
| 8192 Mio | 22 s | vert |
| 12288 Mio | 23 s | vert |

Le defaut de `run.ps1` est donc **12288 Mio** : la plus grande valeur
reellement eprouvee, pour trois secondes de demarrage en plus. 16384 reste
accepte par le parametre mais **n'a pas pu etre verifie ici**, l'hote de
developpement ne disposant que de 15 Gio — QEMU echoue avant le premier
instruction du noyau, avec `cannot set up guest memory`. Ce n'est pas une limite
du noyau, et ce n'est pas une preuve qu'il la franchit.

**Un seul vCPU, et ce n'est pas une timidite.** Le noyau ne sait pas demarrer un
second processeur :

- il ne lit ni ACPI ni la table MADT, donc ignore combien de processeurs
  existent ;
- il n'a pas de LAPIC, donc ne peut pas emettre la sequence INIT/SIPI qui
  reveille un processeur applicatif ;
- il route ses interruptions par le PIC 8259, qui ne parle qu'au BSP ;
- `kernel::task` tient une file d'ordonnancement unique, sans verrou pour deux
  ordonnanceurs concurrents.

Demander huit vCPU donnerait huit cœurs a QEMU dont sept resteraient eteints.
`run.ps1` accepte le parametre et **previent** au lieu de laisser croire a une
acceleration.

## Ce qui bloque encore, et ce qui est desormais ecarte

Le job Internet ne passe toujours pas : `M9_NAVIGATION_COMMITTED` n'apparait
pas pour `https://example.com/`. Mais la mesure a change de nature.

**Ce que la correction a produit sur le vrai scenario Ladybird** — meme job,
avant et apres :

| | RequestServer |
|---|---|
| avant | `cpu 49 %` en continu, cinq minutes |
| apres | `cpu 0 a 10 %` |

La boucle a plein processeur est donc bien morte la aussi, dans le programme
reel et pas seulement dans la sonde.

**Ce qui est ecarte.** LibDNS ne lit pas en bloquant : il pose un
`Core::Notifier` sur son socket et depend donc de `poll()`. L'hypothese la plus
naturelle etait que `poll` ne signale jamais la lisibilite — le meme genre de
defaut que le reveil manquant deja documente pour le corps HTTP (`M9_BODY_DRAIN`).
Le CAS 4 de la sonde reproduit exactement ce chemin :

```text
[dns-probe] CAS4 poll rend=1 revents=1 attente_ms=22
[dns-probe] CAS4 recu=61 id=0x5555
[dns-probe] CAS4_OK
```

`poll` signale la lisibilite en 22 ms et le datagramme est bien la. **Cette
hypothese est donc refutee**, et avec elle toute la couche noyau : routage,
attente, et reveil par `poll` sont mesures corrects sur les quatre motifs.

**Ce qui reste.** Le verrou est au-dessus du noyau — dans `LibDNS`,
`RequestServer` ou la boucle d'evenements de `LibCore` telle qu'elle tourne sous
Bouchaud. Le journal s'arrete apres `M9_DOCUMENT_BODY_LOCAL_UNPAUSED` sans
qu'aucun des trois marqueurs de retour de RequestServer n'apparaisse.

C'est une zone fermee de plus, pas une reussite : la navigation par nom n'est
toujours pas acquise.

## Pourquoi M9 et M12 passent alors que l'essai Internet echoue

La difference entre les trois scenarios tient a un seul caractere dans l'URL.
`M9_HTTP` vise `http://10.0.2.2:18080/`, `M12_HTTPS` vise
`https://10.0.2.2:18443/` : deux **adresses**. L'essai Internet vise
`https://example.com/` : un **nom**.

Or `DNS::Resolver::lookup` court-circuite les adresses litterales — il
reconnait la forme IPv4, fabrique un `LookupResult` sur place et ne touche
jamais au reseau :

```cpp
if (auto maybe_ipv4 = IPv4Address::from_string(name); maybe_ipv4.has_value()) {
    ...  // resultat synthetise, aucune requete emise
}
```

Consequence directe, et elle vaut d'etre ecrite noir sur blanc : **M9 et M12 ne
testent pas le DNS du tout**. Ils traversent bien l'etat `DNSLookup`, mais en
ressortent sans avoir envoye un octet. Tout ce qu'ils prouvent — l'aller-retour
IPC des cookies, libcurl, TLS, le `RequestPipe`, le rendu — est donc acquis
*independamment* du DNS. Et symetriquement, la seule etape que l'essai Internet
exerce en plus des deux autres est la resolution reelle d'un nom.

Cela reduit la zone de recherche, mais ne designe pas encore le coupable : c'est
un raisonnement, pas une mesure.

## La sonde qui nomme l'etat d'arret

`Request` est une machine a etats explicite — `Init`, `ReadCache`,
`WaitForCache`, `DNSLookup`, `RetrieveCookie`, `Connect`, `Fetch`, `Complete`,
`Error` — et chaque passage traverse `transition_to_state`. Une sonde a cet
unique endroit remplace le silence par le nom de l'etat atteint, donc par un
sous-systeme unique a examiner :

| arret observe | sous-systeme designe |
|---|---|
| `DNSLookup` | `LibDNS`, ou le socket UDP du resolveur |
| `RetrieveCookie` | l'aller-retour IPC des cookies |
| `Connect` / `Fetch` | libcurl, TLS, ou les notifiers de socket |

`prepare-m9-diagnostics.py` pose donc six marqueurs, tous derriere
`BOUCHAUD_M9` comme le reste du portage :

| marqueur | ce qu'il etablit |
|---|---|
| `M9_RS_START_RECU` | RequestServer a bien recu la requete par IPC |
| `M9_RS_STATE` | chaque transition, donc l'etat exact ou la machine s'arrete |
| `M9_RS_DNS_QUERY` | le nom interroge et le serveur retenu |
| `M9_RS_DNS_RESOLU` | la resolution a abouti, et en combien d'adresses |
| `M9_RS_COOKIE_DEMANDE` | la demande de cookie est partie vers le client |
| `M9_RS_COOKIE_RECU` | le client a repondu |

Upstream porte deja la ligne qu'il faut, mais derriere `REQUESTSERVER_DEBUG`,
un drapeau de compilation : l'activer recompilerait tout RequestServer en mode
verbeux et noierait la console serie sous le trafic de chaque requete. La sonde
reprend la meme information sous la variable d'environnement que le portage
utilise deja.

Ces marqueurs n'ajoutent aucun comportement : ils n'observent que le chemin
existant, et disparaissent des que `BOUCHAUD_M9` n'est pas pose.

## Une primitive POSIX fausse, trouvee en chemin

En suivant le chemin de lecture de LibDNS jusqu'au noyau, une des trois etapes
s'est revelee fausse chez nous. `Core::UDPSocket::read_some` commence par
demander combien d'octets attendent :

```cpp
auto pending_bytes = TRY(this->pending_bytes());   // ioctl(fd, FIONREAD)
if (pending_bytes > buffer.size())
    return Error::from_errno(EMSGSIZE);
return m_helper.read(buffer, default_flags());
```

Le garde-fou existe parce qu'un `recv` sur une prise a datagrammes en consomme
**un entier** : lire dans un tampon trop petit ne tronque pas la lecture, cela
jette le reste. Ladybird prefere donc rendre `EMSGSIZE` et laisser l'appelant
agrandir son tampon.

Le noyau Bouchaud rendait `0` pour toute prise inet, quelle que soit la file :

```rust
fn pending_bytes(kind: &FdKind) -> usize {
    match kind {
        FdKind::Pipe(shared, true) => shared.borrow().buffer.len(),
        FdKind::SocketPair(inbox, _) => inbox.borrow().octets.len(),
        _ => 0,          // <- toute prise inet tombait ici
    }
}
```

Le test ne se declenchait donc jamais, et un datagramme plus grand que le tampon
aurait ete tronque en silence au lieu d'etre signale.

`net::octets_lisibles` applique desormais la regle de Linux, qui n'est pas la
meme selon la famille : sur un flux, le contenu du tampon de reception ; sur un
datagramme, la taille du **prochain** datagramme — jamais leur somme, sans quoi
un lecteur qui dimensionne son tampon sur cette valeur tronquerait tout ce qui
suit le premier.

**Ce que cette correction n'est pas.** Elle ne debloque pas la navigation par
nom, et le CAS 5 le montre lui-meme : avec `FIONREAD` a `0`, la ligne suivante
lisait quand meme les 61 octets. La comparaison `pending > buffer.size()` est
fausse dans le sens permissif, donc inoffensive tant que le tampon de 16 Kio de
`BufferedSocket` depasse toute reponse DNS. C'est une primitive incorrecte
corrigee parce qu'elle est incorrecte, pas un correctif de la panne.

### Le CAS 5, et les deux hypotheses qu'il ferme

La sonde rejoue la sequence de `Core::UDPSocket` telle quelle : `connect()` pour
poser un pair, puis les **deux** interrogations de lisibilite que
`process_incoming_messages` enchaine — celle de la boucle d'evenements, avec un
delai, puis la sienne, avec un delai **nul** — puis `FIONREAD`, puis `recv`.

```text
[dns-probe] CAS5 connect=0
[dns-probe] CAS5 envoye=29
[dns-probe] CAS5 poll1=1 poll2=1 revents=1
[dns-probe] CAS5 ioctl=0 FIONREAD=61
[dns-probe] CAS5 recu=61 id=0x7777
[dns-probe] CAS5_OK
```

`poll2=1` **refute** l'hypothese la plus seduisante : celle d'une lisibilite
consommee par la premiere interrogation, qui aurait fait sortir LibDNS de sa
boucle sans jamais lire, datagramme toujours en attente. Le noyau signale la
lisibilite autant de fois qu'on la demande.

La preuve negative tient dans les deux sens : en retirant la seule ligne du
correctif, `FIONREAD` retombe a `0` et le CAS 5 echoue avec le message prevu.
La sonde discrimine donc bien ce qu'elle pretend mesurer.

## Ce que M13 ne prouve pas

Ni TLS, ni HTTP, ni Ladybird : la sonde s'arrete a la couche que le defaut
touchait. Que le trajet complet — nom, DNS, TCP, TLS, HTTP, rendu — aboutisse
depuis Ladybird reste a etablir par le scenario Internet de
`ladybird-native-browser.yml`, qui demande le portage complet.
