# Architecture cible — fenêtres universelles et applications ring 3

Date : 5 novembre 2026. Base de travail : PR #253 au-dessus de #252.
**La migration complète du gestionnaire de fenêtres en ring 3 n'est pas faite.**

## Responsabilités visées

| Composant | Ring cible | Contrat |
|---|---|---|
| Noyau | 0 | ordonnanceur, VM, interruptions, mémoire partagée, capacités, IPC, pilotes primitifs |
| `displayd` / `inputd` | 3 (progressif) | accès matériel via capacités, sans politique d'applications |
| `composited` / `windowd` | 3 | surface, focus, z-order, gestion des fenêtres, décorations, chargement générique |
| `appd` | 3 | manifestes, lancement, supervision, isolation, crash/restart, droits |
| `serviced` | 3 | gestion de services, du navigateur comme programme et de ses composants |
| Ladybird, DOOM, Notes, Terminal, etc. | 3 | logique propre, contenu, progression interne, fichiers avec des capacités |

La fenêtre existe indépendamment du programme. Les messages GUI sont ceux de
`src/gui/protocole.rs`. Le contenu est envoyé par l'application dans une surface
et le WM compose cette surface. L'application ne dessine jamais elle-même le
dock, la barre de titre ou la fenêtre d'attente d'Atelier.

## Ce que cette tranche modifie réellement

- `src/gui/applications_externes.rs` centralise l'identité, l'exécutable,
  les arguments, la taille et le titre de Ladybird et DOOM, sans dépendre du
  compositeur ou des détails internes des applications.
- `Client::lance` ne fabrique plus d'arguments DOOM ni de titre Ladybird
  par défaut ; il consulte le registre. Les autres programmes gardent leur nom.
- `lance_application_externe` a une seule procédure pour ouvrir, remonter,
  configurer, fermer et présenter une fenêtre, quelle que soit l'application.
- Le rendu de démarrage utilise **un titre propre à l'application** et un
  texte générique ; le contenu démarre à la première surface composable.
- Le chronomètre d'ouverture fonctionne pour les deux clients. Aucune jauge
  Atelier ne doit recouvrir une page Ladybird après le démarrage.
- La télémétrie `browser_client_start` ne doit pas compter DOOM comme browser.

Les variantes internes `App::Navigateur` et `App::Doom` restent
temporairement des adaptateurs de compatibilité : le navigateur seul a encore
un cycle de vie de courtier/serviced couplé au WM. Ne pas confondre ce jalon
avec une migration des processus de l'interface hors du noyau.

## Ce qui doit quitter le noyau, par lot indépendamment qualifiable

1. **Applications natives** : `src/gui/apps/` (Terminal, Fichiers,
   Calculatrice, Rustpad, Notes, Services, Insights, Journal). Elles doivent
   devenir des processus ring 3 avec leurs fichiers et autorisations propres.
   Utiliser le SDK dans `userland/libs/bouchaud-sdk/`, les services
   `userland/services/` et la tranche Calculatrice de la PR #258.
2. **Lanceur d'applications** : `src/app/launcher.rs`, `manifest.rs`,
   `runtime.rs`, avec des manifests validés et une politique d'installation
   implémentée en ring 3 (`appd`).
3. **Gestionnaire des fenêtres et dessin** : `src/gui/window_manager.rs`,
   `widgets.rs`, `window.rs`, `atelier/`, `icones.rs`,
   `graphics.rs`. Déporter d'abord le plan de scène/rendu vers
   `userland/services/composited/`, puis les fenêtres, les animations,
   le focus et les événements via IPC.
4. **Sessions clientes** : `src/gui/client.rs` contient aujourd'hui
   le lancement des processus, les surfaces, les canaux et l'injection
   d'environnement. Déporter les politiques au superviseur et la gestion
   de fenêtres à `windowd`. Laisser au noyau des mécanismes de handles,
   memoires partagées et vérification des droits.
5. **Courtier Ladybird** : `src/gui/services.rs` et
   `src/gui/demarrage_navigateur.rs` pilotent aujourd'hui le service depuis
   le bureau. Déplacer les décisions de démarrage, relance, arrêt et
   dépendances vers `serviced` ; supprimer tout automatisme browser du WM
   une fois le contrat ring 3 prouvé. Ne pas désactiver le traitement
   des erreurs, le watchdog ou les preuves C22 lors du déplacement.
6. **Pile réseau, systèmes de fichiers, services haut niveau** :
   `src/net/`, politique DNS/TLS et une part de `src/fs/` et des
   services de sécurité doivent évoluer vers des démons ring 3 sous capacités.
   Les pilotes ou chemins critiques encore indispensables au boot restent
   provisoirement dans le noyau, avec contrats stables et tests matériels.

## Tests non négociables pour la sortie effective de ring 0

- **Prouver le PID utilisateur** de chaque processus, et son ring 3 ;
  pas seulement vérifier un ELF présent dans le ramdisk.
- **Faire planter une application** (DOOM, Ladybird, Calculatrice, Notes)
  et prouver qu'Atelier, l'IPC et les autres applications restent vivants.
- **Tuer/reprendre windowd** sans corruption mémoire, fuite de handles ni
  perte de données ; prévoir un fallback d'affichage de secours.
- **Isolation** : aucun client ne peut peindre hors de sa surface,
  manipuler le focus d'un autre, lire ses fichiers ou ouvrir le disque brut
  sans capacité attribuée.
- **Fidélité** : mesures A/B des dégâts, du temps de trame et du réveil,
  images pixel-identiques à position/horloge identiques.
- **Qualité** : Fast/Integration/Reliability verts sur le même SHA,
  P13/SMP strict sans nouveaux faux verts, puis validation physique Trigkey.

Ces étapes doivent chacune être livrées sur une branche isolée, sans faire
passer une simple extraction de source pour une preuve de processus ring 3.
