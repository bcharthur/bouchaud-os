# Architecture de Bouchaud OS

*Ce document decrit ce qui existe. Les versions precedentes — qui decrivaient
une GDT en stub, un noyau sans allocateur et un reseau a l'etat de feuille de
route — sont dans `docs/history/`.*

## Les quatre etages

```text
        ┌──────────────────────────────────────────────────────────┐
  ring 3│  navigateur (chrome Qt)          renderer (moteur Web)   │
        │  historique, politique      ⇄    DOM, CSS, mise en page  │
        │  reseau, temoins, stockage       JavaScript, Workers     │
        ├──────────────────────────────────────────────────────────┤
        │  Qt · Python · QuickJS · FFmpeg   (tools/userland/)       │
        ╞══════════════════════════════════════════════════════════╡
        │  ABI compatible Linux : syscall/sysret                    │
  ring 0│  src/kernel/abi/ · src/kernel/syscall.rs                  │
        ├──────────────────────────────────────────────────────────┤
        │  noyau : processus, fils, ordonnanceur, memoire, VFS,     │
        │  reseau, signaux, descripteurs, ELF        (src/kernel/)  │
        ├──────────────────────────────────────────────────────────┤
        │  x86_64 : GDT+TSS, IDT, PIC/PIT, pagination, ring 3       │
        │                                        (src/arch/x86_64/) │
        └──────────────────────────────────────────────────────────┘
```

La frontiere qui compte le plus n'est pas celle du bas. C'est celle du haut :
entre le navigateur et son renderer, deux processus ring 3 qui ne partagent
qu'un canal de controle et une surface. Voir `BROWSER_ISOLATION.md`.

## L'arbre

```text
src/
├── main.rs              kernel_main : sequence de boot
├── arch/x86_64/
│   ├── gdt.rs           GDT + TSS, ordre impose par syscall/sysret
│   ├── idt.rs           exceptions et IRQ
│   ├── interrupts.rs    PIC 8259, PIT, clavier, souris
│   ├── usermode.rs      bascule ring 0 → ring 3
│   ├── pci.rs           enumeration du bus
│   ├── cpu.rs           CPUID, rdtsc
│   └── rtc.rs           horloge temps reel
├── kernel/
│   ├── memory.rs        frames physiques
│   ├── vmm.rs           espaces d'adressage, pagination
│   ├── heap.rs          tas noyau
│   ├── process.rs       processus, espaces separes
│   ├── task.rs          fils
│   ├── scheduler.rs     ordonnanceur a classes : Interactive / Normale
│   ├── signal.rs        signaux POSIX
│   ├── fd.rs            table de descripteurs
│   ├── handle.rs        objets noyau references par descripteur
│   ├── partage.rs       memoire partagee : memfd, MAP_SHARED, SCM_RIGHTS
│   ├── syscall.rs       point d'entree syscall
│   ├── abi/             traduction de l'ABI Linux
│   ├── elf.rs           chargeur ELF
│   ├── exec.rs          lancement d'un programme
│   ├── sysroot.rs       arborescence systeme
│   ├── timer.rs         ticks
│   ├── input.rs         file d'entrees
│   ├── dmesg.rs         journal noyau
│   ├── power.rs         arret, redemarrage
│   └── autorun.rs       demarrage automatique
├── fs/                  VFS et persistance
├── net/
│   ├── link/            Ethernet, e1000
│   ├── internet/        IPv4, ARP, ICMP
│   ├── transport/       TCP, UDP
│   ├── application/     DNS, HTTP
│   ├── security/        TLS
│   ├── encoding/        base64, URL
│   └── stack.rs         assemblage
├── drivers/             VGA, serie, clavier, souris, framebuffer
├── gui/                 gestionnaire de fenetres, widgets, polices
├── wasm/                interpreteur WebAssembly (wasmi)
├── lang/                interpreteurs embarques
├── shell/               shell interactif
├── git/                 client git minimal
└── users/               sessions

tools/userland/
├── build-*.sh           construction de Qt, Python, QuickJS, FFmpeg
├── mkdisk.sh            image de disque
├── *-probe.c            sondes : ABI, reseau, ordonnanceur, IPC, memoire
└── navigateur/
    ├── hote.cpp         pont Qt ↔ Python : le module `bo`
    ├── bojs.cpp         pont QuickJS ↔ Python
    ├── navigateur.py    le chrome : fenetre, historique, barre d'adresse
    └── moteur/          le moteur Web
```

## Le moteur Web

`tools/userland/navigateur/moteur/` — carte detaillee dans
`WEB_ENGINE_MODULES.md`. Les modules qui portent l'architecture multiprocessus :

| Module | Role |
|---|---|
| `vue.py` | l'interface que le chrome voit d'une page : `VueLocale` ou `VueRenderer` |
| `superviseur.py` | le cote navigateur : forke le renderer, applique la politique, courte les ressources |
| `renderer.py` | le cote renderer : un moteur Web au bout d'une prise |
| `protocole.py` | les trames echangees entre les deux |
| `surface.py` | la surface partagee : `memfd` + `MAP_SHARED`, deux tampons |
| `transport.py` | `Direct` ou `Courtier` : par ou une ressource arrive |
| `privileges.py` | ce dont l'enfant n'herite pas, et ce qu'il peut encore |

## Principes

**L'isolation avant la commodite.** Le renderer nait sans les descripteurs de
son parent, avec un `RLIMIT_AS` a lui, en classe `Normale` pendant que le
navigateur est `Interactive`, et sans le droit de decider d'une navigation. Ce
qu'il ne peut pas encore, `RENDERER_PRIVILEGE_AUDIT.md` le dit sans arrondir.

**Une seule verite par question.** Le foyer appartient au document, pas au
chrome ni au JavaScript. L'historique appartient au chrome, pas au renderer. La
politique appartient au navigateur, pas au moteur. Chaque fois que deux
endroits ont su repondre a la meme question, ils ont fini par se contredire.

**Ce qui n'est pas mesure n'est pas affirme.** Les jalons fonctionnels sont
ecrits par les epreuves (`tests/jalons.json`), l'audit de privileges par
l'epreuve qui l'execute (`tests/audit-privileges.json`), et les sondes refusent
de conclure quand elles ne mesurent rien.
