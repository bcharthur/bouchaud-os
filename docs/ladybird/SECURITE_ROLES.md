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

## Ce qui n'est pas couvert

- iframe isolation (site isolation au niveau des cadres) : non activée ;
- profils distincts par rôle de rendu (`BrowserWorker`, `BrowserImageDecoder`,
  `BrowserCompositor`, `BrowserMedia`) : un seul profil `BrowserContent`
  aujourd'hui ;
- Trigkey physique : non testé ici.
