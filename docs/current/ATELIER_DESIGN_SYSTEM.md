# Bouchaud OS Experience 2.0 — Atelier : cartographie et design system

Branche `claude/atelier-experience-20261103`, partie de `main@69e179a4` (Atomic Platform fusionnée). Rien ici ne touche à P13, à la mise à jour A/B, à l'ordonnanceur ni à la sécurité.

## 1. Cartographie du rendu existant

Tout le bureau est un **fil noyau** (`gui::window_manager`) qui compose par **dégâts** dans un double tampon logiciel (`drivers::display::bochs`, BGA 1280×720 en QEMU BIOS, GOP sur matériel). Il ne redessine que sur événement, et au plus `PERIODE_TRAME_MS` par seconde.

| Couche | Fichier | Rôle | Garde-fous / tests qui le lisent |
|---|---|---|---|
| Boucle, entrée, dégâts | `gui/window_manager.rs` (2 431 l.) | événements, focus/z-order, plan de scène, composition | ~15 `verifie-*.py` (entrée souris/clavier, réveil, présentation…) |
| Scène | `gui/scene.rs`, `gui/degats.rs`, `gui/transition.rs` | calques (`plein` / `transparent` / `avec_ombre`), culling, rectangles à invalider par mutation | `test_scene`, `test_degats`, `test_transitions` |
| Disposition | `gui/disposition.rs` | **définition unique** des bandes haute et basse, curseur, menu | `test_disposition`, `test_transitions` |
| Géométrie des fenêtres | `gui/windowing/*` | rendu (`WindowRenderGeometry`), test d'impact, rayon 10, titre 32 px, ombre 8 | `test_fenetrage`, `test_geometrie_arrondie` |
| Primitives | `gui/graphics.rs` | rectangles arrondis par segments, AA, `melange` | `test_anticrenelage`, `test_rendu` |
| Framebuffer | `gui/framebuffer.rs` → `bochs.rs` | `fill_rect_rgb`, `blend_rect_rgb` (mélange sur le tampon), `ligne_mut`, découpe | — |
| Texte | `gui/font.rs`, `polices.rs` | DejaVu via `fontdue`, glyphes en cache par (car., corps) | `test_texte` |
| Icônes | `gui/icones.rs`, `png.rs` | PNG décodés et réduits une fois par taille | `verifie-rendu-professionnel.py` |
| Peintres | `gui/widgets.rs` | fond, barre haute, icônes, fenêtres, dock, menu, curseur | `verifie-lien-reseau`, `verifie-chargement-unique`, `verifie-surface-tampons`, `tools/dev/verifie-v15.py` |
| Applications natives | `gui/apps/*` | terminal, fichiers, calculatrice, rustpad, notes, services… peignent leur contenu **pour une surface sombre** | `verifie-rendu-professionnel`, `verifie-fenetre-services` |
| Thème historique | `gui/theme/tokens.rs` | palette sombre (`COLOR_*`) utilisée par les applications | — |
| Clients ring 3 | `gui/client.rs`, `surface*.rs`, `protocole.rs` | navigateur Ladybird, surfaces partagées | `verifie-surface-tampons`, `verifie-protocole-gui` |

Ce qui contraint le design :
- **Pas de flou réel** : tout est CPU, sous TCG le flottant est émulé. Le « verre » est un mélange translucide sur un fond doux, ce qui donne visuellement le même résultat sans coût.
- **Une forme = une empreinte déclarée** : tout pixel peint doit être dans les bornes que le plan de scène annonce, sinon il laisse des traînées.
- **Un calque translucide doit être déclaré `transparent`** : déclaré opaque (`plein`), il fait sauter le fond sous lui.

## 2. Architecture du design system `gui::atelier`

```
gui/atelier/
  jetons.rs     PUR   couleurs (encre, verre, surfaces, accent, ombre), rayons,
                      espaces (échelle de 4), corps typographiques, durées
  mouvement.rs  PUR   courbes (linéaire, sortie cubique, entrée-sortie), Interpolation,
                      mélange de couleurs — entier, sans flottant
  dock.rs       PUR   géométrie du dock flottant : cadre, cellules, séparateur, capacité
  fond.rs             paysage d'aube calculé UNE fois par résolution, puis recopié
  matiere.rs          peintres : ombre douce, pavé/filet AA à opacité, plaque de verre,
                      bande de verre, corps de fenêtre
```

Règles :
1. **Les peintres demandent un rôle, pas une valeur** : `jetons::VERRE_DOCK`, pas `0xb4`. Une valeur n'existe qu'une fois.
2. **Pur d'abord** : jetons, mouvement et géométrie sont testés sur l'hôte (`tools/gui/test_atelier.rs`, 49 tests), en incluant les mêmes octets que le noyau.
3. **Une géométrie, trois consommateurs** : `atelier::dock` alimente à la fois le peintre (`widgets::draw_taskbar`), le test d'impact (`window::start_btn(n)`, `window::taskbar_btn(i, n)`) et les dégâts (`disposition::barre_taches`). Des assertions à la compilation (`window.rs`) lient `DOCK_H`, `HAUTEUR_ZONE_DOCK`, `ICONE`/`TAILLE_DOCK` et `OMBRE`/`DEBORD_OMBRE`.
4. **Le coût d'un bureau immobile est nul** : rien n'anime sans demande. Le fond est calculé une fois, et une trame d'horloge ne recompose que la barre du haut.
5. **Mouvement** : durées de 120, 200 ou 240 ms, jamais plus de 320 ; sortie cubique par défaut. Une interpolation finie vaut **exactement** sa valeur d'arrivée.

## 3. Incrément 1 livré : bureau, dock et fenêtres

- **Fond** : ciel d'aube en dégradé, halo, trois chaînes en bruit « en arêtes » avec neige, lac qui reflète, voile bas sous le dock. Entier, calculé en 59 à 75 ms sous TCG (`BOUCHAUD_ATELIER_FOND_CALCULE`).
- **Barre du haut** : bande de verre.
  - À gauche : pastille de marque, « Bouchaud OS », menus (inertes dans cet incrément).
  - Au centre : date et heure en français.
  - À droite, de droite à gauche : réseau, puis FPS à l'encre tenue.
  - Les diagnostics CPU, tas, RAMFS et xHCI quittent l'écran ; ils restent dans le journal série.
- **Dock flottant** : pastille de verre centrée, ombre douce. Il contient la cellule Applications (qui ouvre le menu), un séparateur, puis une icône 40 px par fenêtre ouverte. Un point d'accent marque la fenêtre au premier plan, un point tenu une fenêtre réduite. Le dock grandit et rétrécit en restant centré.
- **Fenêtres** :
  - ombre douce mélangée, à la place des anneaux gris opaques ;
  - barre de titre claire, titre à l'encre, gras si la fenêtre est active ;
  - boutons en pastilles au survol, fermeture rouge.
  - Le **corps** garde la surface sombre : les applications natives peignent encore pour elle (voir §4).
- **Marque** « Bouchaud OS / Atelier » au tiers haut du ciel.
- **Banc de preuve** : `tools/ci/run_bureau_capture.sh`, qui fait un `screendump` réel du framebuffer QEMU. `BO_ATELIER_OUVRE=1,4,9` ouvre des applications au démarrage par `make_app`, c'est-à-dire le chemin du clic.

Captures réelles (QEMU TCG, BIOS, 1280×720) :
- avant : [capture-avant-main-69e179a4.png](atelier/capture-avant-main-69e179a4.png) ;
- après : [capture-atelier-bureau.png](atelier/capture-atelier-bureau.png) et [capture-atelier-quatre-fenetres.png](atelier/capture-atelier-quatre-fenetres.png).

Coût mesuré : `bureau-premiere-trame` en QEMU TCG, même image, sans fenêtre supplémentaire.

| | Démarrages |
|---|---|
| `main` | 4 829 / 4 836 ms |
| Atelier | 4 885 / 5 047 / 5 069 ms |

Soit **+50 à +240 ms, une seule fois**. Le surcoût vient du fond (65 à 73 ms) et de la rastérisation des deux corps de la marque. Une version intermédiaire en flottant coûtait +2,3 s ; elle a été réécrite en entier.

Preuves :
- `test_atelier` : 49 tests ;
- suites GUI hôte : toutes vertes ;
- garde-fous d'architecture : 209/209 ;
- `CI_POLICY_OK`.

## 3 bis. Incrément 2 livré : vrais assets (BOUCHAUD_ATELIER_ASSETS_V1)

**Bibliothèque d'icônes retenue : Lucide** (ISC, `lucide-static` 1.67.0, 2 296 icônes, grille 24 px, trait 2 px).

| Bibliothèque | Licence | Cohérence | Intégration | Choix |
|---|---|---|---|---|
| **Lucide** | ISC | une seule grille, un seul trait, très homogène ; bibliothèque par défaut de shadcn/ui | SVG simples (`path`, `rect`, `circle`) que resvg rend parfaitement | **retenue** |
| Tabler | MIT | homogène, plus de 5 000 icônes, un peu plus « technique » | idem | alternative valable |
| Phosphor | MIT | six graisses, très riche | plus lourde à sous-ensembler | non retenue |
| Papirus / Tela | GPL-3.0 | icônes applicatives colorées, mais d'un autre langage visuel | licence plus contraignante pour des assets embarqués | **écartée** |

Les icônes applicatives sont des **tuiles Atelier** : carré arrondi (23 %), dégradé vertical, reflet doux et glyphe Lucide. On garde ainsi un seul langage iconographique, sans mélange de styles.

**Pipeline** : `tools/assets/fabrique-atelier.py` (dépendances de développement : `resvg-py`, `pillow`).
- **Sources** : `assets/atelier/sources/` contient 14 glyphes Lucide (pas la bibliothèque), le SVG des tuiles, le logo SVG et la photo source.
- **Sorties** : dans `src/assets/atelier/`. Chaque taille (18, 40, 56) est rendue depuis le vecteur, jamais réduite depuis une grande image.
- **Format** : PNG RGBA 8 bits, que décode le décodeur PNG du noyau, déjà en place et testé. Je n'ai pas introduit de nouveau format.
- **Manifeste** : `MANIFEST.json` donne l'empreinte, la taille et la licence de chaque fichier.
- **Garde** : `tools/verifie-assets-atelier.py` (empreintes, licences, logo provisoire, sous-ensemble Lucide, `include_bytes` couverts), avec 8 mutations détectées.
- **Attributions** : `assets/atelier/ATTRIBUTIONS.md`.

**Fond** : photographie « summer 1am » de **Risto Saukonpää** (KDE, LGPL-3.0, texte de licence joint), recadrée « cover » en 1280×720 et en 1920×1080.
- À l'exécution, la variante exacte est décodée une fois. Sinon, la plus grande est décodée puis recadrée « cover » par interpolation bilinéaire, en virgule fixe.
- Si le décodage échoue, un dégradé de repli prend le relais.
- Le marqueur `BOUCHAUD_ATELIER_FOND` publie la source, le mode, la durée et la RAM. Le fond procédural de l'incrément 1 est supprimé.

**Logo** : `bouchaud-os-provisoire.svg`, en 18 px dans la barre et en 72 px au-dessus de la marque. Il est **PROVISOIRE**, car le dépôt n'avait aucun logo officiel.

**Dock** : tuiles de 40 px et glyphe `layout-grid` teinté (masque). Le comportement, le test d'impact et les dégâts sont inchangés.

**Icônes** : `gui::icones` charge les tuiles (cache par icône et par taille), et Moniteur et Journal en ont désormais une. Les anciennes icônes et `fabrique-icones.py` sont retirés.

Captures réelles :
- [comparaison-avant-apres.png](atelier/comparaison-avant-apres.png) ;
- [capture-atelier2-bureau-1280x720.png](atelier/capture-atelier2-bureau-1280x720.png) ;
- [capture-atelier2-quatre-fenetres-1280x720.png](atelier/capture-atelier2-quatre-fenetres-1280x720.png) ;
- [capture-atelier2-uefi-2560x1600.png](atelier/capture-atelier2-uefi-2560x1600.png). OVMF a retenu son plus grand mode GOP, 2560×1600, et non 1920×1080 ; le fond y passe en mode « cover ».

Mesures (QEMU TCG) :

| | `main` | incrément 1 | incrément 2 |
|---|---|---|---|
| `bureau-premiere-trame`, BIOS 1280×720 | 4 829 / 4 836 ms | 4 885 / 5 047 / 5 069 ms | 5 178 / 5 308 / 5 405 ms |
| Préparation du fond | — | 65–73 ms (calcul) | 324–359 ms (décodage PNG), `exact` |
| Fond UEFI 2560×1600 (`cover`) | — | — | 1 095 ms, 16 000 Kio |
| RAM du fond, 1280×720 | — | 3 600 Kio | 3 600 Kio |
| Tuiles décodées (toutes tailles) | — | — | 217 Kio au plus (mesuré, plafond testé à 256 Kio) |
| Image de démarrage (bootimage BIOS) | 26 600 448 o | — | 28 915 712 o (+2,3 Mo) |
| Coût par trame | — | recopie de lignes | identique (recopie de lignes ; tuiles en `blit_argb_span`) |

Le surcoût est payé **une seule fois**, à la première trame, et vient du décodage PNG en TCG. Si besoin, deux pistes : un format à décodage plus rapide, comme QOI, ou un décodage après la première trame.

Preuves :
- `test_assets_atelier` : 7 tests (décodage, tailles, alpha, budgets, entrée corrompue sans panique, recadrage « cover ») ;
- `test_png` mis à jour sur les tuiles ;
- 22 suites GUI vertes ;
- garde-fous d'architecture : 210/210 ;
- `CI_POLICY_OK`.

## 4. Limites connues de l'incrément 1 (assumées)

- **Contenu des applications encore sombre** : Fichiers, Notes, Calculatrice et les autres peignent pour `theme::COLOR_SURFACE`. Le chrome est Atelier, le contenu ne l'est pas encore (prochain incrément). Le Terminal reste sombre volontairement.
- **Pas de mise à l'échelle HiDPI** : en 2560×1600, l'interface garde ses tailles en pixels et paraît minuscule.
- **Icône du navigateur provisoire** (tuile « boussole ») : l'identité de Bouchaud Navigateur et son chrome restent à faire, sans toucher au moteur Ladybird.
- **Le verre n'est pas flouté** : c'est une matière translucide légère. Aucun flou d'arrière-plan n'est implémenté, et rien ne le prétend.
- **Pas encore de transitions animées** : `mouvement.rs` est prêt et testé, mais il n'est pas branché sur la boucle du compositeur (incrément 2).
- **Pas de survol sur le dock** : un déplacement de souris n'invalide que l'empreinte du curseur, donc un voile de survol resterait sur la cellule quittée. Il viendra avec l'incrément 2, qui invalide la cellule quittée et la cellule atteinte.
- **Menu Démarrer encore sombre** : il est repositionné au-dessus du dock, mais pas restylé.
- **Menus de la barre inertes.**
- **Icônes du bureau** : celles d'avant, libellés blancs ombrés.
- **`test_transitions`** modélise encore les anciens boutons de barre alignés à gauche. Ses dégâts sont vérifiés contre la nouvelle bande du dock, et le modèle de position reste à aligner.

## 5. Feuille de route

| Incrément | Contenu | Preuve |
|---|---|---|
| **2 — Mouvement** | Registre d'interpolations dans le compositeur : demande de trame tant qu'une interpolation n'est pas finie, dégâts bornés à l'élément animé. Survol du dock (fondu de 120 ms), ouverture et fermeture de fenêtre (opacité et échelle de 240 ms), menu (200 ms). | test hôte « bureau immobile = 0 trame », mesure de trames pendant l'animation, captures en série |
| **3 — Fichiers Atelier** | Palette claire pour les applications (`atelier::jetons` aussi pour le contenu) ; gestionnaire de fichiers refait : barre latérale (Récents, Documents…, Appareils), grille d'icônes, chemin. | capture, `verifie-rendu-professionnel` mis à jour |
| **4 — Lanceur et centre de contrôle** | Menu Applications en grille avec recherche ; panneau de réglages rapides. | captures et tests d'impact |
| **5 — Paramètres** | Apparence (Clair, Sombre, Automatique), couleur d'accent. | — |

Ordre fixé par le produit : la qualification physique (Trigkey) et le GPU P10+ passent avant les innovations d'interface au-delà de ces incréments.
