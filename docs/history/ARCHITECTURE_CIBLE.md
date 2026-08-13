# Architecture cible Bouchaud OS — etat reel

Mise en place de l'ossature complete demandee. Ce document dit honnetement, pour
chaque brique, ce qui est **reel**, **partiel/socle**, ou **planifie**.

## Arborescence

```
src/
├── gui/
│   ├── mod.rs            (declarations + reexport run)
│   ├── desktop.rs        point d'entree du bureau            [reel]
│   ├── framebuffer.rs    primitives de dessin                [reel]
│   ├── window.rs         fenetres, types, geometrie          [reel]
│   ├── window_manager.rs boucle d'evenements, focus, drag    [reel]
│   ├── widgets.rs        rendu fenetres/taskbar/menu/curseur [reel]
│   ├── event.rs          types d'evenements clavier          [reel]
│   ├── mouse.rs          souris (reexport pilote)            [reel]
│   └── apps/
│       ├── mod.rs            aiguillage entree/rendu         [reel]
│       ├── terminal.rs       terminal (reutilise le shell)   [reel]
│       ├── file_explorer.rs  navigateur de fichiers          [reel]
│       ├── system_info.rs    moniteur systeme                [reel]
│       └── chromium_stub.rs  Bouchaud Browser (pages locales)[reel/partiel]
├── app/
│   ├── mod.rs
│   ├── manifest.rs       parse des .bapp                     [reel]
│   ├── runtime.rs        types d'apps natives                [reel]
│   └── launcher.rs       apps / launch (/apps/*.bapp)        [reel]
├── kernel/
│   ├── process.rs        table de processus, ps/kill         [reel (logique)]
│   ├── scheduler.rs      ordonnanceur cooperatif             [socle]
│   ├── syscall.rs        ABI + dispatch interne              [socle]
│   ├── memory.rs         resume tas (free)                   [reel]
│   ├── handle.rs         table de descripteurs               [socle]
│   ├── heap.rs           allocateur (alloc)                  [reel]
│   ├── timer.rs, dmesg.rs, panic.rs                          [reel]
└── drivers/
    ├── display.rs        infos affichage                     [reel]
    ├── keyboard.rs       PS/2 AZERTY (IRQ1)                  [reel]
    ├── mouse.rs          PS/2 (IRQ12)                        [reel]
    ├── disk.rs           df + socle block device             [socle]
    ├── net.rs            etat NIC                            [socle]
    ├── gfx.rs (+font)    mode VGA 12h 640x480                [reel]
    ├── serial.rs, vga.rs                                     [reel]
```

## Cible vs etat (la liste demandee)

| Brique cible                  | Etat actuel |
|-------------------------------|-------------|
| kernel propre                 | reel (modulaire) |
| memoire dynamique             | reel (heap alloc) ; pagination par processus planifiee |
| processus                     | logique reelle (table, ps/kill) ; pas d'isolation user-mode |
| threads                       | planifie |
| syscalls                      | ABI + dispatch interne ; `int 0x80` user-mode planifie |
| filesystem persistant         | RAMFS reel ; persistance (BFS sur disque) planifiee |
| drivers materiel              | VGA, serie, clavier, souris, PCI, RTC reels ; disque/NIC = socles |
| pile graphique haute res.     | 640x480x16 reel ; truecolor HD = migration bootloader 0.11 |
| souris                        | reel (IRQ12 + curseur) |
| gestionnaire de fenetres      | reel (multi-fenetres, focus, drag, min/max/resize) |
| systeme d'applications natives| reel (.bapp + launcher + apps GUI) |
| services systeme              | socle (process init/desktop/shell) |
| securite utilisateur          | reel (users, mots de passe, permissions Unix) |
| installateur / paquets        | planifie |

## Chantiers longs (cadres, pas bacles)

- **Vraie HD truecolor** : migration vers `bootloader 0.11` (framebuffer lineaire
  RGB) — change le boot, a faire par increments testes en QEMU.
- **Reseau reel** : driver e1000/virtio-net (DMA, rings) puis UDP/DHCP/DNS/TCP/
  HTTP, et **TLS** bien plus tard.
- **Multitache preemptif user-mode** : GDT user segments, TSS, changement de
  contexte sur IRQ0, espaces memoire par processus, vrais syscalls.
- **Compatibilite .exe Windows (PE/Win32)** : loader PE, DLL, subsystem Win32,
  GDI, sockets... chantier enorme, hors de portee a court/moyen terme. La voie
  realiste reste **les applications natives Bouchaud** + un navigateur natif.
