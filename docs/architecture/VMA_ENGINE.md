# VMA Engine — Bouchaud Memory Fabric

Le premier patch file-backed a reduit le montage Ladybird d'environ trois
minutes a environ deux secondes, mais `promesses` restait une liste append-only.
Une operation partielle pouvait supprimer la metadata d'une arene entiere.

```text
mmap 1 Gio MAP_NORESERVE
munmap 64 Kio au milieu
```

Le moteur VMA rend les intervalles split-safe. Une VMA existe meme sans frame :

```text
VMA
├── debut / fin
├── permissions
└── backing
    ├── Zero
    ├── File private
    ├── File shared
    └── Framebuffer
```

`mmap`, `munmap`, `mprotect`, `madvise`, `brk`, les PT_LOAD ELF et le framebuffer
passent maintenant par cette metadata. `MAP_FIXED_NOREPLACE` voit aussi les
reservations PROT_NONE non residentes. `madvise(DONTNEED/FREE)` peut jeter des
frames tout en conservant la VMA, donc la page est rematerialisee au prochain
acces.

Les pages de bordure de plusieurs PT_LOAD restent des overlays explicites.
Une faute non servie affiche `FAULT_FATAL` avec la VMA exacte ou ses voisines.
`meminfo` execute `vma-selftest`, repris par System Health CI.
