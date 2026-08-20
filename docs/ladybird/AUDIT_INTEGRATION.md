# Audit de l'intégration Ladybird dans Bouchaud OS

Ladybird épinglé : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`
Audit conduit en lisant le code de ce commit, fichier par fichier. Aucune ligne
de ce document ne repose sur un souvenir de ce que Ladybird « fait
d'habitude ».

Ce document répond à une question et une seule : **qu'est-ce que le Ladybird
épinglé sait faire, que Bouchaud ne lui laisse pas faire, et pourquoi ?**

---

## 1. Architecture upstream au commit épinglé

`Services/CMakeLists.txt` déclare six services :

| Service | Rôle | Lancé par | Transport |
|---|---|---|---|
| `WebContent` | le moteur : DOM, CSS, layout, JS, peinture | le processus UI | socketpair + `SOCKET_TAKEOVER` |
| `RequestServer` | DNS, TCP, TLS, HTTP, WebSocket | le processus UI | idem |
| `ImageDecoder` | décodage PNG/JPEG/WebP/GIF/BMP/ICO/AVIF/JPEG-XL/TIFF | le processus UI | idem |
| `Compositor` | composition GPU, WebGL hôte, VSync, scrollbars | le processus UI | idem |
| `WebWorker` | agents Worker / SharedWorker / ServiceWorker | le processus UI, **à la demande** | idem |
| `WebDriver` | automatisation W3C | l'utilisateur | socket TCP |

Le processus **UI** n'est pas un service : c'est le programme (`UI/`) qui les
engendre tous et qui répond à leurs questions. Son rôle exact, tel qu'il apparaît
dans `Libraries/LibWebView/Process.cpp` :

```cpp
TRY(Core::System::socketpair(AF_LOCAL, SOCK_STREAM, 0, socket_fds));
TRY(Core::System::set_close_on_exec(socket_fds[0], true));
auto takeover_string = MUST(String::formatted("{}:{}", options.name, socket_fds[1]));
TRY(Core::Environment::set("SOCKET_TAKEOVER"sv, takeover_string, ...));
auto process = TRY(Core::Process::spawn(spawn_options));
```

Le service adopte ensuite son descripteur par
`IPC::take_over_accepted_client_from_system_server`, qui lit `SOCKET_TAKEOVER`
et **vérifie que le descripteur est bien une socket** :

```cpp
if (!System::is_socket(fd))
    return Error::from_string_literal("The fd or handle we got from SystemServer is not a socket");
```

Ce détail a coûté un défaut d'ABI côté Bouchaud (§6).

Au-delà de l'engendrement, l'UI est **le dépositaire de l'état** : cookies,
stockage, HSTS, historique, téléchargements, création d'onglets. WebContent les
lui demande par messages **synchrones**, sans délai d'expiration.

---

## 2. Architecture Bouchaud, aujourd'hui

```
gestionnaire de fenêtres (fil noyau)
    └── /bo-navigateur            = tools/ladybird/webcontent-bootstrap.c
            ├── ImageDecoder      binaire upstream, non modifié
            ├── RequestServer     binaire upstream + adoption de descripteur
            └── WebContent        binaire upstream + chrome Bouchaud
```

`webcontent-bootstrap.c` tient la place du processus UI pour ce qu'il sait
faire : créer les paires de sockets, lancer les services, transmettre les
descripteurs. Il ne tient pas sa place pour ce qu'il ne sait pas faire : répondre
aux questions synchrones du moteur (§5).

La compilation se fait sur Ubuntu (GitHub Actions) ; le résultat est un binaire
`static-pie` sans interpréteur ELF, exécuté en anneau 3 sur Bouchaud. Aucun
Linux, aucun WSL n'existe dans la machine finale — la chaîne de construction
n'est pas une dépendance d'exécution.

---

## 3. Matrice des services

| Composant | Upstream | Compilé | Empaqueté | Lancé | IPC | Testé | État |
|---|---|---|---|---|---|---|---|
| WebContent | oui | oui | oui | oui | descripteur hérité (`BOUCHAUD_WEBCONTENT_FD`) | M8, M9, M12, Internet | ✅ |
| RequestServer | oui | oui | oui | oui | descripteur hérité (`BOUCHAUD_REQUESTSERVER_FD`) | M9, M12, Internet | ✅ |
| ImageDecoder | oui | oui | **oui, désormais** | **oui, désormais** | `SOCKET_TAKEOVER` upstream | suite fonctionnelle : PNG, JPEG, WebP, GIF, SVG décodés | ✅ |
| WebWorker | oui | oui | non | non | — | non | ❌ voir §5 |
| Compositor | oui | non | non | non | — | non | ❌ voir §4 |
| WebDriver | oui | non | non | non | — | non | ⚪ voir §7 |

### Ce qui n'allait pas, et qui est corrigé dans cette branche

**ImageDecoder était construit mais n'arrivait jamais.** Trois défauts en
cascade, chacun silencieux :

1. `browser-upstream.sh` ne le construisait que « si la cible existe » ;
2. la CI testait `test -x native-browser/ImageDecoder` sur un artefact
   téléchargé — or `actions/download-artifact` ne restitue pas le bit
   d'exécution, donc la condition ne pouvait **jamais** être vraie ;
3. `run.ps1` ne le copiait pas du tout.

Et même arrivé, il n'aurait pas démarré : `sys_fstat` rangeait les sockets dans
son fourre-tout `S_IFCHR`, donc `is_socket()` répondait faux et le service
refusait son descripteur avant d'ouvrir sa boucle d'événements.

Conséquence observable : `ImageCodecPlugin::the()` déclenchait
`VERIFY(s_the)` à la **première image** d'un vrai site, deux secondes après une
navigation par ailleurs parfaitement réussie.

---

## 4. Compositor : ce que nous n'avons pas, et ce que cela coûte

Le service `Compositor` d'upstream contient `OpenGLContext.cpp`,
`HostWebGLContext.cpp`, `WebGLObjectMap.cpp`, `VSyncScheduler.cpp`,
`BackingStoreManager.cpp`, `ViewportScrollbarController.cpp`.

Bouchaud déclare `supports_compositor() == false`
(`prepare-browser-runtime-link.py`) et rend la trame par le chemin **capture** :
LibWeb enregistre sa liste d'affichage, `DisplayListPlayerSkia` la rejoue sur le
processeur dans un bitmap, et le chrome copie ce bitmap dans la surface partagée
du gestionnaire de fenêtres.

Ce qui est **conservé** : mise en page, listes d'affichage, cache de commandes
de peinture, rastérisation Skia complète, transparence, dégradés, ombres,
transformations, découpes, filtres — tout `LibWeb/Painting`.

Ce qui est **perdu** :

| Fonctionnalité | Cause précise |
|---|---|
| **Canvas 2D** | `Canvas2DContextBase::ensure_remote_canvas_context()` exige `page->has_compositor_host()`. Sans lui, `has_backing_storage()` est faux : **rien n'est dessiné**, et `read_pixels()` rend `nullptr` — ce qu'upstream commente comme « copier des pixels noirs transparents » |
| WebGL / WebGL2 | `HostWebGLContext` vit dans le processus Compositor |
| Défilement asynchrone | `--disable-async-scrolling` non passé, mais sans compositor le défilement repasse par le moteur |
| Cadence VSync | `VSyncScheduler` est côté Compositor ; le plafond de trames est ici fixé à 30 Hz |
| Composition d'iframes distantes | `compositor_context_id_for_remote_child_frame` reste vide ; site isolation désactivée de toute façon |
| Réutilisation de tuiles (`BackingStoreManager`) | chaque trame repart d'une surface neuve |

La ligne « Canvas 2D » n'était pas prévue, et elle a été trouvée par la suite
fonctionnelle : les cinq formats d'image se décodent (`image_png_decodee` …
`image_svg_decodee` passent) mais les quatre relectures de pixel rendent
`0,0,0`. Ce n'est donc pas le décodage — c'est que **la mémoire d'un canvas vit
entièrement dans le processus Compositor** dans ce commit épinglé. Toute page
qui dessine dans un `<canvas>` ne montre rien, en silence.

**Peut-on intégrer le vrai Compositor ?** Oui, et la lecture du code répond
mieux que l'intuition. `Services/Compositor/main.cpp` n'ouvre un contexte GPU
que si `--force-cpu-painting` est **absent** :

```cpp
if (!force_cpu_painting)
    Gfx::SkiaBackendContext::initialize_gpu_backend();
```

Ni ANGLE, ni pilote GPU, ni `/dev/dri` ne sont donc nécessaires : upstream
lance lui-même ce service en peinture processeur. Il adopte sa socket par le
même `SOCKET_TAKEOVER` qu'ImageDecoder — c'est-à-dire par un mécanisme déjà
éprouvé ici.

Ce qui reste à faire est identifié, et c'est plus qu'un branchement :

1. lancer le service et lui passer sa socket depuis le lanceur (identique à
   ImageDecoder) ;
2. donner à WebContent le descripteur correspondant, là où upstream reçoit
   `connect_to_compositor_process(IPC::TransportHandle)` ;
3. **rendre `supports_compositor()` à `true`** — et c'est le pas risqué :
   il remplace le pont capture par le vrai pipeline de trames, donc réécrit le
   chemin de rendu que M8, M9, M12 et l'essai Internet valident aujourd'hui.

C'est la prochaine priorité, et elle mérite sa propre branche.

**Ce qui, en revanche, était un vrai défaut** et qui est corrigé dans cette
branche : nous repeignions au rythme d'un minuteur au lieu du modèle
d'invalidation de Ladybird. Voir `tools/ladybird/prepare-repaint.py` — le
raisonnement complet y est écrit. En résumé : `queue_screenshot_task()` appelle
`set_needs_repaint()`, et l'appeler toutes les 16 ms revenait à répondre « oui,
repeins » soixante fois par seconde à une question que le moteur n'avait pas
posée. Nous allouions par ailleurs un bitmap de la hauteur du **document**
entier pour n'en peindre que la fenêtre.

---

## 5. Le trou d'architecture : les questions sans répondant

C'est le point le plus important de cet audit.

`Services/WebContent/PageClient.cpp` pose **douze** questions synchrones au
processus UI. Sans UI, `send_sync_but_allow_failure` attend une réponse qui ne
viendra jamais — et il n'a pas de délai d'expiration.

| Message | Avant cette branche | Maintenant |
|---|---|---|
| `DidRequestCookie` | `return {}` — cookies perdus | `WebView::CookieJar` en mémoire |
| `DidSetCookie` | `return` — cookies perdus | idem |
| `DidIsKnownHstsHost` | `return false` — HSTS perdu | `WebView::HSTSStore` |
| `DidRequestStorageItem` | **blocage définitif** | `WebView::StorageJar` |
| `DidSetStorageItem` | **blocage définitif** | idem |
| `DidRemoveStorageItem` | **blocage définitif** | idem |
| `DidRequestStorageKeys` | **blocage définitif** | idem |
| `DidRequestStorageUsage` | **blocage définitif** | idem |
| `DidClearStorage` | **blocage définitif** | idem |
| `StartWorkerAgent` | blocage définitif | ❌ toujours ouvert |
| `DidRequestNewWebView` | blocage définitif | ❌ toujours ouvert |
| `DidStartDownload` | blocage définitif | ❌ toujours ouvert |

Les six blocages de stockage n'avaient jamais été atteints parce qu'aucun test
n'exécutait de JavaScript touchant `localStorage`. Wikipedia et Google le font
tous les deux.

La correction n'invente rien : `WebView::CookieJar`, `WebView::StorageJar` et
`WebView::HSTSStore` sont des classes ordinaires de LibWebView — que WebContent
lie déjà — et chacune offre une fabrique **en mémoire**. Les gestionnaires du
processus UI d'upstream sont des délégations d'une ligne vers ces mêmes classes
(`LibWebView/WebContentClient.cpp` lignes 1354-1437) ; nous appelons les mêmes
méthodes des mêmes classes.

### Ce qu'il reste à faire : un vrai hôte

Les trois questions encore ouvertes demandent de **créer** quelque chose — un
processus, une vue, un fichier — ce qu'un pot en mémoire ne remplace pas.

`WebWorker` en dépend entièrement : `PageClient::start_worker_agent` envoie
`StartWorkerAgent` à l'UI, qui appelle
`WorkerProcessManager::start_worker_agent` puis
`launch_web_worker_process`. Aucune de ces deux fonctions ne vit dans
WebContent, et pour une bonne raison : un worker doit survivre à la page qui
l'a créé, et être partagé entre pages pour `SharedWorker`.

**Conséquence à connaître :** aujourd'hui, une page qui construit un `Worker`
fige WebContent. Ce n'est pas théorique — c'est le prochain mur.

La forme que doit prendre l'hôte est claire, parce qu'upstream l'a déjà écrite :
un processus liant `LibWebView`, implémentant l'interface `WebContentClient`,
et appelant `launch_web_worker_process`. Il remplacerait
`webcontent-bootstrap.c` et reprendrait à son compte les trois pots ci-dessus.

---

## 6. Appels système : ce que Ladybird demande vraiment

| N° | Nom | Appelant réel | Décision |
|---|---|---|---|
| 137 | `statfs` | `LibFileSystem::compute_disk_space` → `CacheIndex` dimensionne le cache HTTP sur l'espace **libre** | **implémenté** — frames réellement disponibles, nœuds RAMFS réellement alloués. Rendre 0 ferait un cache nul ; rendre ENOSYS ferait échouer son initialisation. Sur un système de fichiers en mémoire vive, l'espace libre *est* la mémoire libre |
| 138 | `fstatfs` | idem, variante descripteur | **implémenté**, même source |
| 294 | `inotify_init1` | `LibCore/FileWatcherInotify.cpp` — surveillance de fichiers | à confirmer par la trace (§ ci-dessous). Un `FileWatcher` sert à recharger une ressource modifiée pendant l'exécution ; sur un RAMFS reconstruit à chaque démarrage, rien ne peut changer sous les pieds du processus. Facultatif si la trace confirme l'appelant |
| 86 | `link` | `LibFileSystem` (`copy_file_or_directory` en mode lien) et `LibCore/System.cpp:484` | à confirmer par la trace. Le RAMFS n'a pas de liens durs : `st_nlink` vaut toujours 1. Une implémentation *correcte* demanderait des inodes comptés, pas un `return 0` |
| — | socket + `fstat` | `SystemServerTakeover` vérifie `S_ISSOCK` | **corrigé** : `sys_fstat` déclarait les sockets `S_IFCHR` |

Deux de ces quatre attendent encore leur appelant, et c'était impossible à
savoir : le message ne disait que le numéro. Il porte maintenant le processus et
le déplacement du retour dans son fichier :

```
[syscall] non implemente : 294 (inotify_init1) appelant=WebContent rip=0x… offset=0x…
```

`addr2line -f -C -e WebContent <offset>` donne alors la fonction exacte. Les
binaires étant des PIE statiques chargés à `vmm::user_load_base()`, la
soustraction est directement un déplacement de fichier.

**Pourquoi ne pas implémenter les deux restants tout de suite :** parce que le
choix entre « implémenter la vraie sémantique » et « documenter pourquoi c'est
facultatif » dépend de l'appelant, et que le deviner serait exactement le
`return 0` que ce projet refuse.

### Fontconfig

```
Fontconfig error: Cannot load default config file: No such file: (null)
```

fontconfig cherche `FONTCONFIG_FILE`, puis `$FONTCONFIG_PATH/fonts.conf`, puis
un chemin figé à la compilation — celui de la machine Ubuntu qui a construit le
paquet vcpkg.

Ce n'est pas décoratif. Dans l'arbre épinglé, fontconfig sert à deux choses :

1. `SkFontMgr_New_FontConfig` (`LibGfx/Font/TypefaceSkia.cpp:99`) — le
   gestionnaire de polices de Skia, donc **tout le repli** quand une page demande
   une famille que `Gfx::PathFontProvider` ne connaît pas. Sans configuration, ce
   gestionnaire ne voit aucune police, et `Helvetica, Arial, sans-serif`
   n'obtient rien ;
2. `GlobalFontConfig::hinting_for_font` (`LibGfx/Font/Font.cpp:177`) — les
   options de hinting réellement appliquées à chaque glyphe.

`tools/ladybird/fontconfig/fonts.conf` décrit ce que cette machine possède : le
répertoire de polices que Ladybird embarque, et des alias qui font pointer les
familles génériques de CSS vers **SerenitySans**, seule police de texte de
`Base/res/fonts/` (avec NotoEmoji). Ce n'est pas un raccourci mais l'état des
lieux ; le jour où d'autres familles seront empaquetées, ces alias devront citer
les vraies.

**Limite connue, et importante :** avec une seule police de texte, tout site
rend dans SerenitySans. Les métriques diffèrent donc de celles qu'un designer a
prévues, et les écritures non latines (CJK, arabe, devanagari) n'ont aucune
police du tout. C'est une limite de ce qu'upstream **empaquette**, pas de ce
qu'il sait faire : ajouter des Noto au disque la lèverait, au prix de plusieurs
mébioctets.

---

## 7. Matrice fonctionnelle

Légende : ✅ intégré et prouvé · 🟡 intégré, non prouvé ou incomplet ·
❌ absent · ⚪ non pertinent pour un navigateur d'utilisateur ·
🚧 bloqué par une primitive Bouchaud manquante

### Analyse et modèle du document

| Fonctionnalité | Upstream | Bouchaud | Cause / preuve |
|---|---|---|---|
| Analyseur HTML | `LibWeb/HTML/Parser` | ✅ | M8 analyse une fixture, Internet analyse 120 361 octets de Wikipedia |
| DOM | `LibWeb/DOM` | ✅ | idem |
| XML | `LibWeb/XML` + LibXML | 🟡 | compilé, jamais exercé |
| CSS | `LibWeb/CSS` | ✅ | M8 vérifie un invariant de couleur après cascade |
| Mise en page | `LibWeb/Layout` | ✅ | `M8_CPU_SCREENSHOT_STAGE layout ok` |
| Peinture | `LibWeb/Painting` + Skia | ✅ | rastérisation processeur, trame comparée |
| MathML | `LibWeb/MathML` | 🟡 | compilé, jamais exercé |
| SVG (document et image) | `LibWeb/SVG` | 🟡 | décodé **dans** le moteur (`SVGDecodedImageData`), pas par ImageDecoder ; téléchargé sur Wikipedia, rendu non vérifié |

### Images

| Format | Upstream | Bouchaud | Cause |
|---|---|---|---|
| PNG, JPEG, WebP, GIF | `LibGfx/ImageFormats` via `ImageDecoder` | ✅ | **prouvé** : la suite fonctionnelle vérifie `naturalWidth === 16` pour chacun, ce qui n'est vrai qu'après décodage réussi |
| BMP, ICO, AVIF, JPEG-XL, TIFF | idem | 🟡 | même service, mêmes codecs ; non exercés |
| Animations (APNG, GIF) | `request_animation_frames` | 🟡 | le chemin existe ; nécessite les rappels du greffon, non exercés |
| SVG | dans le moteur (`SVGDecodedImageData`) | ✅ | **prouvé** par la même mesure |

### Polices et texte

| Fonctionnalité | Upstream | Bouchaud | Cause |
|---|---|---|---|
| Chargement de polices embarquées | `Gfx::PathFontProvider` | ✅ | `M8_FONT_READY family=SerenitySans` |
| `@font-face` (polices web) | `LibWeb/CSS` + FreeType | 🟡 | code présent, jamais exercé |
| Repli fontconfig | `SkFontMgr_New_FontConfig` | 🟡 | **était cassé**, configuration désormais fournie |
| Hinting | `GlobalFontConfig` | 🟡 | idem |
| Écritures non latines | dépend des polices empaquetées | ❌ | aucune police CJK/arabe/indienne dans `Base/res/fonts` |
| Emoji | NotoEmoji.ttf | 🟡 | police présente, alias déclarés, non exercé |
| Façonnage HarfBuzz | vcpkg | ✅ | lié statiquement, utilisé par toute rastérisation |
| Unicode / ICU | LibUnicode + ICU | ✅ | lié ; le portage a vérifié que `domain_to_ascii` court-circuite IDNA pour l'ASCII |

### JavaScript

| Fonctionnalité | Upstream | Bouchaud | Cause |
|---|---|---|---|
| LibJS (interpréteur + bytecode) | `Libraries/LibJS` | ✅ | **prouvé** : la suite fonctionnelle est vingt-et-une assertions de JavaScript exécuté |
| Modules ES | `LibJS` | 🟡 | idem |
| Promesses, microtâches | `LibJS` + boucle d'événements | ✅ | **prouvé** : chaîne de `then` et rejet capturé |
| Minuteurs (`setTimeout`) | `LibWeb/HTML` | ✅ | **prouvé** par la suite fonctionnelle |
| Événements DOM | `LibWeb/DOM` | ✅ | **prouvé** : `addEventListener` + `dispatchEvent`. Clavier et souris arrivent au moteur (M11), effets non vérifiés |
| WebAssembly | `Libraries/LibWasm` | 🟡 | compilé — le portage a même dû contourner un plantage de Clang 18 dessus |
| Canvas 2D | `LibWeb/HTML/Canvas*` | 🚧 | `getContext("2d")` rend bien un contexte, mais tout dessin est perdu : le stockage d'un canvas vit dans le processus Compositor (§4). Mesuré par la suite fonctionnelle |
| WebGL | `LibWeb/WebGL` | ❌ | exige le processus Compositor et un contexte OpenGL |
| WebGPU | absent d'upstream | ⚪ | — |

### Réseau

| Fonctionnalité | Upstream | Bouchaud | Cause / preuve |
|---|---|---|---|
| Analyse d'URL | LibURL (analyseur Rust) | ✅ | `M15_URL_PARSE_OK` |
| DNS | LibDNS | ✅ | `M16_DNS_RX id=… rcode=0` ; une seule question par requête (voir `prepare-dns-une-question.py`) |
| DNS over TLS | LibDNS | ⚪ | désactivé délibérément : ferait dépendre la résolution d'une résolution |
| IPv6 | — | ❌ | `sys_socket` rend `EAFNOSUPPORT` pour `AF_INET6` ; délibéré, et c'est ce qui fait retomber `getaddrinfo` sur IPv4 |
| TCP | pile Bouchaud | ✅ | Wikipedia, 120 361 octets |
| TLS / HTTPS | curl + OpenSSL | 🟡 | Wikipedia réussit, **google.com échoue** — voir §8 |
| HTTP/1.1 | curl | ✅ | `M9_RS_HEADERS statut=200` |
| HTTP/2 | curl + ALPN | 🟡 | disponible, non vérifié |
| Redirections | `RequestServer` | 🟡 | `CURLOPT_FOLLOWLOCATION 0` : c'est **Fetch** qui les suit, conforme à la spécification |
| Cache HTTP disque | `LibHTTP/Cache` | ⚪ | désactivé explicitement (`--http-disk-cache-mode disabled`) ; `statfs` désormais implémenté si on le réactive |
| Cache HTTP mémoire | `Fetch::set_http_memory_cache_enabled` | ❌ | `--enable-http-memory-cache` non passé |
| Compression (gzip, brotli, zstd) | curl + `CURLOPT_ACCEPT_ENCODING` | 🟡 | activée par upstream, non vérifiée |
| Cookies | `WebView::CookieJar` | ✅ | **prouvé** : cookie posé par le serveur relu par le document, puis cookie écrit par script relu |
| HSTS | `WebView::HSTSStore` | 🟡 | idem |
| WebSocket | `LibWebSocket` + `WebSocketImplCurl` | 🟡 | compilé, jamais exercé |
| Fetch / XHR | `LibWeb/Fetch`, `LibWeb/XHR` | ✅ | **prouvé** : `fetch()` d'un JSON, statut et corps, puis `XMLHttpRequest` sur la même ressource |
| Autorités de certification | paquet `tools/ladybird/certs` | ✅ | `M12_CA_BUNDLE`, chaîne publique validée |
| Certificats clients | curl | ⚪ | non demandé |

### Stockage et état

| Fonctionnalité | Upstream | Bouchaud | Cause |
|---|---|---|---|
| `localStorage` / `sessionStorage` | `WebView::StorageJar` | ✅ | **prouvé** : aller-retour et suppression, sur les deux entrepôts |
| IndexedDB | `LibWeb/IndexedDB` | 🟡 | compilé ; dépend de `StorageJar`, donc débloqué mais non vérifié |
| Cache API, Service Worker | `LibWeb/ServiceWorker` | ❌ | exige le service `WebWorker` |
| Historique de session | `LibWeb/HTML` + `LibWebView/SessionHistory` | 🟡 | `M9_HISTORY_LOCAL_COMMIT` ; `traverse_the_history_by_delta` câblé au chrome |
| Téléchargements | `DidStartDownload` | ❌ | question synchrone sans répondant — fige le moteur |

### Interaction

| Fonctionnalité | Upstream | Bouchaud | Cause |
|---|---|---|---|
| Clavier | `Web::KeyEvent` | 🟡 | acheminé par le chrome M11 |
| Souris | `Web::MouseEvent` | 🟡 | idem |
| Molette / défilement | `Web::MouseEvent` + `wheel_delta` | 🟡 | acheminé ; le défilement dépend maintenant du modèle d'invalidation |
| Liens | navigation locale | 🟡 | `decide_navigation_process` forcé sur `Local` |
| Formulaires | `LibWeb/HTML` | 🟡 | jamais exercé |
| Presse-papiers | `LibWeb/Clipboard` | ❌ | aucun pont vers le presse-papiers Bouchaud |
| Plein écran | `LibWeb/Fullscreen` | ❌ | `page_did_request_fullscreen_window` part vers un hôte absent |
| Barre d'adresse, boutons | chrome Bouchaud | ✅ | `M11_READY`, `M11_FIRST_FRAME` |

### Multimédia et divers

| Fonctionnalité | Upstream | Bouchaud | Cause |
|---|---|---|---|
| Audio / vidéo | `LibMedia` + FFmpeg | 🟡 | LibMedia est lié ; aucune sortie audio branchée au moteur (Bouchaud a `/dev/dsp`, non relié) |
| Web Audio | `LibWeb/WebAudio` | 🟡 | idem |
| Manettes | SDL3 | ⚪ | `SDL_Init(SDL_INIT_GAMEPAD)` réussit ou le processus meurt — à surveiller |
| Géolocalisation, notifications, permissions | `LibWeb/*` | ❌ | tous passent par l'hôte |
| DevTools | `LibDevTools` | ⚪ | outil de développement |
| WebDriver | `Services/WebDriver` | ⚪ | automatisation ; **absence délibérée et documentée ici**, sans effet sur la navigation normale |
| Fuseau horaire | `Core::TimeZone` + ICU | 🟡 | `--default-time-zone` non passé ; la RTC de Bouchaud alimente `unix_time` |
| Fils d'exécution | `LibThreading` | ✅ | prouvé par le portage (processus séparés, `clone`) |
| Mémoire partagée | `memfd_create` + `MAP_SHARED` | ✅ | c'est le transport de la surface graphique |
| IPC | `LibIPC` | ✅ | trois services connectés |

---

## 8. Google : où exactement la poignée de main casse

Ce qu'on sait, et qui n'est pas rien :

- le nom est résolu (huit adresses IPv4) ;
- `RetrieveCookie` puis `Fetch` démarrent ;
- l'erreur est `CURLE_SSL_CONNECT_ERROR` et **non**
  `CURLE_PEER_FAILED_VERIFICATION`, qui possède son propre libellé
  (`SSLVerificationFailed`).

Donc : la confiance dans le certificat n'est pas en cause. C'est la négociation.

Ce qu'on ne savait pas, et pourquoi : Ladybird ne pose **nulle part**
`CURLOPT_ERRORBUFFER`. Sans ce tampon, curl ne peut rendre que le libellé
générique de son code. Avec, il rend la phrase d'OpenSSL, et ces phrases
désignent des pannes différentes :

```
sslv3 alert handshake failure           un paramètre refusé par le serveur
unexpected eof while reading            la connexion coupée en cours de route
SSL_connect: Connection reset by peer   un rejet immédiat
```

Aucun raisonnement ne permet de choisir entre elles. Le tampon, si.
`tools/ladybird/prepare-tls-diagnostic.py` le pose, imprime la phrase à l'échec,
et ajoute l'adresse réellement jointe, le port et le résultat de vérification.
Sur demande (`BOUCHAUD_CURL_TRACE=1`), la trace verbeuse de curl donne la
poignée de main étape par étape.

Un job d'intégration continue, informatif, vise `google.com` avec cette trace
allumée. **La réponse arrivera de la CI, pas d'une hypothèse.**

---

## 9. Ce qui reste, par ordre de valeur

1. **Le processus Compositor.** C'est lui qui débloque le canvas 2D — donc une
   part sérieuse du web moderne — et qui remplacerait le pont capture par le
   vrai pipeline de trames de Ladybird. Le chemin est identifié (§4) et ne
   demande pas de GPU. C'est le plus gros gain restant, et le plus risqué :
   il réécrit ce que quatre scénarios verts valident aujourd'hui.
2. **Lire la trace TLS de Google** et corriger la vraie cause.
3. **Un hôte navigateur.** C'est ce qui débloque `WebWorker`, les
   téléchargements, les nouvelles vues, le plein écran — et c'est la forme
   finale voulue : `Bouchaud Browser Host` engendrant les services, plutôt qu'un
   lanceur en C qui ne sait pas répondre.
4. **Confirmer les appelants de `inotify_init1` et `link`**, maintenant que le
   message les nomme, puis implémenter ou documenter.
5. **Empaqueter de vraies polices** si l'on veut que les sites ressemblent à ce
   que leurs auteurs ont prévu.
6. **Retirer les sondes** M15/M16/M17 devenues inutiles, une fois la chaîne
   verte — mais pas avant, et en gardant les tests de régression.
