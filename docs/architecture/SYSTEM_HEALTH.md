# System Health CI

`system-health.yml` execute un seul scenario bare-metal QEMU puis reutilise son
journal dans une matrice de checks GitHub independants.

Une regression doit dire **quelle couche a casse**, pas seulement "QEMU rouge".

| Check | Portee |
|---|---|
| `foundation` | boot, devices, noyau |
| `memory` | heap, frames, VMM |
| `storage` | disque + namespace |
| `abi` | ring3, mmap, threads, pipe/poll |
| `osi-l1` | e1000 + carrier |
| `osi-l2` | Ethernet / ARP |
| `osi-l3` | IPv4 / ICMP |
| `osi-l4` | TCP vers fixture locale |
| `osi-l5-6` | TLS/crypto/presentation |
| `osi-l7` | HTTP vers fixture locale |

Les checks bloquants ne dependent d'aucun site Internet. La fixture HTTP tourne
sur le runner et est joignable par l'hote SLIRP QEMU `10.0.2.2`.

## Applications

Ladybird reste fonctionnellement teste dans `ladybird-native-browser.yml` parce
que son build est lourd et possede son propre cache. Les futures applications
peuvent publier un workflow `app-<nom>` ou rejoindre `system-health` si leur
sonde est legere.

## Ajouter une couche

1. Ajouter deux marqueurs dans `tools/health/autorun.bsh`.
2. Ajouter les attentes dans `tools/health/manifest.json`.
3. Ajouter l'identifiant dans la matrice `layers`.
4. Ne jamais rendre un check bloquant dependant d'un service Internet externe.
