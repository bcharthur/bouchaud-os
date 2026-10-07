# Sécurité des rôles du navigateur — matrice appliquée par le noyau

`BOUCHAUD_MATRICE_ROLES_V1`, `BOUCHAUD_PROFIL_PAR_ROLE_V1`

Bouchaud OS n'émule ni seccomp ni pledge. Le noyau classe chaque processus
d'après l'emplacement et le nom de son image (`src/kernel/security/profile.rs`)
et lui applique un `SecurityProfile`. Les chemins permis à chaque profil sont
des fonctions pures du texte du chemin (`src/kernel/security/chemins.rs`),
testées sur l'hôte (`tools/security/test_bac_a_sable_navigateur.rs`) et en
exécution réelle (`tools/ci/run_matrice_roles.sh`).

## Rôles

| Processus Ladybird | Profil noyau | Sandboxé | Capacités |
|---|---|---|---|
| `BouchaudBrowserHost` (UI/Bouchaud) | `BrowserBroker` | non | exec, JIT, périphériques, IPC, réseau |
| `WebContent` | `BrowserContent` | oui (`no_new_privs`) | JIT, IPC |
| `WebWorker` | `BrowserContent` | oui | JIT, IPC |
| `ImageDecoder` | `BrowserContent` | oui | JIT, IPC |
| `Compositor` | `BrowserContent` | oui | JIT, IPC |
| `RequestServer` | `BrowserNetwork` | oui | IPC, réseau |

Un rôle sandboxé ne peut lancer aucun programme. Un binaire placé dans un
emplacement non fiable (`/tmp`, `/Downloads/`…) est `Untrusted`, quel que
soit son nom.

## Matrice (vérifiée par vrais appels système)

`tools/userland/matrice-roles-probe.c`, copiée sous le nom de chaque service.
Un refus attendu doit être `EACCES` ou `EPERM`. `ENOENT` est compté comme un
écart : il prouve seulement que la cible manque.

| Opération | Rendu (WebContent, WebWorker, ImageDecoder, Compositor) | RequestServer |
|---|---|---|
| lire/écrire la base SQL du profil (`/persist/ladybird/data/...`) | REFUS | REFUS |
| lire les réglages (`/persist/ladybird/config/...`) | REFUS | REFUS |
| lire la base SQL du profil éphémère (`/tmp/ladybird-data/...`) | REFUS | REFUS |
| lire/écrire le cache HTTP (`/persist/ladybird/cache/...`) | REFUS | **PERMIS** |
| lire le cache HTTP éphémère (`/tmp/ladybird-cache/...`) | REFUS | **PERMIS** |
| lire les téléchargements (`/persist/Downloads/...`) | REFUS | REFUS |
| lire l'historique et les favoris (`/persist/ladybird-chrome/...`) | REFUS | REFUS |
| écrire sous `/usr` | REFUS | REFUS |
| écrire à la racine de `/persist` | REFUS | REFUS |
| écrire `/dev/dsp` | **PERMIS** (exception, voir plus bas) | REFUS |
| lire `/dev/dsp` | REFUS | REFUS |
| `socket(AF_INET)` | REFUS (`EPERM`) | **PERMIS** |
| lancer un programme (`execve`) | REFUS | REFUS |

La base SQL (cookies, `localStorage`, historique d'upstream) et les réglages
appartiennent au processus navigateur, qui n'est pas sandboxé. RequestServer
ne possède que son `cache_path`, le seul chemin qu'il utilise
(`Services/RequestServer/main.cpp` : cache disque et `alt-svc-cache.txt`). Le
bac à sable upstream le restreint déjà à ce chemin.

### Le profil éphémère

En mode éphémère (bureau live, `BOUCHAUD_LADYBIRD_EPHEMERAL`), le profil vit
sous `/tmp/ladybird`, `/tmp/ladybird-config`, `/tmp/ladybird-data` et
`/tmp/ladybird-cache`. `/tmp` est commun à tous les rôles sandboxés. Jusqu'à
`BOUCHAUD_PROFIL_PAR_ROLE_V1`, un WebContent compromis pouvait donc y lire les
cookies et le cache HTTP. Ces racines sont désormais privées dans les deux
modes. `/tmp/ladybird-runtime` (sockets) n'en fait pas partie.

### Exception documentée : `/dev/dsp` pour le rendu

LibMedia joue le son **dans WebContent** (`PlaybackStream`). Ladybird n'a pas
de serveur audio séparé ; sous Linux, WebContent parle à PulseAudio par une
socket. Le droit accordé est le strict nécessaire : ouvrir `/dev/dsp` en
écriture et régler le format (ioctls OSS). Les limites :

- pas de lecture, donc pas de capture ;
- pas les autres noms du périphérique ;
- pas le rôle réseau.

Le profil `BrowserContent` est partagé par les quatre rôles de rendu.
ImageDecoder, WebWorker et le Compositor ont donc le même droit. Un rendu
compromis peut faire du bruit : c'est le prix, le même qu'avec une socket
PulseAudio. Le supprimer demanderait un profil `BrowserMedia` distinct (ou un
processus audio courtier), ce qui n'est pas fait.

## Refus attendus et refus inattendus

Le noyau journalise chaque refus (`[SECURITY-DENY] op= path= reason=`). Dans
un smoke, sont **attendus** :

- les sondes de confinement (`/.bouchaud-confinement-<service>-<pid>` sous
  `/usr` et `/persist/...`), écrites exprès par chaque service pour prouver
  qu'il est confiné ;
- `op=raw-socket` pour un rôle de rendu ;
- `op=fs-remove path=/usr/share/fonts/.../.uuid` (fontconfig tente de
  nettoyer son cache dans un répertoire système en lecture seule).

Le smoke refuse explicitement les refus qui signalent une **régression** :
`op=fs-create detail=0x1 path=/persist reason` (création du cache impossible),
`op=fs-fchown`, `Unable to create disk cache`, `[LB:SANDBOX] ECHEC`.

## Isolation de site et des cadres

UI/Bouchaud transmet `BOUCHAUD_SITE_ISOLATION` à l'option upstream
`--site-isolation` (`ui-bouchaud/main.cpp`), `top-level` par défaut.

- **top-level** (défaut) : une navigation racine vers un autre site change de
  WebContent (`[LB] PROCESS_SWAP onglet= ancien_pid= nouveau_pid=`). Attention :
  un onglet ouvert par `window.open` naît sur `about:blank`, et upstream laisse
  `about:blank` aller vers n'importe quel site **sans** changer de processus
  (`SiteIsolationManager::navigation_requires_process_swap`). Un tel onglet
  partage donc le WebContent de son ouvreur tant qu'il ne navigue pas lui-même.
- **iframe** : un cadre d'un autre site est hébergé par un autre WebContent
  (`[LB] OOPIF_REMOTE parent_pid= hote_pid=`, banc `run_ladybird_oopif.sh`).
  Limite amont (cdfe5f8) : aucun canal `postMessage` entre processus
  (`WebContentServer.ipc`) ; un cadre isolé ne communique pas avec son parent.
  Mode non activé par défaut.

Un rendu qui faute est tué par le noyau (`PROCESS_FAULT`) ; le navigateur
l'apprend par SIGCHLD (`ProcessMonitor`) et le remplace (reprise upstream :
nouveau WebContent, page d'erreur, au plus 5 plantages rapprochés).

État de la preuve (7 oct. 2026) :

| Maillon | Correctif | Preuve |
|---|---|---|
| SIGCHLD interrompt le `poll` d'un navigateur au repos | `BOUCHAUD_SIGNAL_INTERROMPT_POLL_V1` | QEMU : `sigchld-multifil-probe`, cas `attente` (EINTR) |
| le signal va au fil principal, pas à un fil de travail | `BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1` | QEMU : même sonde, cas `boucle` |
| LibCore réveille la boucle qui a enregistré le gestionnaire | `BOUCHAUD_SIGNAL_BOUCLE_V1` | compilation seulement |
| reprise réelle d'un WebContent planté dans Ladybird | — | **NON PROUVÉE** : banc `run_ladybird_crash_rendu.sh` (job robustesse) en attente |

Tant que la dernière ligne n'est pas verte, la reprise après plantage d'un
rendu n'est pas une garantie de sécurité de ce document.

## Ce qui n'est pas couvert

- profils distincts par rôle de rendu (`BrowserWorker`, `BrowserImageDecoder`,
  `BrowserCompositor`, `BrowserMedia`) : un seul profil `BrowserContent`
  aujourd'hui ;
- Trigkey physique : non testé ici.
