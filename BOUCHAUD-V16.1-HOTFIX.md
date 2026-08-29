# Bouchaud OS V16.1 — compile + CI wait hotfix

Ce drop-in corrige deux regressions observees lors du premier test V16 :

1. `E0689` dans `faute_cluster.rs` : le type de `window` est maintenant explicitement `u64`.
2. `run-ladybird-v16.ps1` ne cherche plus uniquement un run deja `success`. Si le premier workflow V16 est encore en cours, il attend sa fin via `gh run watch`, valide sa conclusion, puis telecharge exactement cet artefact.

`tools/dev/verifie-v16.py` verifie maintenant aussi l'annotation de type qui empeche le retour de E0689.

Extraction : racine du depot, remplacement des fichiers. Aucun script d'application.
