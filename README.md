<div align="center">

# Bouchaud OS

**Souverain. Moderne. Durable.**

Un système d'exploitation natif expérimental, écrit principalement en Rust, avec son propre noyau x86_64.

[![CI Fast](https://github.com/bcharthur/bouchaud-os/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bcharthur/bouchaud-os/actions/workflows/ci.yml)
[![Integration](https://github.com/bcharthur/bouchaud-os/actions/workflows/integration.yml/badge.svg?branch=main)](https://github.com/bcharthur/bouchaud-os/actions/workflows/integration.yml)
[![Reliability V3](https://github.com/bcharthur/bouchaud-os/actions/workflows/reliability-v3.yml/badge.svg?branch=main)](https://github.com/bcharthur/bouchaud-os/actions/workflows/reliability-v3.yml)
[![Native IPC](https://github.com/bcharthur/bouchaud-os/actions/workflows/native-ipc-runtime.yml/badge.svg?branch=main)](https://github.com/bcharthur/bouchaud-os/actions/workflows/native-ipc-runtime.yml)
[![Ladybird](https://github.com/bcharthur/bouchaud-os/actions/workflows/ladybird-native-browser.yml/badge.svg?branch=main)](https://github.com/bcharthur/bouchaud-os/actions/workflows/ladybird-native-browser.yml)

</div>

> **Projet expérimental.** Bouchaud OS n'est pas un remplaçant de Windows, Linux ou macOS. Une fonctionnalité n'est annoncée qu'au niveau de preuve réellement obtenu : `HOST` (tests sur la machine de développement), `TCG` (QEMU émulé), `KVM` (QEMU accéléré) ou `PHYSICAL` (machine réelle). **Aucune campagne physique n'a été menée sur le code actuel** : tout ce qui suit est démontré au mieux sous QEMU.
>
> *English summary:* Bouchaud OS is an experimental, from-scratch x86_64 operating system written mostly in Rust (`no_std`). It has its own kernel, SMP scheduler, virtual memory, native IPC ABI, drivers, desktop and a Ladybird-based browser running in user space. It runs under QEMU; physical hardware validation of the current code has not started.

**Où lire l'état exact :** [`docs/current/status.json`](docs/current/status.json) est la source de vérité datée (SHA de `main`, verdicts, PR ouvertes). Ce README décrit le projet ; il ne recopie ni SHA de qualification ni verdict de CI.

## Sommaire

- [Le projet](#le-projet)
- [L'interface Atelier](#linterface-atelier)
- [Ce qui existe réellement](#ce-qui-existe-réellement)
- [Performances mesurées](#performances-mesurées)
- [Architecture](#architecture)
- [Construire et démarrer](#construire-et-démarrer)
- [Tests et CI](#tests-et-ci)
- [Limitations connues](#limitations-connues)
- [Travaux en cours et feuille de route](#travaux-en-cours-et-feuille-de-route)
- [Organisation du dépôt et contributions](#organisation-du-dépôt-et-contributions)
- [Licences et attributions](#licences-et-attributions)

---

## Le projet

Bouchaud OS est un système d'exploitation **bare-metal** : il démarre directement sur la machine (ou sur QEMU), sans Linux ni Windows en dessous. Le noyau, l'ordonnanceur, la mémoire virtuelle, les pilotes, le réseau, le système de fichiers et le bureau sont développés dans ce dépôt. Une couche de compatibilité Linux/POSIX permet d'y faire tourner des logiciels complexes, dont le moteur web Ladybird. C'est une interface de compatibilité, pas un noyau Linux caché.

**Philosophie**

- **Souverain** : chaque couche, du chargeur au bureau, est comprise, lue et modifiable ici. Les composants tiers sont identifiés, épinglés et attribués.
- **Moderne** : Rust `no_std`, SMP dès le départ, une ABI native à base de handles et d'IPC, des mises à jour atomiques A/B, un bureau conçu comme un produit.
- **Durable** : chaque comportement important a son banc de preuve, rejouable sur le même SHA. Un défaut corrigé laisse un test qui le reproduirait.

**Ambitions, non livrées** : une machine de référence physique pleinement prise en charge (TRIGKEY Speed S5), des services système isolés en espace utilisateur (dont le gestionnaire de fenêtres), un stockage généraliste fiable, le raster GPU, puis des services d'IA locale isolés. Ce sont des **directions** ; leur état réel est donné plus bas.

---

## L'interface Atelier

**Atelier** est le bureau de Bouchaud OS : barre haute, dock, centre de contrôle, fenêtres animées, icônes [Lucide](https://lucide.dev) et fond photographique. La marque est le nom seul (le croissant provisoire a été retiré le 10 novembre 2026) et **aucune fenêtre ne s'ouvre d'elle-même au démarrage** : ni Terminal, ni navigateur.

> **Captures du 10 novembre 2026**, branche `claude/c22-p13-p10-gpu-kernel-0y2ucg`. Ce sont des **captures réelles de l'écran de l'hôte** (QEMU 8.2, `virtio-gpu-gl-pci`, sortie SDL sous Xvfb), en 1280×720, sans retouche. Le contenu des fenêtres DOOM et Mire y est **composé par le GPU de l'hôte** (VIRGL) ; le reste du bureau est peint par le processeur.

<table>
<tr>
<td width="50%"><img src="docs/current/atelier/2026-11-10/bureau.png" alt="Bureau Atelier au démarrage, sans fenêtre ouverte"></td>
<td width="50%"><img src="docs/current/atelier/2026-11-10/doom.png" alt="DOOM composé par le GPU"></td>
</tr>
<tr>
<td><sub><b>Le bureau au démarrage</b> : aucune fenêtre imposée, marque sans croissant.</sub></td>
<td><sub><b>DOOM</b> (Freedoom, processus ring 3) : sa fenêtre est une texture GPU, posée par un <code>BLIT</code> VIRGL.</sub></td>
</tr>
<tr>
<td><img src="docs/current/atelier/2026-11-10/fichiers-doom.png" alt="Fichiers et DOOM côte à côte"></td>
<td><img src="docs/current/atelier/2026-11-10/mire.png" alt="Mire de vérification au pixel"></td>
</tr>
<tr>
<td><sub><b>Fichiers</b> (application native, CPU) sous <b>DOOM</b> (GPU) : la découpe des fenêtres superposées est exacte.</sub></td>
<td><sub><b>Mire</b> : chaque pixel code sa position ; la capture décodée montre 254 934 pixels exacts, 0 faux (banc <code>run_gpu_composition.sh</code>).</sub></td>
</tr>
</table>

Captures plus anciennes (Atelier 2.0, PR #242 à #244) : [`docs/current/atelier/`](docs/current/atelier/).

---

## Ce qui existe réellement

Légende : ✅ démontré à ce niveau · ◐ partiel · — non démontré · 🔀 seulement sur une branche ou une PR ouverte.

La colonne « Physique » désigne un essai sur la machine de référence **avec le code actuel**. Des boots physiques plus anciens figurent dans l'historique, mais sur d'autres SHA : ils ne qualifient pas `main`.

| Composant | Sur `main` | Hôte | QEMU TCG | QEMU KVM | Physique | Remarques |
|---|:---:|:---:|:---:|:---:|:---:|---|
| Noyau x86_64 (démarrage BIOS et UEFI, IDT, APIC, horloges) | ✅ | ◐ | ✅ | ✅ | — | démarrage UEFI via le chargeur A/B (OVMF) |
| Mémoire virtuelle, pagination à la demande, tas compagnon, DMA | ✅ | ◐ | ✅ | ✅ | — | pas de swap |
| Processus, threads, signaux, `wait4`, futex | ✅ | ◐ | ✅ | ✅ | — | anomalie SIGCHLD multi-fils ouverte (voir limites) |
| Ordonnanceur SMP (files par cœur, IPI, préemption) | ✅ | ◐ | ✅ SMP1/2/4 | ✅ SMP1/2/4 | — | SMP8 strict en attente d'un runner dédié |
| ABI native 1.2 et IPC (handles, canaux, waitsets, mémoire partagée) | ✅ | ✅ | ✅ | — | — | contrat gelé et vérifié ; [NATIVE_ABI](docs/current/NATIVE_ABI.md) |
| Compatibilité Linux/POSIX (ELF, `ld.so`, appels système) | ✅ | ◐ | ✅ | ✅ | — | sert à porter Ladybird |
| Pilotes QEMU : NVMe, ATA, xHCI (HID, stockage BOT), e1000, AC'97, BGA, virtio-gpu 2D et 3D (VIRGL), PS/2, UART | ✅ | ◐ | ✅ | ◐ | — | |
| Pilotes du matériel de référence : RTL8111/8168, GOP UEFI, NVMe et xHCI réels | ✅ | — | — | — | — | code présent, **non revalidé physiquement** |
| Réseau : IPv4, ARP, DHCP, DNS, TCP/UDP, TLS, HTTP/1.1 et HTTP/2 | ✅ | ◐ | ✅ | ✅ | — | |
| Bureau et gestionnaire de fenêtres (dans le noyau) | ✅ | ✅ | ✅ | ✅ | — | damage tracking, culling de scène ; fenêtres cachées filtrées par les calques opaques |
| **Compositeur GPU des fenêtres** (VIRGL : textures adossées aux surfaces, `BLIT` GPU) | 🔀 | ✅ | ✅ | ✅ | — | branche du 10/11 ; Ladybird, DOOM et toute application ring 3 ; [P10_GPU_COMPOSITEUR](docs/current/P10_GPU_COMPOSITEUR.md) |
| DOOM (Freedoom) en ring 3 | ✅ | — | ✅ | ✅ | — | même protocole de fenêtre que toute application |
| Atelier Experience 2.0 (design system, dock, centre de contrôle, Fichiers) | ✅ | ✅ | ✅ | — | — | fusionné par les PR #242 à #244 |
| Applications natives : Terminal, Fichiers, Notes, Calculatrice, Rustpad, Journal, Services, Insights, Infos système | ✅ | ◐ | ✅ | — | — | applications du noyau, pas des processus isolés |
| Bouchaud Navigateur (Ladybird, multiprocessus en ring 3) | ✅ | — | ✅ | ✅ | — | WebContent, RequestServer, ImageDecoder, WebWorker, Compositor ; rastérisation Skia **CPU**, composition de la fenêtre par le GPU sous VIRGL |
| Services natifs ring 3 : `serviced`, `healthd`, `updated`, `logd` | ✅ | ◐ | ✅ SMP1/4 | — | — | supervision, santé, mise à jour, journal |
| Atomic Platform A/B : chargeur UEFI, essai, commit de santé, retour arrière, recovery | ✅ | ✅ | ✅ | — | — | manifeste Ed25519 ; **pas de clé de production ni d'anti-rollback** |
| Stockage : RAMFS, `/persist` transactionnel, GPT, FAT32 | ✅ | ✅ | ✅ | — | — | coupure de courant simulée sous TCG ; **pas de VFS disque généraliste** |
| Vidange réelle USB (commit A/B sur clé), liaison volume ↔ support | ✅ | ✅ | ◐ | — | — | PR #246 ; en CI, le commit sur clé est parfois refusé quand l'hôte est lent (voir limites) |
| Diagnostic : boîte noire, profil RIP, BRDP, journal série, console de recovery | ✅ | ◐ | ✅ | ◐ | — | |

Le détail des verdicts (Atomic Platform, P13, anomalies ouvertes) est dans [`status.json`](docs/current/status.json) et [`STATUS_CURRENT.md`](docs/current/STATUS_CURRENT.md).

---

## Performances mesurées

> **Benchmark honnête, à lire avec ses conditions.** Mesures **CI, QEMU KVM** sur un runner GitHub hébergé (invité 2 vCPU, 2 Gio), `virtio-gpu-gl-pci` ; le GL de l'hôte est **llvmpipe** (rendu logiciel de Mesa, le runner n'a pas de GPU). Valeurs = **médianes** des relevés de 30 s (le premier, qui contient le démarrage, est écarté), produites par [`tools/ci/bench_bouchaud.py`](tools/ci/bench_bouchaud.py) à partir du banc [`run_gpu_composition.sh`](tools/ci/run_gpu_composition.sh). SHA `21d7945`, run [42796553984](https://github.com/bcharthur/bouchaud-os/actions/runs/42796553984) (job `gpu-composition / kvm`). Rien ici n'est mesuré sur la machine physique.

| Mesure | Valeur | Conditions |
|---|---:|---|
| Démarrage : noyau → première trame du bureau | 2 709 ms | 5 démarrages, médiane |
| DOOM : lancement → première image | 251 ms | |
| DOOM : trames du bureau par seconde, composition GPU | 70,0 /s | cadence fixée par DOOM, pas par le compositeur |
| DOOM : trames du bureau par seconde, composition CPU | 70,0 /s | idem |
| **Temps CPU de dessin du bureau par trame, GPU** | **23 µs** | fil du bureau, invité |
| **Temps CPU de dessin du bureau par trame, CPU** | **415 µs** | fil du bureau, invité |
| Pixels clients recopiés par le CPU invité, GPU | 21 524 /s | seulement sous le curseur |
| Pixels clients recopiés par le CPU invité, CPU | 17 890 133 /s | toute la fenêtre, à chaque trame |
| Présentation par trame, GPU (VIRGL) | 4 256 µs | attente par interruption : le fil dort, le cœur sert les applications |
| Présentation par trame, CPU (même périphérique VIRGL, transferts 2D) | 3 978 µs | |
| Présentation par trame, virtio-gpu 2D sans VIRGL | 1 248 µs | repli, autre périphérique |
| Fenêtre cachée sous une autre : pixels clients CPU | 406 357 /s | contre 17,9 M/s visible |
| Ladybird réel sous VIRGL : pixels posés par le GPU | 69,3 M / 30 s | run [42784689821](https://github.com/bcharthur/bouchaud-os/actions/runs/42784689821), 0,34 M seulement recopiés par le CPU |

**Ce que ces chiffres disent.** Le compositeur GPU divise par **18** le temps processeur que le bureau passe à dessiner une trame et supprime **99,9 %** des copies de pixels clients : le processeur invité est rendu aux applications (Ladybird, DOOM).

**Ce qu'ils ne disent pas.**
- Ce n'est **pas** une hausse d'images par seconde : DOOM tourne à la même cadence dans les deux modes.
- Sur ce runner, la présentation GPU de bout en bout est **plus lente** que le chemin 2D sans VIRGL (4,3 ms contre 1,2 ms par trame) : le « GPU » de l'hôte est llvmpipe, c'est-à-dire du rendu logiciel. Le gain de présentation d'un vrai GPU hôte n'est **pas mesuré**.
- Le temps de chargement des pages Ladybird, la latence entrée → affichage et tout comportement sur la Trigkey ne sont **pas** dans ce tableau.
- Sous TCG (émulation), les mêmes rapports se retrouvent (dessin ÷14 à ÷16) avec des valeurs absolues environ sept fois plus élevées.

---

## Architecture

```mermaid
flowchart TB
    subgraph boot["Amorçage"]
        uefi["Firmware UEFI ou BIOS"] --> loader["Chargeur A/B (shim UEFI)<br/>choisit le slot, compte les essais"]
    end

    subgraph kernel["Noyau Bouchaud (ring 0, Rust no_std)"]
        direction TB
        core["SMP · ordonnanceur · temps<br/>mémoire virtuelle · tas · DMA"]
        proc["processus · threads · signaux<br/>chargeur ELF · compat Linux/POSIX"]
        abi["ABI native 1.2<br/>handles · canaux · waitsets · mémoire partagée"]
        fs["RAMFS · /persist · GPT · FAT32<br/>couche bloc"]
        net["pile réseau · TLS"]
        drv["pilotes : NVMe · ATA · xHCI · RTL8168 · e1000<br/>AC'97 · virtio-gpu 2D et VIRGL · GOP/BGA"]
        gui["bureau · gestionnaire de fenêtres<br/>applications natives · plan de composition GPU"]
        upd["mise à jour A/B (mécanisme)"]
    end

    subgraph user["Espace utilisateur (ring 3)"]
        services["serviced · healthd · updated · logd"]
        browser["Bouchaud Navigateur (Ladybird)<br/>BrowserHost · WebContent · RequestServer<br/>ImageDecoder · WebWorker · Compositor"]
    end

    loader --> kernel
    services -- "IPC natif" --> abi
    browser -- "appels POSIX" --> proc
    browser -- "surfaces, damage" --> gui
    upd -. "capacité de mise à jour" .- services
```

**Ce qui s'exécute où.** Tout ce qui figure dans le bloc *Noyau* tourne en ring 0. C'est le cas aujourd'hui du bureau, du gestionnaire de fenêtres, des applications natives et de la pile réseau. C'est une dette connue, et l'isolation progressive des services en ring 3 est un chantier ouvert. Le navigateur et les quatre services natifs sont en revanche de vrais processus en ring 3, avec leur propre espace d'adressage.

- **Amorçage.** Il existe deux chemins. L'image BIOS (`bootimage`) est utilisée par la plupart des bancs. L'image UEFI passe par le chargeur A/B, qui lit les métadonnées, choisit un slot, consomme un essai et passe la main au noyau ([UPDATE_RECOVERY](docs/current/UPDATE_RECOVERY.md)).
- **Noyau.** SMP avec APIC local, files d'exécution par cœur et IPI. Mémoire virtuelle avec pagination à la demande. Processus et threads POSIX, et une ABI native à base de handles pour les nouveaux programmes ([NATIVE_ABI](docs/current/NATIVE_ABI.md)).
- **Pilotes.** Ils sont exposés derrière des API de bus et de bloc communes. Les pilotes du matériel de référence (RTL8168, NVMe et xHCI réels) existent, mais attendent leur revalidation physique.
- **Système de fichiers.** Le système de fichiers vivant est en mémoire (RAMFS). `/persist` est une zone de disque écrite par transactions A/B résistantes aux coupures. Il n'y a pas encore de VFS disque généraliste avec cache de pages unifié.
- **Environnement graphique.** Le bureau peint ses propres calques (fond, cadres, dock, curseur) par le processeur, région endommagée par région endommagée, et les présente (GOP, BGA, virtio-gpu 2D). Quand l'hôte propose VIRGL, le **contenu des fenêtres** ring 3 est composé par le GPU : texture adossée aux pages du client, transfert du seul rectangle abîmé, `BLIT` GPU, un flush par trame ([P10_GPU_COMPOSITEUR](docs/current/P10_GPU_COMPOSITEUR.md)).
- **Navigateur.** Ladybird, épinglé et patché, tourne en plusieurs processus ring 3 au-dessus de la compatibilité POSIX. Son Compositor livre des surfaces au gestionnaire de fenêtres ([BOUCHAUD_NAVIGATEUR](docs/current/BOUCHAUD_NAVIGATEUR.md)).
- **Mises à jour et récupération.**
  - `updated` (ring 3, capacité dédiée) installe un slot à partir d'un manifeste signé.
  - `serviced` déclare la santé du système, et le noyau ne valide le slot qu'à ce moment-là.
  - Sans déclaration de santé, le chargeur revient au slot précédent après trois essais.
  - Une console de recovery reste accessible.

Plus de détail : [ARCHITECTURE_CURRENT](docs/current/ARCHITECTURE_CURRENT.md), [ARCHITECTURE_PRODUCT_TRUTH](docs/current/ARCHITECTURE_PRODUCT_TRUTH.md).

---

## Construire et démarrer

### Prérequis

- Linux x86_64 ou WSL2 sous Windows, avec `git`, `python3`, `qemu-system-x86_64` (version 8 ou plus) et `ovmf` pour l'UEFI.
- [rustup](https://rustup.rs). La toolchain nightly exacte est épinglée dans [`rust-toolchain.toml`](rust-toolchain.toml) et s'installe automatiquement, avec `rust-src` et `llvm-tools-preview`.
- `cargo-bootimage`, que le script de construction installe s'il manque.

### Compiler et démarrer sous QEMU (BIOS)

```bash
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os
tools/ci/build_kernel.sh            # produit target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
qemu-system-x86_64 -m 2048 -smp 2 -serial stdio \
  -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

Le noyau démarre jusqu'à l'écran de connexion, puis jusqu'au shell. La commande `desktop` lance le bureau. Le journal du noyau sort sur la console série (`-serial stdio`, ou `-serial file:serie.log`).

Sous Windows, `run.ps1` construit et lance une configuration complète, avec le bureau et le runtime Ladybird :

```powershell
.\run.ps1
```

### Démarrer avec la composition GPU (VIRGL)

Il faut un QEMU avec `virtio-gpu-gl-pci` (virglrenderer) et une sortie OpenGL (SDL ou GTK). Le noyau négocie VIRGL tout seul quand l'hôte le propose, vérifie la convention du `BLIT` par un autotest relu au pixel, puis compose les fenêtres par le GPU ; sans VIRGL, rien ne change.

```bash
tools/doom/build-linux.sh           # DOOM (Doomgeneric épinglé, Freedoom vérifié), facultatif
qemu-system-x86_64 -m 2048 -smp 2 -serial stdio -vga none \
  -device virtio-gpu-gl-pci -display sdl,gl=on \
  -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

Sous Windows : `.\run.ps1 -P10GPU` (et `-Doom` pour ajouter DOOM). Dans le journal, `BOUCHAUD_GPU_COMPOSITION_AUTOTEST_OK` puis `BOUCHAUD_GPU_COMPOSITION_OK` confirment le chemin GPU ; `gpu-composition off` (shell) revient à la composition processeur pour comparer.

### Image UEFI

Le noyau UEFI se compile avec :

```bash
cargo build --target targets/x86_64-bouchaud_os_uefi.json --no-default-features --features uefi-boot,reference-desktop
```

L'image A/B complète (chargeur, deux slots, métadonnées) n'est aujourd'hui produite et démarrée que par le banc [`tools/ci/run_ab_rollback.sh`](tools/ci/run_ab_rollback.sh), sous QEMU et OVMF. C'est aussi la meilleure documentation exécutable du chemin UEFI.

### Image pour la machine physique

```bash
tools/release/build_trigkey_image.sh out/trigkey
sha256sum -c out/trigkey/bouchaud-trigkey.img.sha256
```

Le script refuse un arbre de travail sale. Il produit :
- l'image et son empreinte ;
- le noyau ELF non strippé, nécessaire pour symboliser une panne ;
- un `manifest.json` qui relie l'image au commit.

Sous Windows, l'équivalent est `tools/reference/IMAGE-TRIGKEY.ps1`, et l'image se teste d'abord sous QEMU :

```powershell
.\tools\reference\run-trigkey-single-usb-qemu.ps1 -Accel tcg -SkipBuild
```

> **⚠ Écriture sur disque.** Une image se flashe en mode DD : la clé cible est **intégralement écrasée**. Ne choisissez jamais le disque interne de la machine comme cible.
>
> **L'installation et le démarrage physiques ne sont pas validés sur le code actuel.** [TRIGKEY_CAMPAIGN](docs/current/TRIGKEY_CAMPAIGN.md) décrit le protocole, qui n'a pas encore été exécuté. L'image actuelle n'embarque pas encore les services natifs A/B (`serviced`, `healthd`, `updated`, `logd`).

### Journaux et diagnostic

- **Console série** : tous les marqueurs de preuve (`BOUCHAUD_…`) y sont publiés, et les bancs ne lisent rien d'autre.
- **Boîte noire** : sur une machine réelle sans câble série, le journal est écrit sur la clé.
  - Extraction : `tools/reference/extract-blackbox.py --image <clé>`.
  - Symbolisation : `tools/reference/symbolise-blackbox.py`, avec le `kernel.elf` de la même image.
- **Commandes du shell** : `hwinfo`, `lsusb`, `usbetat`, `netdiag`, `bootlog`, `services`, `diag-save`.

---

## Tests et CI

Une preuve vaut pour **un SHA** et **un niveau**. Un PASS sous QEMU TCG ne dit rien de KVM, et aucun des deux ne dit rien du matériel : une CI verte n'est **pas** une preuve physique.

| Famille | Où | Quand | Ce qu'elle établit |
|---|---|---|---|
| Tests hôte | `tools/**/test_*.rs`, lancés par `tools/ci/run_host_tests.sh` | local, CI | la logique pure (décodeurs, politiques, A/B, rendu), incluse **telle quelle** depuis `src/` |
| Garde-fous d'architecture | `tools/ci/run_architecture_guards.sh` (`tools/verifie-*.py`) | local, CI | invariants de code, contrats d'ABI, interdits, avec leurs témoins négatifs |
| CI Fast | `ci.yml` | push, PR | construction, garde-fous, tests hôte : le contrôle court |
| Integration | `integration.yml` | push, PR | démarrages QEMU, stockage, réseau, USB |
| Reliability V3 | `reliability-v3.yml` | push, PR, chaque nuit | matrice de fiabilité, coupures, injections de pannes |
| Native IPC Runtime | `native-ipc-runtime.yml` | push, PR (filtré par chemins) | ABI native, services ring 3, matrice A/B de `run_ab_rollback.sh` |
| Security Runtime | `security-runtime.yml` | push, PR | tests négatifs : l'interdit échoue, l'autorisé passe |
| Ladybird | `ladybird-native-browser.yml`, `ladybird-sync.yml` | push, PR, chaque jour | construction du navigateur, WPT, sites, robustesse, endurance |
| P13 primitives | `p13-primitives-noyau.yml`, `p13-cible-kvm.yml` | push, PR | primitives du noyau sous TCG et KVM (SMP1/2/4) |
| P13 strict | `p13-dedicated-kvm.yml`, `p13-final-candidate.yml` | manuel | qualification sur runners KVM4/KVM8 dédiés (**bloquée** : runners non enregistrés) |
| Nightly, endurance, soak | `nightly.yml`, `endurance.yml`, `soak.yml` | nuit, semaine, manuel | campagnes longues |

Avant de pousser, la barrière locale rejoue l'essentiel :

```bash
tools/ci/run_host_tests.sh
tools/ci/run_architecture_guards.sh
tools/ci/run_barriere_locale.sh
```

**Lire un résultat.**
- `PASS` signifie que la preuve nommée a tourné et qu'elle est verte, au niveau indiqué.
- `skipped`, `cancelled`, `NON_QUALIFYING` ou une preuve absente ne sont **pas** des PASS.
- Pour un `pull_request`, GitHub teste un commit de fusion synthétique, qu'il faut distinguer de la tête de branche.

Le détail est dans [CI_AND_PROOFS](docs/current/CI_AND_PROOFS.md).

---

## Limitations connues

| Domaine | État |
|---|---|
| Matériel physique | campagne Trigkey **non commencée** sur le code actuel ; Wi-Fi, Bluetooth, HDA, gestion d'énergie et suspend non pris en charge |
| Stockage | pas de VFS disque généraliste, pas de cache de pages unifié, pas de `mmap` en écriture sur un fichier disque ; `/persist` réécrit sa table entière à chaque transaction |
| Isolation | le bureau, les applications natives, le réseau et TLS tournent encore dans le noyau |
| Graphique | composition GPU des fenêtres sous QEMU VIRGL seulement ; le raster (Skia/Ladybird, applications natives) reste CPU ; pas de Vulkan invité, pas de pilote AMD (la Trigkey reste en GOP) |
| Mises à jour | pas de clé de version de production, pas de politique anti-retour arrière |
| Qualification | P13 strict SMP1/2/4 vert sous KVM hébergé ; SMP8 et campagne finale bloqués faute de runners auto-hébergés `bouchaud-kvm4`/`bouchaud-kvm8` ; `main` sans protection de branche |
| Anomalies ouvertes | `SIGCHLD_MULTIFIL_ECHEC` (une occurrence, non reproduite en 125 essais) ; une panique SMP4 dans un banc de comptabilité |

Liste complète et priorités : [KNOWN_LIMITATIONS](docs/current/KNOWN_LIMITATIONS.md).

---

## Travaux en cours et feuille de route

**Pull requests et branches ouvertes**, état au 2026-11-10, à revérifier sur GitHub.

| PR / branche | Sujet | État |
|---|---|---|
| `claude/c22-p13-p10-gpu-kernel-0y2ucg` | compositeur GPU des fenêtres, client graphique générique, bureau sans croissant ni Terminal imposé, qualification C22/P13 | CI verte (TCG, KVM, Ladybird sous VIRGL) ; voir [SESSION_2026-11-10](docs/current/SESSION_2026-11-10.md) |
| [#281](https://github.com/bcharthur/bouchaud-os/pull/281) | P10 VIRGL, première version | repris et remplacé par la branche ci-dessus |
| [#280](https://github.com/bcharthur/bouchaud-os/pull/280), [#279](https://github.com/bcharthur/bouchaud-os/pull/279) | instantané local P10, docs du 9/11 | obsolètes |

**Chantiers ouverts**, par ordre de priorité. Ce sont des projets, pas des fonctionnalités livrées.

| Chantier | Objectif |
|---|---|
| P13 strict | runners KVM4/KVM8 dédiés, qualification sur un SHA unique, endurance de 30 min |
| Stockage généraliste et VFS | inodes, montages, cache de pages, `read`/`write`/`mmap` cohérents, `fsync`, volumes externes, journalisation |
| Plateforme Trigkey | démarrage physique, USB, NVMe, réseau, audio, horloges, énergie, endurance, diagnostics reproductibles |
| Performance mesurée | latence des interactions, coût de l'IPC, copies, allocations, temps de chargement |
| Moteur graphique | fait : composition GPU des fenêtres (VIRGL). Ensuite : compositeur ring 3 ([plan](docs/current/FENETRES_RING3_PLAN.md)), raster GPU (Venus/Vulkan invité), GPU AMD |
| Runtime applicatif isolé | applications et services hors du noyau, contrats IPC, supervision |
| Pilotes matériels | HDA, Wi-Fi, gestion d'énergie, isolation DMA |
| Sécurité de production | clé de version hors ligne, anti-retour arrière, protection de `main` |
| Installation physique | installateur et mise à jour sur le disque interne, après la campagne physique |
| Plus tard | IA locale comme service isolé, conteneurs, modes d'usage |

Feuille de route détaillée : [ROADMAP_CURRENT](docs/current/ROADMAP_CURRENT.md).

---

## Organisation du dépôt et contributions

| Chemin | Contenu |
|---|---|
| `src/arch/x86_64/` | démarrage, interruptions, APIC, SMP, PCI, passage en ring 3 |
| `src/kernel/` | mémoire, processus et threads, ordonnanceur, ABI native, services, mise à jour A/B |
| `src/compat/linux/` | personnalité Linux/POSIX |
| `src/drivers/` | pilotes (bloc, réseau, USB, affichage, audio, entrées, série) |
| `src/fs/`, `src/net/` | systèmes de fichiers et persistance ; pile réseau |
| `src/gui/` | bureau, gestionnaire de fenêtres, applications natives |
| `userland/` | services ring 3 et `bouchaud-sdk` |
| `system/update-core/` | politique A/B partagée par le chargeur et le noyau |
| `tools/` | bancs QEMU (`tools/ci/`), tests hôte, garde-fous, Ladybird, images |
| `docs/current/` | documentation courante ; `docs/history/` et `docs/historique/` contiennent les archives |

**Conventions**

- Une PR par incrément, avec ses preuves (tests hôte, banc QEMU, garde-fous) au même SHA. Pas de push direct sur `main`.
- Un comportement important porte un marqueur (`BOUCHAUD_…_V1`), dans le code et dans le banc qui le vérifie.
- Un garde-fou s'accompagne d'un témoin négatif : la mutation qu'il doit refuser.
- On ne relâche ni un seuil ni un garde-fou pour faire passer un test. Un rouge est attribué avant d'être corrigé.
- Le code et les commentaires sont rédigés en français.

Pour reprendre le projet : [`docs/current/README.md`](docs/current/README.md).

---

## Licences et attributions

Le code de Bouchaud OS est publié sous **double licence MIT ou Apache-2.0**, au choix ([`LICENSE`](LICENSE), [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)). Cette licence ne s'applique pas aux composants tiers, qui gardent la leur :

- **Ladybird** (BSD 2-Clause), épinglé et patché, ainsi que ses dépendances (ICU, OpenSSL, Skia, HarfBuzz, FreeType…) : voir [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) et [`THIRD_PARTY_LICENSES/`](THIRD_PARTY_LICENSES/).
- **Crates Rust** du noyau (fontdue, smoltcp, wasmi, hashbrown, x86_64…) : licences permissives, recensées dans les notices.
- **Polices DejaVu** : licence Bitstream Vera / DejaVu.
- **Atelier** :
  - icônes [Lucide](https://lucide.dev) (ISC) ;
  - photographie de fond « summer 1am » de Risto Saukonpää (LGPL-3.0, KDE plasma-workspace-wallpapers) ;
  - sous-ensemble de DejaVu Sans.

  Leurs attributions et le manifeste des assets sont dans [`assets/atelier/ATTRIBUTIONS.md`](assets/atelier/ATTRIBUTIONS.md). Le logo provisoire (croissant) n'est plus affiché ; le fichier reste dans les assets.
