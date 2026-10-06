# UI/Bouchaud — le frontend natif de Ladybird (BOUCHAUD_UI_V1)

Réalise le plan de `FRONTEND_BOUCHAUD.md` (T1 à T4). Ce document dit ce qui
existe, comment c'est construit, ce qui est prouvé et à quel niveau — et ce qui
ne l'est pas encore.

## 1. Avant / après

| | Avant (M11) | Après (UI/Bouchaud) |
|---|---|---|
| Chrome (barre, onglets, menus, recherche) | **dans WebContent** (`prepare-m11-chrome.py`) | dans le processus navigateur `BouchaudBrowserHost` |
| Canal GUI + surface du WM | hérités par **chaque** WebContent | `FD_CLOEXEC` + variables retirées : navigateur seul |
| Entrée clavier/souris | injectée localement dans WebContent (accusés falsifiés, `Queue.h:50`) | `ViewImplementation::enqueue_input_event`, file upstream |
| Présentation d'une trame | WebContent **re-rend** une capture (`render_screenshot`) en plus de la trame du Compositor | trame du Compositor (`did_accept_presented_backing_store`), **une** peinture |
| Dégât | recalculé par un patch LibWeb (`prepare-repaint.py`) | natif : `BackingStoreManager` → `server_did_paint` |
| Onglets | pages créées dans WebContent (`PageHost::create_page`) | une `BouchaudWebView` par onglet, WebContent neuf par onglet utilisateur |
| Cookies / stockage / HSTS / historique de session | partiellement locaux à WebContent (`BOUCHAUD_M9`) | upstream (`WebView::Application`) |
| Téléchargements | écrits **par WebContent** dans `/persist/Downloads` | `WebView::FileDownloader` dans le navigateur, observé par le chrome |
| Console JS → série | patch de `PageClient` | `ViewImplementation::on_console_message` (`[LB:JS]`) |
| Bac à sable | `--disable-sandbox`, implémentations `Unimplemented` | confinement noyau **vérifié** par chaque service (fail-closed) |
| Compositor (profil noyau) | `BrowserBroker` (non sandboxé, DEVICE_IO, NET_CONNECT) | `BrowserContent` (rôle de rendu confiné) |
| Divergence upstream (hors frontend) | 49 fichiers, +9 563 lignes | **26 fichiers, +726 lignes** |
| `Services/WebContent/{ConnectionFromClient,PageClient}.cpp` | +539 / +918 lignes | **upstream à l'octet** |

## 2. Architecture

```
Bouchaud WM ──Key/Pointer/Wheel (GUI v1)──▶ BouchaudBrowserHost  (UI/Bouchaud)
     ▲                                       ├─ BouchaudUI::Application : WebView::Application
     │                                       │     presse-papiers, téléchargements, onglets
     │                                       ├─ BrowserWindow : onglets ⇄ chrome
     │                                       │     Core::Notifier(gui_fd), tic 16 ms (chrome seul)
     │                                       ├─ BouchaudWebView : HeadlessWebView (1 par onglet)
     │                                       └─ BouchaudChrome.h (dessin, saisie, magasin)
     │                                              │ enqueue_input_event / load / reload...
     │                                              ▼
     │                                         WebContent (IPC upstream, contenu seul)
     │                                              │ display list
     │                                              ▼
     │                                         Compositor (Skia CPU, backing stores partagés)
     │                                              │ did_present_frame(damage)
     └── FrameReady(dégât) + surface ◀── BouchaudChrome::present ◀── did_accept_presented_backing_store
```

Sources (dans le dépôt, copiées — jamais patchées — dans `UI/Bouchaud/` par
`tools/ladybird/prepare-ui-bouchaud.py`) :

- `tools/ladybird/ui-bouchaud/` : `main.cpp`, `Application.{h,cpp}`,
  `BrowserWindow.{h,cpp}`, `BouchaudWebView.{h,cpp}`, `CMakeLists.txt` ;
- `tools/ladybird/chrome/Bouchaud*.h` : le chrome (namespace `BouchaudChrome`),
  V15/V16 intégrés à la source (plus de `modernise-v15/16.py`) ;
- `tools/ladybird/sandbox/` : `BouchaudConfinement.h` et les quatre
  implémentations `apply_sandbox` Bouchaud.

Le binaire garde son nom, `BouchaudBrowserHost` : le noyau le lance sous ce nom
(profil, supervision, `/bo-navigateur`).

## 3. La chaîne de préparation

`browser-upstream.sh` applique, dans l'ordre (`tools/verifie-chain-ladybird.py`
l'impose) : `browser-source`, `m9-source` (sondes LibRequests seulement),
`m9-diagnostics`, `m16-dns`, `dns-une-question`, `fonts-systeme`, `v16-fonts`,
`tls-diagnostic`, `browser-runtime-link`, `sandbox-bouchaud`,
`full-browser-host` (LibCore/LibWebView seulement), `network-live`,
`ui-bouchaud` (en dernier : il refuse un WebContent qui porterait encore du
chrome).

Retirés : `prepare-m11-chrome`, `-m11-page-registry`, `-m11-input-ownership`,
`-v19-navigateur`, `-repaint`, `-browser-host`, `-console`, `-image-decoder`,
`-platform-complete`, `-m9-navigation`, `chrome/modernise-v15/16`,
`apply-browser-host-overlay`, `test-input-ownership.sh`.

Réseau : `m16-dns`, `dns-une-question`, `tls-diagnostic`, `network-live` et les
sondes LibRequests sont **inchangés** (la chaîne RTL8168 → TLS est considérée
fonctionnelle).

## 4. Présentation et dégât (P2, P3)

`BouchaudWebView::did_accept_presented_backing_store(bitmap_id, damage)` lit
`m_client_state.front_bitmap` (tampon de face juste échangé par
`server_did_paint`) et le remet à `BrowserWindow::present`, qui appelle
`BouchaudChrome::present(onglet, bitmap, largeur_peinte, hauteur_peinte, dégât)`.
Le chrome planifie (`BouchaudDegat::Suivi`), recopie **les seules lignes du
plan** dans la surface du WM et publie `FrameReady` avec le rectangle publié.

Sûreté de la référence : la vue ne rend le tampon au Compositor
(`notify_presented_bitmap_ready_to_paint`) qu'après l'échange suivant, et
`present()` remplace la référence avant toute réécriture. Un onglet inactif
range sa trame sans la composer.

Télémétrie : `BOUCHAUD_UI_FIRST_FRAME`, `[LB:FRAME] onglet= seq= t= degat= zone=
present_us=` (4 096 premières puis 1/64), `[LB:PERF] trames= degat_px= zone_px=
copie_px= trames_partielles= trames_completes= sans_effet= present_us_moy=
present_us_pire=` toutes les 256 trames.

## 5. Bac à sable (P6)

Le noyau confine par profil dès l'exec (`src/kernel/security/profile.rs`) :
WebContent, WebWorker, ImageDecoder **et Compositor** → `BrowserContent` ;
RequestServer → `BrowserNetwork` ; `BouchaudBrowserHost` → `BrowserBroker`.
Chaque service **vérifie** son confinement avant de traiter une donnée du
réseau (`BouchaudConfinement::verifie`) : `no_new_privs`, écriture `/usr`
refusée, et pour un rendu écriture `/persist/{ladybird,Downloads,ladybird-chrome}`
et `socket(AF_INET)` refusées. Une sonde qui réussit arrête le service
(`[LB:SANDBOX] ECHEC ...`). `--disable-sandbox` n'est plus passé par défaut
(`BOUCHAUD_DISABLE_SANDBOX` pour un diagnostic).

Droits retirés au rendu : `/persist/Downloads` et `/persist/ladybird-chrome`
(lecture et écriture) — ils n'existaient que parce que le chrome vivait dans
WebContent. Tests négatifs : `tools/security/test_bac_a_sable_navigateur.rs`
(`aucun_role_sandboxe_n_atteint_le_depot_ni_le_magasin_du_chrome`,
`le_compositor_est_un_role_de_rendu_confine`),
`tools/ladybird/test_roles_livres.rs`.

## 5 bis. Profil, base SQL, cache HTTP (P7)

`--disable-sql-database` et `--disable-http-disk-cache` ne sont plus passés
(ni par Stage 2, ni par le smoke) ; ils restent disponibles pour un diagnostic
(`BOUCHAUD_DISABLE_SQL`, `BOUCHAUD_DISABLE_DISK_CACHE`).

- **Prérequis noyau** : `rename`/`renameat`/`renameat2` POSIX
  (`src/fs/renommage.rs`, sonde `renommage-probe` 24/24) ; le WAL de SQLite
  (verrous d'enregistrement, `MAP_SHARED` du `-shm`) est couvert par
  `wal-probe`.
- **Profil** (BOUCHAUD_PROFIL_XDG_V1) : plus de `--profile-path`, qui rangeait
  aussi le *runtime* (sockets, pid) sous `/persist`. Le profil `default`
  d'upstream sous les racines XDG donne
  `/persist/ladybird/{config,data,cache}/Ladybird/Profiles/default`, runtime
  sous `/tmp/ladybird-runtime`. Journal : `[LB:PROFILE]`, `[LB:CACHE]`.
- **Stage 2** reste en RAM (`BOUCHAUD_LADYBIRD_EPHEMERAL`) : le montage NVMe
  y est différé après l'arrivée au bureau et dépose les fichiers du disque
  par-dessus ceux du RAMFS — une base SQLite ouverte avant lui serait réécrite
  sous les pieds du navigateur. SQL et cache y sont actifs, en RAM.
- **Cache jetable** (BOUCHAUD_PERSIST_CACHE_JETABLE_V1) : la zone persistante
  porte 2048 fichiers et ~64 Mio ; au-delà, la synchronisation échouait pour
  TOUT `/persist` (cookies et réglages compris, `fsync` → `EIO`). Le navigateur
  étiquette son cache `CACHEDIR.TAG` ; la persistance écarte un arbre étiqueté
  **en entier** (index SQLite compris) quand il ne tient plus, jamais le
  non-jetable (`src/fs/cache_jetable.rs`, `tools/fs/test_cache_jetable.rs`,
  `tools/ci/run_persist_cache.sh`, preuves dans
  `docs/preuves/persist-cache-jetable-20261006/`).
- **`statfs(/persist)`** annonce la zone (66 580 480 octets, 2048 entrées) et
  non la RAM (~1 Gio en QEMU) : c'est sur ce chiffre qu'upstream dimensionne le
  cache. Plafond Bouchaud supplémentaire : 32 Mio (le RAMFS n'a que 4096
  inodes pour tout le système ; un réglage utilisateur n'est pas écrasé).

## 6. Journal structuré

`[LB:UI]`, `[LB:PROFILE]`, `[LB:CACHE]`, `[LB:TAB]`, `[LB:NAV]`, `[LB:FRAME]`, `[LB:PERF]`, `[LB:JS]`,
`[LB:DOWNLOAD]`, `[LB:DIALOG]`, `[LB:CRASH]`, `[LB:SANDBOX]`. Jalons :
`BOUCHAUD_UI_CHROME_OWNER browser`, `BOUCHAUD_UI_WEBCONTENT_CHROME 0`,
`BOUCHAUD_UI_V1_READY`, `BOUCHAUD_UI_FIRST_FRAME`.

## 7. Ce qui est prouvé, et à quel niveau

| Preuve | Niveau |
|---|---|
| La chaîne s'applique sur l'arbre épinglé et `verifie-chrome.sh` passe | hôte (exécution réelle, `verifie-chain-ladybird.py --complet`) |
| Les TU de `UI/Bouchaud`, les sandbox, et les `.cpp` patchés compilent (`-fsyntax-only`, en-têtes générés par `genere-entetes-locaux.py`) | hôte, syntaxe/sémantique C++ (hors Skia réel, curl, OpenSSL ≥ 3.2) |
| Gardes d'architecture (UI, chaîne, onglets, presse-papiers, téléchargements, menus, clavier, calques, repeinture) avec tests négatifs | statique |
| Bac à sable : prédicats de chemins et classification des rôles | hôte (tests Rust) |
| Build Ladybird complet, lien statique | **CI** (`ladybird-native-browser`) — à constater |
| Fenêtre, trame Compositor, entrée, onglets, sandbox à l'exécution | **QEMU** (smoke `run_ladybird_browser_host.sh`) — à constater |
| Trigkey physique | **non validé** |

## 8. Limites connues (non masquées)

- Boîtes de dialogue (`alert/confirm/prompt`) : fermées automatiquement et
  journalisées (`[LB:DIALOG]`) ; le chrome ne les dessine pas encore.
- `<select>`, sélecteur de fichier, géolocalisation, couleur : non branchés.
- Boutons précédent/suivant toujours actifs (l'état « peut reculer » n'est pas
  encore relu de la vue).
- Menu contextuel de lien : le lien est celui du dernier survol.
- Historique/favoris : magasin texte du chrome (`/persist/ladybird-chrome`),
  pas encore `WebView::HistoryStore`.
- Cache HTTP : aucune réutilisation après redémarrage n'est encore mesurée par
  un banc navigateur (la persistance, elle, l'est en QEMU). Le RAMFS reste borné
  à 4096 inodes.
- `BOUCHAUD_M9` reste exporté : il ne commande plus que l'instrumentation
  LibRequests et le drainage du corps (contournement du notificateur).
