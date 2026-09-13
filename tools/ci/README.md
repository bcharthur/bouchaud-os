# tools/ci

Scripts communs aux workflows CI V2. Ils ont trois objectifs :

1. garder le YAML lisible ;
2. rendre chaque scenario reproductible localement ;
3. construire le noyau une fois par pipeline puis partager son bootimage.

Tous les scripts sont `set -euo pipefail` et acceptent leurs artefacts en
arguments lorsque cela evite une recompilation.

## `run_barriere_locale.sh` — la liste, dans le depot

Le point 2 ci-dessus dit « reproductible localement ». Il l'etait ; encore
fallait-il savoir CE QU'IL FAUT lancer.

Un lot a ete pousse avec « barriere locale verte » pour seule preuve, et la
barriere GitHub l'a refuse : `dns-probe` CAS 3 mesurait qu'une attente
bloquante de cinq secondes consommait cinq secondes de processeur. Le defaut
etait reproductible ici depuis le debut -- simplement, le balayage fait a la
main ne contenait pas `tools/net/verifie-dns.sh`.

Une liste tenue de tete n'est pas une liste :

```bash
cargo bootimage
tools/ci/run_barriere_locale.sh
```

Il enchaine les garde-fous, les suites hote, l'analyse de securite, les douze
scenarios QEMU et `verifie-dns.sh`. Il NOMME les deux scenarios qu'il ne peut
pas jouer -- ceux qui exigent l'image du navigateur, absente du depot -- au
lieu de les omettre, et sa derniere ligne le redit : un scenario saute n'est
pas un scenario vert.
