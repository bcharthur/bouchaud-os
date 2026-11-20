# Suivi WHPX de la PR P0

Sessions interactives du 19 novembre 2026, builds vérifiés dans les
journaux. Même configuration et artefact déclarés. Les durées, actions
et pages effectivement chargées diffèrent entre les sessions.

| Indicateur | MAIN 12c69eac | P0 7849961a |
|---|---:|---:|
| Trames avec present_us | 1 193 | 282 |
| P95 présentation, µs | 1 574 | 33 179 |
| Trames complètes, effectif / P95 µs | 92 / 5 857 | 132 / 51 930 |
| Trames partielles avec copie, effectif / P95 µs | 1 039 / 832 | 56 / 34 752 |
| P99 attente interactive, dernier relevé | 1,049 ms | 2,097 ms |
| P99 attente normale, dernier relevé | 16,777 ms | 33,554 ms |
| PERSIST-SLOW au seuil noyau de 250 ms | 8 | 21 |
| Timeouts HTMLParserEndState | 2 | 4 |

Les latences diffèrent à catégorie de trame égale. Les distributions
d’ordonnancement sont cumulatives et agrègent les fils par classe.

## Chronologie

~~~text
python tools/perf/chronologie_ladybird.py 20261119-165126-serie.log > main.jsonl
python tools/perf/chronologie_ladybird.py 20261119-165623-serie.log > p0.jsonl
~~~

Les secondes série sont relatives au premier horodatage. Cette horloge
a une résolution d’une seconde et la publication peut être retardée.
Les mesures monotones sont exportées séparément.

| Session, secondes série | Observations |
|---|---|
| MAIN 20–93 | Google est demandé et se termine. YouTube est demandé à 29 s et se termine à 93 s. |
| P0 20–43 | Google et YouTube sont demandés. Aucune fin correspondante dans le journal. |
| P0 122 | Transaction lente de 1,033 s. Trames, attente cumulée et relevés WebContent voisins. Pour le PID 23, 3 159 fautes zéro et 362,6 ms cumulés, contre 3,3 ms en fautes fichier. |
| P0 149 | Présentation de 98,902 ms sur l’onglet 2. Aucun PERSIST-SLOW à ±1 s. |
| MAIN 179 | UD2 du WebContent PID 22 et transaction lente de 355,6 ms à la même seconde série. |
| P0 182 | Transaction lente de 1,859 s, acquisition du verrou en 30 ns. Pour le PID 23, 411,7 ms cumulés en fautes zéro contre 3,4 ms en fichier. Trames et ordonnancement visibles à proximité. |
| P0 216 | Pire présentation, 366,394 ms (onglet 4, séquence 231). Aucun PERSIST-SLOW à ±2 s. |
| P0 229 | Timeout du parseur PID 22 et transaction de 418 ms à la même seconde série. |

Dernier relevé du disque de pages fichier, 4 817/4 823 lectures,
environ 316 Mo dans chaque session. Le temps d’attente cumulé de ce flux
est de 396,046 ms sur MAIN et 0,139 ms sur P0. Au dernier PERSIST-SLOW,
les écritures cumulent 3,816 s et 10,186 s respectivement, et les
barrières 3,708 s et 1,721 s. Plusieurs synchronisations peuvent
séparer deux relevés de persistance. Les traces ne permettent pas
d’attribuer le P95 de présentation à un fil, un fsync ou un commit.

Les nouveaux événements lents indiquent le PID appelant, le temps de fin
monotone et les durées de la transaction pour le snapshot RAMFS,
l’écriture et les barrières. Leur effet sur les fils interactifs reste
à isoler lors du prochain essai.

## Correctif et mesure de persistance

L’ancien index mémorisait la dernière demi-zone écrite. Le prochain
commit écrivait dans l’autre moitié, donc la comparaison des secteurs
empêchait la réutilisation. Le nouveau code garde un index exact par
demi-zone, avec 4 Mio de contenu par moitié. La mise à jour peut détenir
temporairement l’ancien et le nouvel index de la moitié traitée,
en plus de l’instantané. La comparaison inclut le chemin, le secteur
et les octets. L’index est adopté après les deux barrières confirmées
et invalidé en cas d’erreur. La table, le superbloc et les deux barrières
gardent leur ordre. Le contenu aligné est écrit directement depuis
l’instantané, dont la copie reste sous le verrou RAMFS.

Linux TCG SMP1, 2 048 Mio, QEMU 10.0.13. Même disque initial en
snapshot, même sonde musl. Un fichier de 3 Mio reste stable et un
fichier de deux octets varie à chaque fsync. Les deux premiers cycles
remplissent les deux demi-zones. Ordre A1/B1/B2/A2/A3/B3.

| Démarrage | Cycles 0–1 µs | Cycles 2–7 µs | Médiane 2–7 µs |
|---|---|---|---:|
| Avant 1 | 34575,24983 | 18209,16254,16154,16775,13785,14594 | 16 204 |
| Avant 2 | 69477,27124 | 15825,16534,16759,24690,15901,15205 | 16 217,5 |
| Avant 3 | 83036,46848 | 17727,16870,17813,17266,17092,16105 | 17 179 |
| Après 1 | 23452,15544 | 6244,5968,6298,5910,5447,4888 | 5 939 |
| Après 2 | 20949,16359 | 6755,6135,6154,6131,6030,6168 | 6 144,5 |
| Après 3 | 28938,27378 | 6704,5937,6392,5716,5443,5778 | 5 857,5 |

Le test de coupures existant a réussi ses huit redémarrages TCG SMP2
avec zéro incohérence, régression ou invention de version. Pour
rejouer le banc, compiler tools/userland/persist-repetition-probe.c
avec musl-gcc -O2 -static-pie, l’installer comme bin/probe dans un
scénario mkdisk.sh et mettre /bin/probe dans autorun. Démarrer avec ce
scénario en second disque snapshot et exiger PERSIST_REPETITION_OK.
Pour les coupures, utiliser tools/ci/run_coupure_persistance.sh BOOTIMAGE 8.
La charge synthétique ne mesure pas la fluidité de Ladybird.

## Navigation et WebContent

Le code épinglé de Ladybird (cdfe5f858eb5) associe la phase 3 à
WaitingForLoadEventDelay. Le timeout est une ligne de diagnostic.
Le chrome journalise document_charge dans on_load_finish. L’absence de
ce marqueur peut donc coexister avec une page déjà partiellement peinte.
Les erreurs de transfert Google Video sur MAIN et les EOF TLS sur P0
ne désignent pas la ressource qui retarde load.

Dans les deux traces, le PID 27 hérite de l’image WebContent au fork,
tente execve lors de sa première exécution puis sort 127 après 186/187 ms,
sans confirmation d’une nouvelle image. Son parent indiqué est le PID 25.
L’errno de la tentative manquait dans les traces. Il est désormais
journalisé avec le PID au retour d’erreur d’execve. La cause de l’échec
et le sort du parent restent à qualifier.

Sur MAIN, l’instruction du PID 22 est UD2 (0f0b), à 0x400009d9e43b.
La base vaut 0x400000400000 et l’offset ELF 0x999e43b. La pile est
présente. La fonction et la ligne restent à résoudre avec l’ELF exact
et ses symboles via tools/ci/symbolise_fautes.py JOURNAL DOSSIER_BINAIRES.

## Campagne sur le PC Windows

Compiler BASE 12c69eac, GUI b11ab4c3, mémoire 8b420265 et le candidat
séparément. BASE→GUI isole le rendu, GUI→mémoire le parcours des tables,
mémoire→candidat les changements de suivi et de persistance. Les résultats
synthétiques TCG antérieurs sont dans LADYBIRD_P0_TCG.md.

~~~powershell
python tools/e2e/ladybird/ab_whpx.py --qemu qemu-system-x86_64.exe --disque C:\banc\disque-e2e.img --sortie C:\banc\p0-ab --bras $BaseSha C:\banc\base.bin --bras $GuiSha C:\banc\gui.bin --bras $MemoireSha C:\banc\memoire.bin --bras $CandidatSha C:\banc\candidat.bin
~~~

Fournir les SHA Git complets. Par défaut, trois essais alternés par
bras, SMP1, 12 288 Mio et irqchip off, disque identique vérifié par
SHA-256 et protégé par snapshot. Le scénario rejoue les mêmes actions,
onglets et URL de la fixture. Les échecs et écarts de provenance sont
conservés. Rejouer ensuite Google et YouTube séparément avec la même
séquence et les mêmes délais pour chaque bras. WHPX et l’effet sur
les fils interactifs restent à valider sur le PC.
