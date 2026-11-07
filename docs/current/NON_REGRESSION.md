# Bouchaud OS — protocole de non-régression v1

Ce protocole produit un verdict **PASS_STATIC** ou **BLOCK** sur un SHA Git
exact. Un PASS_STATIC signifie uniquement que les invariants structurels et
les suppressions détectables ont été contrôlés : **il ne remplace jamais**
un boot QEMU, la qualification P13 stricte, Native IPC Runtime, les 16
scénarios A/B, ni le test physique sur Trigkey.

## Couche 1 : invariants immuables

`tools/ci/non_regression.py` vérifie les protections concrètes déjà perdues
lors de fusions précédentes : diagnostics `AB_DIAG`, relecture d'état
`SERVICED_UPDATE_STATE` après commit, Échap sans extinction du bureau,
corrélation USB au TRB soumis, test de corrélation, contrat de `serviced`/
`logd`, calculatrice ring 3 et existence des verdicts CI.

L'absence ou la modification d'un témoin critique est rouge, y compris si
les autres suites sont vertes. Les tests du garde-fou injectent eux-mêmes
des mutations destructrices pour vérifier qu'elles sont refusées.

## Couche 2 : anti-suppression et anti-relâchement

Sur une PR, le gate compare la base et le HEAD exact avec Git (historique
complet, `--no-renames`) et refuse :

- suppression de tests, gardes, chemins CI protégés ;
- réduction du nombre de fonctions de test d'un fichier de tests existant ;
- modification des budgets/seuils de fiabilité protégés ;
- ajout de `continue-on-error: true` dans les workflows critiques.

Une modification légitime de contrat doit être une migration séparée,
avec justification, remplacement prouvé, tests des cas adverses et évolution
**explicite** de la politique : ne jamais contourner silencieusement ce gate.

## Couche 3 : qualification par domaine

Le rapport JSON indique les workflows *attendus* pour les domaines touchés
(USB/A-B, SMP, GUI, stockage, IPC, boot matériel, sécurité, navigateur).
Cette liste ne déclare PAS que ces workflows ont été exécutés ni réussis.
Avant fusion, vérifier leurs résultats sur le SHA candidat, ainsi que les
gates Fast, Integration, Reliability, et Native IPC lorsqu'il est applicable.
Un test `skipped`, un run annulé, en cours, manquant ou d'un autre SHA ne
compte pas comme preuve. Ne jamais réduire un seuil pour obtenir du vert.

## Activer la protection réelle de main

Le workflow `.github/workflows/non-regression.yml` tourne sur **toutes**
les PR, y compris celles qui modifient uniquement la documentation, et sur
tous les push de main. Pour empêcher les merges non qualifiés, le check
`non-regression-gate` doit aussi être inscrit comme **required status
check** dans la règle de protection de `main`.

`tools/ci/configure_protection.ps1` le prévoit. À lancer depuis un
PowerShell GitHub CLI authentifié, avec les droits d'administration :

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ci\configure_protection.ps1
```

**Sans protection GitHub active, une CI rouge n'interdit techniquement pas
un push direct vers main.** Le workflow n'a pas les droits pour activer
automatiquement cette protection.

## Exécution locale

```bash
python3 -m unittest discover -s tools/ci/reliability -p 'test_non_regression.py' -v
python3 tools/ci/non_regression.py
python3 tools/ci/non_regression.py --base <BASE_SHA> --head <HEAD_SHA>
```

Artefact : `target/non-regression.json`, comprenant les erreurs et exigences
de qualification, sans faux verdict de stabilité physique.
