# CI et preuves — audit horodaté 2026-11-02

## Contrat de preuve

STATIC → HOST → TCG → KVM → PHYSICAL. Pas d'inférence entre niveaux. `NON_QUALIFIANT`, `INCONCLUSIF`, `cancelled`, `skipped` et une preuve absente **ne sont pas PASS**. Une CI de développement Ladybird n'est pas le palier `qualification` du workflow.

| Workflow | Run | SHA | Event | Résultat au relevé | Qualification réelle / limites |
|---|---:|---|---|---|---|
| CI Fast | 41534263901 | `ba4244fea534` | push main | in_progress | Garde syntaxe, hôte et politique ; ne qualifie pas SMP8/physique |
| Reliability V3 | 41534263911 | `ba4244fea534` | push main | in_progress | Matrice, tests hôte/fiabilité ; ne suffit pas au gate P13 |
| CI Fast | 41532849519 | b8436617 | push main | success | SHA antérieur ; pas preuve du HEAD |
| Reliability V3 | 41532849530 | b8436617 | push main | success | SHA antérieur |
| Integration | 41532849374 | b8436617 | push main | success | SHA antérieur |
| CI Fast | 41530447782 | `5c3cd43a7a2a` | push main | success | Gate court |
| Integration | 41530447736 | `5c3cd43a7a2a` | push main | success | QEMU/assemblage, pas qualification physique |
| Reliability V3 | 41530447906 | `5c3cd43a7a2a` | push main | success | Tests de fiabilité |
| Native IPC Runtime | 41530447729 | `5c3cd43a7a2a` | push main | success | IPC natif ; pas preuve filesystem ou driver physique |
| Security Runtime | 41530447803 | `5c3cd43a7a2a` | push main | success | Tests de sécurité ciblés |
| ladybird-native-browser | 41530447823 | `5c3cd43a7a2a` | push main | in_progress | Build/diagnostic produit, non conclusion finale au relevé |
| p13-primitives-noyau | 41530447865 | `5c3cd43a7a2a` | push main | in_progress | Ne confondre avec gate strict final |
| userland-legacy | 41530447851 | `5c3cd43a7a2a` | push main | in_progress | Fabrique legacy, distinct des services ring3 canoniques |
| ladybird-native-browser | 41518023294 | `5c3cd43a7a2a` | pull_request | success | C22 6/6 KVM ; palier développement, **pas** convergence P13 |
| Nightly Full | 41424125876 | 788c5446 | schedule | failure | Artefact Ladybird absent ; ancien SHA |
| Nightly Full | 41475434027 | 47d96f0c | workflow_dispatch | success | Autre SHA V26 : ne qualifie pas main |
| Endurance hebdomadaire | UNKNOWN | UNKNOWN | schedule/manual | UNKNOWN | Aucune preuve actuelle récupérée sur ce HEAD |
| P13 dedicated KVM | UNKNOWN | UNKNOWN | dispatch | UNKNOWN | Rien de concluant sur ce SHA |
| P13 final | UNKNOWN | UNKNOWN | dispatch | UNKNOWN | `P13_FINAL_STRICT_OK` non démontré |

Les identifiants ci-dessus sont des **runs réels**, pas des simulations. Les durées détaillées, tous les jobs skipped et la provenance binaire au niveau artefact restent **UNKNOWN** lorsqu'ils n'ont pas été inspectés individuellement. En particulier, l'artefact de `41518023294` doit être vérifié par `tools/ci/verifie_artefact_ladybird.sh` lors de toute réutilisation.

## C22 V29 — dossier causal compact

```text
SYMPTÔME   : augmentation intermittente du RSS WebContent après répétitions de fermetures d'onglets.
OBSERVATION: ancien candidat B4 +1252 Kio > 1024 ; V29 candidat M2-M1 144..164 Kio.
HYPOTHÈSES: mimalloc/decommit, références Document, Request callbacks, streams et GC ; les tests
            antérieurs ne permettent PAS d'exclure universellement toutes ces contributions.
CAUSE RETENUE COMME CANDIDAT: racines fortes conservées après réponse réseau normalement terminée,
            et timer de curseur capturant potentiellement Document après destruction.
CORRECTIF:  tools/ladybird/prepare-echange-processus.py, sections
            BOUCHAUD_C22_V29_RELEASE_COMPLETED_REQUEST_ROOTS et DROP_DESTROYED_DOCUMENT_TIMER.
            defer_teardown() après user_on_finish ; stop + detach timer et garde anti-restart.
AVANT      : témoin historique intermittent ; GitHub 41511176390 6/6 candidat vert,
            0/6 témoin rouge => ancien gate échouait sans échec produit.
APRÈS      : 41518023294, 6/6 KVM sur V29, croissance 144..164 Kio, borne 1024 Kio.
LIMITES    : pas de preuve PHYSICAL ; pas de preuve que toute fuite générale est supprimée ;
            durée/couverture limitées à la fixture et aux 10 onglets/phase.
```

**Gate produit** : `BO_QEMU_KVM=1`, 6 essais, 10 onglets par phase, `MEMOIRE_PROCESSUS ... WebContent ... verdict=borne`, `LADYBIRD_MEMOIRE_OK`, seuil **inchangé 1024 Kio**, source et artefact liés au SHA. **Banc causal** : témoin historique hors gate bloquant parce qu'intermittent. Le conserver comme recherche séparée pour observer la sensibilité, sans exiger une fuite artificielle.

## Points de surveillance des faux verts

- `pull_request` peut exécuter un SHA de merge synthétique ; noter HEAD de branche, `github.sha`, parent testé et manifeste Ladybird.
- `workflow_dispatch` sur `main` vs ancien SHA : même nom de workflow ne prouve pas identité.
- `if: always()`, `skipped` et déduplication : vérifier le job de convergence et sa sortie, pas la couleur générale seule.
- Des `|| true` peuvent être nécessaires pour **collecter des logs**, jamais pour convertir une erreur en succès. Auditer leur placement avant toute modification.
- Manque d'artefact, de marqueur, de mesure M0/M1/M2, de QEMU, ou de contrôle d'identité = inconclusif/rouge, pas zéro.
- `tools/ci/analyse_memoire.py` exige des échantillons stables, le même processus et une fenêtre M2-M1 ; `g2 > 1024` est le critère lu dans le code.
- Ne pas requalifier P13 avec `P13_STRICT=0` ou un runner KVM4 hébergé utilisé comme SMP8.

Les 17 workflows actuellement conservés dans le dépôt : ci, reliability-v3, integration, native-ipc-runtime, security-runtime, ladybird-native-browser, ladybird-sync, os-primitives, p13-primitives-noyau, p13-cible-kvm, p13-dedicated-kvm, p13-final-candidate, nightly, endurance, soak, release, userland. Aucun n'a été supprimé pendant cette préparation.
