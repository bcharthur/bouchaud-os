# tools/ci

Scripts communs aux workflows CI V2. Ils ont trois objectifs :

1. garder le YAML lisible ;
2. rendre chaque scenario reproductible localement ;
3. construire le noyau une fois par pipeline puis partager son bootimage.

Tous les scripts sont `set -euo pipefail` et acceptent leurs artefacts en
arguments lorsque cela evite une recompilation.
