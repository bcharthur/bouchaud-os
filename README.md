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
scripts & tools/           # build, santé, Ladybird, perf
```

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

## Licence

MIT OR Apache-2.0. Voir `LICENSE`, `LICENSE-MIT` et les notices tierces.
