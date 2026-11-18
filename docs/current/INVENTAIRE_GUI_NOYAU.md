# Ce que le noyau exécute encore et qui relève de l'espace utilisateur

Ce document est un inventaire mesuré au 18 novembre 2026, branche de la PR « bureau ring 3 utilisable ». Les tailles viennent de `wc -l` sur le code réel.

- **Périmètre :** l'interface graphique, les fenêtres, le bureau, les applications et les services non privilégiés.
- **Hors périmètre :** les pilotes, les interruptions, la mémoire, l'ordonnanceur et les primitives IPC, qui restent dans le noyau.
- **Volume en jeu :** `src/gui/` compte 21 723 lignes, toutes exécutées en anneau 0.

## Ce qui existe déjà hors du noyau (et sert de chemin de migration)

| Pièce ring 3 | Ce qu'elle prouve |
|---|---|
| `composited` (`userland/services/composited`) | Compose, présente, route souris et clavier, gère le foyer, déplace, redimensionne, ferme, rouvre ; dock ; il est relancé par le noyau s'il meurt (`banc_bureau_utilisable.py`, 23 contrôles). |
| Applications du protocole GUI (BOGU) : Ladybird, DOOM, `mire-gpu` | Une application = un processus, avec sa surface partagée et son canal. Elle tourne indifféremment sous le bureau noyau ou sous `composited`. |
| **`calculatrice-gui`** (nouvelle) | **Première application du bureau noyau sortie du noyau** : `src/gui/apps/calculator.rs` est supprimé. |

## Inventaire

Chaque composant est décrit dans un tableau de propriétés, puis par ses primitives manquantes et ses fichiers supprimables.

### `apps/calculator.rs` (138 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Calculatrice : dessin et évaluation |
| Dépendances | `framebuffer`, `lang::expr` |
| Pourquoi dans le noyau | Historique : le premier bureau dessinait tout lui-même |
| Cible | Ring 3, client BOGU |
| État | **Fait** (`calculatrice-gui`) |

- **Primitives nécessaires :** aucune.
- **Supprimable :** **supprimé**.

### `apps/notes.rs`, `apps/rustpad.rs` (710 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Éditeurs de texte |
| Dépendances | `ramfs`, `font`, `widgets` |
| Pourquoi dans le noyau | Accès direct au ramfs et aux polices TrueType |
| Cible | Ring 3, client BOGU |

- **Primitives nécessaires :**
  - accès fichiers (POSIX existe déjà : `open`, `read`, `write`) ;
  - rendu de texte de qualité côté client.
- **Supprimable :** `notes.rs` et `rustpad.rs` entiers, plus leurs aiguillages dans `apps/mod.rs`.

### `apps/file_explorer.rs`, `atelier/fichiers.rs` (1 311 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Gestionnaire de fichiers |
| Dépendances | `ramfs`, `fs::dossiers`, `widgets` |
| Pourquoi dans le noyau | Lit l'arbre du ramfs directement |
| Cible | Ring 3 |

- **Primitives nécessaires :** `getdents` (existe), ouverture d'un fichier « dans » une autre application (canal ou URI).
- **Supprimable :** `file_explorer.rs` et `atelier/fichiers.rs`.

### `apps/terminal.rs` + aiguillage dans `apps/mod.rs`

| Propriété | Valeur |
|---|---|
| Responsabilité | Terminal intégré |
| Dépendances | `shell::run_capture` (shell noyau) |
| Pourquoi dans le noyau | Le shell est lui-même dans le noyau |
| Cible | Ring 3 : un émulateur de terminal + un shell ring 3 |

- **Primitives nécessaires :** pseudo-terminal (pty), shell utilisateur.
- **Supprimable :** `terminal.rs` et l'aiguillage `App::Terminal`.

### `apps/services.rs`, `gui/services.rs` (1 896 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Vue des services et supervision |
| Dépendances | `kernel::services`, journal |
| Pourquoi dans le noyau | Lit les tables du noyau |
| Cible | Ring 3 (client de `serviced` et `logd`, déjà ring 3) |

- **Primitives nécessaires :** un appel natif « état des services » en lecture.
- **Supprimable :** `apps/services.rs` (902 lignes).

### `apps/journal.rs`, `insights.rs`, `system_info.rs`, `releve.rs` (464 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Journal, mesures, informations système |
| Dépendances | blackbox, compteurs `[PROC-STAT]` |
| Pourquoi dans le noyau | Lisent des compteurs internes |
| Cible | Ring 3 |

- **Primitives nécessaires :** lecture des compteurs (un `/proc` minimal ou un appel natif).
- **Supprimable :** les quatre fichiers.

### `window_manager.rs`, `window.rs`, `windowing/*` (4 319 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Boucle du bureau : fenêtres, z-order, foyer, glisser, menus, dock, icônes |
| Dépendances | Tout `gui/` |
| Pourquoi dans le noyau | Le bureau par défaut des images livrées |
| Cible | `composited` (ring 3) |

- **Primitives nécessaires :** elles **existent** (tranches 1 à 3, plus le dock). Il manque :
  - le fond d'écran et les icônes ;
  - le menu Démarrer ;
  - l'horloge et la barre supérieure.
- **Supprimable :** à terme `window_manager.rs`, `windowing/*`, `window.rs`. On ne retire qu'après la bascule par défaut **et** un chemin de retour.

### `atelier/*` hors `fichiers.rs`, `widgets*`, `jauge`, `transition`, `degats`, `composition`, `scene` (≈ 5 300 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Rendu du bureau Atelier : fond, effets, animations, widgets |
| Dépendances | `framebuffer`, `font`, `png` |
| Pourquoi dans le noyau | Le compositeur était le noyau |
| Cible | Ring 3, dans `composited` ou une bibliothèque cliente |

- **Primitives nécessaires :** lecture des assets (fichiers) et rendu de texte côté client.
- **Supprimable :** tout, une fois le bureau noyau retiré.

### `font.rs`, `polices.rs`, `texte.rs`, `png.rs`, `icones.rs` (1 281 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Polices TrueType, PNG, icônes |
| Dépendances | Assets embarqués |
| Pourquoi dans le noyau | Le rendu se faisait en anneau 0 |
| Cible | Bibliothèque ring 3 |

- **Primitives nécessaires :** aucune nouvelle.
- **Supprimable :** tout. Exception : l'écran de faute garde son atlas statique (`platform/pc/ecran_faute.rs`).

### `protocole.rs`, `client.rs`, `surface*.rs` (3 059 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Côté noyau du protocole GUI : surfaces, validation des messages, contrat de tampons |
| Dépendances | `memfd`, `canal` |
| Pourquoi dans le noyau | **Légitime** : arbitre de surfaces partagées entre processus |
| Cible | Reste noyau, réduit à la médiation |

- **Primitives nécessaires :** —
- **Supprimable :** une partie du formatage d'entrées, une fois le bureau noyau retiré.

### `lang/*` (1 766 lignes)

| Propriété | Valeur |
|---|---|
| Responsabilité | Interpréteurs (`expr`, mini-Rust, Python, pip) |
| Dépendances | Shell noyau, Rustpad, wasm |
| Pourquoi dans le noyau | Le shell noyau les appelle |
| Cible | Ring 3 |

- **Primitives nécessaires :** shell ring 3.
- **Supprimable :** `lang/expr.rs` n'a plus que l'autotest du shell comme utilisateur. La calculatrice ne l'appelle plus.

## Ordre de migration retenu

Chaque étape doit retirer du code noyau, prouvée par un banc. Le chemin de retour est le bureau noyau, tant qu'il reste le défaut.

1. **Fait (cette PR) :**
   - Calculatrice → `calculatrice-gui` ;
   - `calculator.rs` supprimé (−163 lignes nettes) ;
   - le bureau noyau la lance comme DOOM (`banc_calculatrice_bureau_noyau.py`).
2. **Notes et Rustpad → un éditeur ring 3.** Il faut d'abord une bibliothèque de texte côté client, parce que la police 5×7 actuelle ne suffit pas pour éditer.
3. **Journal, Insights, Système, Services → applications ring 3.** Il faut d'abord un appel natif de lecture des compteurs et des services.
4. **Fichiers → ring 3.**
5. **Le bureau par défaut devient `composited`.** Il faut fond, icônes, menu et barre. Le bureau noyau reste disponible par configuration (retour).
6. **Retrait de `window_manager.rs`, `windowing/*`, `atelier/*`, `font`/`png`/`icones` du noyau**, une fois l'étape 5 démontrée stable.

Limite visible de l'étape 1 : la calculatrice ring 3 dessine avec une police bitmap 5×7. L'ancienne utilisait le TrueType anticrénelé du noyau. La bibliothèque de texte de l'étape 2 rendra cette qualité aux applications ring 3.
