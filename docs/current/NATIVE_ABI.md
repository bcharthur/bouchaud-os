# ABI native Bouchaud — v1.1 (référence)

**Source de vérité du contrat :** `src/kernel/native/abi/{types,numbers,wire}.rs`.
**Description C unique :** `userland/libs/bouchaud-sdk/include/bouchaud/abi.h` (+ wrappers `sdk.h`).
**Gardes :** `tools/verifie-contrat-abi-v1.py` (contrat gelé `tools/architecture/abi-v1-contract.json` : aucun numéro, droit, erreur ou struct existant ne bouge) et `tools/ci/verifie_sdk_abi.py` (le SDK décrit exactement le noyau, testé par mutation). Les deux tournent en CI Fast.
**Preuve d'exécution :** `tools/ci/run_native_ipc_probe.sh` et `tools/ci/run_native_services.sh` (QEMU, ring 3), job `native-ipc-runtime`.

Un programme natif n'inclut que `bouchaud/sdk.h`, définit `int bo_main(const struct bo_start *)` et se construit avec `userland/libs/bouchaud-sdk/build-program.sh` (freestanding, statique, W^X vérifié). Il n'écrit jamais d'appel système brut.

## 1. Version

`bo_version()` rend `major << 16 | minor`. **Même majeur = compatible.** Un mineur n'ajoute que des numéros, des types d'objets, des bits de signal ou des erreurs ; il ne change jamais ce qui existe. Un programme vérifie `major == 1 && minor >= minimum requis`, jamais l'égalité stricte.

| Version | Ajouts |
|---|---|
| 1.0 | handles, canaux (+ envoi atténué), événements, waitsets (poll), régions partagées |
| 1.1 | `PROCESS_SPAWN/INFO/KILL`, objet Process, signal `TERMINATED`, `WAITSET_WAIT`, `TIME_NOW`, erreur `TimedOut` |

## 2. Appels

Espace de noms `0x424F_0000 + n` (disjoint des numéros Linux). Retour : `>= 0` succès, `-BO_E_*` erreur.

| n | Appel | Droits exigés | Remarques |
|---|---|---|---|
| 0x00 | `version` | — | |
| 0x01–0x04 | `handle_close/dup/info/count` | dup: DUP, info: INSPECT | dup ne peut que **réduire** les droits |
| 0x10–0x13 | `channel_create/send/recv/send_attenuated` | send: WRITE, recv: READ ; handles transférés : TRANSFER | max 256 Kio + 16 handles par message, 64 messages / 2 Mio en file |
| 0x20–0x23 | `event_create/signal/reset/query` | signal/reset: SIGNAL, query: WAIT | |
| 0x30–0x33 | `waitset_create/add/remove/poll` | add/remove: WRITE sur le waitset, WAIT sur la cible ; poll: READ | |
| 0x34 | `waitset_wait(ws, out, cap, timeout_ns)` | READ | bloquant, voir §6 |
| 0x40–0x43 | `shm_create/size/read/write` | size: INSPECT, read: READ, write: WRITE | plafond par objet selon profil de sécurité |
| 0x50 | `process_spawn(path, len, handles, n, masks, out)` | TRANSFER sur chaque handle | voir §5 |
| 0x51 | `process_info(proc, out)` | INSPECT | `{pid, state, exit_code}` |
| 0x52 | `process_kill(proc)` | SIGNAL | idempotent |
| 0x60 | `time_now()` | — | ns monotones depuis le boot |

## 3. Objets, handles, droits

- **Handle** : identifiant opaque **local au processus** (slot 32 bits + génération 31 bits ; bit 63 nul). Un handle fermé puis réutilisé change de génération : l'ancien identifiant rend `BadHandle` (pas d'ABA).
- **Objet** : compté par références. Il vit tant qu'un handle, un message en file ou un waitset le référence.
- **Droits** (`READ WRITE SIGNAL MAP DUP TRANSFER INSPECT WAIT`) : attachés au **handle**, pas à l'objet. Ils ne croissent jamais : `dup` et `send_attenuated` intersectent.
- **Types** : Channel (1), Event (2), WaitSet (3), SharedRegion (4), **Process (5)**.
- **DENY par défaut** : un processus neuf n'a **aucun** handle. Il ne reçoit que ce que son lanceur lui passe (§5) ou ce qu'on lui envoie par canal.

## 4. Transfert et propriété

- Un handle envoyé par canal est **copié** (le détenteur garde le sien) avec ses droits ∩ masque. Pour céder, l'émetteur ferme le sien.
- **Interdit (1.1)** : envoyer une extrémité de canal à travers son propre canal (elle ou son pair) → `InvalidArgument`. Sinon la file posséderait l'extrémité qui possède la file : fuite définitive, `PEER_CLOSED` jamais levé.
- **Interdit (1.1)** : ajouter un waitset dans un waitset → `WrongType`. Un waitset qui se surveillait faisait **paniquer le noyau** depuis ring 3 (reproduit : `userland/tests/native-cycles-probe.c`).
- Un processus sans `IPC_TRANSFER` (profils de rendu) ne transfère rien, ni par canal ni par spawn.

## 5. Processus

`process_spawn` lance `path` (absolu) comme **fils réel** de l'appelant :

- parent = appelant, uid/gid = ceux de l'appelant (jamais ceux de la session) ;
- porte d'exécution de l'appelant (`execution::authorize_node`) : un rôle sandboxé ne lance rien ;
- politique de sécurité héritée (`policy::inherit`, transition d'exec, NNP monotone) **avant** que la première tâche du fils soit ordonnançable ;
- jusqu'à 8 handles bootstrap, chacun atténué à son masque, installés avant la première instruction et nommés par `BO_BOOTSTRAP=<id>,<id>` (le runtime SDK les place dans `start->bootstrap[]`, dans l'ordre donné) ;
- résultat : `{pid, process}`, `process` portant `SIGNAL|DUP|TRANSFER|INSPECT|WAIT`.

**Mort.** À la mort (sortie volontaire ou tuée), dans cet ordre : descripteurs fermés, **table de handles natifs libérée** (les pairs voient `PEER_CLOSED`), `TERMINATED` levé sur les objets Process, parent notifié. Garantie exploitable : *quand `TERMINATED` est visible, les canaux du mort sont déjà fermés.* La dernière fermeture d'un handle Process sur un fils terminé récolte son zombie.

## 6. Attente et concurrence

- `waitset_poll` : instantané non bloquant des objets dont un signal est levé.
- `waitset_wait` : **level-triggered**, rend dès qu'un signal est levé, `TimedOut` à l'échéance, `0` si un signal POSIX destiné au fil interrompt l'attente. Réveil par la source d'activité native (`src/kernel/native/activite.rs`, `WaitSource` existant) : le ticket est pris **avant** le poll, chaque montée de signal (envoi, fermeture d'extrémité, événement, mort de processus) avance la génération → aucun réveil perdu, aucun verrou tenu en dormant, aucun réveil périodique d'un service inactif. Une tranche de 20 ms ne sert qu'à relire signaux POSIX / état zombie. Coût connu : une montée de signal réveille tous les attendants natifs (qui re-pollent) ; des files par objet pourront le remplacer **sans changer l'ABI**.
- Mesures (TCG, `run_native_services.sh`, RTT ping/pong 200 échantillons) : SMP4 p50 ≈ 0,1 ms ; SMP8 p50 66 µs / p99 0,48 ms ; **SMP1 p50 ≈ 4 ms** = un quantum UP (4 ms) avec 10–11 tâches noyau prêtes — attribution (contention de file vs élection retardée) **non établie**, point ouvert de latence interactive P13.
- Tous les appels sont sûrs en SMP ; chaque objet a son propre verrou, aucun n'utilise le gros verrou noyau. Les destructeurs d'objets ne s'exécutent jamais sous le verrou d'une table de handles.

## 7. Erreurs

`InvalidCall 1, InvalidArgument 2, BadHandle 3, WrongType 4, AccessDenied 5, WouldBlock 6, BufferTooSmall 7, QueueFull 8, PeerClosed 9, NoSpace 10, Fault 11, TooLarge 12, NotFound 13, TimedOut 14`.
Un appel en erreur ne laisse **aucun effet partiel** : ni handle installé, ni message consommé, ni processus lancé (les arguments sont lus et validés avant toute mutation ; les échecs tardifs sont annulés).

## 8. Limites connues (v1.1)

- Pas de `mmap` des régions partagées (lecture/écriture par appel ; le droit MAP est réservé).
- Réveil global (tous les attendants natifs re-pollent) ; latence SMP1 non attribuée (§6).
- `argv` du fils = `[path]` seulement ; la configuration passe par le canal bootstrap.
- Console et sortie passent encore par `write(1)` / `exit_group` (isolés dans le runtime SDK).
- Preuves : TCG et KVM hébergés (CI) ; **PHYSICAL non démontré**.
