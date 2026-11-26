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
[Journal source](https://github.com/bcharthur/bouchaud-os/actions/runs/45841863506/job/139985378606)

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

Ces contrats sont bloquants quand le plan les sélectionne. Les commandes,
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

## Windows

`run.ps1` prépare le scénario sous `target/windows-scenarios/`.
L'image finale conserve son emplacement ignoré pour les outils Windows
qui la consomment. Depuis un checkout propre, exécuter
`powershell -File .\run.ps1 -VerifierDepotPropre -PreparerSeulement`
pour contrôler construction et préparation, ou retirer
`-PreparerSeulement` pour contrôler un cycle avec QEMU.
Le contrôle échoue si Git relève une modification à l’entrée ou à la fin.
Le cycle complet nécessite Windows, QEMU et l’artefact Ladybird.

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
