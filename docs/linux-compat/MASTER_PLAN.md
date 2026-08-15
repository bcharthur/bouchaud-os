# Executer des applications Linux sur Bouchaud OS

## Pourquoi

Deux raisons, et la seconde est la plus importante a court terme :

1. L'utilisateur doit pouvoir telecharger un binaire Linux x86-64 et le lancer.
2. **Une bonne compatibilite Linux debloque Ladybird.** Les memes primitives
   (`futex`, `mmap`, `poll`, sockets, signaux, TLS de thread, ELF dynamique)
   servent aux deux chantiers. Elles sont suivies dans un tableau commun :
   `../PORTABILITY_MATRIX.md`.

## Etat mesure

Ce n'est pas un depart de zero. Le dispatch de `src/kernel/abi/mod.rs` route
**153 appels systeme**. Sur les 103 appels que reclament LibJS, LibCore, LibIPC et
une application Linux ordinaire, **101 sont deja routes** ; manquent `clone3` et
`renameat`.

Le chargeur ELF gere `PT_LOAD`, `PT_INTERP`, argv/envp/auxv, statique-PIE et
dynamique avec `ld-musl-x86_64.so.1`. `tools/userland/build.sh` produit deja les
trois formes : `freestanding`, `musl` statique, `musl-dynamic`.

## Ce qui manque n'est pas une liste de syscalls

Supporter ELF ne suffit pas ; il faut l'ABI **et** les attentes d'environnement.
Le vrai risque n'est pas l'appel absent — il est visible, il rend `ENOSYS` — mais
l'appel present dont le **comportement** differe. Un `poll` qui ne signale pas
`POLLHUP`, un `readlink("/proc/self/exe")` qui ment, un `statx` qui remplit mal
un champ : rien ne remonte, et l'application se comporte mal ailleurs.

D'ou la methode.

## Methode : mesurer, pas deviner

Reference explicite : le projet **Loupe** (arXiv:2309.15996). Plutot
qu'implementer Linux au complet, on mesure ce que les charges reelles demandent.
Loupe montre qu'un ensemble d'applications etudie ne reclamait que 37 appels la
ou un developpement non systematique en avait produit 92.

Outil a construire : un traceur noyau qui, pour chaque processus, enregistre

    numero d'appel, arguments, pid/tid, resultat, ENOSYS ?

et rend un `linux-compat-report.json`. Chaque application testee produit son
rapport ; l'union des rapports pilote la file de travail. Aucun appel n'est
implemente parce qu'il figure sur une liste : il l'est parce qu'une trace le
reclame.

## Cibles, par difficulte croissante

| Cible | Interet | Statut |
|---|---|---|
| `hello-musl` statique | valide la chaine | deja possible |
| `busybox-musl` | des dizaines d'outils d'un coup | a tester |
| `curl-musl` | reseau + TLS | a tester |
| Python | deja embarque, sert de reference | acquis |
| `git`, CMake, Ninja | chaine de developpement | plus tard |
| Ladybird | l'objectif | chantier 1 |

## Registre de formats binaires

Inspire conceptuellement de `binfmt_misc` de Linux, qui reconnait un executable
par octets magiques ou extension et delegue a un interpreteur — Wine pour `MZ`
etant l'exemple donne par la documentation du noyau.

    trait FormatBinaire {
        fn reconnait(entete: &[u8]) -> bool;
        fn charge(...) -> Result<Processus>;
    }

    BinfmtBouchaudElf   7F 45 4C 46, notre ABI
    BinfmtLinuxElf      7F 45 4C 46, ABI Linux
    BinfmtScript        #!
    BinfmtPE            MZ  -- prepare, pas implemente

La distinction entre les deux premiers se lit dans `EI_OSABI` et dans les notes
ELF ; a defaut, Linux est le repli raisonnable puisque notre ABI en est un
sur-ensemble compatible.

**Windows n'est pas porte.** Seule l'architecture est preparee, pour qu'un jour
`PE -> Wine -> couche POSIX Bouchaud` reste possible.

## Securite

Un binaire Linux n'obtient pas plus de droits qu'une application Bouchaud. Meme
table d'utilisateurs, memes permissions de fichiers, meme isolation de processus.
Le mecanisme qui refuse `/dev/input/*` a un client graphique s'applique
identiquement.

## Etapes

- **A** : `hello-linux-static` s'execute sans recompilation.
- **B** : traceur + `linux-compat-report.json`.
- **C** : combler les manques reveles par les traces, par ordre de frequence.
- **D** : musl d'abord ; glibc ensuite seulement.
- **E** : ELF dynamique complet (relocations, TLS, auxv complet).
- **F** : arborescence Linux (voir `FILESYSTEM_LAYOUT.md`).
- **G** : `bo-install`, **apres** que l'execution soit fiable. Pas avant.
