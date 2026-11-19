# Runner CI épinglé sur Ubuntu 24.04 — stabilisation TEMPORAIRE

`BOUCHAUD_RUNNER_EPINGLE_V1` — 19/11/2026.

## Ce qui s'est passé

Le 19/11, l'étiquette `ubuntu-latest` de GitHub est passée à **Ubuntu 26.04,
variante `amd64v3`** (journal apt du runner :
`resolute-updates/main amd64v3 qemu-system-x86 1:10.2.1+ds-1ubuntu3.3`).
Sur cette variante, glibc et musl statiques sont compilées pour
**x86-64-v3** (AVX2, BMI2, FMA…).

Toutes les sondes ring 3 liées en statique sur le runner sont mortes en
silence sous Bouchaud : `main@29abe9a6`, sans aucun changement de code, a
échoué sur `dns udp ring3` (banc de lecture), `os primitives`,
`tempête fork+exec`, `mm-ng6 smp4`, `native-ipc-runtime`, `ab-rollback` ;
`userland / legacy build` ne liait plus (libcrypto 26.04, jitterentropy).

## Preuve (reproduite localement, sans runner)

Même sonde (`tools/userland/lecture-fichiers-probe.c`), même noyau
(`main`), liée contre la glibc 2.43 d'Ubuntu 26.04 :

| glibc liée | `readelf -n` : ISA needed | QEMU `qemu64` | QEMU `-cpu max` |
|---|---|---|---|
| `libc6-dev_2.43-2ubuntu2_amd64` | x86-64-baseline | OK | OK |
| `libc6-dev_2.43-2ubuntu2.7_amd64v3` | baseline, **v2, v3** | **code 132 (SIGILL)** | **code 132 (SIGILL)** |

Sous `-cpu max`, le noyau rapporte `avx=1 avx2=1` mais **`osxsave=0`** :
le CPU sait faire AVX, Bouchaud ne l'active pas. Une instruction VEX en
anneau 3 lève donc `#UD`.

## Pourquoi c'est temporaire

L'épinglage remet l'environnement d'hier ; il ne règle pas la cause. La
cause est dans le noyau :

- l'état FPU est sauvé par `fxsave64` seul
  (`src/arch/x86_64/usermode.rs`, `ordonnancement.rs`, `preemption.rs`) ;
- ni `CR4.OSXSAVE`, ni `XCR0`, ni `XSAVE`/`XRSTOR` : les registres YMM ne
  sont ni activés ni préservés au changement de contexte.

Tant que ce support manque :

- aucun binaire x86-64-v3 (distributions récentes, variantes optimisées)
  ne tourne sous Bouchaud, même sur un CPU qui sait faire AVX2 (Ryzen
  7800X3D sous WHPX compris) ;
- les bibliothèques qui choisissent leur code à l'exécution (glibc, Skia
  dans Ladybird) restent sur leurs chemins SSE, faute d'`OSXSAVE`.

## Résolution définitive (chantier à ouvrir)

1. Activer `CR4.OSXSAVE` et programmer `XCR0` (x87 | SSE | AVX, plus
   AVX-512 si présent) selon `CPUID.0xD`.
2. Dimensionner la zone FPU par tâche selon `CPUID.0xD.0:EBX` et sauver /
   restaurer par `XSAVE`/`XRSTOR` (ou `XSAVEOPT`) au lieu de `FXSAVE`.
3. Banc : la sonde `amd64v3` ci-dessus doit passer sous `-cpu max` et sous
   KVM, et rester refusée proprement (SIGILL) sous `qemu64`.
4. Seulement alors, retirer l'épinglage — ou garder 24.04 pour les bancs
   `qemu64`, et lier les sondes avec un `-march` explicite.

La règle de `tools/ci/check_pipeline_policy.py` (`runs-on: ubuntu-latest`
refusé) empêche un nouveau changement d'image silencieux ; la retirer est
une décision, documentée ici.
