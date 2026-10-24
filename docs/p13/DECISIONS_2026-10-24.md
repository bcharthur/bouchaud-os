# P13 — Décisions méthodologiques du 24 octobre 2026

**Statut : décisions de politique adoptées pour le prochain candidat, non encore implémentées dans les portes CI.**  
**Base documentaire :** `audit/p13-p0-v16-20261024` (`14f872f23d662cc90e5d954186250d2b2c967642`).  
**Source :** `docs/current/P13_CLOSEOUT_2026-10-20.md` §6.1(a), §6.2(a) et §6.5 ; `docs/current/REGISTRE_CONVERGENCE.md` (règles 1–4, C4).  
**Périmètre :** critères d'interprétation des mesures. Aucun seuil de latence du noyau n'est abaissé. Aucun `NON_QUALIFIANT` n'est converti en `QUALIFIE_OK`.

## D-6.1(a) — A/B réveil précis : témoin négatif et bras candidat

**Décision : §6.1(a) entérinée.** Le bras `on` (réveil précis activé) est le **seul bras dont le verdict de fonctionnement strict doit être `QUALIFIE_OK`** pour évaluer la politique candidate. Le bras `off` est un **contrôle négatif**, volontairement privé du mécanisme corrigé : il n'a pas à satisfaire la porte historique « la charge dégrade la latence » ni à afficher `QUALIFIE_OK`.

Le contrôle `off` doit toutefois être **valide comme mesure** : phase au repos exploitable, attribution effectivement présente, contamination < 5 %, absence de `QUALIFIE_ECHEC` réel portant sur les propriétés qui lui sont applicables. Les critères comparatifs P1/P2 de §6.5 continuent d'être vérifiés sur les deux bras et ne deviennent pas facultatifs. Une contamination, une absence d'attribution, une preuve manquante ou une dégradation réelle entraîne `NON_QUALIFIANT` ou `QUALIFIE_ECHEC`, jamais un succès automatique.

**Pourquoi :** le closeout du 20/10 constate un bras `off` dont le repos est plus lent que la charge (p95 ≈ 930–980 µs) ; la condition préliminaire de qualification historique est incompatible avec le rôle de ce bras. Exiger `QUALIFIE_OK` pour les deux bras rendait le quorum 0/4 par construction. Le protocole P1–P5 (`fc7e00b1`) doit fournir les verdicts individuels, avec tests négatifs et traçabilité.

## D-6.2(a) — Porte « charge sans effet » : plancher fondé sur le bruit observé

**Décision : §6.2(a) entérinée.** Le plancher historique de 2 µs (2 × grain d'horloge) ne suffit pas pour déterminer si la charge dégrade de façon discriminable la distribution de latence. Pour la propriété **P3 : bénéfice de la priorité**, n'évaluer la comparaison de priorité que lorsque :

```text
p95_charge_normale > p95_repos + bruit_repos
bruit_repos = max(0, p95_repos - p50_repos)
```

Calculer ces valeurs sur des observations propres du **même bras et du même tour**, avec leurs unités, effectifs et exclusions de contamination journalisés. Pas de constante arbitraire de remplacement. La formule de §6.2(a) est le fondement explicite de cette modification de porte, et non un assouplissement des seuils de latence.

Si la différence ne dépasse pas le bruit observé, **P3 = NON_QUALIFIANT (pas d'effet de charge démontrable)** : ni `QUALIFIE_OK`, ni `QUALIFIE_ECHEC` sur la comparaison de priorités. Ce seul motif, documenté, peut être **non bloquant pour le verdict agrégé** si les autres propriétés applicables et la politique `on` sont effectivement qualifiées ; il doit rester visible au registre et dans le closeout. Aucune anomalie réelle P3 démontrée au-dessus du bruit n'est ignorée.

**Preuve citée :** `ed5602f4`, run GitHub Actions `39479633757`, tour 1 A/B précis SMP4 : p95 sous charge normale 99 µs contre 68 µs au repos (écart ≈ 31 µs) ; l'ancienne porte 2 µs s'ouvrait et jugeait ensuite des distributions de l'ordre de 100 µs. Les séries brutes et leur variance doivent être conservées comme preuves ; ces valeurs ne justifient pas à elles seules un nouveau seuil fixe.

## Règles de migration de la porte

1. Garder l'ancien verdict historique **en parallèle** des propriétés P1–P5 jusqu'à modification explicite et testée de l'analyseur de qualification. Ce document autorise l'évolution de la porte, mais ne prétend pas l'avoir implémentée.
2. Ajouter des tests négatifs où le bras `on` perd réellement la précision, où le bras `off` est contaminé, où l'attribution manque, et où P3 échoue avec un écart nettement supérieur au bruit. Tout faux vert interdit la promotion.
3. Avant la première campagne de qualification : tests unitaires de l'analyseur et tests de mutation ; vérifier la stabilité des étiquettes `QUALIFIE_OK`, `NON_QUALIFIANT`, `QUALIFIE_ECHEC` et des statistiques de contamination.
4. Aucun seuil de performance P1/P2/P4/P5, seuil d'attribution, tolérance de contamination ou invariant d'ordonnancement ne change sans nouvelle décision versionnée.

## Critères de sortie P13 actualisés

- Un **seul SHA candidat gelé**, contenant les décisions implémentées et les correctifs effectivement vérifiés, sans nouveau push pendant chaque campagne.
- **Trois campagnes complètes, indépendantes, au même SHA** : les **15 jobs** de convergence doivent réussir, avec `P13_STRICT_OK` sous KVM pour SMP1, SMP2 et SMP4 à chaque campagne ; conserver les identifiants de runs et leurs preuves.
- A/B réveil précis interprété selon D-6.1(a) : politique `on` qualifiée, témoin `off` valable comme contrôle négatif, comparaison P1/P2 attestée ; aucune contamination ni anomalie non évaluée.
- Porte P3 évaluée selon D-6.2(a), sans verdict positif fabriqué lorsque l'effet de charge est indiscernable ; P1/P2/P4/P5 restent jugées suivant leurs règles documentées.
- Registre des **P0 bloquants vide au regard de preuves reproductibles** : C1g, C2, C3 attribués ou corrigés avec négatives ; C5/C6 non clôturables sur la seule absence de reproduction ; C7/C10 et tous autres P0 réellement bloquants traités. Tout reclassement hors P13 nécessite justification explicite, sans masquer le défaut.
- Clôture par un **closeout** référençant SHA gelé, versions du protocole, résultats des trois campagnes, échecs/limites et statut des P0.

**Ce document ne déclare PAS P13 terminé.** Jusqu'à implémentation/validation de la nouvelle porte, les vérifications CI historiques gardent leur sémantique et ne deviennent pas implicitement vertes.

## Règle 5 — discipline d'instrumentation

Aucun nouveau commit d'instrumentation spécifique à un P0 tant que son mécanisme n'est pas appuyé par une reproduction négative ciblée, sauf instrumentation minimale strictement nécessaire à produire cette reproduction, isolée sur une branche expérimentale et justifiée par écrit. Les protections existantes et les tests nécessaires au C11 restent évalués comme correctifs de sûreté, non comme prétexte à multiplier les sondes.

## Traçabilité

- Source §6 : `docs/current/P13_CLOSEOUT_2026-10-20.md`, sections « Décisions humaines requises » et « Protocole A/B en cinq verdicts ».
- Source registre : `docs/current/REGISTRE_CONVERGENCE.md`, règles 1–4 et fiche C4.
- Source bruit : commit `ed5602f4`, run `39479633757`, tour 1.
- Application : **prochaine étape distincte**, vérifiée localement puis sous KVM ; ne pas réétiqueter rétroactivement les anciens runs.
