# Portage de WebContent

## Etat actuel

Bouchaud a deja un renderer separe : `moteur/superviseur.py` forke un processus,
communique par `socketpair`, partage une surface, applique `RLIMIT_AS`. La forme
est la bonne ; le contenu est notre moteur Python.

`Services/WebContent` d'upstream est le processus equivalent chez Ladybird.

## Correspondance

| Ladybird | Bouchaud actuel | Etat |
|---|---|---|
| Processus `WebContent` | renderer forke | forme identique |
| Transport `LibIPC` | protocole maison | a remplacer (M5) |
| Surface partagee | `memfd` + `mmap` | supporte |
| Limites memoire | `RLIMIT_AS` via `prlimit64` | supporte |
| `RendererSandboxLinux.cpp` | restrictions ad hoc | a formaliser |
| Un renderer par onglet | un seul renderer | a etendre (M13) |

## Politique de privileges cible

Elle prolonge celle deja appliquee au navigateur actuel, ou le **noyau** — et non
la bonne volonte du client — tient la regle : `/dev/fb0` redirige vers la surface
de la fenetre, `/dev/input/*` refuse (`EACCES`).

| Processus | Framebuffer | Entrees | Sockets Internet | Fichiers | IPC |
|---|---|---|---|---|---|
| UI | surface WM | oui | non | profil utilisateur | oui |
| `WebContent` | surface partagee seule | **non** | **non** | **non** | oui |
| `RequestServer` | non | non | **oui** | cache seul | oui |
| `ImageDecoder` | non | non | non | **non** | oui |
| `WebWorker` | non | non | non | non | oui |

La ligne qui compte est `WebContent` : c'est lui qui execute du contenu distant.
Il ne doit avoir ni ecran, ni clavier, ni reseau.

Bouchaud sait deja refuser un peripherique a un processus (`Process::ecran`
conditionne `/dev/fb0` et interdit `/dev/input/*`). Le mecanisme existe ; il faut
le generaliser en **politique nommee** plutot qu'en cas particulier du navigateur.

## Etapes

1. WebContent demarre et repond a la poignee de main IPC (M7).
2. Il rend une page HTML **locale** dans une surface (M8).
3. La surface est composee par le WM dans une fenetre Bouchaud.
4. CSS (M9), puis JavaScript par LibJS (M10).
5. Un renderer par onglet (M13).
6. Politique de privileges appliquee et testee : un WebContent qui tente
   d'ouvrir `/dev/fb0` ou une socket doit **echouer**, et le test doit le
   verifier (M14).

## Risque principal

Le nombre de processus. Un onglet = un WebContent ; chacun porte LibJS, donc ICU.
Sur une machine QEMU a 2 Gio, la question de l'empreinte se posera avant celle de
la vitesse. A mesurer des M7 avec le releve `[ps]` deja en place.
