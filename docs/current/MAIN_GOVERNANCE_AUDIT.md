# Gouvernance de main — actions humaines requises

La PR de ce chantier apporte un **audit en lecture seule**, pas une
protection effective. Le connecteur de développement n'applique pas des
rulesets d'administration.

## Première action après revue

Sous **Settings → Rules → Rulesets**, importer
`tools/github/ruleset-main.json`. Relire les checks nécessaires avant de
l'activer : `fast-gate`, `integration-gate` et `reliability-gate`.
Interdire push direct, force-push et suppression de `main` ; exiger une PR.

## Preuve indépendante

Lancer Actions → **Governance audit (manuel)** → Run workflow, sur `main`.
Le rapport `governance-report.json` indique le HEAD observé et l'état réel
de protection, ainsi que l'ancienneté de
`docs/current/status.json`. Un code de retour 2 signifie **NON QUALIFIÉ**,
même si des CI produit sont vertes.

`status.json` est un snapshot daté, pas une API de vérité en temps réel :
ne jamais modifier seulement `observed_main_head` pour fabriquer un vert.
Actualiser également les PR ouvertes, anomalies, preuves et verdicts.
L'audit est uniquement un contrôle ; il n'écrit aucune donnée distante.

Le rapport ne prétend pas vérifier l'intégralité des paramètres de ruleset :
la propriété GitHub `branches/main.protected` est le témoin minimal, à
compléter par l'examen humain du ruleset importé.
