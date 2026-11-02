# Remise en ordre — 2026-11-02

## Avant / après

| Sujet | Avant | Après cette préparation |
|---|---|---|
| Entrée documentaire | `README.md` renvoyait à P13 octobre | `README → STATUS → ARCHITECTURE → LIMITATIONS → PROOFS → CLAUDE` |
| Statut | `STATUS_CURRENT.md` daté du 21/10 | état horodaté 02/11, UNKNOWN explicites |
| CI | 17 workflows productifs, anciens C22 archivés | aucun critère modifié ni workflow supprimé dans cette préparation |
| Branches | 6 branches + tag de forensics C22 | aucune suppression supplémentaire sans revue de commits uniques |
| Provenance | non centralisée | runs et SHA de référence dans CI_AND_PROOFS |
| C22 | correctif intégré sans dossier causal unique | mini dossier causal + état CLOSED_AT_KVM borné |
| P13 | historiques nombreux et contradictoires | PARTIAL + un point d'entrée et protocole strict |
| P10+/Ladybird | descriptions inégales | chemins de code et niveaux de preuve explicités |

## Décisions sûres réalisées

Six pages actives consolidées, statut JSON, rapport de remise en ordre et outil de vérification de statut. **Zéro modification noyau/driver/browser, zéro seuil abaissé, zéro changement de quorum.**

Les historiques dans `docs/current` **ne sont pas supprimés** : les liens et preuves peuvent être encore consommés par scripts. La nouvelle README précise qu'ils ne sont plus normatifs. L'objectif de réduction de l'entropie est documentaire et n'implique pas de réécrire l'histoire.

## Réserves et audit restant

- Le SHA principal est daté ; les nouveaux commits du dépôt après ce relevé doivent être repérés par `git rev-parse HEAD`.
- `main` n'a pas de protection — défaut de processus P0. Recommandation : PR requise, checks `fast-gate`, `integration-gate`, `reliability-gate` stables ; pas de force push, pas de suppression. Les autres checks (IPC/security/browser) doivent être obligatoires seulement si les chemins concernés les déclenchent de manière garantie, sinon les rulesets peuvent bloquer les PR sans rapport. Configuration administrative non réalisée.
- Le niveau PHYSICAL Trigkey sur HEAD actuel reste UNKNOWN.
- Le gate final P13 est présent ; son exécution réussie et le couple runners KVM4/KVM8 ne sont pas prouvés.
- `docs/current/status.json` est un **snapshot**, jamais un substitut à GitHub. L'outil de vérification le refuse si le HEAD local diffère.
- Aucun nettoyage destructif complémentaire n'a été entrepris.
