# Bouchaud Calculatrice — première tranche native ring 3

La Calculatrice historique d'Atelier tourne encore dans le noyau.
Ce nouveau binaire **ne la remplace pas dans le bureau** : c'est une
application native indépendante qui prouve que la logique applicative et
son exécution peuvent vivre dans un processus utilisateur sans libc.

## Compiler

```bash
bash userland/libs/bouchaud-sdk/build-program.sh \
  target/calculatrice-ring3 userland/apps/calculatrice/calculatrice.c
```

Le binaire est freestanding et contrôlé W^X par le SDK.

## Exécuter / prouver

```bash
bash tools/ci/run_native_services.sh \
  target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

Ce banc inclut `/bin/calculatrice` dans le disque userland, l'exécute par
l'autorun du noyau, et exige `CALCULATRICE_RING3_OK` dans le journal série
QEMU. Une simple compilation ne suffit pas.

## Contrat IPC optionnel

Un superviseur peut lancer `/bin/calculatrice` avec **un canal bootstrap**
en lecture/écriture/attente uniquement. Messages ASCII bornés (128 octets) :

- `ADD 2 3` -> `OK 5`
- `SUB 2 3` -> `OK -1`
- `MUL 6 7` -> `OK 42`
- `DIV 4 0` -> `ERR DIV_ZERO`

Débordement entier -> `ERR OVERFLOW`; requête invalide -> `ERR FORMAT`.
`stop` termine le processus. Pas d'accès ambient au disque ni au réseau.

## Étapes suivantes, non réalisées

Un hôte d'applications doit ouvrir le canal, superviser les cycles de vie,
fournir une surface GUI via le protocole existant et router clavier/souris.
Ensuite seulement retirer `App::Calc` du noyau et refaire les tests
crash/restart et mémoire sur QEMU et sur Trigkey.
