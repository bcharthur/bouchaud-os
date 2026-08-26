# Frontend Ladybird natif Bouchaud

Plan etabli en inspectant l'arbre epingle (`cdfe5f8`), pas la documentation
d'upstream. Les nombres ci-dessous sont mesures sur cet arbre-la.

## Ce que le M11 actuel est, et pourquoi il doit partir

`prepare-m11-chrome.py` le decrit lui-meme : « chrome du navigateur, greffe
dans WebContent ». La barre d'outils, l'omnibox et les boutons vivent DANS le
processus de rendu, dessines a la main dans `BouchaudChrome.h` avec une police
bitmap 8x8 embarquee dans le fichier.

Cela a prouve ce qu'il fallait prouver -- entree, navigation, presentation --
mais l'architecture est fausse sur trois points :

- le chrome partage le sort du processus de rendu : une page qui tue WebContent
  emporte la barre d'adresse ;
- il court-circuite `LibWebView`, donc onglets, historique, telechargements et
  cycle de vie des services sont hors d'atteinte ;
- il duplique une couche que l'upstream fournit deja.

## La mesure qui change tout

`WebView::ViewImplementation` -- la classe qu'un frontend doit specialiser --
n'a que **trois methodes virtuelles pures** :

```cpp
virtual Web::DevicePixelSize viewport_size() const = 0;
virtual Gfx::IntPoint to_content_position(Gfx::IntPoint) const = 0;
virtual Gfx::IntPoint to_widget_position(Gfx::IntPoint) const = 0;
```

Tout le reste du navigateur est deja generique : 63 en-tetes dans
`Libraries/LibWebView/` couvrent navigation, historique, onglets, cookies,
telechargements, autocompletion, marque-pages, HSTS, et le cycle de vie de
WebContent / RequestServer / ImageDecoder / Compositor.

La surface de plateforme a ecrire est donc **petite**. Ce qui est gros dans
`UI/Qt` (17 329 lignes) n'est pas le navigateur : c'est Qt.

## Le point de depart : HeadlessWebView, pas UI/Qt

`Libraries/LibWebView/HeadlessWebView.{h,cpp}` fait **253 lignes** et est une
implementation complete et fonctionnelle de `ViewImplementation`. Ses trois
methodes de plateforme sont triviales parce que la vue occupe toute la fenetre.

C'est le germe de `UI/Bouchaud`, et non `UI/Qt` :

- `UI/Qt` est la reference **visuelle et fonctionnelle** -- proportions,
  comportements, ce qu'un utilisateur attend d'une omnibox ;
- `HeadlessWebView` est la reference **structurelle** -- ce qu'il faut
  reellement implementer.

Qt n'est porte sous aucune forme.

## Tranches

### T1 -- la vue

`UI/Bouchaud/BouchaudWebView.{h,cpp}`, calque sur `HeadlessWebView` :

- les trois methodes de plateforme, decalees de la hauteur du chrome ;
- `initialize_client` / `update_zoom` ;
- la surface partagee du protocole GUI comme cible de presentation, a la place
  de la file de captures d'ecran de M11.

Critere : une page s'affiche, sans aucune barre. C'est deja plus que M11 sur le
plan architectural, et moins sur le plan visuel -- assume.

### T2 -- l'application et la fenetre

`UI/Bouchaud/Application.{h,cpp}` specialise `WebView::Application`
(`BouchaudBrowserHost` le fait deja) et `BrowserWindow` tient le chrome :
omnibox, precedent, suivant, recharger/arreter, titre, favicon.

Le chrome est dessine par les primitives Bouchaud -- vectorielles, section D --
et non par une police bitmap.

Critere : navigation au clavier et a la souris, titre et favicon a jour.

### T3 -- l'entree

Le protocole GUI porte deja tout ce qu'il faut depuis
`fix(input): preserve physical key transitions` : appui, relachement,
modificateurs, coordonnees de molette. La vue les convertit en
`Web::KeyEvent` / `Web::MouseEvent` et les remet a `ViewImplementation`, qui
possede la file d'attente et l'accuse de reception.

C'est ce qui rendra `prepare-m11-input-ownership.py` inutile : l'entree
redeviendra celle de l'hote, avec un accuse par mise en file, et le drapeau
`report_completion_to_client` pourra disparaitre.

### T4 -- retrait

`BouchaudChrome.h` et les scripts `prepare-m11-*` sortent de la chaine quand T1
a T3 sont verts. Pas avant : le M11 reste la seule chose qui affiche une page.

### Ensuite

Onglets, popups, menus contextuels, telechargements, selecteur de fichiers,
plein ecran. Tous existent deja dans LibWebView ; il ne manque que la
presentation.

## Ce qui reste a trancher

- **Presentation.** M11 presente un readback de captures. La vraie voie est la
  surface du Compositor. Cela se decide en lisant `CompositorClient` et
  `CompositorHostBase`, pas au jugement.
- **Boucle d'evenements.** `UI/Qt` fournit `EventLoopImplementationQt` (515
  lignes). Bouchaud a besoin de l'equivalent au-dessus de son protocole GUI, ou
  peut s'appuyer sur `Core::EventLoop` si le fd GUI suffit comme source de
  reveil -- c'est le premier point a verifier en T1.
