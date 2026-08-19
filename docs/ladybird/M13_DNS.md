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

## La correction

C'est la primitive POSIX qui est reparee, pas un cas particulier Ladybird.

| Avant | Apres |
|---|---|
| datagramme d'un autre port : jete | route sur son port de destination |
| anneau vide : rappeler `poll_ip` | s'arreter |
| `recvfrom` bloquant : 3 000 000 de tours | delai nomme, avec `schedule()` puis `hlt` |
| `poll` : 20 000 tours | un seul passage |

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

Il construit le noyau, la sonde et le disque, lance QEMU, et verifie trois
marqueurs. Il est **bloquant en CI** (`DNS-UDP-ring3`) et ne demande aucun
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

## Ce que M13 ne prouve pas

Ni TLS, ni HTTP, ni Ladybird : la sonde s'arrete a la couche que le defaut
touchait. Que le trajet complet — nom, DNS, TCP, TLS, HTTP, rendu — aboutisse
depuis Ladybird reste a etablir par le scenario Internet de
`ladybird-native-browser.yml`, qui demande le portage complet.
