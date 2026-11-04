# Bouchaud userland natif

Les composants ring 3 **natifs** de Bouchaud OS, écrits contre l'ABI native ([`docs/current/NATIVE_ABI.md`](../docs/current/NATIVE_ABI.md)) et non contre la compatibilité POSIX.

## Ce qui existe

| Chemin | Rôle | Preuve |
|---|---|---|
| `libs/bouchaud-sdk/` | en-têtes C de l'ABI native (`abi.h`) et enveloppes (`sdk.h`) | `tools/ci/verifie_sdk_abi.py` (le SDK décrit exactement le noyau, testé par mutation) |
| `services/serviced/` | superviseur : lance les services, déclare la santé du système, déclenche le commit A/B | QEMU TCG, workflow `native-ipc-runtime` |
| `services/healthd/` | service critique minimal de santé | idem |
| `services/updated/` | installe un slot A/B à partir d'un manifeste signé (capacité de mise à jour seule) | idem, matrice `tools/ci/run_ab_rollback.sh` |
| `services/logd/` | journal système | idem |
| `services/composited/` | prototype de compositeur en espace utilisateur (une tranche) | prototype : le bureau réel est encore dans le noyau |
| `tests/` | sondes natives (`native-cycles-probe`, `native-process-probe`) et un `healthd` défaillant pour le banc A/B | CI |

Ils sont construits par la CI avec la chaîne du SDK (voir `.github/workflows/ci.yml` et `native-ipc-runtime.yml`).

## Ce qui est prévu, sans exister encore

- une bibliothèque graphique (surfaces, scène) et un toolkit UI ;
- un compositeur complet en espace utilisateur ;
- des services réseau hors du noyau ;
- des applications first-party natives.

Le bureau et la chaîne navigateur actuels ne sont pas déplacés d'un coup : ils migreront composant par composant, à mesure que les contrats noyau/userland se stabilisent ([ROADMAP_CURRENT](../docs/current/ROADMAP_CURRENT.md)).

Le userland **historique** (musl, CPython, Qt, QuickJS, ancien navigateur) est ailleurs : [`tools/userland/`](../tools/userland/README.md).
