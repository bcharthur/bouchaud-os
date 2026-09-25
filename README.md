# Bouchaud OS

Bouchaud OS est un système d'exploitation **bare-metal écrit from scratch en Rust**.
Il ne repose ni sur Linux ni sur Windows à l'exécution. La cible réellement
exécutée aujourd'hui est **x86_64 sous QEMU** ; le dépôt est désormais structuré
pour accueillir **AArch64**, d'abord sous QEMU `virt`, puis sur **Raspberry Pi 4**.

> La fondation AArch64 présente dans le dépôt est une architecture de portage,
> pas encore un backend bootable. Le chemin x86_64 reste la référence fonctionnelle.

## Vision technique

```text
Applications / Ladybird
          │
          ▼
Userland et services
          │  syscalls / IPC
          ▼
Compatibilité ABI (Linux aujourd'hui, Bouchaud native à terme)
          │
          ▼
┌─────────────────────────────────────────────┐
│                 Kernel core                 │
│ mémoire · process · scheduler · IPC · FS    │
│ objets · sync · temps · réseau              │
└───────────────────┬─────────────────────────┘
                    │
            Driver contracts
                    │
        ┌───────────┴───────────┐
        ▼                       ▼
     Drivers                 Platform
 e1000/ATA/BGA/...       PC / QEMU / Pi
        │                       │
        └───────────┬───────────┘
                    ▼
              Architecture
             x86_64 / AArch64
                    │
                    ▼
                 Hardware
```

La règle de dépendance cible est simple : **le cœur générique ne doit pas connaître un
CPU ou un périphérique concret**. `x86_64`, `AArch64`, `e1000`, VGA, ATA ou un
Raspberry Pi sont des backends, pas des concepts du kernel. La structure existe,
mais la suppression de toutes les dépendances historiques reste progressive.

## État actuel

> **Références vivantes :** [Current status](STATUS.md) distingue les éléments
> prouvés, implémentés, en cours et planifiés. Le périmètre de la première
> release est défini dans [Bouchaud OS 0.1](docs/BOUCHAUD_OS_0_1.md).

- noyau Rust `no_std`, mémoire virtuelle, ELF, ring 3 et ABI Linux-compatible ;
- processus, threads et ordonnanceur SMP ; Gate0 a validé trois boots QEMU
  SMP4, sans généraliser cette preuve à SMP8 ou au matériel physique ;
- pile réseau, TCP/IP, DNS et TLS ;
- framebuffer/GUI historique et entrées clavier/souris ;
- intégration de WebContent/Ladybird et de plusieurs services ; le frontend
  Ladybird complet, sa sandbox et la compatibilité Web générale restent à faire ;
- x86_64/QEMU : cible fonctionnelle ;
- AArch64/QEMU `virt` : structure créée, bring-up à faire ;
- Raspberry Pi 4 : cible matérielle suivante ;
- Raspberry Pi 5 : hors du premier port, volontairement.

Le BKL historique et plusieurs structures issues de la première architecture
restent des dettes connues. La restructuration multiplateforme ne prétend pas les
masquer : elle crée les frontières nécessaires pour les supprimer proprement.

## Organisation du dépôt

```text
src/
├── arch/                 # ISA : x86_64, AArch64
├── boot/                 # contrat BootInfo indépendant du chargeur
├── platform/             # PC, QEMU virt, Raspberry Pi
├── drivers/
│   ├── api/              # contrats par classe de device
│   ├── audio/
│   ├── block/
│   ├── display/
│   ├── input/
│   ├── network/
│   ├── serial/
│   └── bus/
├── kernel/
│   ├── memory/
│   ├── process/
│   ├── scheduler/
│   ├── object/
│   ├── sync/
│   ├── syscall/
│   ├── time/
│   └── debug/
├── compat/
│   └── linux/            # personnalité Linux, pas cœur du kernel
├── fs/
├── net/
└── gui/                  # GUI historique, migration userland progressive

userland/
├── libs/                 # futur SDK / Graphics / UI
├── services/             # init / compositor / network
└── apps/

targets/                  # cibles rustc bare-metal

tools/
├── ci/                   # barrière locale, scénarios QEMU, build
├── reference/            # image Trigkey, blackbox, symbolisation
├── ladybird/             # pipeline navigateur
├── dev/ perf/ gui/ net/ fs/ platform/   # suites hôte et gardes ciblés
├── verifie-*.py          # garde-fous d'architecture (découverte auto)
└── historique/           # scripts de lot d'une époque, conservés non actifs

docs/
├── *.md                  # documentation vivante
└── historique/           # notes, manifestes et correctifs de lot archivés
```

### La racine reste courte, et c'est une règle

Cent quarante-sept fichiers vivaient à la racine, dont soixante-cinq notes de
lot (`BOUCHAUD-V13.2-COMPILE-FIX.md`, `*-MANIFEST.json`, `VERIFY-*.ps1`…)
référencées par rien. Elles racontent l'histoire du projet et méritaient d'être
gardées — mais pas à l'entrée.

Elles ont été **déplacées, pas supprimées** : `docs/historique/` pour les notes
et manifestes, `tools/historique/` pour les scripts de lot. Un `git log
--follow` les retrouve, et la racine tient désormais en treize fichiers.

Un garde-fou dont le contrat pointait vers un de ces fichiers a été **reciblé**
sur sa nouvelle adresse, pas désactivé : le contrat ne change pas, seule
l'adresse bouge.

Pendant la transition, certains anciens chemins Rust (`kernel::fd`,
`drivers::e1000`, etc.) restent valides via des façades `#[path]`. C'est
volontaire : **déplacer les sources et changer leur comportement dans le même
commit rendrait les régressions impossibles à isoler**.

## Construire et lancer

Prérequis : Rust/rustup, QEMU et `cargo install bootimage`.

```powershell
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os
.\run.ps1
```

Pour compiler sans lancer QEMU :

```powershell
.\check.ps1
```

La cible x86_64 personnalisée vit maintenant dans
`targets/x86_64-bouchaud_os.json` et `.cargo/config.toml` la sélectionne par
défaut.

## Navigateur

Le travail navigateur est centré sur l'intégration native de Ladybird :
WebContent, RequestServer, ImageDecoder, Compositor, WebWorker et le host
Bouchaud sont empaquetés dans le userland/scénario de développement. Le but est
que le navigateur soit un citoyen normal de Bouchaud OS et non une démonstration
spéciale liée au noyau.

```powershell
.\run.ps1
```

Le bureau démarre, puis le navigateur se lance depuis son entrée graphique. Les
modes `-LadybirdM8` et `-LadybirdM9Test` restent des scénarios de régression.

### Observabilité du démarrage à froid

Le navigateur fonctionne ; il démarre lentement. Le chantier en cours sépare
ces deux questions et refuse de les confondre.

**Deux statuts CI indépendants.** `ladybird / browser-host smoke` bloque sur la
capacité, `ladybird / performance` bloque sur les budgets, et un échec de l'un
n'est jamais présenté comme un échec de l'autre. Le verdict de performance
était auparavant écrit mais lu par personne : il dépendait d'une variable
qu'aucun workflow ne définissait.

**Capacités vertes**, vérifiées à chaque run : canvas, images 11/11 codecs,
iframes, JS 17/17, WebWorker HTTP et blob, et la mire réellement retrouvée
dans une trame composée — capture prise pendant que QEMU vit, corrélée à un
numéro de trame, et non plus après sa mort.

**Ce que la mesure a établi, et ce qu'elle a réfuté.** Le premier WebWorker
coûte environ 130 s là où les suivants coûtent 1,5 à 9 s. Les bornes posées
sur le chemin noyau ont successivement innocenté :

| poste | mesure | verdict |
|---|---|---|
| `fork` | 28 ms | hors de cause |
| `fork` → `execve` | 8 ms | hors de cause |
| `execve` | 22 ms | hors de cause |
| `exec` → `main` | ~94 s | **le poste réel** |
| fautes fichier dans ce segment | ~10 s | 11 %, pas la cause |

Le segment de 43,4 s longtemps attribué à l'ordonnanceur n'existait pas : il
était mal borné.

**Cause confirmée, et corrigée : le balayage de secours du cache de pages.**
À chaque défaut de cache, dès que la table atteint son plafond de 16 384
pages, le noyau parcourait **toute** la table en prenant le verrou d'état de
chaque entrée, sous le verrou global — pour n'y rien trouver. Mesuré sous
Ladybird : 22 773 balayages, 632 millions d'entrées parcourues, **277 s**,
soit 57 % du temps noyau du run. Le modèle « un balayage par défaut de cache
une fois la table pleine » se vérifie à 0,0 % près sur banc local et 1,8 % sur
Ladybird. La correction sort quand le compteur d'entrées récupérables vaut
zéro — le balayage est alors garanti de ne rien trouver. Banc à 80 Mio, trois
exécutions par bras : durée **−58 %**, temps noyau **−62 %**, avec des témoins
identiques (`entrees=20480`, `recuperees=0`) qui prouvent qu'aucune
récupération n'a été perdue.

**Remesuré sous Ladybird (run #358), et le gain dépasse le banc.**
`HOST_WORKER_BLOB_PERF_FIRST` passe de **126 670 ms à 8 996 ms** — un facteur
14, sous un budget de 30 000 ms qui n'a pas bougé. Le temps noyau du run tombe
de 490 240 ms à 103 500 ms ; le premier WebWorker, de ~113 s à 8,985 s de
`sys_ms`. Deux témoins restent **identiques au run précédent** —
`candidats_suffisants=12722` et `miss=52534` — donc le chemin rapide
d'éviction a fait exactement le même travail et le cache a manqué exactement
les mêmes pages : seul le balayage a disparu. Ce qui domine maintenant, ce sont
132 s de lectures disque réelles, et c'est la prochaine question.

**Le même commit a figé la machine, et c'était la sonde.** `Integration #256`
s'arrête net après `SESSION_PERE_SORT fils=4`, machine vivante, scénario
bloqué. Le chef de session publie ses sondes sans encombre ; ce sont les
**quatre tâches arrêtées avec lui** qui ne publient jamais leur `PROCESS_EXIT`.
`exit_current` tient `process.lifecycle` pendant la publication des sondes, et
`balayage_temoins()` y prenait `CACHE.lock()` : une arête d'ordre de verrous
sur un chemin de sortie. Ce correctif était juste — la règle est écrite dans le
fichier même — mais il **n'a pas suffi** : `Integration #257` a échoué au même
endroit, et une passe locale verte ne prouve rien contre une course.

**Un réveil perdu dans `wait4`, antérieur — et ma correction l'a aggravé.**
`[SCHED-RESUME] pretes=2 ... au_repos=4 en_file=0` — deux tâches prêtes, quatre
cœurs au repos, file vide : le shell n'a jamais été remis en file. `sys_wait4`
cherchait les fils zombies, n'en trouvait aucun, **puis** se déclarait en
attente. Un fils mourant dans cet intervalle trouvait `waiting_for_child`
encore à faux ; son réveil tombait dans le vide et le parent s'endormait pour
toujours. Les sondes de sortie de processus n'ont fait que déplacer le timing
dans cette fenêtre. Corrigé en posant `Blocked` **avant** le drapeau — sinon le
réveilleur consomme le drapeau puis échoue sur l'état — et en **revérifiant**
après s'être déclaré. **Cette correction a été retirée : elle empirait le
défaut**, déplaçant le blocage du 8ᵉ marqueur au 3ᵉ. La relecture du protocole
établi (`blocage.rs`) est un compteur ; la mienne parcourait tous les processus
en prenant un verrou par processus, tâche déjà marquée bloquée. La course reste
**ouverte et documentée, non corrigée**.

**Ce qui est en périmètre, c'est le déclencheur.** Les trois sondes globales
étaient publiées *dans* le scope de `process.lifecycle` — trois écritures série
sous un verrou de processus. Elles sont désormais publiées après sa fermeture ;
mêmes chiffres, section critique rendue à sa longueur. Deux règles tirées de la
même erreur : une sonde ne prend pas de verrou, et une sonde ne rallonge pas la
section critique qu'elle observe. Reproducteur : sous contention CPU le défaut
sort **3 fois sur 3**, là où une passe non contrainte passait et m'avait fait
conclure trop vite. La règle était déjà
écrite dans le fichier même (« Reporting must stay lock-free »), et le
commentaire au-dessus du site d'appel mettait en garde contre ce geste exact.
Corrigé par un compteur atomique ; vérifié par
`tools/ci/verifie-sondes-sans-verrou.py`, qui refuse toute sonde d'`exit_current`
dont le corps contient `.lock()`.

**La mesure corrigée a inversé la conclusion.** Avec la frontière posée, le
premier WebWorker mesure `user_ms=724` et `sys_ms=113594` : il passe 0,7 s en
espace utilisateur et 113 s dans le noyau. `_dl_relocate_static_pie` s'exécute
en espace utilisateur — l'hypothèse des 405 396 relocations de démarrage est
donc **réfutée par borne supérieure**, et le banc A/B d'édition de liens a été
retiré plutôt que laissé rouge. Ce qui reste à expliquer est net : 113 594 ms
de noyau dont le livre des fautes n'explique que 13 285. Deux candidats
(balayage de secours du cache, chaîne de reprise des fautes) ont été posés puis
réfutés localement en quelques minutes — `appels=0` et `reprises=0`.

**Une conclusion a été retirée, parce que l'instrument était faux.** Ce
tableau portait « dont ~93 s de CPU » et la phrase « le premier worker
n'attend pas, il calcule ». Les deux venaient d'un relevé `user_ms=92250
sys_ms=671`. Or les frontières de comptabilité n'existaient qu'autour des
appels système : le gestionnaire de faute de page n'en avait aucune, et tout
ce qu'il fait — y compris **déclencher et attendre une lecture ATA** —
tombait dans `user_ns`. Mesuré sur banc local, trois exécutions par variante,
la frontière posée déplace 94 % du « temps utilisateur » vers le noyau
(1320 ms → 61 ms côté utilisateur, 14 ms → 1204 ms côté noyau, total
conservé). Rien n'est devenu plus rapide : l'étiquette était fausse. Le
partage réel du segment `exec` → `main` demande un nouveau run, et la piste
des relocations de démarrage perd l'argument qui la soutenait.

Dans la foulée, la variante ET_EXEC qui devait falsifier cette piste s'est
révélée **impossible à lier** : la glibc statique référence des symboles
faibles indéfinis résolus à l'adresse zéro, et un `R_X86_64_PLT32` ne peut pas
porter le déplacement depuis `0x400000000000`. Le micro-binaire qui semblait
la valider était lié en `-nostdlib`. Elle est remplacée par une variante RELR
(`-z pack-relative-relocs`), vérifiée localement : 26 280 octets de table
deviennent 288, l'ASLR est conservée.

**L'outillage de mesure est lui-même sous test.** Décomposition des fautes de
page attribuée par PID et non globalement, avec test de chevauchement de deux
processus ; `acquire` rend son coût à la faute qui l'a payé ; les sous-champs
« dont » ne sont jamais additionnés à leur contenant ; les compteurs globaux
portent `scope=global` pour ne pas se faire passer pour une attribution. Chaque
garde-fou a été mis en échec volontairement avant d'être retenu.

**Ce qui reste ouvert** est documenté dans
[docs/MESURE_DEMARRAGE_A_FROID.md](docs/MESURE_DEMARRAGE_A_FROID.md) :
la décomposition des ~94 s avant `main` avec un partage utilisateur/noyau
désormais honnête, et le coût propre de l'instrumentation à forte charge de
fautes.

## Roadmap multiplateforme

1. **Foundation** — séparation arch/platform/drivers/kernel et compatibilité x86.
2. **Boot contract** — remplacer `bootloader::BootInfo` dans le cœur par
   `boot::BootInfo`.
3. **Arch façade** — supprimer les imports directs `arch::x86_64` du code
   générique.
4. **Driver model** — stabiliser `BlockDevice`, `NetworkDevice`, `DisplayDevice`,
   `InputDevice` et les mécanismes de découverte.
5. **AArch64 QEMU** — UART, exceptions, Generic Timer/GIC, MMU, EL0/SVC, SMP.
6. **Raspberry Pi 4** — Device Tree, UART, framebuffer, stockage, USB/input,
   réseau.
7. **Userland AArch64** — libc/ABI, services et Ladybird recompilés ARM64.
8. **Graphics NG** — compositeur userland + BouchaudGraphics/BouchaudUI.

## Principes de portabilité

- `arch` n'est pas `platform` : AArch64 ne signifie pas Raspberry Pi ;
- un driver PCI n'est pas intrinsèquement x86 ;
- le noyau générique ne doit pas importer de matériel concret ;
- Linux est une personnalité de compatibilité ;
- QEMU `virt` est le banc de bring-up ARM avant le vrai Raspberry Pi ;
- le backend x86_64 doit rester vert pendant chaque étape du portage.

## Documentation

Les documents de référence se trouvent dans `docs/`. La fondation actuelle est
décrite dans `docs/architecture/MULTIPLATFORM_FOUNDATION.md`. L'ancien README
x86-centric a été conservé dans `docs/history/README_PRE_MULTIPLATFORM.md` pour
ne perdre aucune information historique.

Documents utiles :

- `STATUS.md` — statut avec preuves et limites explicites ;
- `docs/BOUCHAUD_OS_0_1.md` — scope et Definition of Done de la version 0.1 ;
- `docs/ARCHITECTURE_DIRECTION.md` — architecture actuelle et cible sans les
  confondre ;
- `docs/ETAT_DES_LIEUX.md` — état réel et preuves ;
- `docs/VISION.md` — direction du système ;
- `docs/ARCHITECTURE.md` — architecture générale ;
- `docs/PORTABILITY_MATRIX.md` — portabilité ;
- `docs/architecture/AARCH64_RASPBERRY.md` — cible ARM/Raspberry ;
- `docs/architecture/MULTIPLATFORM_FOUNDATION.md` — règles de la refonte ;
- `docs/ladybird/MASTER_PLAN.md` — intégration Ladybird.
- `docs/BOUCHAUD_LAB_REMOTE.md` — protocole BRDP et télémétrie de survie ;
- `docs/REMOTE_CONTROL.md` — contrôle distant et procédure TRIGKEY.

## Matériel de référence : TRIGKEY

Le banc physique est un **TRIGKEY Speed S5** (Ryzen 7, 16 CPU logiques, NVMe
interne, RTL8168, boot UEFI depuis une clé USB).

Ce que la machine a réellement exercé, et l'état de chaque chemin, est tenu à
jour dans `docs/TRIGKEY_AUDIT_2026-09-15.md` — avec, pour chaque verdict, le
chiffre du relevé physique qui le prouve. Un chemin qui a réussi **une fois**
n'y est jamais présenté comme stable.

La procédure d'image et de flash est dans `tools/reference/IMAGE-TRIGKEY.ps1`,
qui refuse de construire sur un arbre modifié et publie le commit, le SHA256 de
l'image et le manifeste de symbolisation — sans lesquels aucun RIP relevé par
la blackbox ne veut dire quoi que ce soit.

## Licence

MIT OR Apache-2.0. Voir `LICENSE`, `LICENSE-MIT` et les notices tierces.
