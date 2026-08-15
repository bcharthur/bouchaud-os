# Divergences Bouchaud par rapport a Ladybird upstream

Un patch par sujet, numerote, applique dans l'ordre par `../fetch.sh`.

## Regles

1. **Un patch n'existe que s'il est specifique a Bouchaud.** Tout ce qui
   corrige un defaut ou ameliore la portabilite en general se propose en amont,
   pas ici. Un patch garde localement est une dette a chaque montee de SHA.
2. **Un fichier de patch, un sujet.** Le nom dit lequel :
   `0001-libcore-plateforme-bouchaud.patch`.
3. **L'en-tete du patch explique pourquoi**, et si une proposition amont existe,
   son numero.
4. Un patch qui cesse de s'appliquer est le signal d'une divergence a examiner.
   `fetch.sh` s'arrete alors : il ne force jamais.

Ce repertoire est vide tant que le portage n'a pas besoin de diverger. C'est
l'etat souhaitable.
