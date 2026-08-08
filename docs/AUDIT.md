# Audit — Bouchaud OS et son navigateur (août 2026)

Cet audit remplace celui de juillet 2026, devenu faux : il décrivait **Nautile**,
le moteur web qui vivait dans le noyau, supprimé depuis (`b2d2615`).

**Méthode.** Rien n'est repris sur parole, y compris de la documentation du
dépôt. Chaque ligne ci-dessous est soit une compilation réelle contre la cible
`x86_64-bouchaud_os.json`, soit une exécution de la suite de vérifications, soit
une lecture du code qui décide. Ce qui n'a pas pu être exécuté est marqué comme
tel plutôt que supposé.

**Limite de cet audit.** QEMU n'est pas disponible dans l'environnement où il a
été mené : **aucune vérification à l'exécution sous l'OS** n'a pu être faite. Les
affirmations sur le comportement au démarrage viennent du code et du journal des
commits, pas d'un boot observé. Le harnais existe (`tools/test.sh`) et reste le
seul juge sur ce plan.

Barème : ✅ vérifié ici · 🟢 solide (lecture) · 🟡 fonctionne, manques connus ·
🔴 embryonnaire ou bloqué.

---

## 1. Ce qui a été vérifié par exécution

| Vérification | Commande | Résultat |
|---|---|---|
| Compilation du noyau | `cargo build` | ✅ **succès, 0 warning**, 1 min 09 s |
| Moteur du navigateur | `tools/userland/test-moteur.sh` | ✅ **573/573**, 0 échec |
| Toolchain épinglée | `rust-toolchain.toml` | ✅ nightly-2026-06-01 s'installe seule |

Deux résultats à souligner. Un noyau `no_std` de 32 700 lignes qui compile
**sans un seul avertissement** n'est pas la norme. Et les vérifications du
moteur tournent avec le **vrai QuickJS**, pas un bouchon : seul l'hôte Qt est
simulé, tout le reste du moteur est celui qui tourne sous l'OS.

Le chiffre de « 92 vérifications » annoncé dans `docs/ROADMAP.md` est périmé
d'un facteur six. *(388 au moment de l'audit ; 532 après les deux chantiers
« web moderne » décrits au §9.)*

---

## 2. Il y a trois navigateurs dans ce dépôt, et c'est le principal problème

C'est la découverte structurante de cet audit. Trois programmes portent le nom
de navigateur, deux sont morts, et rien dans l'arborescence ne le dit.

| # | Chemin | Ce que c'est | État |
|---|---|---|---|
| 1 | `tools/userland/navigateur/` | Moteur Python + hôte Qt + QuickJS + ffmpeg, ring 3 | 🟢 **le vrai** |
| 2 | `src/assets/python/browser.py` | Mode texte, RustPython/WASM, via `/dev/web` | 🟡 doublon vivant |
| 3 | `tools/qt-browser/browser.py` | PyQt5 + QtWebEngine | 🔴 **ne tournera jamais ici** |

**Le n° 3 est un leurre.** C'est le tutoriel `QWebEngineView` recopié tel quel.
Son propre en-tête admet qu'il ne peut pas tourner sous Bouchaud OS — il lui
faudrait Chromium, un serveur graphique et un éditeur de liens dynamique. Il ne
sert qu'à documenter d'où vient l'idée. Placé dans `tools/` sans distinction, il
laisse croire que « le navigateur Qt » du projet est celui-là.

**Le n° 2 est un doublon.** `pybrowser` affiche des pages en texte dans la
console du noyau, via un pont `/dev/web` et un RustPython de **16,4 Mo embarqué
dans le binaire du noyau** par `include_bytes!`. Il a été écrit pour prouver que
les couches s'enchaînaient, à une époque où le n° 1 n'existait pas. Depuis que le
n° 1 tourne, il coûte 16 Mo d'image de boot pour une fonction que le n° 1 remplit
mieux.

**Le n° 1 est bon, et sous-estimé.** L'architecture est juste : Qt ne fait que
la fenêtre, le framebuffer et la peinture ; le moteur est du Python pur qui rend
une liste d'affichage plate. Le moteur ne touche jamais un objet Qt — c'est
précisément ce qui permet aux vérifications de tourner sans écran. Refuser
PyQt au profit d'un module `bo` de quelques centaines de lignes est le bon appel :
PyQt statique n'existe pas vraiment.

---

## 3. Le navigateur (n° 1) — ce qui marche

Vérifié par la suite de tests, sauf mention contraire.

**Analyse et style.** HTML tolérant (balises non fermées, imbrications
interdites, entités). Sélecteurs de balise, classe, identifiant, attribut,
descendance, enfant direct, avec spécificité, cascade et héritage. Feuille de
l'agent utilisateur. Propriétés personnalisées (`--x`/`var()` avec secours),
`:root`, `rem`. `@media` évalué contre la fenêtre réelle. `::before`/`::after`
avec `attr()`. `calc()`, unités de fenêtre, `box-sizing`, bornes `min-*`/`max-*`.

**Mise en page.** Bloc et en ligne avec retour à la ligne mesuré sur la vraie
fonte. Flex complet (base, `grow`/`shrink`, `wrap`, `justify-content`,
`align-items`, colonnes). Grille (`repeat()`, `minmax()`, `fr`, `gap`, placement
auto et explicite). `position: absolute`/`fixed`. `overflow: hidden` réellement
rogné.

**JavaScript.** QuickJS lié en statique — donc l'ECMAScript entier, ramasse-miettes
et expressions rationnelles compris. Par-dessus : DOM, événements à trois phases,
minuteries, promesses, XHR/fetch, `getComputedStyle` **résolu**,
`MutationObserver`/`IntersectionObserver`/`ResizeObserver`, `customElements` avec
cycle de vie, `attachShadow`, canvas 2D, modules ES avec `import` chargé sur le
réseau.

**Réseau et état.** HTTP/HTTPS, redirections, jeux de caractères, `file://`.
Connexions réutilisées, 4 sous-ressources en parallèle, corps `gzip`. Témoins,
cache HTTP et `localStorage` **persistés sur le disque**. Client DNS écrit sur
mesure (la glibc statique ne sait pas résoudre : elle délègue à NSS par `dlopen`).

**Médias.** H.264/VP9/AAC/Opus par libavcodec, `<video>`/`<audio>`, Media Source
Extensions, sortie AC'97. Images PNG/JPEG/GIF/BMP/ICO décodées par Qt.

**Performance.** L'index de règles fait tomber la mise en page d'une feuille de
1 600 règles de **3,57 s à 0,059 s** (×60). C'est la bonne optimisation : sans
index, styler un élément coûtait un essai par règle de la feuille — plus d'un
million d'essais par mise en page sur une page de 800 éléments, refaits à chaque
battement du JavaScript.

---

## 4. Le navigateur — ce qui ne marche pas

*Les quatre premières lignes de ce tableau ont été traitées depuis — voir le
§9. Elles sont laissées ici parce qu'elles disent ce que l'audit a trouvé.*

| Manque | Portée | Suite donnée |
|---|---|---|
| ~~**`transform`**~~ | Le manque le plus grave : du placement, pas de la décoration | **fait** (§9) |
| ~~**`grid-template-areas`**~~ | Retombait sur le placement automatique | **fait** (§9) |
| ~~**`order` en flexbox**~~ | Ordre du source conservé | **fait** (§9) |
| ~~**Bords par côté, coins, ombres, dégradés**~~ | `border-bottom` ne peignait rien | **fait** (§9) |
| ~~**`transition`, `animation`**~~ | La page s'affichait à son état final | **fait** (§9) |
| ~~**`getImageData`, dégradés, ombres (canvas)**~~ | L'hôte ne prêtait pas de pixels | **fait** (§9) |
| ~~**Isolement du Shadow DOM**~~ | Les sélecteurs de la page atteignaient la racine d'ombre | **fait** (§9) |
| **Chargement parallèle des modules ES** | Graphe d'`import` rapporté module par module (chargeur QuickJS synchrone) | ouvert |
| **Dégradés coniques, `createPattern`, composition** | Manques ciblés, nommés dans le README | ouvert |
| **WebGL, WebAssembly, EME/Widevine** | Hors de portée, et assumé comme tel | — |

**Un défaut de méthode, aussi.** Le §13 du README (`pywebview`) affirmait que le
moteur « ne fait pas de JavaScript », qu'il n'avait ni canvas, ni Web Components,
ni `IntersectionObserver`, et que `getComputedStyle` rendait le style en ligne.
Le §12 du même fichier — et les tests — disaient le contraire pour chacun de ces
points : le §13 n'avait pas été relu après l'arrivée de QuickJS. Une
documentation qui sous-estime le produit fait le même tort qu'une qui le
surestime. *(Corrigé depuis, voir §9.)*

---

## 5. Le système — ce qui marche

| Couche | État | Note |
|---|---|---|
| `arch/x86_64` | 🟢 | GDT complète, TSS RSP0, `syscall`/`sysretq`, `iretq`, PCI, RTC |
| Mémoire virtuelle | 🟢 | Frames physiques, espace d'adressage par processus (`vmm.rs`, 614 l.) |
| **Ordonnanceur** | 🟢 | **Préemptif** sur IRQ0 pour le ring 3 (`task.rs`, 1 010 l.) |
| Chargeur ELF64 | 🟢 | `PT_LOAD`, `PT_INTERP`, `PT_TLS`, auxv complet |
| ABI Linux | 🟢 | ~125 branches de dispatch, numéros et structures Linux |
| Processus / signaux | 🟢 | `fork`, `execve`, `wait4`, `clone`, futex, signaux |
| Réseau | 🟢 | Ethernet→TLS 1.3 **maison**, HTTP/2, brotli ; sockets POSIX par-dessus |
| Graphique (ring 3) | 🟢 | `/dev/fb0` mmap, ioctls fbdev, evdev clavier/souris, tick 1000 Hz |
| **Persistance** | 🟢 | Pilote ATA (374 l.) + zone inscriptible (`persistance.rs`) |
| Bureau natif | 🟡 | Fenêtres, glisser/redimensionner, z-order ; repeint tout à chaque frame |

Deux de ces lignes contredisent l'audit de juillet, qui datait d'avant le
travail correspondant : l'ordonnanceur **est** préemptif, et la persistance
**existe**. C'est la raison d'être de cette révision.

Le tick à 1000 Hz mérite une mention : sans lui, la granularité de 55 ms du PIT
par défaut rendrait toute animation saccadée et tout `poll` imprécis. C'est ce
qui rend la boucle d'événements de Qt utilisable.

---

## 6. Le système — ce qui ne marche pas

### 6.1 Bloquant, par ordre de gravité

**`listen`/`accept` ne sont pas implémentés** (`abi/net.rs:803`). Conséquence
directe et mesurable : toute application pywebview servant des fichiers locaux
(`create_window(url='index.html')`) échoue, parce que pywebview démarre un
serveur HTTP interne. Plus largement, l'OS ne peut héberger aucun service.

**Le chargeur dynamique de la glibc n'atteint pas `main()`.** C'est l'état
consigné par le dernier commit sur le sujet (`4df176b`) : blocage après
`prlimit64`, cause non identifiée. Cela ferme la voie la plus prometteuse du
projet — exécuter les binaires Linux du monde réel (dont WebKit, disponible en
paquet Ubuntu) plutôt que porter du code.

Cette voie mérite d'être défendue : elle est juste. C'est la méthode du
linuxulator de FreeBSD, de WSL1 et de gVisor. Et la preuve est déjà faite —
Qt tourne, des millions de lignes de C++ sans une ligne modifiée. Il ne manque
que la glibc.

Le journal de bord contient au passage la meilleure leçon d'ingénierie du dépôt :
`rseq` renvoyait **0 (succès)** pour un appel non implémenté, et la glibc
avançait dans le vide. Répondre « réussi » à ce qu'on n'implémente pas est pire
que de l'avouer. **Ce piège est à rechercher systématiquement** — partout où un
appel non implémenté rend une valeur de succès.

**Aucun gestionnaire de fenêtres pour le ring 3.** Un binaire Qt prend
`/dev/fb0` en entier. Les fenêtres suivantes s'affichent l'une après l'autre dans
la même surface. Il y a donc deux mondes graphiques disjoints : le bureau du
noyau (avec son WM, ses apps natives) et le plein écran des applications ring 3.

**Le navigateur n'est nulle part dans le bureau.** Le menu Démarrer
(`gui/window.rs:24`) propose Terminal, Fichiers, Moniteur, Calculatrice, Rustpad.
Pas de navigateur. Le seul moyen de le lancer est `exec /bo-navigateur` depuis le
shell. Le produit phare du projet est invisible depuis son interface.

### 6.2 Structurel

**Le navigateur n'est pas constructible à partir d'un dépôt frais.** Il exige
Qt 5.15 statique **et** un CPython glibc embarquable, tous deux à compiler
(`build-qt.sh`, `build-python.sh`) — des heures. `out-quickjs/` et `out-ffmpeg/`
sont présents ici mais **ignorés par git** : ils viennent de l'environnement, pas
du dépôt. Il n'existe aucune intégration continue, aucune image préconstruite.
En pratique, une seule machine au monde peut produire ce binaire de 32 Mo.

**Code mort qui ment.** `kernel/scheduler.rs` déclare
`« coopératif (pas de préemption ; round-robin sur timer planifié) »` alors que
`task.rs` préempte réellement depuis IRQ0. Avec `syscall.rs` (44 l.) et
`handle.rs` (48 l.) — **zéro utilisation externe, vérifié** — ce sont des vestiges
de la V0.6 que `kernel/abi/` a remplacés.

**Documentation périmée à trois endroits.** Le `README.md` racine décrit encore
la « phase 0 » : il annonce un écran VGA texte, et sa feuille de route laisse
GDT, IDT, interruptions, mémoire et shell **décochés** — tous faits depuis
longtemps. Il présente `tools/qt-browser/` comme la référence et `pybrowser`
comme le navigateur de l'OS, sans mentionner le vrai. `docs/WEB_ENGINE_MODULES.md`
décrit l'architecture cible de Nautile, supprimé. `docs/ROADMAP.md` s'ouvre sur
un V0.35 qui annonce « le bureau ouvre par défaut Nautile ».

C'est le README racine qui compte le plus : c'est la première page que voit un
visiteur, et elle décrit un projet dix fois moins avancé que le vrai.

---

## 7. Ce que je recommande, par valeur rendue

L'ordre suit le rapport entre ce que ça débloque et ce que ça coûte.

### Rendre visible et honnête ce qui existe déjà — quelques heures

1. **Réécrire le `README.md` racine.** Le projet est très en avance sur sa
   vitrine. Aucun code à écrire, effet immédiat.
2. **Mettre le navigateur dans le menu Démarrer.** Une entrée qui fait
   `exec /bo-navigateur`.
3. **Supprimer `tools/qt-browser/`** (ou le déplacer sous `docs/` en l'annonçant
   comme référence historique) et **retirer `pybrowser`** avec son RustPython :
   **−16 Mo sur l'image de boot**, et un seul navigateur au lieu de trois.
4. **Supprimer `scheduler.rs`, `syscall.rs`, `handle.rs`** ; router `GetPid` vers
   `task.rs`.
5. **Corriger le §13 du README userland**, qui sous-estime le moteur.

### Rendre le navigateur moderne — le vrai chantier

6. **`transform` et `transition`/`animation`.** Le manque le plus visible.
   `transform` d'abord : c'est de la mise en page, pas de l'animation, et son
   absence place des éléments au mauvais endroit. `transition` ensuite, en
   s'appuyant sur les minuteries qui existent déjà.
7. **Isolement du Shadow DOM** (`:host`, `<slot>`, portée des sélecteurs) : sans
   lui, les composants d'une page moderne fuient les uns dans les autres.
8. **Tampon de pixels pour le canvas** (`getImageData`, dégradés, ombres).

### Débloquer le système

9. **`listen`/`accept`.** Le README les annonce comme « le prochain manque à
   combler », et c'est juste : ils débloquent pywebview servant des fichiers
   locaux, et tout service local.
10. **Reprendre le banc d'essai glibc** avec la méthode qui a trouvé `rseq` :
    tracer, lire l'appel exact, ne rien supposer. En balayant d'abord tous les
    appels non implémentés qui renvoient un succès — c'est la classe de défaut
    qui a déjà mordu une fois.
11. **Un gestionnaire de fenêtres pour le ring 3**, ou à défaut assumer le plein
    écran et le documenter comme un choix.

### Rendre le tout reproductible

12. **Une intégration continue** qui lance au minimum `cargo build` et
    `test-moteur.sh` — les deux tournent en moins de deux minutes et auraient
    attrapé la dérive de documentation relevée ici.
13. **Publier les artefacts** (`out-qt`, `out-python-embed`) ou fournir un
    conteneur de compilation, pour que le navigateur soit constructible ailleurs
    que sur une seule machine.

---

## 8. En un paragraphe

Le projet est bien plus solide que sa documentation ne le laisse croire. Le
noyau compile sans un avertissement, l'ABI Linux est assez complète pour faire
tourner Qt sans modification, la pile TLS 1.3 est écrite à la main, et le moteur
du navigateur passe 388 vérifications avec un vrai moteur JavaScript. La décision
d'avoir sorti le moteur web du ring 0 était la bonne, et celle d'exécuter les
binaires Linux plutôt que de les porter l'est aussi. Ce qui manque n'est pas de
la puissance : c'est de la **finition** — un navigateur qu'on ne peut pas lancer
depuis le bureau, trois navigateurs dont deux morts, une vitrine qui décrit la
phase 0, et une chaîne de construction qui ne fonctionne que sur une machine. Le
plus grand pas en avant disponible ne demande pas d'écrire un moteur de rendu :
il demande de faire le ménage, puis d'ajouter `transform`.

---

---

## 9. Ce qui a été fait après l'audit

L'audit a été suivi d'un chantier sur la même branche, dont le but était de
rendre le moteur capable d'afficher les sites récents. Ce qui a changé :

**Un seul moteur de sélecteurs**, servant la cascade et `querySelector` — ils
étaient deux, avec deux jeux de limites, si bien qu'une règle pouvait styler un
élément qu'un script ne trouvait pas. Y sont entrés les combinateurs `+` et `~`
(ramenés jusque-là à la descendance), les pseudo-classes structurelles
(`:nth-child()` et sa famille), les fonctionnelles (`:not()`, `:is()`,
`:where()`, `:has()`), les sélecteurs d'attribut côté cascade, et les noms de
classe échappés (`md\:flex`) que produisent les cadres CSS répandus. Les
pseudo-classes d'état ne désignent plus personne au repos : les effacer, ce que
faisait le moteur, peignait en permanence la couleur réservée au survol — et
faisait de `:not(.x)` un sélecteur qui désignait tout ce qu'il devait exclure.

**Le placement** : `transform`, `position: sticky`, `fixed` qui ne défile plus,
`order` en flexbox, zones nommées de grille.

**La décoration** : coins arrondis, ombres portées, dégradés linéaires,
opacité, et bords par côté — `border-bottom: 1px solid #eee`, le séparateur le
plus répandu du web, ne peignait rien.

**Les règles `@`** : une règle sans bloc (`@import url(…);`, `@layer a, b;`)
collait son texte au sélecteur suivant, et la règle d'après était perdue avec
elle. `@layer` avec bloc était sautée entièrement — or les cadres CSS récents y
rangent la totalité de leur feuille.

**Le navigateur est atteignable depuis le bureau** : menu Démarrer et icône.

Cinq opérations de peinture ont été ajoutées à l'hôte Qt. **Elles n'ont pas été
compilées** : Qt est absent de l'environnement et sa construction prend des
heures. Le C++ a été vérifié contre des bouchons déclarant les mêmes signatures,
ce qui éprouve la syntaxe et les types, pas le rendu. **Le rendu réel reste à
constater sous l'OS** — c'est la première chose à faire au prochain démarrage.

### Second chantier : ce qui fait qu'une page vit

**Les pseudo-classes d'état** (`:hover`, `:active`, `:focus`) suivent le
pointeur. `:hover` désigne l'élément **et toute sa lignée**, ce qui tient un
menu déroulant ouvert. Une page dont aucune règle ne parle d'interaction n'est
jamais recalculée : sans ce court-circuit, bouger la souris relancerait la
cascade complète à chaque pixel.

**Les animations et les transitions** (`moteur/animation.py`) : `@keyframes`
était sauté comme une règle `@` inconnue. Les deux mécanismes partagent une
interpolation — longueurs et nombres avec leur unité, couleurs canal par canal,
transformations matrice par matrice — et les rythmes `ease*`, `cubic-bezier()`
et `steps()`. L'horloge ne dépend pas du JavaScript et s'arrête quand plus rien
ne bouge.

**Le DOM d'ombre isole** vraiment : les règles de la page s'arrêtent à la
frontière, celles de l'ombre n'en sortent pas, `:host` traverse vers l'hôte, et
les `<slot>` distribuent le contenu clair. Deux défauts plus larges ont été
trouvés là : la feuille de l'agent utilisateur était elle aussi bloquée à la
frontière (le contenu d'ombre se retrouvait sans `display`, donc invisible), et
**un bloc dans un élément en ligne disparaissait** — `<span><p>…</p></span>`
n'affichait rien, ni aucune balise inconnue, donc aucun composant web.

**`object-fit` et `aspect-ratio`** : toute image était étirée à sa boîte.

**La toile rend ses pixels.** `getImageData` fait jouer les opérations par
l'hôte dans une image hors écran — le même peintre que l'écran, donc ce qu'on
lit est ce qu'on voit. Avec les dégradés, les ombres, le détourage, et les
chemins remplis exactement au lieu de leur boîte englobante.

**Les règles `@` sans bloc** (`@import url(…);`, `@layer a, b;`) avalaient la
règle suivante, et `@layer` avec bloc était sautée entièrement — or les cadres
CSS récents y rangent toute leur feuille.

### Troisième chantier : ce que de vraies pages ont montré

Un outil d'aperçu (`tools/userland/navigateur/apercu.py`) exécute le moteur et
peint sa liste d'affichage sans Qt ni QEMU. Rendre pypi.org a révélé, en une
soirée, plus de défauts que la suite de tests n'en couvrait — les tests posaient
des cas propres, une page réelle pose des cas sales :

- l'**espace entre deux balises** était jeté (« HelpDocsSponsors ») ;
- un **article flex « au contenu »** réclamait la largeur entière ;
- les **cellules de tableau** étaient en ligne, sans colonnes ;
- **`display: inline-block`** ne peignait jamais sa boîte, ni les **champs de
  formulaire** ;
- le **SVG** n'était pas lu (module QtSvg absent de la construction) ;
- **`float` et `clear`** n'existaient pas — 45 déclarations dans la seule
  feuille de pypi, d'où un en-tête empilé et des onglets verticaux ;
- **`@font-face`** n'existait pas non plus, et la famille demandée n'atteignait
  même pas le peintre : une page ne pouvait jamais s'afficher dans sa police.

Deux régressions ont été introduites puis rattrapées dans la même session, l'une
et l'autre repérées en comparant les captures : un `display: table` sans cellule
qui perdait tout son contenu, et une police retenue par écriture qui faisait
sortir la page entière en carrés.

Reste ouvert, par ordre de valeur : les polices en **WOFF2** (brotli et la
transformation `glyf`/`loca`), le chargement parallèle des modules ES,
`listen`/`accept`, et la reprise du banc d'essai glibc.

---

*Audit mené le 2026-08-07 sur la branche `claude/audit-qt-browser-0fd4eu`.
Compilation et suite de vérifications exécutées ; aucun démarrage sous QEMU
(émulateur absent de l'environnement).*
