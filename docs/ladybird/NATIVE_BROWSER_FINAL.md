# Ladybird natif — convergence M7/M8

Cette chaine part du dernier jalon prouve (M6 LibGfx/Skia CPU en ring3) et du premier morceau M7 (`build-libweb-gen.sh`). Elle ne remplace aucun composant Ladybird par un renderer maison.

## Strategie

Le port historique M1-M6 construit des bibliotheques ciblees a la main pour rendre chaque incompatibilite visible. A partir de LibWeb, cette methode devient contre-productive : le graphe depasse deux mille unites C++/Rust et les CMake upstream encodent des dependances de generation importantes.

La chaine finale bascule donc vers **le CMake upstream epingle**, mais dans un worktree jetable et avec trois adaptations Bouchaud minimales :

1. construire `Services/` sans l'UI desktop Qt/AppKit ;
2. utiliser `RendererSandboxUnimplemented.cpp` jusqu'au jalon sandbox M14 ;
3. permettre a `WebContent` d'adopter un socket IPC deja ouvert par le processus Bouchaud via `BOUCHAUD_WEBCONTENT_FD`.

L'arbre `third_party/ladybird` reste au SHA indique dans `third_party/UPSTREAM.md`.

## Construction

```bash
tools/ladybird/native-browser-final.sh --cible
```

Artefacts :

```text
third_party/native-browser-bouchaud/
    WebContent
    webcontent-bootstrap
    RequestServer             (si cible presente dans le SHA)
    ImageDecoder              (si cible presente)
    WebContentCompositor      (si cible presente)
    WebWorker                 (si cible presente)
    resources/
```

`WebContent` est exige en `static-pie`. La chaine refuse un ELF marque `dynamically linked`.

## Test QEMU

Le workflow `ladybird-native-browser` construit un disque minimal et lance :

```text
webcontent-bootstrap
    -> socketpair(AF_UNIX)
    -> fork()
    -> BOUCHAUD_WEBCONTENT_FD=3
    -> exec WebContent --disable-sandbox --headless ...
```

Le vrai `WebContent` adopte alors le transport LibIPC et imprime :

```text
[ladybird-bouchaud] WEBCONTENT_READY pid=... fd=3
```

Le parent verifie que le processus reste vivant apres l'initialisation complete de LibWeb/LibJS/LibGfx.

## Ce que ce verdict prouve

- le vrai executable `Services/WebContent` du SHA epingle a ete construit ;
- sa fermeture de dependances LibWeb/LibWebView/Media/Wasm/HTTP/Requests/etc. est linkable en statique ;
- le loader Bouchaud accepte l'ELF ;
- le processus demarre en ring3 ;
- socketpair/fork/exec et le bootstrap IPC passent le noyau Bouchaud ;
- l'initialisation WebContent + VM JS + fontes + Skia ne plante pas immediatement.

## Ce qu'il reste pour une fenetre interactive

Le test ci-dessus est volontairement le dernier verrou **moteur/processus**. Pour une fenetre navigable, le chrome Bouchaud doit ensuite jouer le role de client WebContent : initialiser une page, fournir les connexions RequestServer/ImageDecoder/Compositor, relayer clavier/souris, et publier les surfaces du compositor dans le protocole du Window Manager.

Cela doit etre fait dans l'adaptateur UI Bouchaud, pas en modifiant LibWeb. Le navigateur historique reste disponible pendant cette transition.

## Pourquoi la CI est separee

Le cache vcpkg est sauvegarde immediatement apres sa construction. Ainsi, une erreur tardive dans LibWeb/WebContent ne force plus la recompilation de Skia/FFmpeg/OpenSSL a chaque tentative.
