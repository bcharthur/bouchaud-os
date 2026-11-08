# Passation Bouchaud OS — audit et consolidation du 8 novembre 2026

> **Document daté, non automatique.** Il décrit les faits vérifiés sur GitHub et les observations locales fournies le 8 novembre. À chaque reprise, refaire `git fetch origin` et consulter les **workflows sur le SHA exact**. Ne jamais transformer une CI historique verte en preuve de la dernière version. Ce document n'est **pas** une déclaration de release stable.

## Référence GitHub vérifiée

- Dépôt : `bcharthur/bouchaud-os`.
- Base de cet audit : `main@593a015ddc8476fb08ab53211cf108b27f8b0ede` (fusion #270, garde de non-régression).
- Première observation avant fusion : `main@26b74d5d20991e9021b0da4067b0b085a5569f8a` (#269, garde A/B et Échap).
- À l'ouverture de l'audit : **11 PR ouvertes, toutes en draft**, **1 issue ouverte** (#264, panique SMP4 intermittente) et **26 branches distantes**. Une PR a été intégrée durant l'audit (#270). Les chiffres sont des instantanés et évoluent.
- `docs/current/status.json` porte encore `as_of=2026-11-04`, `observed_main_head=e0257c60` : document historique, **pas** le SHA courant. Ne pas réécrire les verdicts sans campagne de qualification.
- La protection de branche `main` n'a pas pu être lue par le connecteur GitHub (HTTP 403), statut **UNKNOWN**, et non « protégée ».

## Fonctionnalités déjà intégrées à main

- Atelier Experience 2.0, dock, barre, centre de contrôle, Fichiers : PR #242–#244.
- DOOM natif en ring 3, icône officielle et port Windows reproductible : #252, #253 et #266 (les deux dernières intégrées à la branche de DOOM avant #252). **Ne pas refusionner les anciennes branches de DOOM à l'aveugle**.
- USB BOT et garde A/B : #263, #265, #269.
- Barrière de non-régression : #270, fusionnée après vérification des CI du commit `ddf7de75`. Les CI de `main@593a015d` sont à vérifier au SHA après la fusion.

## Expériences du 8 novembre : TRAVAIL LOCAL, NON QUALIFIÉ SUR MAIN

Séquence rapportée localement : V10 (logo et immersion), V11/V11.1 (fenêtrage, URL Google), V12 (chrome Ladybird/onglets), V13 (tentative de baisse des traces), V14 (tentative d'optimisation).

- Branche distante V12 : `feat/atelier-ladybird-v12-20261108-020817@e82ad93f` ; trois fichiers C++ touchés : `BouchaudChrome.h`, `BrowserWindow.cpp`, `main.cpp`. CI Fast, Integration et Reliability sont vertes au SHA `e82ad93f` ; le workflow natif 42432926721 était **encore en cours au moment du contrôle**. Ne pas affirmer l'artefact produit avant verdict.
- Des builds Ladybird WSL ont été testés en local ; des traces et captures montrent Google affiché. Le temps de chargement observé reste d'environ une minute et le défilement est saccadé. **Objectif 3 secondes = cible, pas mesure atteinte**.
- Le 8 novembre, les commandes fournies montrent encore `make_app(0, home, &mut spawn_n)` dans `src/gui/window_manager.rs:639`, aucune occurrence de `PROFIL_INTERACTIF_RAPIDE` dans `services.rs`, aucune de `BO_INTERACTIVE_PERF` dans `stage2.rs`, et marqueur `native-browser-m9/V14_PERF_CAPABLE` absent. Donc **V14 n'est PAS démontrée installée**.
- Les logs transmis montrent des inventaires `C22:VMA` massifs, des coûts de page faults/IPC, des synchronisations `PERSIST-SLOW` et beaucoup de rafraîchissements du chrome. Ne pas les masquer ou supprimer les gardes de tests pour obtenir un faux gain.
- Le noyau porte encore le gestionnaire de fenêtres Atelier (`src/gui/window_manager.rs`, fil noyau). Les moteurs Ladybird **sont déjà en ring 3** (`/bo-navigateur`, `RequestServer`, `WebContent`, `ImageDecoder`, `Compositor`), donc « sortir Ladybird du noyau » est une formulation incorrecte. `windowd` #261 constitue une première tranche, pas un remplacement du gestionnaire existant.

## CI et anomalies ouvertes

- `Nightly Full` #42358478852 sur `main@26b74d5d` : **rouge**, le job « resolve Ladybird » ne trouve aucun artefact canonique non expiré ; le smoke navigateur est alors `skipped` et `nightly-gate` refuse le faux vert, conformément au contrat. L'absence d'artefact ne prouve pas une panique noyau.
- Les jobs Fast, Integration, Reliability et Native IPC de `main@26b74d5d` étaient verts. Leur statut **ne couvre pas** le nouveau commit #270.
- Issue [#264](https://github.com/bcharthur/bouchaud-os/issues/264), panique intermittente SMP4, **à conserver ouverte** jusqu'au soak et qualification KVM dédiée ; #267 propose un correctif mais demande des preuves supplémentaires.
- Les CI de chaque branche candidate ne prouvent pas les interactions entre PR. Exiger CI au **SHA de main après chaque fusion** ; ne pas changer budgets, conditions ou `continue-on-error` pour verdir.

## Ordre de consolidation suggéré (conditionnel, NON permission de forcer)

1. **Socle** : #270 non-régression (intégrée). Attendre CI sur son commit.
2. **Petits correctifs indépendants** : #271 (IPv4), #268 (observabilité), #274 (sélecteur Ladybird), #256 (VFS) — un à un, avec vérification exacte de SHA.
3. **Infrastructure et ring 3** : #261 (`windowd` prototype), #257 (préparation Trigkey) — qualification croisée, sans inventer une preuve physique.
4. **Fenêtres et produit** : #260 (lancement externe unifié), #273 (chrome fenêtres), #272 (modes d'usage) — résoudre les chevauchements de `src/gui/window_manager.rs`, `src/gui/widgets.rs` et conserver l'UX DOOM.
5. **SMP** : #267 — garder en draft jusqu'au soak SMP4 et banc KVM4/KVM8 requis, issue #264 non résolue.
6. **Ladybird V12 puis V13/V14** : ouvrir une PR propre de V12 après build natif vert ; distinguer les changements du chrome C++ des correctifs noyau. Les modifications non commitées Windows/WSL ne peuvent pas être récupérées depuis GitHub.

## Règles de reprise et de nettoyage

- Capturer `git status --porcelain=v1`, `git diff --binary` et un manifest SHA-256 avant de toucher au worktree Windows. **Aucun `git reset --hard`, `git clean -fdx` ou suppression en masse**.
- Chaque branche avec PR ouverte reste protégée du nettoyage ; les branches `backup/` gardent de la valeur tant que la consolidation n'est pas démontrée.
- PR fermée ou fusionnée ≠ branche supprimable sans test `git cherry` / équivalence de patch / absence d'utilisation par un worktree.
- Les issues fermées représentent un historique de diagnostic ; ne pas les supprimer. Conserver #264.
- Actualiser les docs seulement avec des tests, versions et captures datés, sans confondre preuves HOST/TCG/KVM/PHYSICAL.
- Après la prochaine session, lancer une vraie qualification : terminal absent au boot, Google interactif mesuré, défilement chronométré, p95 entrée→présentation, P13 SMP4, non-régression DOOM et Ladybird, Nightly avec artefact compatible.

## Sources de contrôle

- [Pull requests](https://github.com/bcharthur/bouchaud-os/pulls), [Actions](https://github.com/bcharthur/bouchaud-os/actions), [branches](https://github.com/bcharthur/bouchaud-os/branches).
- [Issue SMP4 #264](https://github.com/bcharthur/bouchaud-os/issues/264).
- [Nightly fail-closed #42358478852](https://github.com/bcharthur/bouchaud-os/actions/runs/42358478852).
- [Build Ladybird V12 #42432926721](https://github.com/bcharthur/bouchaud-os/actions/runs/42432926721).
