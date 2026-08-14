# ABI ELF x86-64

## Etat verifie

`src/kernel/elf.rs` (476 lignes) et `src/kernel/exec.rs` implementent deja :

| Element | Etat |
|---|---|
| En-tete ELF64, `e_ident`, `ET_EXEC` / `ET_DYN` | supporte |
| Segments `PT_LOAD` | supporte |
| `PT_INTERP` -> chargement de `ld-musl-x86_64.so.1` a une base separee | supporte |
| Pile initiale : argc, argv, envp | supporte |
| `auxv` | partiel — `AT_PHDR`, `AT_ENTRY`, `AT_BASE`, uid/gid |
| Statique-PIE | supporte |
| Dynamique (ld.so fait les relocations) | supporte |
| TLS (`arch_prctl(ARCH_SET_FS)`) | supporte |

Le noyau ne resout **aucun symbole** : pour un binaire dynamique il charge ld.so
et lui donne la main. C'est exactement le contrat de Linux, et c'est ce qui
permet d'employer un `ld-musl-x86_64.so.1` non modifie.

## Ce qui reste

1. **`auxv` complet.** A confronter a ce que lit reellement musl puis glibc :
   `AT_HWCAP`, `AT_HWCAP2`, `AT_CLKTCK`, `AT_RANDOM`, `AT_SECURE`, `AT_PAGESZ`,
   `AT_SYSINFO_EHDR` (vDSO). L'absence de `AT_RANDOM` fait echouer le canari de
   pile de certaines libc.
2. **Pas de vDSO.** Les applications qui attendent `__vdso_clock_gettime`
   retombent sur l'appel systeme : correct, mais plus lent.
3. **`ET_DYN` sans interprete** (PIE statique) : verifier le calcul de la base.
4. **Distinction Bouchaud / Linux ELF.** Aujourd'hui il n'y en a pas — tout est
   traite pareil, ce qui fonctionne parce que notre ABI est compatible Linux. Le
   registre de formats la rendra explicite.

## Registre de formats

    src/kernel/binfmt/
        mod.rs        trait FormatBinaire + registre ordonne
        elf_linux.rs  7F 45 4C 46
        script.rs     #!
        (pe.rs        MZ -- plus tard)

Le registre est consulte dans l'ordre ; le premier qui reconnait charge. Un
`#!/bin/sh` doit relancer l'interprete avec le chemin du script en argv[1], comme
Linux.

## Tests

- `hello-linux-static` compile **hors** de l'arbre Bouchaud, execute sans
  recompilation.
- Comparaison de la trace d'appels avec celle du meme binaire sous Linux.
- Un script `#!` s'execute.
- Un binaire dynamique musl s'execute.
