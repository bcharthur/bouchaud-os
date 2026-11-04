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

## 3 ter. Incrément A livré : barre haute réelle et centre de contrôle

Branche `claude/atelier-increment-a-20261104`, issue de `f2cbb068` (tête de la PR #242).

**Barre haute (BOUCHAUD_ATELIER_BARRE_HAUTE_V2).**
- **Gauche** : logo PROVISOIRE et nom. Les menus « Fichiers, Édition, Affichage, Aller, Aide » de l'incrément 1 sont retirés : ils étaient inertes, et une commande visuelle qui ne fait rien n'a pas sa place.
- **Centre** : date et heure en français, inchangées.
- **Droite** : un seul bouton, qui ouvre le centre de contrôle. Il porte l'état du lien et le glyphe Lucide `sliders-horizontal`, avec un voile au survol et la teinte d'accent quand le centre est ouvert.

**État réseau.** Le dessin de prise en rectangles (avec sa barre oblique) est supprimé. L'état se lit par la **forme**, grâce à trois glyphes Lucide distincts rendus par la fabrique :

| État | Glyphe | Teinte |
|---|---|---|
| Connecté, lien sain | `ethernet-port` | encre |
| Connecté, lien dégradé | `ethernet-port` | avertissement |
| Sans adresse | `circle-alert` | avertissement |
| Déconnecté | `unplug` | danger |
| Sans carte | `unplug` | encre tenue |

**Absent de la barre, car rien ne le pilote :**
- **Wi-Fi** : aucun pilote ;
- **son** : `ac97.rs` n'a ni volume ni sourdine au runtime ;
- **notifications** : aucun service ;
- **recherche** : aucun index.

Aucune icône factice ne les représente. Le centre écrit « Non pris en charge pour l'instant : Wi-Fi, son, thème sombre ». Le glyphe `search`, réservé à l'incrément 2, est retiré du sous-ensemble Lucide.

**FPS.** Le compteur ne s'affiche qu'en **mode diagnostic** (`atelier::reglages`). On le bascule depuis le centre, ou au démarrage avec `export BO_ATELIER_DIAGNOSTIC=1`. Il reste dans la mise en page unique de droite à gauche, et dans chaque ligne du journal (`[FPS:…]`).

**Centre de contrôle (BOUCHAUD_ATELIER_CENTRE_V1).** Le panneau de verre de 320 px est ancré sous la barre, à droite. Sa géométrie est pure (`atelier::centre`) : le peintre, le clic et le survol lisent les mêmes boîtes.

| Bloc | Source | Interaction |
|---|---|---|
| Réseau | `etat_reseau()`, `libelle_reseau()`, `net::our_ip()`, `net::gateway()` | lecture seule |
| Affichage | `fb::width()/height()`, « rendu logiciel » (aucun GPU n'est prétendu) | lecture seule |
| Compteur de trames (FPS) | `atelier::reglages::diagnostic()` | interrupteur réel |
| Non pris en charge | texte | aucune |
| Redémarrer / Éteindre / Quitter | `power::reboot`, `power::shutdown_avec_raison(…, "centre_eteindre")`, `quit` | mêmes chemins que le menu Applications |

**Fermeture et clics.**
- Un clic hors du panneau ferme le centre ; le clic s'arrête là et ne traverse pas.
- Échap ferme le centre en priorité, même si un client a le focus.
- Le centre et le menu Applications s'excluent.

**Dégâts.**
- L'ouverture ou la fermeture invalide `centre::empreinte`, c'est-à-dire le cadre et l'ombre sur les quatre côtés, plus la barre haute.
- Le survol invalide l'ancienne et la nouvelle boîte d'action, rien d'autre.
- Le calque `Element::Centre` est `transparent` (du verre).

**Preuves.**
- 6 tests hôte dans `test_atelier`, soit 55 au total, sur 3 résolutions :
  - le panneau tient entre la barre et le dock ;
  - l'empreinte contient le panneau et son ombre ;
  - les actions sont dans le panneau et ne se chevauchent pas ;
  - le survol et le clic désignent, pixel par pixel, la boîte peinte ;
  - les blocs se suivent ;
  - le bouton reste dans la barre.
- `verifie-lien-reseau.py` est **durci** (8 mutations vérifiées à la main, dont 3 nouvelles) :
  - l'état se lit par la forme (`glyphe_reseau`) ;
  - les trois glyphes sont trois fichiers distincts ;
  - aucun `fill_rect_rgb` dans `dessine_reseau` ;
  - le FPS est soumis au mode diagnostic et reste dans la mise en page.

**Captures QEMU réelles** : clic et Échap sont injectés par le moniteur, avec `BUREAU_GESTES` dans `run_bureau_capture.sh`.

| Fichier | Ce qu'il montre |
|---|---|
| `atelier/comparaison-incA-barre-haute.png` | Barre avant (incrément 2) et après |
| `atelier/capture-incA-bureau-1280x720.png` | Bureau, état connecté |
| `atelier/capture-incA-centre-ouvert.png` | Centre ouvert après un vrai clic, survol de l'interrupteur |
| `atelier/capture-incA-centre-diagnostic-actif.png` | Après un clic sur l'interrupteur : FPS dans la barre |
| `atelier/capture-incA-apres-echap.png` | Après Échap : centre fermé, aucun résidu |
| `atelier/capture-incA-sans-carte-reseau.png` | `BUREAU_QEMU="-nic none"` : glyphe `unplug`, « Pas de carte » |

**Mesures** : QEMU TCG, BIOS 1280×720, même scénario de 62 s au repos, images construites l'une après l'autre.

| | Base `f2cbb068` | Incrément A |
|---|---|---|
| Première trame | 5364 ms | 5301 ms (5348 et 5418 sur deux autres boots ; 4513 ms sans carte réseau) |
| `desktop cpu_pct` à t = 30 s / 60 s | 3 / 0 | 2 / 0 |
| Trames utiles en 55 s de repos | 1 | 1 |
| Bootimage | 28 915 712 octets | 28 931 584 octets (+15,9 Ko : 12 glyphes) |

Le centre n'est peint que lorsqu'il est ouvert. Au repos, la barre n'est recomposée qu'au tic de l'horloge, comme avant.

## 3 quater. Incrément B livré : ouverture et fermeture animées (BOUCHAUD_ATELIER_EFFETS_V1)

**Architecture.** Le compositeur peint chaque calque directement dans le tampon arrière : une fenêtre n'a pas de surface à elle. Plutôt que de faire semblant, on lui en fabrique une, le temps d'une découpe :
1. garder le dessous (les calques inférieurs déjà peints) ;
2. peindre la fenêtre ;
3. copier ce qu'elle a peint, puis remettre le dessous ;
4. mélanger la copie, décalée de `dy` et à l'opacité de l'instant.

Tout ce qui est touché hors de la découpe est remis à l'identique avant la présentation.

**Fermeture.** Elle lit un **instantané**, c'est-à-dire une copie de pixels :
- il est pris dans `ferme_fenetre` *avant* le `remove` et avant la fin du client ;
- aucune surface n'est lue après sa libération ;
- un client mort disparaît sans animation.

| Module | Rôle |
|---|---|
| `atelier::animations` | **Pur** : registre borné (8 animations), datation, courbes, modes, union des zones. Inclus dans `test_atelier`. |
| `atelier::effets` | Noyau : `ouvre`, `ferme` (instantané), `tour` (zones à recomposer), `compose_ouverture`, `dessine_instantane`, mesure du coût par trame. |
| `atelier::reglages` | `animations()` : `normales`, `reduites` (fondu seul, plus court), `aucune`. Réglable dans le centre ou par `BO_ATELIER_ANIMATIONS=`. |

**Paramètres** (horloge monotone, courbes entières) :

| | Normal | Réduit | Aucun |
|---|---|---|---|
| Ouverture | 200 ms, sortie cubique, opacité 0→1, montée de 10 px | 110 ms, opacité seule | — |
| Fermeture | 150 ms, entrée-sortie cubique, opacité →0, réduction à 96 % (virgule fixe, plus proche voisin), ombre qui s'efface | 90 ms, opacité seule | — |

**Règles qui protègent le reste.**
- **Registre vide ⇒ boucle identique à avant.** `tour` ne salit rien ; l'échéance de veille ne devient « trame due » que si `effets::actif()`. Test hôte : une minute sans geste, zéro trame.
- **Datation à la première trame composée** (`EN_ATTENTE`), et non au lancement. Sous TCG, la première trame après une ouverture coûte environ 90 ms : datée de la création, l'animation n'en montrait qu'une image.
- **Rien ne s'anime avant la première trame du bureau** : les fenêtres du démarrage apparaissent avec lui, sans alourdir la trame la plus chère.
- **Calques.**
  - Une fenêtre qui s'ouvre est déclarée `transparent` sur l'union de sa position finale et de sa position décalée. Opaque, elle masquerait le fond qu'elle laisse voir ; bornée à la position finale, la découpe l'écarterait.
  - L'instantané forme un calque `Element::Fantome`, transparent, au-dessus des fenêtres.
- **Dégâts.**
  - Chaque trame recompose l'ancienne **et** la nouvelle zone (`Registre::recouvre`) : une fenêtre déplacée pendant son ouverture ne laisse pas de bande.
  - Une animation finie, évincée ou annulée rend sa dernière zone pour une ultime trame propre.
- **Interruption.**
  - Fermer pendant l'ouverture annule l'ouverture ; la fermeture part de l'opacité atteinte.
  - Réduire la fenêtre arrête l'animation.
  - Sur un registre plein, la plus ancienne animation est évincée, et son instantané libéré.
  - Les tampons de travail rendent leur mémoire dès que le registre est vide.
- **Garde-fous.**
  - Pas d'instantané en composition directe dans le scanout virtio : peindre hors trame y serait un accès pendant transfert.
  - Au-delà de 1920×1080 px, pas d'instantané non plus.
  - Allocations en `try_reserve_exact`.

**Preuves.**
- `test_atelier` : 67 tests (+12), dont :
  - bureau immobile = 0 trame ;
  - trames demandées seulement pendant la durée ;
  - animation rendue une seule fois à sa fin ;
  - registre plein qui évince sans dépasser la capacité (1000 lancements) ;
  - relance sans doublon ;
  - ancienne et nouvelle zone recouvertes ;
  - annulation ;
  - bornes exactes des courbes ;
  - mode réduit sans déplacement, mode aucun sans lancement ;
  - durées dans les fourchettes de la mission (ouverture 160–220 ms, fermeture 120–180 ms) ;
  - datation à la première trame.
- 22/22 suites, gardes 210/210, `CI_POLICY_OK`.

**Captures QEMU réelles** :
- l'exécuteur `tools/ci/qemu_gestes.py` garde une seule connexion au moniteur ;
- la VM est figée (`stop`/`cont`) le temps de chaque `screendump` ;
- clics et touches sont réellement injectés.

| Fichier | Ce qu'il montre |
|---|---|
| `atelier/sequence-incB-ouverture.png` | Double-clic sur Fichiers : fenêtre translucide, puis opaque |
| `atelier/sequence-incB-fermeture.png` | Clic sur la croix du Terminal : l'instantané s'efface et se réduit ; aucun résidu |
| `atelier/sequence-incB-interruption.png` | Échap pendant l'ouverture, puis rafale de trois ouvertures et deux Échap : aucun fantôme résiduel |
| `atelier/capture-incB-centre-animations.png` | Réglage Normales / Réduites / Aucune dans le centre |

**Mesures** (QEMU TCG, BIOS 1280×720, rendu **logiciel**) : ligne `BOUCHAUD_ATELIER_ANIMATION`, coût d'une trame composée qui anime.

| Scénario | Trames | Coût moyen | Coût max |
|---|---|---|---|
| Fermeture Terminal (380×280), Normal | 6 | 14,5–16,3 ms | 32–51 ms |
| Ouverture Fichiers (première trame plein écran comprise), Normal | 3–4 | 42–62 ms | 75–114 ms |
| Fermeture, Réduit | 3 | 19,3 ms | 36,6 ms |
| Ouverture, Réduit | 2 | 56,4 ms | 77,5 ms |
| Mode Aucun | 0 (aucune ligne émise) | — | — |

Sous TCG, l'ouverture plafonne à environ 15–25 images/s, parce que sa première trame est plein écran (fond, icônes, fenêtre neuve). La fermeture tourne à environ 60 images/s. Ce sont des chiffres d'émulation logicielle, sans accélération GPU.

**Repos** (scénario de 62 s) :
- `desktop cpu_pct` : 3 à t = 30 s, 0 à t = 60 s, identique à la base ;
- 2 trames utiles en 54 s.

**Première trame** : 5480 / 5481 / 5576 ms. L'incrément A mesurait 5301–5418 ms ; la plage historique va de 5178 à 5405 ms.

**Bootimage** : 28 963 840 octets.

**Non fait dans cet incrément** (prévu, à faire proprement) :
- réduction vers l'icône du dock et restauration inverse ;
- interpolation de la maximisation ;
- transition de focus ;
- fondu du centre et du menu.

## 3 quinquies. Incrément C livré : dock interactif (BOUCHAUD_ATELIER_DOCK_SURVOL_V1)

**Géométrie unique.** `atelier::dock::cible_sous(l, h, n, x, y)` ne lit que `cellule_applications` et `cellule_fenetre`, les mêmes boîtes que `window::start_btn` et `taskbar_btn`. Le peintre, l'infobulle, l'invalidation et le clic désignent donc la même cellule. Un test hôte le vérifie pixel par pixel, sur 3 résolutions et de 0 à 6 fenêtres.

**Ce qui est livré.**
- **Voile de survol** sur la cellule, en fondu de 120 ms (80 ms en mode réduit, instantané en mode aucun), par le registre de l'incrément B (`Nature::Survol`).
- **Levée de l'icône** de 3 px au terme du fondu. Elle reste dans la cellule : la zone de clic ne bouge pas avec l'image. Pas de levée en mode réduit.
- **Infobulle** au-dessus du dock :
  - texte « Applications » ou titre de la fenêtre, avec « (réduite) » si besoin ;
  - calque propre `Element::Infobulle`, transparent ;
  - boîte `dock::infobulle`, gardée dans l'écran.
- **Clic sur la cellule de la fenêtre active** : elle se réduit, par le même chemin que le bouton de la barre de titre. Un clic sur une cellule réduite la restaure.
- **Invalidation** :
  - le survol n'est mesuré que près du bas de l'écran ;
  - un changement de cible invalide l'ancienne cellule et sa bulle, puis la nouvelle ;
  - aucun survol avec le menu ou le centre ouvert.

**Preuves.**
- `test_atelier` : 72 tests (+5) :
  - cible au pixel près ;
  - rien hors du dock ;
  - infobulle collée au-dessus du dock et dans l'écran ;
  - levée dans la cellule ;
  - durées.
- 22/22 suites, gardes 210/210, `CI_POLICY_OK`.

**Captures QEMU réelles.**
- `atelier/sequence-incC-survol-dock.png` :
  - survol d'Applications ;
  - survol du Terminal (icône levée) ;
  - sortie du dock, sans résidu.
- `atelier/sequence-incC-reduire-restaurer.png` : clic sur la cellule active (réduite, infobulle « Terminal (réduite) »), puis nouveau clic (restaurée, indicateur d'accent).

**Mesures** (TCG) :
- un fondu de survol produit 10 à 11 trames, à 2,7–4,2 ms par trame en moyenne et 6,1 ms au pire ;
- au repos (62 s), `desktop cpu_pct` vaut 3 à 30 s puis 0 à 60 s, avec 1 trame utile en 54 s ;
- première trame : 5456 / 5502 ms ;
- bootimage : 28 971 008 octets.

**Non fait** :
- grossissement au-delà de la levée (pas de taille d'icône intermédiaire en assets) ;
- animation de lancement ;
- apparition et disparition animées des cellules ;
- menu contextuel du dock.

## 4. Limites connues de l'incrément 1 (assumées)

- **Contenu des applications encore sombre** : Fichiers, Notes, Calculatrice et les autres peignent pour `theme::COLOR_SURFACE`. Le chrome est Atelier, le contenu ne l'est pas encore (prochain incrément). Le Terminal reste sombre volontairement.
- **Pas de mise à l'échelle HiDPI** : en 2560×1600, l'interface garde ses tailles en pixels et paraît minuscule.
- **Icône du navigateur provisoire** (tuile « boussole ») : l'identité de Bouchaud Navigateur et son chrome restent à faire, sans toucher au moteur Ladybird.
- **Le verre n'est pas flouté** : c'est une matière translucide légère. Aucun flou d'arrière-plan n'est implémenté, et rien ne le prétend.
- ~~Pas encore de transitions animées~~ : ouverture et fermeture animées depuis l'incrément B ; la réduction vers le dock et la maximisation restent à faire.
- ~~Pas de survol sur le dock~~ : livré par l'incrément C (voile, levée, infobulle, invalidation de l'ancienne et de la nouvelle cellule).
- **Menu Démarrer encore sombre** : il est repositionné au-dessus du dock, mais pas restylé.
- ~~Menus de la barre inertes~~ : retirés par l'incrément A.
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
