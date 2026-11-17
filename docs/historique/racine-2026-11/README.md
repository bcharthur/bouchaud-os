# Artefacts retirés de la racine (17/11/2026)

Ces fichiers servaient à appliquer à la main des correctifs livrés en archive ZIP, ou consignaient un résultat ponctuel. Aucun code, aucun workflow et aucun script actif ne les appelle. Ils sont gardés ici tels quels, hors de `tools/`, que les tests hôte et le garde de compilation Python parcourent.

`gate0/` reste à la racine : il contient un fichier de test (`test_commutation_final.rs`), et la barrière de non-régression compare sans renommage (`--no-renames`). Déplacer un test y compte comme le supprimer, et cette règle n'est pas assouplie pour un rangement.

| Fichier | Ce que c'était |
|---|---|
| `APPLY-PAGE-LIFECYCLE-A36B3E4.ps1`, `PATCH-README.txt` | patch M11 « page lifecycle » pour la base `a36b3e4` de la branche `claude/rtl8168-ladybird-internet` |
| `DOCS_ADD_ONLY.json` | consigne d'un dépôt de documentation additive (branche `claude/ladybird-observability-performance`) |
| `gate0-results/` | résultat de la porte 0 du 26/08 (commutation finale, `gate0/` à la racine), rejouée par `tools/historique/RUN-GATE0-FINAL.ps1` |
| `gate1a/` | dégâts épars de la porte 1A (`tools/historique/MEASURE-GATE1A.ps1`) |
| `internal/`, `payload/` | scripts de patch ponctuels (plateforme SMP4, vivacité de WebContent) |
