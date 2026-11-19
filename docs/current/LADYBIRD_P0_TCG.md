# Fluidité P0 sous TCG

Référence 12c69eaca80713aefa6121dd73d406f48b776259. QEMU 10.0.13, TCG,
un vCPU de modèle max, 2 048 Mio, démarrage BIOS, VGA standard et composition
CPU. Hôte AMD EPYC 9V74 avec quota de deux CPU. Images compilées avec
nightly-2026-06-01 et les fonctionnalités Cargo par défaut. Le
[relevé JSON](ladybird-p0-tcg-results.json) contient leurs SHA-256, les
résultats Atelier et les 24 durées de premier accès de chaque démarrage mémoire.

## Bureau

Le gestionnaire invalidait tout le bureau à l’ouverture ou à la fermeture,
quelle que soit la taille de la fenêtre. Il invalide maintenant l’empreinte
peinte, ombre comprise, la fenêtre qui perd ou reprend le focus et la barre
des tâches. La disparition d’un client suit le même calcul. Les effets
signalent leurs zones animées et le repli CPU du GPU garde son invalidation
plein écran.

A/B alterné, trois cycles d’ouverture et de fermeture d’Insights par
démarrage, suivis d’un survol, d’un déplacement et d’un redimensionnement.
Les valeurs viennent du dernier relevé périodique GUI-DAMAGE de chaque
démarrage. Le compteur full recense les invalidations plein écran. Les
pixels présentés comptent les régions réellement présentées.

| Paire | Invalidations plein écran BASE → corrigé | Pixels présentés BASE → corrigé |
|---|---:|---:|
| 1 | 5 → 1 | 14 751 086 → 12 771 328 (−13,4 %) |
| 2 | 5 → 1 | 13 660 672 → 10 316 084 (−24,5 %) |

Durée de composition par trame en µs, P50 / P95 / P99 / max.

| Paire et phase | BASE | Corrigé |
|---|---:|---:|
| 1, ouverture | 693 / 21 656 / 33 741 / 75 860 | 545 / 22 933 / 40 514 / 67 261 |
| 1, fermeture | 586 / 22 488 / 28 877 / 28 877 | 660 / 20 161 / 20 933 / 20 933 |
| 2, ouverture | 468 / 26 629 / 37 944 / 60 622 | 435 / 28 049 / 103 557 / 119 063 |
| 2, fermeture | 673 / 22 373 / 23 240 / 23 240 | 656 / 53 499 / 77 005 / 77 005 |

Dans la deuxième paire, le seul temps de présentation passe de
32 / 1 011 / 1 458 / 1 871 à 33 / 1 345 / 2 959 / 22 657 µs à
l’ouverture, et de 30 / 1 643 / 1 675 / 1 675 à
32 / 1 368 / 4 462 / 4 462 µs à la fermeture. Le coût en pixels baisse
sur les deux paires. Les durées hautes varient, avec des dégradations
dans la deuxième paire. La baisse de latence reste à démontrer.

L’oracle hôte vérifie le framebuffer simulé pour une ouverture, une
fermeture avec transfert de focus, une fermeture en arrière-plan et la
suppression d’une fenêtre minimisée. Sous TCG, une capture QEMU avant et
après fermeture d’Insights confirme le retour exact à l’image initiale
sur 327 320 pixels de l’empreinte testée. Le curseur était au même
point extérieur (1100,600) avant les deux captures. L’IPC natif contrôle
aussi 64 000 pixels exacts de la mire ring 3.

## Mémoire

Le chemin map_alloc_accounted parcourait les tables pour tester la présence,
puis les parcourait à nouveau pour publier une page absente. Il conserve
désormais l’entrée feuille. Les PTE présentes sont préservées. Chaque
nouvelle frame est mise à zéro et attribuée à l’espace avant publication,
avec invalidation locale et comptage RSS de la classe de la nouvelle PTE.

La sonde parcourt 8 192 pages anonymes par essai dans un ordre non séquentiel,
vérifie le zéro avant chaque première écriture et libère la plage. Le même
exécutable musl statique sert aux deux bras. Les fautes zéro dominent le
temps de faute mesuré pour cette charge synthétique. Les deux bras intègrent
le correctif GUI. Les 24 mesures par démarrage donnent les quantiles du
temps de premier accès des 8 192 pages en µs.

| Essai | Fautes zéro, nombre / total ms / max µs | Fautes fichier, nombre / total ms | P50 / P95 / P99 / max en µs |
|---|---:|---:|---:|
| Avant 1 | 196 608 / 1 745 / 5 167 | 8 / 7 | 122 163 / 198 230 / 201 361 / 201 361 |
| Après 1 | 196 608 / 1 572 / 3 879 | 8 / 6 | 117 830 / 133 010 / 141 028 / 141 028 |
| Après 2 | 196 608 / 1 631 / 5 076 | 8 / 8 | 121 009 / 156 123 / 177 503 / 177 503 |
| Avant 2 | 196 608 / 1 639 / 8 614 | 8 / 5 | 124 292 / 147 044 / 151 378 / 151 378 |

La durée cumulée des fautes zéro baisse de 9,9 % et 0,5 % par paire. Le
premier accès médian baisse dans les deux paires, avec une queue plus
lente dans la deuxième. Les six démarrages initiaux face au noyau de
référence restent dans le JSON, avec des variations entre démarrages.
La mesure noyau est une durée écoulée par faute. Le chronométrage de
la sonde inclut la boucle et les accès. Le temps CPU et l’effet sur
Ladybird restent à mesurer.

## Reproduction

Construire la base et la branche dans deux worktrees avec
tools/ci/build_kernel.sh. Copier les images bootimage-bouchaud-os.bin hors
de target. Pour isoler le changement mémoire, comparer b11ab4c3 et 8b420265.
Les images de développement ont été construites avant les commits
correspondants et sont identifiées dans le JSON par leur SHA-256.

~~~bash
python3 tools/ci/banc_atelier.py --boot "$BOOT" --sortie "$RESULTATS" \
  --accel tcg --smp 1 --cycles 3 --animations reduites --duree 2 \
  --repos 2 --demarrage 45
~~~

Pour le banc mémoire, utiliser le même disque en mode snapshot.

~~~bash
mkdir -p "$SCENARIO/bin"
musl-gcc -Wall -Wextra -O2 -static-pie -fPIE \
  tools/userland/page-zero-probe.c -o "$SCENARIO/bin/page-zero"
printf '/bin/page-zero\nfautes\n' > "$SCENARIO/autorun"
IMAGE="$DISQUE" tools/userland/mkdisk.sh "$SCENARIO"
qemu-system-x86_64 -accel tcg -cpu max -smp 1 -m 2048 -display none \
  -drive "format=raw,file=$BOOT,snapshot=on" \
  -drive "format=raw,file=$DISQUE,snapshot=on" \
  -serial "file:$JOURNAL" -no-reboot
~~~

Vérifier PAGE_ZERO_OK et la fin d’autorun avec statut 0, puis alterner
les bras en conservant les journaux. Les commits fonctionnels c900e795
et 8b420265 peuvent être révoqués indépendamment.

## Qualification

- **PASS local** pour la compilation BIOS, la matrice Reliability locale
  TCG SMP1/2/4/8 (45 s par topologie), les 216 garde-fous d’architecture
  avant le changement mémoire et les contrôles ciblés C22/RSS et P13 de
  publication après ce changement. Les 279 tests GUI hôte, 35 tests C22
  hôte et 23 tests des outils E2E hôte passent.
- **PASS local** pour Atelier A/B, la calculatrice ring 3 sous bureau
  noyau (7 + 2 = 9), l’IPC natif SMP4 avec comparaison au pixel et la
  sonde P13 de fautes multifils SMP4 (16 cas et reprise de l’autorun).
- **BLOCKED_EXTERNAL** pour le transfert à la VM de l’artefact
  bouchaud-ladybird-native-browser du run 44509281279. Le proxy a refusé
  le téléchargement. Les parcours Ladybird, le RSS, les pauses
  d’ordonnancement, le délai jusqu’à la première image utile, les repaints
  du moteur et les variantes mimalloc restent à mesurer. Le FAIL
  historique E2E-02 reste ouvert à vérification. Atelier utilise
  le bouton Démarrer pour un scénario distinct.
- **NOT_RUN** pour WHPX, KVM et la composition GPU matérielle. Le
  journal interactif privé n’était pas joint. Les pauses de persistance
  sont suivies en #346 et les sorties WebContent en #347.

Sur Windows, utiliser le même artefact Ladybird, la même variante
mimalloc et une image disque fraîche identique pour chaque noyau.

~~~powershell
.\run.ps1 -Accel whpx -CpuCount 1 -RamMiB 12288 -WhpxIrqchip off -Fullscreen
~~~

Rejouer Démarrer, example.com, une page lourde, la navigation, les onglets,
les clics, le défilement, le retour et la fermeture. Capturer l’écran blanc
et la première image utile séparément. Conserver les journaux série de
chaque démarrage et les résultats E2E. Comparer les scénarios avec
tools/perf/analyse_session.py --compare et les journaux BrowserHost
avec tools/ci/bench_ladybird_ab.py. En mode GPU, relever le périphérique
QEMU, le chemin de rendu observé et le résultat du repli CPU.
