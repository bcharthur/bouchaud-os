# CI/CD Fabric

## Référence

`main` après #384 est `bbce0e7b3ad44e3177174766ef0dcb1e180cd8f8`.
Son arbre `3d057385688971e668cf8696cc2f3d8a2fa4254b` est identique à celui
du candidat WHPX `b7cd529b798620ba8462cde3c64c3b47ef4f0ff0`. Les
événements PR et push restent distincts. [Qualification de main](https://github.com/bcharthur/bouchaud-os/actions?query=branch%3Amain+sha%3Abbce0e7b3ad44e3177174766ef0dcb1e180cd8f8) ·
[Qualification WHPX](https://github.com/bcharthur/bouchaud-os/pull/384#issuecomment-7195342326)

La première campagne post-fusion a déclenché 11 workflows. Les timestamps
des jobs de la première tentative couvrent 112 exécutions et 486,93
minutes cumulées. L’API liste 89 artefacts totalisant 717 026 736 octets
au relevé du début de la deuxième tentative P13. Ce volume indique ce qui
était publié au relevé. Les minutes sont des durées de jobs observées.

Dix workflows ont réussi. Le banc horaire P13 a réussi ses trois
démarrages. Le banc séparé de réveil précis KVM SMP4 donne 7 bras
admissibles sur 8. Au premier bras `off`, le P99 interactif est de
3 795 µs, contre 3 328 µs en charge normale. Le même analyseur donne 8/8
sur le candidat avant fusion. Un seul rejeu de ce job mesure la
variabilité observée et conserve le résultat de la première tentative.
Le rejeu de ce seul job rapporte 8/8 en SMP1 et SMP4.
[Rejeu ciblé](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863506/job/140010752399) ·
[Journal source](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863506/job/139985378606)

## Cartographie des 26 workflows de référence

| Workflow de référence | Événements | Jobs YAML | Destination ou rôle |
|---|---|---:|---|
| `ci.yml` | push, workflow_dispatch | 8 | FAST / gardes hôte et fast-gate |
| `doom-port-windows.yml` | workflow_dispatch, pull_request | 1 | plateforme Windows / port DOOM |
| `endurance.yml` | schedule, workflow_dispatch | 3 | FULL / endurance périodique |
| `governance-audit.yml` | workflow_dispatch | 1 | gouvernance / audit manuel |
| `gpu-composition.yml` | pull_request, push, workflow_dispatch | 3 | GPU / migré dans Bouchaud CI |
| `integration.yml` | pull_request, push, workflow_dispatch | 23 | système / integration-gate |
| `ladybird-native-browser.yml` | pull_request, push, workflow_dispatch | 31 | produit Ladybird et producteur canonique |
| `ladybird-run-selector-v7.yml` | pull_request | 3 | plateforme Windows / sélection et préparation |
| `ladybird-sync.yml` | pull_request, push, schedule, workflow_dispatch | 2 | épinglage amont Ladybird |
| `ladybird-vulkan-experimental.yml` | push | 2 | produit / Vulkan expérimental |
| `native-ipc-runtime.yml` | pull_request, push, workflow_dispatch | 4 | IPC / agrège System Update pendant la transition |
| `nightly.yml` | schedule, workflow_dispatch | 7 | FULL / qualification nocturne |
| `non-regression.yml` | pull_request, push, workflow_dispatch | 1 | gouvernance / non-regression-gate |
| `os-primitives.yml` | workflow_dispatch, workflow_call | 4 | CPU, mémoire, GPU, I/O / redistribuer sans changer les preuves |
| `p13-cible-kvm.yml` | workflow_dispatch | 5 | CPU / témoins de qualification |
| `p13-final-candidate.yml` | workflow_dispatch | 4 | CERTIFICATION / KVM dédié |
| `p13-primitives-noyau.yml` | pull_request, push, workflow_dispatch | 3 | CPU et mémoire / déclencheur historique |
| `release-evidence.yml` | workflow_dispatch, schedule, pull_request | 1 | gouvernance / publication de preuves |
| `release-trigkey.yml` | workflow_dispatch | 6 | livraison Trigkey |
| `release.yml` | push, workflow_dispatch | 1 | livraison noyau attestée |
| `reliability-v3.yml` | pull_request, push, schedule, workflow_dispatch | 6 | système / reliability-gate et FULL |
| `security-runtime.yml` | pull_request, push, workflow_dispatch | 2 | sécurité / migré dans Bouchaud CI |
| `soak.yml` | workflow_dispatch | 1 | endurance longue dédiée |
| `trigkey-ab-preflight.yml` | pull_request, workflow_dispatch | 1 | plateforme Trigkey / préflight |
| `unix-fs-reliability.yml` | pull_request, push, workflow_dispatch | 1 | stockage / migré dans Bouchaud CI |
| `userland.yml` | push, workflow_dispatch | 2 | produit historique encore consommé |

## Première migration

`bouchaud-ci.yml` établit le plan des bancs GPU, sécurité et Unix/FS sur
PR et push main. Le graphe des consommateurs est dans
`tools/ci/fabric_plan.py`. Les anciens motifs sont conservés comme
plancher dans `fabric_compat_paths.json`. Un chemin inconnu ou une
modification de l’infrastructure CI déclenche le périmètre migré complet.
Les déclencheurs historiques des autres workflows continuent d’appliquer
leurs qualifications.

| Contrat | Propriétaire | Couche | Périmètre | Preuve |
|---|---|---|---|---|
| GPU.L3.PIXEL_BLIT | GPU | L3 | TCG et témoins du banc de composition | Journaux, captures, verdicts pixel |
| GPU.L6.COMPOSITION | GPU | L6 | TCG/KVM, DOOM et composition | Artefacts des bras et verdicts des scripts |
| GPU.L7.LADYBIRD_VIRGL | GPU | L7 | Artefact Ladybird vérifié, smoke produit | Provenance native et journaux de composition |
| SYSTEM.L5.SECURITY_RING3 | Sécurité | L5 | Garde statique, tests hôte, TCG | Preuve ring 3 exigée |
| STORAGE.L6.USB_WRITE_ISOLATION | Sécurité | L6 | Standard, clé autorisée et témoin, TCG/KVM | Garde de mutation, journaux et identité des médias |
| STORAGE.L6.UNIX_FS | Stockage | L6 | TCG/KVM, témoin de persistance et tests hôte | Verdict Unix/FS et journaux |
| STORAGE.L6.OTA_AB_ROLLBACK | Mise à jour | L6 | Matrice NVMe/USB et variantes OTA | Chronologies et verdict de la matrice |

Le registre exécutable `tools/ci/fabric_contracts.json` relie les contrats
aux étapes, preuves, dépendances et configurations. Le plan publié inclut
les contrats sélectionnés. Ces contrats sont bloquants quand le plan les sélectionne. Les commandes,
scénarios, témoins et critères existants restent dans leurs workflows
réutilisables. Le plan indique aussi les domaines transitifs en attente
de migration. Le verdict Fabric couvre les trois domaines migrés et
publie `PASS`, `FAIL`, `SKIPPED_BY_PLAN`, `BLOCKED`, `CANCELLED` ou
`NOT_RUN` selon les résultats. Il n’est pas encore le verdict global du
dépôt.

`build-kernel.yml` produit une image BIOS standard par run, dont le
manifeste lie l’image au commit testé, à l’arbre, au verrou Cargo, à la
configuration, à la cible et à la toolchain. Chaque consommateur vérifie
les empreintes avant QEMU. Les deux variantes USB de sécurité restent
construites à part et la provenance Ladybird suit son mécanisme existant.
Les trois campagnes migrées partageaient six constructions BIOS
standard lorsqu’elles étaient toutes sélectionnées. Le plan commun en
prévoit une, en plus des variantes. Ce comptage est structurel. Les
temps, artefacts et minutes réels doivent être comparés sur Actions.

La matrice A/B, OTA et rollback se trouve dans `system-update.yml`.
Native IPC reste le point d’entrée de compatibilité et agrège son résultat
avec les tests IPC. Le workflow historique de production userland reste
consommé par le chemin Windows `-Legacy`.

## Première mesure sur GitHub Actions

Trois anciens runs de `main` (GPU, sécurité et Unix/FS) sont comparés au
run Bouchaud CI de `f5e5fcfaf1d0e3e70eed2ac35d1c0a9c6540efc9`.
Les sources noyau et les recettes d'essai sont identiques. Le SHA embarqué,
le type d'événement, l'attente des runners et les caches diffèrent.
Tous les bancs du nouveau périmètre ont réussi.
[Run Bouchaud CI](https://github.com/bcharthur/bouchaud-os/actions/runs/45850990090) ·
[GPU de référence](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863303) ·
[Sécurité de référence](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863316) ·
[Unix/FS de référence](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863438)

| Indicateur | Référence | Bouchaud CI |
|---|---:|---:|
| Campagnes indépendantes | 3 | 1 |
| Jobs | 7 | 10 |
| Constructions BIOS standard | 6 | 1 |
| Minutes de jobs observées | 42,12 | 37,02 |
| Durée du run le plus long, attente comprise | 12,18 min | 16,73 min |
| Artefacts | 6 | 9 |
| Octets listés par l’API | 6 585 302 | 12 440 439 |

Après cette mesure, le témoin GPU à panne injectée est devenu une
deuxième identité de compilation. Les bras TCG et KVM vérifient le même
témoin, produit séparément de l’image standard.

La durée cumulée des jobs baisse de 12 %, tandis que l’attente et les
dépendances du nouveau run allongent le délai observé. L'image commune
et la publication du plan et du verdict augmentent le volume archivé.
Une répétition appariée est nécessaire pour isoler le coût du graphe de
l'effet de la file d'attente. CI Fast a par ailleurs détecté que son
garde des verdicts négatifs référençait encore l'emplacement initial
du rapport facultatif A/B. Son entrée a suivi le rapport dans
`system-update.yml`, avec la même règle.
[Détection CI Fast](https://github.com/bcharthur/bouchaud-os/actions/runs/45850980263/job/140017211974)

## Windows

`run.ps1` prépare le scénario sous `target/windows-scenarios/`.
L'image finale conserve son emplacement ignoré pour les outils Windows
qui la consomment. Depuis un checkout propre, exécuter
`powershell -File .\run.ps1 -VerifierDepotPropre -PreparerSeulement`
pour contrôler construction et préparation, ou retirer
`-PreparerSeulement` pour contrôler un cycle avec QEMU.
Le contrôle échoue si Git relève une modification à l’entrée ou à la fin.
Le job Windows qui vérifie le vrai artefact exécute aussi la préparation
complète avec ce contrôle. Le cycle avec démarrage et arrêt QEMU
nécessite un hôte de test Windows. La préparation est verte sur le SHA
`45bc53be65c1212ca9f3552aa8f1c66ee7f04e78`.
[Préparation Windows](https://github.com/bcharthur/bouchaud-os/actions/runs/45853872163/job/140027460810)

## Protection et retour arrière

L’API de branche confirme que `main` exige `fast-gate`,
`integration-gate`, `reliability-gate` et `non-regression-gate`.
Ces quatre producteurs gardent leurs noms et déclencheurs. La lecture de
la configuration administrative détaillée renvoie 403 au connecteur.
Pour une migration ultérieure, un administrateur relève la configuration
complète, ajoute le verdict global en exigence supplémentaire après
validation de sa publication sur les SHA PR, vérifie la coexistence sur
les catégories de changements, puis retire les anciens contextes devenus
redondants. En cas de retour, il restaure les anciens contextes avant de
retirer le nouveau. Ce lot n’exige aucun changement de protection.
Pour revenir aux workflows indépendants, rétablir les trois workflows du
parent du commit de migration et retirer l’orchestrateur et le producteur
communs dans le même revert.

La migration des autres contrats, le registre fin, les profils FULL et
CERTIFICATION et la mesure avant/après exhaustive restent à qualifier.
La PR #375 touche déjà la console et #379 dépend de sa branche.
Les PR #369 et #366 touchent le lanceur Windows. La récolte autonome des
slots et piles noyau reste traitée par [#385](https://github.com/bcharthur/bouchaud-os/issues/385).

## Migration P13 et IPC

`bouchaud-ci.yml` appelle directement les quatre jobs historiques de
`os-primitives.yml`. Les déclencheurs P13 de l'ancien wrapper sont repris
comme plancher dans `fabric_compat_paths.json`. Le workflow autonome
`p13-primitives-noyau.yml` est retiré. Les paliers Ladybird et leur
appel strict, ainsi que le workflow manuel des primitives, restent en place.
Les seuils et les recettes des bancs de ce workflow restent identiques.
Les suites IPC et A/B sont désormais lancées par le même orchestrateur.
Leur verdict exige les deux résultats. C22 et la provenance relèvent
toujours du workflow Ladybird.

Les jobs P13 et IPC démarrent après le plan, en parallèle des constructions
BIOS communes aux autres domaines. Le résultat central est limité au
périmètre migré. Son rapport indique `PASS_SCOPE` et
`global_qualification: NOT_RUN` tant que les preuves globales sont absentes.

### Cycle Windows avec QEMU

Depuis un checkout propre du candidat, sous PowerShell Windows avec QEMU
installé, exécuter chaque commande puis fermer proprement l'invité et
vérifier le code de sortie avant de lancer la suivante.

```powershell
powershell -NoProfile -File .\run.ps1 -Accel whpx -CpuCount 1 -VerifierDepotPropre
if ($LASTEXITCODE -ne 0) { throw 'WHPX SMP1 ou propreté Git en échec' }
powershell -NoProfile -File .\run.ps1 -Accel whpx -CpuCount 4 -VerifierDepotPropre
if ($LASTEXITCODE -ne 0) { throw 'WHPX SMP4 ou propreté Git en échec' }
git status --porcelain=v1 --untracked-files=all
```

Relever l'identité du noyau, les cœurs en ligne, la signature invitée,
le choix WHPX sur l'hôte, la première trame et la navigation Ladybird,
puis le code QEMU et la propreté Git. Conserver les journaux sous
`target/journaux-qemu/`.

### Référence console #375

L'agrandissement de l'anneau de 64 à 512 Kio a supprimé les attentes
producteur dans les paires TCG à coût UART injecté de #375. Le volume
transmis, le coût par octet et les gains interactifs restent à mesurer
séparément. Le banc à injection, les mesures ABBA et le diagnostic des
tenues HID sont réutilisables pour la future PR console, après intégration
de CI/CD Fabric. [PR #375](https://github.com/bcharthur/bouchaud-os/pull/375)

`fabric-network.yml` porte désormais les bancs DNS, retransmission TCP,
DHCP et reprise RX. Integration garde l'éligibilité et le verdict
protégé. Le groupe DNS garde la construction instrumentée et démarre
sans attendre l'image BIOS d'Integration. Le groupe DHCP consomme cette
dernière. Les recettes des deux groupes ont été comparées au SHA validé
`018a5287`, étapes et paramètres compris. La convergence des profils
et l'agrégation des preuves des workflows indépendants restent à établir.

Les trois variantes P10 de panne DMA, de réponse étrangère et
d'interruptions s'exécutent à la suite du job GPU de P13, en parallèle
des sondes CPU. Elles gardent leurs compilations distinctes et leurs
bras TCG/KVM. Leurs journaux passent de l'archive
`os-primitives-debug` à `os-primitives-gpu-tampon`. Les chemins de
preuve CPU utilisés par le rapport P13 restent dans l'archive d'origine.


## BIOS partagé et preuve Memory Fabric

Dans Bouchaud CI, P13 utilise l’image BIOS standard attestée du run.
Le job Memory Fabric de faute multithread exécute les bras historiques
TCG SMP1/8 et KVM SMP4 en parallèle des sondes CPU. Ses journaux sont
publiés dans `os-primitives-memory-fault`. Les appels autonomes qui ne
fournissent pas d’image continuent à compiler localement. Les témoins
et toutes les constructions instrumentées conservent leur identité.
La dépendance au producteur BIOS ajoute une attente au démarrage de
P13. Il faut la mesurer avec le temps économisé sur les compilations.

Depuis un clone qui contient la référence main, la commande
`python3 tools/ci/audit_fabric_workflows.py` compare les étapes shell
historiques avec leur environnement et leurs conditions de pas. Elle
signale chaque différence documentée et échoue sur une étape manquante
hors des remplacements déclarés. Ce contrôle structurel doit être
complété par les preuves de déclenchement, de matrice, de provenance
et d’exécution sur GitHub.

Le workflow de qualification Ladybird mutualise aussi le BIOS standard
pour ses 18 recettes QEMU compatibles. Le BIOS se construit pendant la
résolution de provenance du navigateur. Les consommateurs vérifient
l’image du run puis la provenance Ladybird selon le contrat existant.
La production seule de l’artefact natif et les constructions des bras
instrumentés conservent leurs chemins.
