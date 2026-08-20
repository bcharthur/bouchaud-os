# Ladybird sur Bouchaud OS — ce qui est prouve, et ce qui ne l'est pas

Ce document separe strictement trois choses : ce qu'une execution reelle a
montre, ce qui existe dans le code sans avoir jamais tourne, et ce qui manque.
Un point n'entre dans la premiere liste qu'avec le journal qui l'atteste.

## 1. Prouve par execution

Run `32423119213`, `ladybird-platform-smp4`, QEMU `-smp 4` (emulation TCG, pas
de KVM), noyau construit depuis la branche, artefact Ladybird du run
`32421532825`. Duree de la machine : 68 secondes.

    [kernel] SMP4_DISCOVERED count=4
    [kernel] SMP4_AP_STARTED count=3 expected=3
    [kernel] SMP4_SCHEDULER online=1 mode=UP-pending-refactor
    [ladybird-bouchaud] BROWSER_HOST_PLATFORM persistent=yes sql=enabled
        disk_cache=enabled async_scrolling=enabled site_isolation=top-level
        timezone=Europe/Paris audio=/dev/dsp
    33.665 WebContent(8): (js log) "PLATFORM_CANVAS OK"
    43.168 WebContent(8): (js log) "PLATFORM_WORKER OK"
    43.725 WebContent(8): (js log) "PLATFORM_WASM OK"
    45.678 WebContent(8): (js log) "PLATFORM_INDEXEDDB OK"
    45.796 WebContent(8): (js log) "PLATFORM_FULL_OK passed=19/19"

`WebContent(8)` est un processus separe, lance par le BrowserHost via le
mecanisme upstream `SOCKET_TAKEOVER`.

Les dix-neuf verifications : localStorage, sessionStorage, temoins de
connexion, `fetch` HTTP reel, DOM/querySelector, promesses tenues et rompues
plus exception synchrone, ordre des minuteries, fuseau horaire vu par
`Intl.DateTimeFormat`, Canvas 2D, PNG, JPEG, **pixels decodes** d'une image
reseau, image absente sans emporter la page, WebWorker, WebAssembly,
IndexedDB, navigation entre documents, historique arriere/avant, rechargement.

Le test d'image qui compte n'est pas l'evenement `load` mais `drawImage` suivi
de `getImageData` : il verifie la chaine RequestServer -> ImageDecoder -> Skia.

Aucune fonctionnalite n'a ete desactivee pour obtenir ce resultat :
`site_isolation=top-level`, `disk_cache=enabled` et `async_scrolling=enabled`
sont tous actifs.

### Primitives OS mesurees separement

Chaque sonde est d'abord validee **sur Linux** — ou elle doit rendre zero
echec — puis passee sous Bouchaud. Une sonde qui echouerait des deux cotes ne
prouverait rien.

| Sonde | Ce qu'elle etablit |
| --- | --- |
| `verrous-probe.c` | verrous d'enregistrement POSIX, prerequis de SQLite |
| `wal-probe.c` | fichier `-shm` du WAL : `ftruncate`, `MAP_SHARED` par le chemin entre deux processus, verrous `[120,128)`, `msync`, `fsync` |
| `exec-fd-probe.c` | heritage de descripteur a travers `execve`, `S_IFSOCK` apres exec, `FD_CLOEXEC` — ce dont depend `SOCKET_TAKEOVER` |
| `posix-probe.c` | `fork`/`wait`, isolation memoire, `execve`, signaux, `mmap` partage, `socketpair`, quota memoire, reseau |
| `shm-probe.c`, `ipc-probe.c` | memoire partagee par descripteur herite, tubes et paires de sockets |

`/persist` survit a un vrai redemarrage, y compris **sans `fsync` explicite**
depuis que `kernel::power::shutdown` ecrit la zone avant de couper :

    boot 1  [kernel] persistance: 1 fichier(s) ecrit(s) a l'extinction
    boot 2  [kernel] persistance: 1 fichier(s) restaure(s) depuis le disque
            survecu-a-l-extinction-42

L'archive de demarrage peut depasser 768 Mio : verifie sur une image de
1,05 Gio dont un fichier traverse l'ancienne borne, et dont le programme range
au-dela s'execute.

## 2. Present dans le code, jamais prouve a l'execution

Ces points sont cables et s'annoncent dans les journaux, mais aucune execution
ne les a exerces. Ne pas les compter comme acquis.

- **HTTPS reel, DNS reel** : la fixture n'a prouve HTTP que jusqu'a 10.0.2.2.
  Les tests `https` et `reseau_echec` existent desormais mais n'ont pas encore
  tourne sous Bouchaud.
- **Telechargement vers `/persist/Downloads`** : le chemin upstream est
  identifie (`page_did_start_download` -> `default_path_for_downloaded_file` ->
  `StandardPaths::downloads_directory()`, qui honore `XDG_DOWNLOAD_DIR`), la
  fixture le declenche, mais aucun fichier n'a encore ete constate par l'OS.
- **Persistance du PROFIL du navigateur apres redemarrage** : la persistance de
  l'OS est prouvee, celle du profil ne l'est pas.
- **WebDriver** : construit, empaquete, static PIE, sans `PT_INTERP`. Jamais
  lance.
- **Compositor** : construit et lance, mais aucun test ne distingue son
  rendu de celui de WebContent.
- **Audio** : `SDL_AUDIODRIVER=oss` et `AUDIODEV=/dev/dsp` sont poses, et le
  journal l'annonce. Aucun octet PCM n'a ete constate sur le peripherique.
- **Plein ecran, fenetres, popups** : les rappels upstream de `HeadlessWebView`
  sont en place ; rien ne les a declenches.
- **Presse-papiers** : abstraction upstream, texte seulement, non exercee.

### Defaut ouvert

Depuis que le scenario enchaine deux demarrages, le BrowserHost meurt seize
secondes apres le demarrage, deux secondes apres sa banniere :

    Runtime error: disk I/O error

C'est le texte de `SQLITE_IOERR`. L'hypothese du WAL a ete testee puis ecartee
(`wal-probe.c` passe sous Bouchaud). `strace echecs` et les diagnostics de
`persistance::synchronise` sont en place pour nommer la cause.

Un cout structurel est par ailleurs identifie, sans qu'il soit encore etabli
qu'il soit la cause : **chaque `fsync` sous `/persist` reecrit la zone
entiere**. SQLite valide par `fsync`, et le cache disque HTTP vit lui aussi
sous `/persist` : le cout est donc en O(taille totale de /persist) par
validation.

## 3. Absent — ce qu'il faudrait construire

- **WebGL / GPU** : Bouchaud n'expose aucun peripherique graphique au guest
  au-dela du framebuffer. Il faudrait une interface GPU invitee (virtio-gpu ou
  equivalent), un pilote Bouchaud, puis une liaison OpenGL/EGL, ou un rendu
  logiciel que Skia/ANGLE accepte. Le rendu CPU fonctionne et reste la
  reference.
- **Sandbox** : `--disable-sandbox` est passe explicitement, et le journal le
  dit. Ce n'est pas un sandbox Bouchaud degrade : il n'y en a aucun. Il
  faudrait decider ce qu'un processus de rendu a le droit de faire (systeme de
  fichiers, reseau, descripteurs, creation de processus) et l'appliquer dans le
  noyau.
- **Geolocalisation, notifications, permissions** : aucun service OS. La
  reponse correcte reste « refuse / indisponible » ; ne jamais rendre de
  fausses coordonnees ni accorder une permission d'office.
- **Presse-papiers multi-MIME partage par l'OS** : le texte d'abord, et il
  n'est pas encore exerce.
- **Ordonnanceur SMP utilisateur** : les quatre vCPU demarrent reellement
  (`SMP4_AP_STARTED count=3`), mais `kernel::task` reste mono-CPU et l'annonce
  (`mode=UP-pending-refactor`). Les processus Ladybird tournent donc tous sur
  le BSP. Un vrai ordonnanceur SMP demande pile, GDT, TSS et GS par CPU, timer
  LAPIC, files par CPU, verrous noyau, invalidation TLB inter-processeurs, et
  la suppression des `static mut` et `Rc<RefCell>` non partageables.

## Trampoline SMP

Le demarrage des AP utilise une trampoline en memoire basse a 0x8000 et un
compteur a 0x9000. Cela fonctionne sous QEMU mais suppose ces deux pages
libres. Une reservation explicite dans la carte memoire du chargeur reste a
faire avant de considerer ce bring-up comme robuste.
