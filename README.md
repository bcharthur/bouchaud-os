# Bouchaud OS

Système d'exploitation **bare-metal écrit from scratch en Rust** (`no_std`,
x86_64), qui fait tourner le navigateur **Ladybird natif** — multi-processus,
confiné par le noyau — sans Linux ni Windows à l'exécution.

> **EXPÉRIMENTAL.** Rien ici n'est un produit. Chaque ligne « DONE » du
> tableau ci-dessous renvoie à une preuve rejouable ; ce qui n'a pas de
> preuve est écrit « NOT TESTED », jamais « OK ».

## 1. État — tableau de bord

| | |
|---|---|
| Date du relevé | 2026-10-07 |
| Branche | `claude/ladybird-observability-performance` |
| HEAD des preuves | voir la ligne « Dernières preuves » de chaque lot ; le HEAD courant est `git rev-parse HEAD` |
| Machine de référence | **TRIGKEY Speed S5** — AMD Ryzen 7 5700U, 16 CPU logiques, NVMe, RTL8168, UEFI depuis clé USB |
| Banc d'exécution | QEMU x86_64 (TCG et KVM), `-smp 4` |
| Ladybird épinglé | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` ([third_party/UPSTREAM.md](third_party/UPSTREAM.md)) |
| Statut | **EXPÉRIMENTAL** — convergence Ladybird en cours, **Trigkey NOT TESTED** sur ce HEAD |

### Les lots P1–P13

Niveaux de preuve : **STATIC** (garde-fou sur le code), **HOST** (test exécuté
sur l'hôte), **QEMU** (comportement observé dans une exécution réelle),
**PHYSICAL** (observé sur la Trigkey). États : **DONE**, **PARTIAL**,
**BLOCKED**, **NOT TESTED**.

| Lot | Sujet | État | Niveau | Preuve (marqueur — banc) | Ce qui manque |
|---|---|---|---|---|---|
| P1 | UI/Bouchaud, BrowserHost | DONE | QEMU | `BOUCHAUD_UI_V1_READY`, `M11_GUI_HANDSHAKE_OK` — `run_ladybird_browser_host.sh` | Trigkey |
| P2 | Compositor → gestionnaire de fenêtres | DONE | QEMU | `BOUCHAUD_UI_FIRST_FRAME`, un seul Compositor sur 10 min — endurance | Trigkey |
| P3 | Dégât (damage) | DONE | QEMU | `[LB:PERF] trames_partielles=` — smoke | Trigkey |
| P4 | Défilement asynchrone | DONE | QEMU | `HOST_SCROLL_CHAINE OK` — smoke | Trigkey |
| P5 | Web Workers | DONE | QEMU | `HOST_WORKER_BATTERIE OK 10/10`, `LADYBIRD_WORKER_CYCLE_OK` (run 37654172489) ; endurance : WebWorker 29 créés / 29 récoltés | Trigkey |
| P6 | Bac à sable | PARTIAL | QEMU + HOST | `MATRICE_ROLES_OK`, `[LB:SANDBOX] service=WebContent role=rendu` ; course d'héritage fork/exec corrigée (`7649056a`, tests hôte 16/16) | prouver en QEMU l'absence de `NNP_ABSENT` sur l'endurance |
| P7 | Persistance (profil, SQL, cache HTTP) | DONE | QEMU | `LADYBIRD_CACHE_REDEMARRAGE_OK`, `PERSIST_CACHE_OK` | Trigkey |
| P8 | Audio | PARTIAL | QEMU | `HOST_AUDIO_CHAINE OK`, `OSS_HORLOGE_CHAINE_OK` (AC'97 de QEMU) | **Trigkey HDA : BLOCKED** — aucun pilote HDA |
| P9 | Isolation de site / OOPIF | PARTIAL | QEMU | `LADYBIRD_OOPIF_OK`, `LADYBIRD_CRASH_RENDU_OK` | `postMessage` entre processus : limite amont (0/3) |
| P10 | GPU | BLOCKED | — | aucun backend GPU : rendu CPU + scanout linéaire | pilote 3D (aucun faux backend) |
| P11 | WPT | DONE | QEMU | `HOST_WPT_FIN`, `LADYBIRD_WPT_OK` : 50 fichiers, 4632/4641 (Linux 4628/4641), égaux 49, mieux 1, moins 0, échéances 0 | élargir le corpus |
| P12 | Documentation | DONE | STATIC | ce README ; `tools/verifie-readme-commandes.py` | — |
| P13 | Convergence | PARTIAL | QEMU | verdict `BOUCHAUD_LADYBIRD_CONVERGENCE_OK` non atteint | endurance (≥ 60 cycles, aucun cadre > 30 s) ; banc cache/SQL (panique `/persist` corrigée en `d75d2b3b`, à rejouer) |

**P18 (lot historique)** : `/proc/stat`, `/proc/self/stat`, `/proc/<pid>/stat`
dynamiques, verrouillés par garde-fou (STATIC) et exercés par
`COMPTA_STRESS_OK` (QEMU).

L'état mesuré élément par élément (34 lignes, deux indicateurs séparés) est
**généré** dans [docs/ladybird/INTEGRATION_STATUS.md](docs/ladybird/INTEGRATION_STATUS.md).

## 2. Ladybird dans Bouchaud OS

```text
 ┌──────────────────────── ring 3 ─────────────────────────────────────────┐
 │  BouchaudBrowserHost (UI/Bouchaud : onglets, barre, chrome)  BrowserBroker
 │      │ IPC (sockets Unix)                                                │
 │      ├── WebContent ×N (un par site)       ── BrowserContent (confiné)   │
 │      ├── WebWorker ×N                      ── BrowserContent             │
 │      ├── ImageDecoder                      ── BrowserContent             │
 │      ├── RequestServer (HTTP/TLS/cache)    ── BrowserNetwork             │
 │      └── Compositor ──trames──► surface du WM (FrameReady + dégât)       │
 ├──────────────────────── noyau Bouchaud ─────────────────────────────────┤
 │  ABI Linux (glibc statique) · ordonnanceur SMP · mémoire · ramfs/persist │
 │  profils de sécurité par image (no_new_privs d'office) · réseau TCP/TLS  │
 │  signaux POSIX (masque par fil) · ATA DMA · AC'97 · e1000 / RTL8168      │
 └──────────────────────────────────────────────────────────────────────────┘
```

| Fonction | Où | Preuve |
|---|---|---|
| Fenêtre, onglets, barre d'adresse | `UI/Bouchaud` (BrowserHost), hors WebContent | `BOUCHAUD_UI_WEBCONTENT_CHROME 0` |
| Rendu d'une page | WebContent → Compositor → WM | `BOUCHAUD_UI_FIRST_FRAME` |
| Réseau, TLS, cache disque | RequestServer | `[LB:CACHE] disque=oui` ; HTTPS : PHYSICAL au 2026-09-18 seulement |
| Profil, cookies, localStorage | `/persist/ladybird` (XDG) | `LADYBIRD_CACHE_REDEMARRAGE_OK` |
| Confinement | noyau, par image (`src/kernel/security/profile.rs`) ; chaque service le vérifie | `[LB:SANDBOX]`, `MATRICE_ROLES_OK` |
| Mort d'un rendu | SIGCHLD → ProcessMonitor → page d'erreur, autres onglets vivants | `LADYBIRD_CRASH_RENDU_OK` |
| Audio | WebContent → `/dev/dsp` → AC'97 | `HOST_AUDIO_CHAINE OK` (QEMU seulement) |
| Isolation de cadres | WebContent par site, cadre distant | `LADYBIRD_OOPIF_OK` |

Détails : [UI_BOUCHAUD.md](docs/ladybird/UI_BOUCHAUD.md),
[SECURITE_ROLES.md](docs/ladybird/SECURITE_ROLES.md),
[MASTER_PLAN.md](docs/ladybird/MASTER_PLAN.md).

## 3. Démarrage rapide

Prérequis : Rust/rustup (toolchain du dépôt), QEMU, `cargo install bootimage`.

**Windows (PowerShell), bureau + navigateur sous QEMU :**

```powershell
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os
.\run.ps1
.\run.ps1 -Accel tcg -CpuCount 4 -RamMiB 8192
.\check.ps1
```

`run.ps1` télécharge l'artefact Ladybird du dernier run vert de la branche
(`-LadybirdRunId` pour en choisir un, `-RefreshLadybird` pour le reprendre).

**Linux, noyau et bancs :**

```bash
cargo bootimage
tools/test.sh
tools/ci/run_os_primitives.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_architecture_guards.sh
python3 tools/verifie-readme-commandes.py
```

`BO_QEMU_KVM=1` devant `run_os_primitives.sh` rejoue les mêmes sondes sous
KVM (`/dev/kvm` requis).

## 4. Image Trigkey (clé USB)

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1
.\tools\reference\run-trigkey-single-usb-qemu.ps1 -Accel tcg -SkipBuild
```

Le script refuse un arbre modifié et produit dans `target\reference\` trois
fichiers **à garder ensemble** : l'image `.img`, le noyau non strippé
`.img.kernel.elf`, et le manifeste `.img.manifeste.json` (commit, SHA256).
La seconde commande démarre exactement cette image sous QEMU avant tout flash.

### Flasher avec Rufus

1. Rufus → périphérique : **la clé USB**. Vérifier deux fois : **ne jamais
   choisir le NVMe interne**, Rufus l'effacerait.
2. Sélection : le fichier `.img`, puis **mode DD** (pas ISO).
3. Sur la Trigkey : Secure Boot **désactivé**, Boot Override → la clé UEFI.

> **Avertissement.** Une image Trigkey n'est **pas** validée physiquement par
> la CI. L'AC'97 de QEMU n'est pas le HDA de la Trigkey (BLOCKED, pas de
> pilote). Un essai physique se rapporte avec le manifeste de l'image, sans
> quoi aucune adresse de la blackbox n'est symbolisable.

Après un essai : `python3 tools/reference/extract-blackbox.py --image <img> --output <dossier>`,
puis `python3 tools/reference/symbolise-blackbox.py --manifeste <manifeste> <RIP>`.
Bundle de debug distant : `.\tools\remote\collect-ladybird-debug.ps1`.

## 5. Marqueurs — aide-mémoire

| Marqueur | Banc / source | Veut dire |
|---|---|---|
| `M11_GUI_HANDSHAKE_OK` | smoke navigateur | BrowserHost ↔ bureau établi |
| `BOUCHAUD_UI_V1_READY` / `BOUCHAUD_UI_FIRST_FRAME` | smoke | UI prête / première trame Compositor présentée |
| `LADYBIRD_ARTIFACT_MANIFEST_OK` | CI | l'artefact Ladybird correspond au HEAD et au SHA épinglé |
| `LADYBIRD_CRASH_RENDU_OK` | robustesse | un rendu meurt, les autres onglets et le Compositor vivent |
| `LADYBIRD_OOPIF_OK` | robustesse | un cadre d'un autre site vit dans un autre WebContent |
| `LADYBIRD_WORKER_CYCLE_OK` | robustesse | workers : crash, navigation, sortie, recolte |
| `HOST_WPT_FIN` + `LADYBIRD_WPT_OK` | wpt | corpus fini, aucune régression contre Linux |
| `LADYBIRD_SITES_OK` | sites réels | pages réelles chargées et peintes |
| `LADYBIRD_ENDURANCE_OK` | endurance 10 min | cycles, cadres, workers, aucun crash |
| `LADYBIRD_CACHE_REDEMARRAGE_OK` | persistance | cookies, localStorage, cache relus après redémarrage |
| `BOUCHAUD_LADYBIRD_CONVERGENCE_OK` | verdict de convergence | tous les bancs ci-dessus, même HEAD |
| `OS_PRIMITIVES_OK` | os-primitives | sondes POSIX (verrous, WAL, disque, signaux, comptabilité CPU, mtime) |
| `SIGCHLD_MULTIFIL_OK` / `SIGMASQUE_FIL_OK` | os-primitives | SIGCHLD livré et récolté / masque des signaux par fil |
| `COMPTA_STRESS_OK` | os-primitives | comptabilité CPU cohérente sous migrations (seqlock) |
| `BOUCHAUD_TSC_CONTROLE` | tout démarrage | fréquence du TSC, sa source, contrôle par les ticks PIT |
| `KERNEL PANIC`, `PROCESS_FAULT`, `[LB:SANDBOX] ECHEC`, `NNP_ABSENT` | tout banc | à lire en premier |

## 6. Arbre de diagnostic

1. **La machine ne démarre pas / s'arrête tôt** → dernier repère du journal
   série ; `KERNEL PANIC` ? Les bancs impriment en fin d'échec les adresses
   noyau symbolisées (`tools/ci/symbolise_noyau.py`). Sur Trigkey :
   blackbox → `extract-blackbox.py` → `symbolise-blackbox.py`.
2. **Le bureau vient, pas le navigateur** → `M11_GUI_HANDSHAKE_OK` absent :
   vérifier `LADYBIRD_ARTIFACT_MANIFEST_OK` (artefact du bon HEAD), puis
   `[LB:SANDBOX] ECHEC` (un service a refusé de tourner non confiné).
3. **Une page ne s'affiche pas** → `BOUCHAUD_UI_FIRST_FRAME` ? Sinon
   `PROCESS_FAULT` (faute d'un WebContent, symbolisée par
   `tools/ci/symbolise_fautes.py`), puis le réseau (`[LB:CACHE]`, TLS).
4. **Un onglet mort ne revient pas** → `[LB] SIGCHLD_RECU` / `SIGCHLD_FILS`
   présents ? Sinon problème de signaux noyau : rejouer `sigchld-multifil-probe`
   et `sigmasque-fil-probe` (os-primitives).
5. **Lenteur** → `[PROC-STAT]` (CPU par processus), `ATA_CONTROLEUR`
   (attente disque, `lots_dma`, `lots_dma_ecrits`), `BOUCHAUD_TSC_CONTROLE`
   (une horloge fausse fausse toutes les durées).
6. **Sous KVM seulement** → `BOUCHAUD_TSC_CONTROLE` : `source=` dit d'où
   vient la fréquence retenue (`pit2`, `cpuid15`, `hyperviseur`…) ; un
   `rapport_pour_mille=` loin de 1000 mesure les **ticks d'IRQ0** du
   démarrage (KVM rattrape les ticks en souffrance), pas l'horloge monotone,
   qui ne dépend plus d'eux. Les délais comptés en ticks se lisent dans
   `[PROC-STAT] ticks_ms= mono_ms=`. `replis_apic=` élevé : identité du
   cœur recalculée par CPUID (coûteux sous KVM imbriqué).

## 7. Sources de vérité

| Question | Source |
|---|---|
| Qu'est-ce qui marche, à quel niveau ? | [docs/ladybird/INTEGRATION_STATUS.md](docs/ladybird/INTEGRATION_STATUS.md) — généré par `python3 tools/ladybird/mesure-integration.py --ecris`, vérifié par `tools/verifie-integration-ladybird.py` |
| Les verdicts d'un run | les journaux de jobs `ladybird-native-browser` et `os-primitives` (marqueurs §5) |
| L'artefact Ladybird | `BOUCHAUD_ARTIFACT_MANIFEST.json` (HEAD Bouchaud + SHA Ladybird) |
| Le noyau d'une image Trigkey | `.img.manifeste.json` + `.img.kernel.elf` |
| Les contrats d'architecture | `tools/verifie-*.py` (161 garde-fous, `tools/ci/run_architecture_guards.sh`) |
| L'état général, daté | [STATUS.md](STATUS.md) |

## 8. Prochaines priorités

1. **Endurance (P13)** : 29 cycles en 10 min sous TCG pour ≥ 60 exigés,
   6 cadres au-delà de 30 s ; profiler par étape avant de toucher au budget.
2. **Compositor** : RSS de 1 à 88 Mio en 10 min (pente 2,9 Mio/min) —
   fuite ou cache non borné, à attribuer.
3. **P6** : confirmer en QEMU que plus aucun WebWorker ne lit `no_new_privs=0` (`NNP_ABSENT`).
4. **KVM** : confirmer la fréquence du TSC (`BOUCHAUD_TSC_CONTROLE`) et le
   disque en DMA, puis comparer TCG/KVM (cycles, latence, CPU, RSS).
5. **Trigkey** : image du HEAD convergé, essai physique, HDA (pilote absent).

## 9. Journal des corrections récentes (noyau)

| Commit | Correction | Preuve |
|---|---|---|
| `7649056a` | un fils déjà exécuté n'hérite plus du profil du **courtier** (course posix_spawn : un WebWorker tournait avec les droits du BrowserHost) | `NNP_ABSENT … profil=BrowserBroker` (3/29) ; tests hôte 16/16 |
| `d75d2b3b` | instantané de `/persist` sous une seule prise du RAMFS (panique noyau pendant un `fsync`) | `persist-course-probe` : panique reproduite sans, `PERSIST_COURSE_OK` avec |
| `6fdd5421` | une seule horloge murale (dates de fichier = `clock_gettime`) | KVM : `MTIME_STABLE` échouait |
| `eea4b8a7` | masque des signaux **par fil** (il était par processus : SIGCHLD restait bloqué après la première mort d'un fils) | `sigmasque-fil-probe` : avant 0/20 récoltes, après 20/20 ; `LADYBIRD_WORKER_CYCLE_OK` |
| `fe049123` | écritures ATA en DMA bus-master | `disque-probe` TCG 53 s → 11 s |
| `b94542b7` | fréquence du TSC publiée par l'hyperviseur | KVM : horloge 4,5× trop rapide avant ; contrôle publié |
| `3d5f5f0e`, `e2528579` | identité du cœur sans CPUID par interruption | KVM : plus de blocage au démarrage |
| `2de74c38` | `stat` rend la vraie date de modification | WPT : plus de faute dans `FcValueCanonicalize` |
| `65fb2b39` | comptabilité CPU par seqlock | `COMPTA_STRESS_OK` |


## 10. Organisation du dépôt

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

## 11. Roadmap multiplateforme

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

## 12. Principes de portabilité

- `arch` n'est pas `platform` : AArch64 ne signifie pas Raspberry Pi ;
- un driver PCI n'est pas intrinsèquement x86 ;
- le noyau générique ne doit pas importer de matériel concret ;
- Linux est une personnalité de compatibilité ;
- QEMU `virt` est le banc de bring-up ARM avant le vrai Raspberry Pi ;
- le backend x86_64 doit rester vert pendant chaque étape du portage.

## 13. Documentation

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

## 14. Matériel de référence : TRIGKEY

Le banc physique est un **TRIGKEY Speed S5** (Ryzen 7, 16 CPU logiques, NVMe
interne, RTL8168, boot UEFI depuis une clé USB) — AMD Ryzen 7 5700U.

Ce que la machine a réellement exercé, et l'état de chaque chemin, est tenu à
jour dans `docs/TRIGKEY_AUDIT_2026-09-15.md` — avec, pour chaque verdict, le
chiffre du relevé physique qui le prouve. Un chemin qui a réussi **une fois**
n'y est jamais présenté comme stable.

La procédure d'image et de flash est dans `tools/reference/IMAGE-TRIGKEY.ps1`,
qui refuse de construire sur un arbre modifié et publie le commit, le SHA256 de
l'image et le manifeste de symbolisation — sans lesquels aucun RIP relevé par
la blackbox ne veut dire quoi que ce soit.

## 15. Licence

MIT OR Apache-2.0. Voir `LICENSE`, `LICENSE-MIT` et les notices tierces.
