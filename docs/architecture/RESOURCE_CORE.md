# Resource Core v1

## But

Resource Core unifie l'observation et les invariants des trois ressources qui
conditionnent la reactivite du systeme :

```text
CPU scheduler ─┐
PMM / VMM ─────┼── Resource Core ── diagnostics / policies / CI
DMA ───────────┤
GPU ───────────┘
```

Il ne remplace pas les sous-systemes ; il leur donne un contrat commun.

## CPU

Le timer distingue maintenant :

- ticks user ;
- ticks kernel ;
- ticks idle.

`hlt` est explicitement marque comme idle. Une tache qui dort ne recoit donc
plus artificiellement 100 % du CPU simplement parce qu'elle est la seule tache
connue du scheduler.

Les pourcentages par processus utilisent le temps mur du timer comme
denominateur. La somme des processus peut donc etre inferieure a 100 % : la
difference est du vrai idle ou du travail noyau non attribue.

Le scheduler historique `kernel/scheduler.rs` n'est plus une deuxieme
implementation : c'est une facade du scheduler reel de `kernel/task.rs`.

## Memoire

Le PMM expose desormais :

- pages utilisees/libres/totales ;
- high-water mark ;
- nombre d'allocations/liberations ;
- echecs d'allocation.

Le VMM distingue :

- VSS : union des VMA reservees ;
- RSS : PTE utilisateur reellement presentes ;
- anonymous ;
- file-private ;
- shared ;
- device/framebuffer ;
- pages presentes sans VMA connue.

Une reservation LibJS de 4 TiB en `PROT_NONE` augmente VSS mais pas RSS.

L'arene DMA garde le backend contigu actuel, mais son utilisation et ses echecs
sont maintenant comptes. La prochaine etape peut donc remplacer ce backend par
des pages pinees sans changer Resource Core.

## GPU

`drivers::gpu` devient le contrat stable du sous-systeme graphique.

Le backend implemente aujourd'hui :

```text
CPU raster
   ↓
backbuffer RAM
   ↓
BGA linear framebuffer
   ↓
scanout QEMU
```

Le GPU Core compte le mode, le scanout, la memoire de backbuffer, les presents,
les octets copies et le handoff userland.

Ce n'est pas encore une acceleration 3D. Le prochain backend vise `virtio-gpu`
puis, separement, une API de commandes/objets/fences. Le gestionnaire de
fenetres ne devra pas etre reecrit pour ce changement.

## Commandes

- `resstat` : etat CPU/PMM/DMA/GPU et pression memoire ;
- `memtop` : CPU + RSS + VSS par processus ;
- `gpuinfo` : backend et accounting graphique ;
- `resource-selftest` : invariants utilisables par la CI.

## CI

`system-health` possede une couche `resource` independante. Elle exige :

```text
[resource-selftest] OK
```

et la CI Ladybird est relancee des qu'un fichier CPU/PMM/GPU/Resource Core
change.

## Invariants v1

1. `used + free == total` pour les frames.
2. `user + kernel + idle == total` pour les ticks CPU comptes.
3. `RSS <= VSS` pour les mappings ordinaires ; les mappings device restent
   comptabilises dans RSS.
4. Une VMA non residente ne consomme aucune page RSS.
5. Une frame DMA active est visible dans l'accounting.
6. Un GPU inactif ne pretend pas avoir de scanout.
7. Le backend BGA reste le fallback fonctionnel de reference.
8. Ladybird M8 reste une barriere de regression obligatoire.

## Hors v1, sans dette d'interface

Ces evolutions se brancheront sur les contrats poses ici :

- COW et refcounts de frames ;
- zero-page globale ;
- reclaim / compression / swap ;
- page cache MemoryObject unifie ;
- allocation DMA par pages pinees/IOMMU ;
- virtio-gpu ;
- buffer objects + fences ;
- SMP/APIC et affinities CPU.
