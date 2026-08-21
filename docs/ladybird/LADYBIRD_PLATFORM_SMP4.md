# Ladybird sur Bouchaud OS — ce qui est prouve, et ce qui ne l'est pas

Ce document separe strictement trois choses : ce qu'une execution reelle a
montre, ce qui existe dans le code sans avoir jamais tourne, et ce qui manque.
Un point n'entre dans la premiere liste qu'avec le journal qui l'atteste.

## 1. Prouve par execution

Run `32476709016`, `ladybird-platform-smp4`, **vert**. QEMU `-smp 4` (emulation
TCG, pas de KVM), noyau construit depuis la branche, artefact Ladybird du run
`32473543012`. DEUX demarrages sur la MEME image, 99 s puis 93 s.

    [kernel] SMP4_DISCOVERED count=4
    [kernel] SMP4_AP_STARTED count=3 expected=3
    [kernel] SMP4_SCHEDULER online=1 mode=UP-pending-refactor
    [ladybird-bouchaud] BROWSER_HOST_PLATFORM persistent=yes sql=enabled
        disk_cache=enabled async_scrolling=enabled site_isolation=top-level
        timezone=Europe/Paris audio=/dev/dsp

    demarrage 1  77.477 WebContent(8): "PLATFORM_FULL_OK passed=21/21"
                 [ladybird-bouchaud] BROWSER_HOST_EXIT boucle_quittee code=0
                 [ladybird-bouchaud] BROWSER_HOST_ARRET services fermes
                 [kernel] persistance: 23 fichier(s) ecrit(s) a l'extinction

    demarrage 2  [kernel] persistance: 23 fichier(s) restaure(s) depuis le disque
                 + cat /persist/Downloads/preuve-bouchaud.bin
                 Bouchaud download proof
                 46.048 "PLATFORM_PERSIST_APRES_REDEMARRAGE OK
                         localStorage+cookie indexeddb=perdu-en-memoire-upstream"
                 73.559 WebContent(8): "PLATFORM_FULL_OK passed=21/21"

`WebContent(8)` est un processus separe, lance par le BrowserHost via le
mecanisme upstream `SOCKET_TAKEOVER`.

Les vingt et une verifications : localStorage, sessionStorage, temoins de
connexion, `fetch` HTTP reel, **HTTPS reel avec DNS reel**, **echec reseau sur
un nom qui ne resout pas**, DOM/querySelector, promesses tenues et rompues plus
exception synchrone, ordre des minuteries, fuseau horaire vu par
`Intl.DateTimeFormat`, Canvas 2D, PNG, JPEG, **pixels decodes** d'une image
reseau, image absente sans emporter la page, WebWorker, WebAssembly,
IndexedDB, navigation entre documents, historique arriere/avant, rechargement.

Le test d'image qui compte n'est pas l'evenement `load` mais `drawImage` suivi
de `getImageData` : il verifie la chaine RequestServer -> ImageDecoder -> Skia.
Le test HTTPS ne se contente pas d'un code 200 : il relit la reponse de
`raw.githubusercontent.com` pour y retrouver le SHA Ladybird epingle.

La liste des marqueurs exiges par la CI est LUE dans la fixture
(`tools/health/marqueurs_fixture.py`) : deux listes tenues a la main avaient
diverge, et le verdict a reclame « passed=19/19 » pendant plusieurs runs alors
que la fixture en emettait 21.

Aucune fonctionnalite n'a ete desactivee pour obtenir ce resultat :
`site_isolation=top-level`, `disk_cache=enabled` et `async_scrolling=enabled`
sont tous actifs.

### Ce que le second demarrage etablit en propre

- **Le profil du navigateur survit a un redemarrage** : `localStorage` et le
  temoin de connexion sont retrouves, via les magasins SQL de Ladybird, donc
  via un vrai fichier sur le disque de Bouchaud.
- **Un telechargement atteint `/persist/Downloads`** et son contenu exact est
  relu par le shell de l'OS apres extinction et redemarrage.
- **Le navigateur s'arrete proprement** sur `window.close()`, l'invite reprend,
  et c'est cette extinction qui ecrit la zone persistante.
- **Le cache HTTP disque se relit apres redemarrage** : au second demarrage la
  reponse HTTPS vient du cache, ce qui exerce `sendfile(2)`.

IndexedDB, lui, ne survit pas — et pas a cause de Bouchaud. Au SHA epingle ses
bases vivent dans une table statique du processus WebContent
(`Libraries/LibWeb/IndexedDB/Internal/Database.cpp:16`) et rien ne les ecrit
sur disque. Le marqueur l'annonce au lieu de le taire.

### Primitives OS mesurees separement

Chaque sonde est d'abord validee **sur Linux** — ou elle doit rendre zero
echec — puis passee sous Bouchaud. Une sonde qui echouerait des deux cotes ne
prouverait rien. Le workflow `os-primitives` les fait toutes tourner.

| Sonde | Ce qu'elle etablit |
| --- | --- |
| `verrous-probe.c` | verrous d'enregistrement POSIX, prerequis de SQLite |
| `wal-probe.c` | fichier `-shm` du WAL : `ftruncate`, `MAP_SHARED` par le chemin entre deux processus, verrous `[120,128)`, `msync`, `fsync` |
| `exec-fd-probe.c` | heritage de descripteur a travers `execve`, `S_IFSOCK` apres exec, `FD_CLOEXEC` — ce dont depend `SOCKET_TAKEOVER` |
| `disque-probe.c` | cinq lecteurs paginant un gros fichier pendant 120 validations sous `/persist` : le scenario qui expose le pilote ATA |
| `nom-long-probe.c` | `NAME_MAX` a 255, et le refus d'un nom plus long par `ENAMETOOLONG` et non par `ENOSPC` |
| `sendfile-probe.c` | `sendfile(2)` : copie complete, decalage explicite sans deplacer la position, `EINVAL` sur une source qui n'est pas un fichier |
| `session-probe.c` | un programme de premier plan qui sort en laissant des fils vivants doit rendre la main a l'invite |
| `posix-probe.c` | `fork`/`wait`, isolation memoire, `execve`, signaux, `mmap` partage, `socketpair`, quota memoire, reseau |
| `shm-probe.c`, `ipc-probe.c` | memoire partagee par descripteur herite, tubes et paires de sockets |

L'archive de demarrage peut depasser 768 Mio : verifie sur une image de
1,05 Gio dont un fichier traverse l'ancienne borne, et dont le programme range
au-dela s'execute.

### Defauts trouves et corriges pendant ce jalon

Chacun a ete diagnostique sur un journal, pas suppose, et chacun a laisse une
sonde derriere lui.

| Defaut | Consequence observee |
| --- | --- |
| `sys_fcntl` rendait 0 a `F_SETLK` sans rien verrouiller | SQLite croyait tenir un verrou |
| Attentes ATA bornees en tours de boucle, pas en temps | `ata::write` rendait 0 secteur, `fsync` rendait EIO, SQLite disait « disk I/O error » |
| `NAME_LEN` valait 64, Linux garantit 255 | AUCUN telechargement ne pouvait aboutir : le nom temporaire de Ladybird fait 67 octets |
| Toute erreur de creation rendait `ENOSPC` | un nom trop long se presentait comme un disque plein |
| Le nom etait valide APRES l'allocation d'un inode | chaque refus perdait un inode pour de bon |
| `task::run` ne rendait la main que si PLUS AUCUNE tache n'etait prete | l'invite ne revenait jamais apres le navigateur, donc `/persist` n'etait jamais ecrit a l'extinction |
| `sendfile(2)` n'existait pas | le cache HTTP disque etait illisible apres redemarrage |
| `shutdown` annoncait « rien a ecrire » sur un echec | une panne d'ecriture passait pour normale |
| 34 appels systeme et 40 codes d'erreur sans nom dans la trace | un diagnostic a moitie muet |
| Le shell lisait `Node::content` au lieu du disque | `tail` sur un runtime de 190 Mio ne rendait rien |
| Le shell refusait `/chemin/programme` | le BrowserHost n'avait jamais pu demarrer une seule fois |
| `persistance::synchronise` tronquait un chemin trop long | corruption silencieuse au redemarrage suivant |

## 2. Present dans le code, jamais prouve a l'execution

Ces points sont cables et s'annoncent dans les journaux, mais aucune execution
ne les a exerces. Ne pas les compter comme acquis.

- **WebDriver** : construit, empaquete, static PIE, sans `PT_INTERP`. Jamais
  lance.
- **Compositor** : construit et lance, mais aucun test ne distingue son rendu
  de celui de WebContent.
- **Audio** : `SDL_AUDIODRIVER=oss` et `AUDIODEV=/dev/dsp` sont poses, et le
  journal l'annonce. Aucun octet PCM n'a ete constate sur le peripherique.
- **Video** : aucun test de lecture.
- **Plein ecran, fenetres, popups** : les rappels upstream de `HeadlessWebView`
  sont en place ; rien ne les a declenches.
- **Presse-papiers** : abstraction upstream, texte seulement, non exercee.
- **Lanceur de bureau** : le navigateur se lance depuis le shell, pas depuis
  une icone du bureau.
- **Navigations de tete successives** : la fixture navigue dans une IFRAME,
  pour que le traversable de tete garde une seule entree d'historique et que
  `window.close()` reste autorise. Une suite de navigations du document de tete
  n'est donc pas couverte.

### Cout structurel connu

**Chaque `fsync` sous `/persist` reecrit la zone entiere.** SQLite valide par
`fsync`, et le cache disque HTTP vit lui aussi sous `/persist` : le cout est en
O(taille totale de /persist) par validation. Cela n'empeche rien aujourd'hui —
23 fichiers, deux demarrages de moins de 100 s — mais c'est la premiere chose
qui cedera si le profil grossit.

Cet effet a toutefois une consequence utile, et mesuree : le profil du
navigateur a survecu a un redemarrage AVANT meme que l'extinction propre ne
fonctionne (run `32427953935`), parce que les `fsync` de SQLite ecrivaient la
zone entiere au passage.

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
- **`PR_SET_PDEATHSIG`** : absent, donc le portage le desactive. Les services
  du navigateur ne meurent pas avec leur pere par eux-memes ; c'est
  `task::exit_current` qui termine la session cote noyau. Le comportement
  observable est le bon, mais l'appel manque.
- **Persistance d'IndexedDB** : manque chez Ladybird au SHA epingle, pas chez
  Bouchaud. La lui ajouter serait reimplementer une fonctionnalite du moteur.
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
