# Matrice de la pile de patchs Ladybird (générée)

**Source :** `python3 tools/ladybird/inventaire_patchs.py --verifie --md …` — ce tableau est **généré** depuis `tools/ladybird/prepare-*.py`, `tools/ladybird/browser-upstream.sh` et `tools/ci/`. Le régénérer plutôt que l'éditer. Détail par script (fichiers upstream exacts, marqueurs vérifiés) : [PATCH_MATRIX.json](PATCH_MATRIX.json). Upstream épinglé : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`.

La **catégorie est proposée** à partir de signaux explicites (nom, chemins) et reste « à confirmer » par revue ligne à ligne ([AUDIT_PREPARATEURS.md](AUDIT_PREPARATEURS.md), état 2026-10-08, périmé : 18 préparateurs appliqués alors, 23 aujourd'hui). Aucune suppression n'est décidée ici.

| # | Script | Appliqué | Octets | Fichiers upstream | Ancres | Marqueurs émis / vérifiés CI | Catégorie proposée |
|---|---|---|---:|---:|---:|---|---|
| 1 | `prepare-browser-source.py` | oui | 4269 | 3 | 4 | 0 / 0 | PLATFORM |
| 2 | `prepare-m9-source.py` | oui | 7345 | 2 | 4 | 6 / 1 | TEMPORARY |
| 3 | `prepare-m9-diagnostics.py` | oui | 16845 | 3 | 5 | 15 / 0 | DIAGNOSTIC |
| 4 | `prepare-m16-dns.py` | oui | 11196 | 1 | 2 | 11 / 0 | UPSTREAMABLE |
| 5 | `prepare-dns-une-question.py` | oui | 4800 | 3 | 0 | 5 / 0 | UPSTREAMABLE |
| 6 | `prepare-fonts-systeme.py` | oui | 3143 | 1 | 1 | 0 / 0 | PLATFORM |
| 7 | `prepare-v16-fonts.py` | oui | 8149 | 3 | 4 | 0 / 0 | PLATFORM |
| 8 | `prepare-tls-diagnostic.py` | oui | 8326 | 2 | 2 | 0 / 0 | DIAGNOSTIC |
| 9 | `prepare-browser-runtime-link.py` | oui | 11026 | 7 | 6 | 0 / 0 | PLATFORM |
| 10 | `prepare-sandbox-bouchaud.py` | oui | 2122 | 0 | 2 | 0 / 0 | PLATFORM |
| 11 | `prepare-audio-bouchaud.py` | oui | 2560 | 3 | 3 | 1 / 0 | PLATFORM |
| 12 | `prepare-worker-terminate.py` | oui | 8249 | 2 | 2 | 1 / 0 | UPSTREAMABLE |
| 13 | `prepare-compositor-lien.py` | oui | 34647 | 4 | 10 | 20 / 6 | PLATFORM |
| 14 | `prepare-control-channel-diagnostics.py` | oui | 11357 | 7 | 1 | 1 / 0 | DIAGNOSTIC |
| 15 | `prepare-compositor-memoire.py` | oui | 8164 | 1 | 3 | 1 / 1 | UPSTREAMABLE |
| 16 | `prepare-echange-processus.py` | oui | 30698 | 18 | 17 | 14 / 2 | UPSTREAMABLE |
| 17 | `prepare-c22-partitions.py` | oui | 9035 | 2 | 8 | 2 / 2 | UPSTREAMABLE |
| 18 | `prepare-gc-retention-proof.py` | oui | 14072 | 4 | 4 | 2 / 2 | DIAGNOSTIC |
| 19 | `prepare-cache-journal.py` | oui | 5723 | 1 | 1 | 6 / 2 | DIAGNOSTIC |
| 20 | `prepare-libjs-valeur-diagnostic.py` | oui | 3019 | 1 | 1 | 1 / 0 | DIAGNOSTIC |
| 21 | `prepare-full-browser-host.py` | oui | 20987 | 6 | 8 | 0 / 0 | PLATFORM |
| 22 | `prepare-network-live.py` | oui | 5576 | 5 | 8 | 0 / 0 | PLATFORM |
| 23 | `prepare-ui-bouchaud.py` | oui | 3629 | 3 | 3 | 1 / 0 | PLATFORM |
| 24 | `prepare-m13-dns-diagnostics.py` | **non** | 11454 | 1 | 2 | 19 / 0 | OBSOLETE |
| 25 | `prepare-m14-dns-retry-fix.py` | **non** | 3363 | 1 | 2 | 1 / 0 | OBSOLETE |

## Ce que la génération établit (2026-11-03)

- **23/25 appliqués**, dans l'ordre de `browser-upstream.sh`. **2 hors chaîne** (`prepare-m13-dns-diagnostics.py`, `prepare-m14-dns-retry-fix.py`) : OBSOLETE, déjà recommandés à la suppression par l'audit du 08/10 ; conservés tant que leurs analyseurs (`analyse-m13/m14-dns-log.py`) le sont. `--verifie` échoue si un nouveau script apparaît hors chaîne sans être déclaré.
- **18/25 sans aucun marqueur vérifié par un banc CI** : leur effet n'est prouvé que par le build et les tests fonctionnels globaux (smoke, WPT, sites). Ce n'est pas un défaut en soi pour PLATFORM (intégration), mais c'est la liste des patchs dont une régression silencieuse ne serait pas nommée.
- Les plus gros patchs (`prepare-compositor-lien.py` 35 Kio, `prepare-echange-processus.py` 31 Kio / 18 fichiers upstream) concentrent le risque de mise à jour upstream.

## Trajectoire (une conversion à la fois, prouvée équivalente)

```text
25 patchers texte → SHA épinglé + couche plateforme Bouchaud + petite pile de patchs explicites + diagnostics optionnels
```

1. Retirer les OBSOLETE (après décision humaine sur leurs analyseurs).
2. Séparer les DIAGNOSTIC derrière une option de build (`BOUCHAUD_LADYBIRD_DIAG=1`), le produit ne les embarquant plus par défaut — exige une preuve A/B (mêmes résultats smoke/WPT/C22 avec et sans).
3. Convertir un UPSTREAMABLE en patch `git format-patch` contre le SHA épinglé, prouvé identique au résultat du patcher (diff vide de l'arbre généré), puis le proposer upstream.
