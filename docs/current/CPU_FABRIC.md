# CPU Fabric · audit de #384

Base `fdef86bef300852a37584eefdb86b0dc27f71082`, qui fusionne #383. Tête Claude `dc280eabf904c52ca1af0ee16264351b30ee66d8`, neuf commits sans divergence. [Brouillon #384](https://github.com/bcharthur/bouchaud-os/pull/384)

| Commit | Changement relu |
|---|---|
| `04f0456c` | Historique des élections par CPU et rapport des retards HID. Paramétrage du banc BrowserHost pour simuler un coût par octet et changer le nombre de vCPU. La simulation série n'établit pas une qualification WHPX. |
| `5ee70edb` | Saut des sous-arbres absents dans le démontage de plages. Destruction des feuilles présentes, maintien du shootdown et du rendu séparé des frames. |
| `454ff152` | Filtre du banc sans arrêt prématuré du lecteur sous pipefail. |
| `4a70c913` | Sonde de trous, protections, réemploi, fichiers et bilan de mémoire, avec mutant. |
| `a0063503` | Rattache le contrat documentaire à `prepare_unmap`. |
| `d3700bb0` | Profil par appel côté noyau et sonde glibc/musl côté utilisateur. |
| `6b949ab6` | Ajoute ELF vDSO, page vvar, auxv, publication séquencée du TSC, `getcpu`, repli syscall et protections des écritures utilisateur. |
| `325b4920` | SIGKILL distant via condamnation de groupe, y compris un fil en calcul pur rattrapé à une frontière IRQ. Ajoute un banc de latence par charge. |
| `dc280eab` | Parcours des feuilles en une passe, constitution du fils par blocs, comptabilité RSS à la pose de chaque PTE et regroupement des invalidations. Table COW à adressage ouvert sous verrou global. |

## Invariants et corrections

- **Fork et TLB.** Un premier fork peut relâcher le verrou Mm avant la fin du shootdown. Un second fork peut alors voir uniquement des PTE déjà marquées COW. Un fork partiellement échoué peut également laisser des PTE rétrogradées. Le candidat invalide aussi dans le cas des PTE COW précédemment inscriptibles, localement et sur les cœurs actifs du parent avant publication du fils. Le chemin distant conserve l'attente des ACK hors du verrou Mm. Un fils incomplet reste invisible.
- **COW et RSS.** Une part par frame distincte, même avec des alias. Les pages empruntées gardent leur propriétaire et les PTE gardent la classe RSS. Chaque PTE installée compte une fois pour le RSS du fils. La table réserve la place du lot avant tout changement de compte. Un échec de croissance ne publie aucun compte partiel. Le fils incomplet est détruit après retrait des frames simplement recensées. Allocation des vecteurs de travail et extension du suivi des tables deviennent faillibles. L'ordre Mm → COW reste inchangé. Le coût d'un redimensionnement sous le verrou global, les autres allocations du syscall, une injection de panne à chaque site et le bilan RSS sous contention demandent des essais ciblés.
- **VDSO.** Le chemin statique vérifie ELF, symboles, absence de relocations, permissions initiales et protections d'écriture. Le candidat refuse aussi de rendre vvar exécutable par `mprotect`. Les sondes glibc et musl vérifient auxv non nul, les horloges et la protection. Sous TCG SMP2 avec RDPID et RDTSCP masqués, glibc et musl repassent chacune par le noyau pour les 5 000 lectures de CPU, tandis que le TSC reste utilisable pour les horloges. Le repli complet des horloges quand le TSC est inéligible reste à provoquer. `sched_setaffinity` est encore un retour de compatibilité dans ce noyau. La sonde d'échange n'établit donc pas à elle seule que les deux participants ont réellement tourné sur deux cœurs distincts. Certifier les CPU observés, la comparaison vDSO/syscall et les replis avec CPU/horloges non éligibles avant GO.
- **SIGKILL et récolte.** La sonde vérifie maintenant `WIFSIGNALED` et `WTERMSIG == SIGKILL` pour chaque perturbateur. L'état zombie du processus, le dernier fil effectivement sorti de son contexte et le recyclage du slot de tâche restent trois événements distincts.

## Mesures indépendantes

TCG SMP1, 1 Gio, ABBA, deux boots par bras, 20 forks par boot, 64 Mio privés touchés avec un accès volatile par page. Temps du parent autour de `fork`. Chaque enfant doit observer sa page et rendre son statut. Même binaire et même disque de sonde en snapshot. Base `fdef86be` contre la première tranche corrigée. Médianes par boot A1 **60,526 ms**, B2 **8,946 ms**, B3 **8,910 ms**, A4 **61,852 ms**. Étendues A 59,537–72,072 ms et B 8,094–21,290 ms. Les journaux locaux, le code de sonde et les SHA d'images sont archivés sous `/workspace/scratch/cpu-fabric-results/`. Ces mesures reproduisent l'ordre de grandeur du gain de fork sous TCG. Une deuxième série ABBA, même hôte et même sonde de latence en phase fork de trois secondes, a mesuré les P50 **15,056 / 1,803 / 2,224 / 15,244 ms**, avec **83 / 19 / 22 / 81** réveils au-delà de 10 ms et **129 / 747 / 682 / 127** échantillons. P95 **60,496 / 9,194 / 9,026 / 61,629 ms**. Le gain sur la latence est reproduit. Le compte brut exact de Claude (77 à 7) varie avec le nombre de réveils. Ces deux séries précèdent le commit de récolte.

Le banc versionné `tools/ci/run_cpu_fabric_fork64.sh BOOT_A BOOT_B NOUVELLE_SORTIE 1` rejoue la mesure. Sur le lot de retrait, ce banc donne 66,443 / 9,353 / 9,194 / 63,677 ms (médianes ABBA, 20 mesures par passage). Pour la latence, lancer `tools/ci/run_latence.sh BOOT 1 3 fork` dans le même ordre ABBA, avec `GARDE_JOURNAL` différent par passage. Archiver les empreintes et ne comparer que des essais complets à paramètres identiques.

Sonde glibc et musl et mort SIGKILL des deux perturbateurs en calcul pur observées sous TCG SMP1/2/4. Modèle COW de 200 000 opérations et nouveau test de croissance par lots, 8/8. Garde vDSO positive et mutants réussis. Les 1 593/1 573 cycles, 154 suites hôte et 217 gardes publiés par Claude restent des résultats de sa session. La CI Integration lance la tempête KVM SMP1/2/4 déjà issue de #383 et ajoute vDSO et SIGKILL en mode strict avec journaux.

## Retrait différé des espaces d’adressage

Le mode `zombie-force` de la sonde de #383 reproduit la rétention sous TCG SMP2 sans rouvrir d'application. Mémoire libre 143 Mio vivant, 143 Mio après le délai et 143 Mio immédiatement après `waitpid`. La sonde reçoit le statut SIGKILL et échoue sur la restitution. Elle ne reproduit pas le décompte des 82 tâches WHPX. Le travail non poussé de Claude n'apparaît pas dans la branche.

Le second lot ajoute une file tenant les processus condamnés jusqu'au traitement depuis un fil noyau. Il attend l'état Zombie et l'absence de CPU et de commutation pour tous les fils de la victime. Pour un Mm indépendant, il bloque les activations, installe le Mm vide réservé par #383, puis attend la barrière CR3 avant de permettre la destruction. Pour un enfant vfork, il détache la référence du fils et libère ensuite le lien sans prendre sa retraite sur le CR3 commun. Les parents qui ont encore prêté leur Mm restent différés. La sortie tardive d'un fil ne remplace plus le statut déjà publié. Le statut reste disponible pour waitpid. La sonde vfork-force fait tuer un emprunteur en calcul par un troisième processus, puis vérifie le retour du prêteur et son waitpid. Elle passe en TCG SMP1/2/4.

Sur l'image avec ce correctif, la sonde avant waitpid repasse au vert TCG SMP1/2/4 sur sortie normale et forcée. Après mort forcée, libre de 143 à 191, 191 et 185 Mio en 14, 18 et 12 ms dans ces passages. Aucun lancement d'application n'intervient entre le signal et la mesure. Les références résiduelles du Mm sont rendues à sa destruction. Les slots de tâches stables et les piles noyau gardent encore le recyclage existant. Leur récolte autonome et les sorties concurrentes restent à qualifier. Sous TCG SMP4, la cohorte de 82 victimes restitue 77 Mio avant waitpid (103 → 180 Mio) avec 82 statuts SIGKILL. Le témoin sur le lot précédent reste à 103 Mio. Ce test porte sur le Mm et les statuts, tandis que le compteur de slots et le passage WHPX restent à vérifier.

## Preuve GitHub KVM acquise

Référence qualifiée `8bf4f6eaa49559d6051bbb7883b23a49d20c1d51`. Neuf workflows PR terminés avec succès. Le journal Integration identifie KVM sur SMP1/2/4 et donne les résultats suivants. [Campagne](https://github.com/bcharthur/bouchaud-os/actions/runs/45782160966/job/139780758748)

| Mesure | SMP1 | SMP2 | SMP4 |
|---|---:|---:|---:|
| Cycles fork / vide / complet | 905 / 366 / 698 | 903 / 366 / 691 | 894 / 362 / 686 |
| Échecs sur ces 5 871 cycles | 0 | 0 | 0 |
| clock_gettime monotone glibc / musl, ns | 38 / 39 | 33 / 33 | 33 / 33 |
| Transitions noyau sur 200 000 lectures par libc | 0 | 0 | 0 |
| Réveils en phase calcul / retard max µs | 3 000 / 613 | 3 000 / 75 | 3 000 / 508 |

La cohorte SMP4 passe de 104 à 190 Mio libres avant waitpid. Le vfork forcé passe sur les trois configurations. OOM et témoin de quarantaine passent, le job vérifie la chaîne réserve, attente et rendu pour la même victime. Le workflow Ladybird contenant C22 et l'endurance est vert. Ces conclusions concernent le SHA de référence. [Ladybird](https://github.com/bcharthur/bouchaud-os/actions/runs/45782161156)

## Dernière passe ciblée

- **Sorties et RSS.** Sur le SHA de référence, deux fils du parent attendant le même enfant peuvent bloquer après sa collecte alors que d’autres enfants restent dans la table. Le parent se rendort à cause d’un test de présence trop large. La correction vérifie l’existence du PID demandé avant et après la publication de l’attente. La sonde `rss-course` sur TCG SMP4 passe 20 tours, soit 80 enfants. Quatre frères écrivent sur le même MiB hérité. Chaque enfant garde le même RSS après COW et perd exactement 1 024 Kio après `munmap`. Les données du parent restent intactes. Deux collecteurs attendent le premier enfant à chaque tour, avec un gagnant et un retour ECHILD. Cette preuve porte sur les sorties normales concurrentes et la collecte, tandis que les sorties forcées restent couvertes par les sondes du lot précédent.
- **VDSO.** L’échange entre processus est intégré au banc SMP2/4 et valide les numéros de CPU par syscall autour des lectures glibc. Il exige des passages sur deux CPU distincts et zéro recul. Le repli getcpu avec RDPID/RDTSCP masqués vérifie les compteurs noyau. Le repli des horloges existe en preuve TCG SMP2 sur le témoin local qui force le mode syscall, deux lectures de chronométrage comprises. Le test des horloges en mode syscall et la détection d’un TSC naturellement inéligible sont des étapes distinctes.
- **Injection.** Le refus déterministe d’une réservation impossible est vérifié sur la table COW. Les comptes, propriétaires et décisions de libération restent corrects. La matrice d’injection en vrai noyau, incluant PML4, table feuille, réserve de parts et acquisition de références, exige encore des témoins d’échec partiel avec contrôle PTE/TLB et RSS.
- **Slots et piles.** Le registre a des indices stables, la génération protège la réutilisation, et la destruction attend les lecteurs et la fin de commutation. Une récolte autonome générale implique des trous dans ce registre et la validation des lecteurs d’indices bruts, ou une séparation de la durée de vie du slot et de celle de la pile. Elle reste hors de cette tranche ciblée. Le retrait autonome du Mm reste en place.

**Verdict NO-GO pour fusion immédiate.** Le SHA KVM historique a un défaut de waitpid désormais reproduit. Le correctif et ses contrôles doivent être qualifiés sur la nouvelle tête. L’injection de bout en bout reste ouverte. Le test utilisateur WHPX est indépendant de ces preuves.
