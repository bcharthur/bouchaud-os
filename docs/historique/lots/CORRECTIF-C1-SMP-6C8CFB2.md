# Correctif C1/SMP pour la base `6c8cfb2`

Ce paquet est un **overlay source minimal**. Il doit etre extrait a la racine
du depot `bouchaud-os`, uniquement lorsque `git rev-parse --short HEAD` rend
`6c8cfb2`. Il remplace les fichiers concernes et ajoute les tests de
falsification. Il ne contient ni image disque, ni log, ni `target/`.

Revision 2 : le test hote de reutilisation libere explicitement son garde de
lecture avant le recyclage. Cela evite qu'une extension de duree de vie d'un
temporaire Rust fasse attendre le test sur son propre lecteur.

Revision 3 : correction de l'integration Rust du noyau reel (`token(cpu)` dans
la trace de reprise BKL, et duree de vie explicite des deux `VueRegistre` de
comptabilite). Ces sites ont ete signales par `cargo check` apres validation du
modele hote.

## Verdict de l'audit

Le commit `6c8cfb2` corrige bien le blocage initial du shootdown TLB : le test
atteint desormais `MMSTRESS_CHURN_BEGIN`. Il ne termine toutefois pas le
chantier, car CHURN revele ensuite :

```text
smp_lock: OWNER local sans profondeur dans try_enter
```

La reemission TLB n'explique pas cette panique. Trois frontieres de correction
devaient etre traitees ensemble :

1. `OWNER` et `DEPTH` etaient deux atomiques. L'etat interdit
   `OWNER=local, DEPTH=0` restait donc representable entre deux publications.
2. Le registre generationnel invalidait les anciens handles, mais rendait
   encore des references Rust pendant que le recycleur pouvait remplacer une
   `Task` sur place. Une generation ne peut pas invalider une reference deja
   obtenue ; CHURN est precisement le scenario qui recycle ces emplacements.
3. Une base GS absente ou incoherente pouvait etre assimilee silencieusement au
   CPU0. Deux coeurs partageant un jeton local rendent toute BKL reentrante
   incorrecte, meme si son etat est atomique.

## Changements apportes

- BKL empaquetee dans un seul `AtomicU64` : token proprietaire et profondeur
  changent par le meme CAS lors de l'acquisition, reentrance, suspension,
  reprise et liberation.
- Identite CPU validee par l'adresse GS, l'index publie et, des que la topologie
  existe, l'APIC materiel ; repli APIC explicite au lieu d'un CPU0 implicite.
- Registre de taches dote d'un rendez-vous lecteurs/ecrivain. Le recycleur
  masque les IRQ locales, se publie, attend la quiescence de tous les gardes,
  incremente la generation puis seulement remplace la `Task`.
- Shootdown TLB : reemission ciblee conservee ; absence persistante d'ACK
  transformee en panique fail-closed avec masque des CPU manquants, jamais en
  continuation avec un TLB perime ni en boucle silencieuse infinie.
- Traces `[BKL-DETACHED]` emises seulement lors d'une violation, afin de ne pas
  serialiser chaque tour normal de l'ordonnanceur.
- Tests hote deterministes pour l'ancien demi-etat BKL, le recyclage sous un
  lecteur vivant, le premier IPI perdu et le CPU definitivement muet.

## Ce qui est deja verifie

- `git diff --check` : propre.
- garde-fous d'architecture : **34/34**.
- garde-fous dedies : etat BKL, registre, shootdown, parking, comptabilite et
  identite CPU : verts.

La compilation du noyau et le runtime QEMU ne sont pas declares verts dans ce
rapport : ils doivent etre executes sur la machine de developpement qui possede
le toolchain Bouchaud OS. Le paquet fournit le validateur correspondant.

## Validation sous PowerShell

Apres extraction, sans rien indexer ni commiter :

```powershell
Set-Location "C:\Users\Arthur\RustroverProjects\bouchaud-os"
git status --short --branch
& ".\VALIDER-C1-SMP-6C8CFB2.ps1"
```

Le validateur lance les suites hote, `cargo check`, `cargo bootimage`, puis le
vrai runtime `mm-ng6` dans Bouchaud OS sous QEMU TCG SMP4. WSL sert uniquement
a construire la sonde et a piloter QEMU ; aucun composant Windows, WSL ou Linux
n'est integre au produit bare-metal.

Le seul verdict de cloture acceptable est :

```text
C1_SMP_MM_NG6_OK
```

Si l'environnement WSL ne possede pas les outils hote requis, le script nomme
l'outil manquant. On peut verifier compilation et suites hote avec :

```powershell
& ".\VALIDER-C1-SMP-6C8CFB2.ps1" -SansQemu
```

Cela ne remplace pas la preuve runtime.
