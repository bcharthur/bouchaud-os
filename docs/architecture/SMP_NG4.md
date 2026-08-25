# SMP-NG4 — accounting et équilibrage adaptatif

## Cause NG3

NG3 possédait deux représentations de pression: une `RunQueue` physique dont les
entrées sont retirées dès la sélection, et la table des Tasks Ready. Le fallback
de `pick_next` volait ensuite n'importe quelle Task distante même lorsque son
CPU ne possédait qu'une seule Task Ready. Un CPU idle la volait, puis un autre
CPU idle pouvait la revoler au passage suivant. Cette décision instantanée,
sans résidence minimale ni poids temporel, explique simultanément les milliers
de steals/migrations et l'absence d'équilibre durable.

## Politique NG4 initiale

* le donor est choisi depuis la pression Ready canonique, pas la longueur
  transitoire/stale du vecteur RunQueue;
* aucun vol si `donor_pressure <= local_pressure + 1`;
* une Task migrée reste 20 ms sur son CPU avant d'être de nouveau éligible;
* parmi les candidates, le voleur préfère le plus grand runtime récent, EWMA
  7/8 historique + 1/8 dernière tranche;
* un seul vol est réalisé avant réévaluation;
* les compteurs distinguent tentatives, succès, rejet équilibre et rejet
  affinité/résidence.

Cette règle ne fabrique pas du parallélisme: une seule Task CPU-bound produit
légitimement `[100/0/0/0]`. Quatre Tasks Ready indépendantes peuvent en revanche
occuper quatre CPU.

## Convention CPU

Les runtimes Task sont des nanosecondes monotones, séparées user/kernel aux
frontières syscall et imputées au CPU d'exécution. La vue Process utilise
`100% = un CPU logique`; elle peut afficher 400% sur quatre CPU. La topbar garde
`100% = capacité totale de la machine`. `cpu_map` est le delta propre au
Process sur chaque CPU, jamais la charge globale du CPU.

RSS reste la mémoire principale affichée. VSS est conservé séparément comme
réservation virtuelle, notamment pour les arènes mimalloc.

## Diagnostics

`[SMP-LOAD]` publie succès/tentatives de vol, rejets et migrations. Une ligne
`[BKL-STATS]` publie attente, possession et acquisitions cumulées en ns. Le
benchmark cible se construit avec `tools/userland/build-smpbench.sh`, puis se
lance avec `smpbench 1`, `2`, `4` et `8`.
