# `tests/`

Ce dossier est vide : il n'est pas l'endroit où vivent les tests de Bouchaud OS.

| Tests | Où |
|---|---|
| tests hôte des modules purs du noyau | `tools/**/test_*.rs`, lancés par `tools/ci/run_host_tests.sh` |
| garde-fous d'architecture | `tools/verifie-*.py`, `tools/*/verifie-*.py`, lancés par `tools/ci/run_architecture_guards.sh` |
| bancs QEMU | `tools/ci/run_*.sh` |
| sondes ring 3 | `userland/tests/`, `tools/userland/` |

Vue d'ensemble : [README principal](../README.md#tests-et-ci).
