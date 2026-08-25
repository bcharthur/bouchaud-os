# Fondation multiplateforme

Cette refonte sépare trois notions qui étaient historiquement imbriquées :

1. **Architecture CPU (`src/arch`)** — ISA, contexte, MMU, exceptions, entrée
   usermode et primitives SMP.
2. **Plateforme (`src/platform`)** — assemblage de la machine, ACPI/Device Tree,
   topologie et routage matériel.
3. **Pilotes (`src/drivers`)** — implémentations concrètes de périphériques,
   classées par fonction et destinées à implémenter des API stables.

Le cœur (`src/kernel`) est regroupé par domaines : mémoire, processus,
ordonnanceur, objets, synchronisation, temps et diagnostic. L'ABI Linux est
physiquement sortie du cœur vers `src/compat/linux` : elle reste pour l'instant
exposée sous `kernel::abi` via une façade de compatibilité afin de ne pas casser
le chemin Ladybird existant.

## Règles

- le code générique ne doit plus ajouter de nouvel import direct vers
  `arch::x86_64` ; utiliser la façade `arch` ;
- un sous-système réseau ne dépend pas directement de `e1000` ; il dépendra
  d'un contrat `NetworkDevice` ;
- le noyau ne doit à terme plus connaître `bootloader::BootInfo` ; les chargeurs
  remplissent `boot::BootInfo` ;
- Linux est une couche de compatibilité, pas le modèle interne du noyau ;
- le port AArch64 démarre d'abord sous QEMU `virt`, puis Raspberry Pi 4 ;
- aucun stub ARM ne doit prétendre fonctionner : les backends sont ajoutés avec
  leurs tests de bring-up.

## Migration sans big-bang

Les fichiers ont été déplacés physiquement mais leurs anciens chemins de modules
Rust sont temporairement maintenus avec `#[path]`. Cela permet de valider la
réorganisation séparément des refactors fonctionnels. Les façades seront retirées
progressivement lorsque les consommateurs utiliseront les nouvelles API.
