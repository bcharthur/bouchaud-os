# Documentation historique

**Rien dans ce dossier ne decrit l'etat actuel du projet.**

Ces documents disaient vrai a leur date. Plusieurs ne disent plus vrai
aujourd'hui — ils annoncent une GDT en stub, un noyau sans allocateur, un
moteur Web sans `<iframe>`, sans IndexedDB, sans Same-Origin Policy, ou un
renderer qui n'est qu'un prototype. Toutes ces choses existent maintenant.

Ils sont conserves parce qu'ils portent quelque chose que la documentation
courante ne porte pas : **pourquoi** une decision a ete prise, et contre quoi.
Un audit qui a motive un chantier reste utile apres le chantier ; il devient
nuisible seulement quand on le lit comme un etat des lieux.

Pour l'etat reel, voir le `README.md` a la racine et les documents de `docs/`.

## Ce qu'ils sont

| Document | Ce qu'il etait |
|---|---|
| `README_V0.5.md` … `README_V0.15_WINDOW_MANAGER.md` | notes de version, du premier boot au gestionnaire de fenetres |
| `ARCHITECTURE_CIBLE.md` | l'ossature visee au moment de la V0.15, avec son etat brique par brique |
| `AUDIT.md` | l'audit du noyau qui a motive les processus, la memoire virtuelle et l'ABI Linux |
| `CODEX_BROWSER_AUDIT.md` | l'audit du navigateur qui a motive le moteur Web natif |
| `BROWSER_COMPATIBILITY_AUDIT.md` | l'ecart mesure avec les navigateurs reels, avant les chantiers CSS et JS |
| `BROWSER_WORKERS_AUDIT.md` | l'audit qui a motive les Web Workers et le clonage structure |
| `PORTAGE_WEBKIT.md` | l'etude de portage de WebKit, et pourquoi elle n'a pas ete suivie |

## La regle

Un document qui cesse d'etre vrai descend ici, il n'est pas supprime. Un
document de `docs/` qui contient une affirmation historiquement vraie et
actuellement fausse est un defaut : soit on le corrige, soit on le descend.
