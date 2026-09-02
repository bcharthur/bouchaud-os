# C1.2 — fluidite Ladybird par RSS incremental et faults groupes

Base attendue : `d780b8f` (correctif des descripteurs partages inclus).

## Cause mesuree

Le journal reel ne montre ni manque de RAM ni reprise du BKL. Il montre :

- 155 273 fautes anonymes et plus de 600 000 fautes resolues ;
- un WebContent de 18 threads ;
- 23 attentes du verrou `Mm` dans `processus.rs:24` ;
- une attente maximale de 2 137 ms ;
- un trou maximal entre trames utiles de 88 966 ms.

Le gel etait amplifie par le releve de ressources lui-meme. Pour chaque thread,
`mesure_processus()` appelait `memory_usage()`. Celui-ci reparcourait les quatre
niveaux des tables de pages sous `Mm`. Un seul releve de WebContent pouvait donc
revisiter environ `155273 x 18 = 2 794 914` PTE, sans compter les autres
processus. Pendant ce temps les threads de rendu tournaient sur le meme verrou.

## Correction structurelle

1. Chaque PTE porte sa classe RSS dans les bits logiciels x86 9..11.
2. `AddressSpace` maintient six compteurs lors de `map` et `unmap`.
3. `mprotect` preserve le tag ; `fork` le recopie avec la PTE.
4. `resident_stats()` devient une simple lecture O(1).
5. Le releve ne lit la memoire qu'une fois par PID, et non une fois par thread.
   La copie des VMA est prise sous `Mm`, mais leur tri pour le VSS se fait apres
   liberation du verrou.
6. Une grappe de pages fichier chaudes est publiee sous une seule prise de
   `Mm`, avec revalidation de chaque token et liberation du cache hors `Mm`.
7. Le cluster anonyme grandit jusqu'a 32 pages uniquement apres une longue
   sequence confirmee ; il retombe a 8 ou 2 sous pression memoire.
8. Le target `webcontentservice` declare directement `skia`. Cela propage le
   repertoire d'en-tetes et l'archive statique au code du chrome V15, sans
   chemin propre au runner ni dependance a son systeme d'exploitation.
9. L'analyseur runtime detecte UTF-8 et UTF-16. Les journaux crees par
   `Tee-Object` sous Windows PowerShell ne perdent donc plus leurs balises.

Tout est implemente dans le noyau Bouchaud : aucun service, ordonnanceur,
memoire virtuelle ou cache de l'hote n'est introduit.

## Installation

Extraire le ZIP directement a la racine de `bouchaud-os` en acceptant le
remplacement des fichiers. Les deux fichiers locaux modifies dans `scenario-m9`
ne font pas partie du lot et ne sont pas touches.

Puis lancer :

```powershell
.\VALIDER-C1.2-FLUIDITE-LADYBIRD.ps1
```

Pour la preuve runtime :

```powershell
.\run.ps1 2>&1 | Tee-Object .\ladybird-c1-2.log
.\VALIDER-C1.2-FLUIDITE-LADYBIRD.ps1 `
    -Journal .\ladybird-c1-2.log `
    -SansBootimage
```

Le journal fourni a deja prouve `[MM-RSS-O1]` et l'absence d'attente `Mm` au
site historique `processus.rs:24`. La ligne `[MM-CLUSTER]` expose
desormais `mm_locks`; sur une charge sequentielle, `mapped/mm_locks` doit etre
superieur a 1. Le validateur runtime refuse une attente `Mm` superieure a
250 ms par defaut.

## Correctif CI Ladybird

Le job GitHub `ladybird / build once` de la PR 214 arrivait a 2714/2720, puis
les deux objets qui incluent `BouchaudChrome.h` echouaient sur
`core/SkBitmap.h`. `LibGfx` lie Skia en prive : le chrome en-tete ne pouvait pas
heriter de son include `skia/`. Le preparateur V15 ajoute maintenant Skia au
target qui compile vraiment le chrome, et le garde-fou verifie ce contrat avant
les heures de compilation C++. Le validateur C1.2 execute ce garde-fou.

## Limites honnetes

Le lot supprime l'amplification prouvee par le journal et reduit les fautes
sequentielles. La fluidite finale doit encore etre mesuree sous QEMU/TCG sur la
machine cible. Le validateur ne transforme donc pas un simple `cargo check` en
preuve de performance.
