# Faire tourner des binaires natifs (et, au bout, un navigateur Qt)

Objectif : executer dans Bouchaud OS un navigateur moderne fourni par
l'ecosysteme Python + Qt. Ce document dit ce que cela exige reellement, ce qui
est fait, et dans quel ordre attaquer la suite.

## L'ecart a combler

Qt est une bibliotheque **C++**. QtWebEngine y ajoute **Chromium**. Pour les
heberger, un systeme doit fournir, dans cet ordre de dependance :

| Couche | Ce que c'est | Etat |
| --- | --- | --- |
| 1. Memoire virtuelle | pages, frames, espaces d'adressage | **fait** (`kernel::vm`) |
| 2. Mode utilisateur | ring 3, TSS, entree/sortie noyau | a faire |
| 3. Chargeur ELF64 | lire et mapper un executable | a faire |
| 4. Appels systeme POSIX | le plancher qu'une libc exige | ebauche (`kernel::syscall`) |
| 5. Processus et threads | `fork`/`clone`, ordonnancement, `futex` | ebauche (`kernel::process`) |
| 6. libc | musl statique d'abord | a faire |
| 7. Editeur de liens dynamique | `ld.so`, relocations, `.so` | a faire |
| 8. Runtime C++ | `libstdc++`, exceptions, RTTI | a faire |
| 9. Serveur graphique | framebuffer + entrees + protocole | partiel (framebuffer HD) |
| 10. Qt | ~5 millions de lignes C++ | a faire |
| 11. QtWebEngine | Chromium, ~30 millions de lignes | a faire |

Aucune de ces couches ne se saute. Une libc sans mode utilisateur ni
chargeur ELF n'a rien a faire tourner ; Qt sans editeur de liens dynamique ne
se charge pas ; Chromium exige en plus des processus isoles, de la memoire
executable en ecriture (JIT de V8) et une IPC.

L'ordre de grandeur est donc celui d'un projet au long cours, pas d'une
fonctionnalite. Le present document existe pour que chaque etape soit un
jalon verifiable plutot qu'une promesse.

## Couche 1 — memoire virtuelle (faite)

`src/kernel/vm.rs`.

Le bootloader nous laisse la memoire physique entierement mappee a un offset
connu et CR3 sur une table valide. On **reprend** cette table au lieu d'en
construire une : c'est ce qui permet de mapper des pages sans se couper
l'herbe sous le pied pendant l'operation.

Deux ressources distinctes :

* **frames physiques** — `FrameArena` les sert depuis les regions que la carte
  memoire du bootloader declare libres, **moins** ce que le tas et l'arene DMA
  ont deja pris (`memory::reserved_range()`). Servir ces frames-la ecraserait
  le tas du noyau, avec une panique immediate et difficile a relier a sa
  cause. Les frames rendues repassent en tete du pool.
* **pages virtuelles** — mappees via les tables x86_64 a quatre niveaux, les
  tables intermediaires manquantes etant creees a la volee.

Toute frame servie est mise a zero : sans cela un espace d'adressage
utilisateur heriterait des restes du precedent.

Verification :

```
bsh> vm            # frames libres, servies, rendues, plage reservee
bsh> vm test       # autotest
```

L'autotest est le seul moyen de prouver que la pagination fonctionne
vraiment. Il mappe une page a une adresse inutilisee, y ecrit, **relit par la
fenetre physique** — un autre chemin de traduction, donc une preuve que la
page pointe bien sur la frame annoncee — puis demappe et verifie que
l'adresse redevient intraduisible. Il refait ensuite l'operation sur une
plage de seize pages.

## Couche 2 — mode utilisateur (prochaine)

Il faut :

* des descripteurs de code et de donnees ring 3 dans la GDT (`arch::x86_64::gdt`
  n'a aujourd'hui que le ring 0) ;
* un TSS avec `rsp0`, pour que le CPU sache ou basculer la pile lors d'une
  interruption venue du ring 3 ;
* une entree noyau : `syscall`/`sysret` (MSR `STAR`, `LSTAR`, `SFMASK`) ou
  `int 0x80` ;
* le retour en ring 3 par `iretq`.

C'est la couche la plus delicate du lot : une erreur de descripteur ou de pile
ne se manifeste pas par un message mais par un triple fault, donc un reboot
silencieux de la machine. A traiter avec un test minimal — une seule fonction
utilisateur qui fait un `syscall` d'ecriture puis `exit`.

## Couche 3 — chargeur ELF64

Lire les en-tetes de programme, mapper les segments `PT_LOAD` avec les droits
qu'ils demandent, monter une pile utilisateur, poser `argv`/`envp`/`auxv`, et
sauter au point d'entree. Statique d'abord : un binaire lie statiquement n'a
besoin d'aucun editeur de liens.

## Couche 4 — appels systeme

musl statique demande une soixantaine d'appels pour un programme simple :
`read`, `write`, `openat`, `close`, `fstat`, `lseek`, `mmap`, `munmap`,
`mprotect`, `brk`, `exit_group`, `set_tid_address`, `clock_gettime`,
`ioctl`, `writev`, `readv`, `getpid`, `uname`, `rt_sigaction`,
`rt_sigprocmask`... Les fichiers passent par le RAMFS existant ; `mmap`
s'appuie directement sur la couche 1.

## Couche 9 — serveur graphique

L'OS a deja un framebuffer HD (1280x720x32, double tampon) et un curseur
souris. Il manque le protocole : un compositeur qui attribue des surfaces aux
clients et route les entrees. Qt peut eviter X11 et Wayland avec ses plugins
`linuxfb` (framebuffer brut) ou `eglfs` — `linuxfb` est de loin le plus
simple, et c'est la cible a viser.

## Ce qui tourne en attendant

`pybrowser` (voir `README.md`) est un navigateur simpliste en Python qui
verifie de bout en bout que les couches actuelles s'enchainent pour afficher
du contenu Web : interpreteur Python, pont WASI, RAMFS, pile TCP/TLS,
extraction HTML, console. `pybrowser --check` rapporte chaque maillon.

`tools/qt-browser/browser.py` est la version Qt de reference : elle tourne sur
un poste de travail ordinaire et sert de cible fonctionnelle — c'est ce que
l'OS doit finir par heberger.
