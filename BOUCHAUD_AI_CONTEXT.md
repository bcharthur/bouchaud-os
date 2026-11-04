# Bouchaud OS — contexte de reprise (humain ou IA)

Ce fichier n'est plus un état du projet : il renvoie aux sources de vérité.

1. `git fetch && git rev-parse HEAD origin/main`.
2. [`docs/current/README.md`](docs/current/README.md) : comprendre le projet en cinq minutes.
3. [`docs/current/status.json`](docs/current/status.json) : instantané daté (SHA de `main`, verdicts, PR ouvertes).
4. [`README.md`](README.md) : présentation, fonctionnalités par niveau de preuve, construction, tests.

**Hiérarchie de vérité** : le code du HEAD, puis les journaux et la CI du **même** HEAD, puis `docs/current/`, puis le reste. Une preuve vaut pour un SHA et un niveau (HOST, TCG, KVM, PHYSICAL), jamais au-delà.

L'ancienne version de ce fichier (octobre 2026, HEAD `3658af04`) est archivée dans [`docs/history/BOUCHAUD_AI_CONTEXT_2026-10.md`](docs/history/BOUCHAUD_AI_CONTEXT_2026-10.md).
