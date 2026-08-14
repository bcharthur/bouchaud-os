# Portage de LibIPC

## Etat actuel

Bouchaud possede deja **deux** protocoles IPC artisanaux :

1. `src/gui/protocole.rs` + `hote.cpp` — protocole GUI v1 entre le gestionnaire
   de fenetres et un client ring 3 (en-tete 16 octets, 13 genres de message).
2. `moteur/protocole.py` + `superviseur.py` — protocole navigateur/renderer.

Les deux fonctionnent. Aucun n'est LibIPC.

## Ce que LibIPC demande

Mesure sur `Libraries/LibIPC` (39 fichiers) : AK(106), LibCore(36), LibSync(10),
LibURL(7), LibThreading(6). Cote systeme, la surface est **minuscule** :
`socket` ×1, `socketpair` ×1, `poll` ×1, `signal` ×1.

Autrement dit : LibIPC n'a presque aucune exigence propre. Tout passe par
LibCore. **Si LibCore fonctionne, LibIPC fonctionne.**

## Transport

| Besoin LibIPC | Bouchaud | Etat |
|---|---|---|
| `socketpair(AF_UNIX, SOCK_STREAM)` | `sys_socketpair` (deux canaux croises) | supporte |
| Passage de descripteurs (`SCM_RIGHTS`) | `sendmsg`/`recvmsg` avec file de descripteurs | supporte |
| Surveillance non bloquante | `poll` + `O_NONBLOCK` via `fcntl`/`FIONBIO` | supporte |
| Contre-pression | capacite 64 KiB par canal, comme Linux | supporte |
| Tampon partage | `memfd_create` + `mmap(MAP_SHARED)` | supporte |

Les quatre primitives que Ladybird exige de son IPC sont donc **deja la**, et
eprouvees : le protocole GUI v1 les utilise en production.

## Plan

1. Compiler LibIPC pour la cible (apres LibCore).
2. Test d'aller-retour entre deux processus Bouchaud : un message, une reponse.
3. Test de passage de descripteur : le parent cree un `memfd`, le passe, l'enfant
   le mappe et ecrit, le parent relit.
4. Test de contre-pression : l'ecrivain sature le canal, verifie l'absence de
   corruption de cadrage. **Ce test existe deja en substance** — c'est le defaut
   corrige sur le canal du renderer (reemission d'une trame entiere apres
   `EAGAIN`).
5. Test de mort du pair : le processus enfant meurt, le parent doit voir `POLLHUP`
   et non se bloquer.

Critere de succes : jalon M5.

## Coexistence

Le protocole GUI v1 **n'est pas remplace** par LibIPC. Il relie le noyau au
userland ; LibIPC relie des processus userland entre eux. Les deux coexistent :

    WM (noyau) --protocole GUI v1--> UI Ladybird --LibIPC--> WebContent

Remplacer le protocole GUI v1 par LibIPC supposerait de faire entrer LibIPC dans
le noyau Rust. Ce n'est pas souhaitable et ce n'est pas prevu.
