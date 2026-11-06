# Bouchaud OS — preuves de qualification en direct (lecture seule)

## Pourquoi
`docs/current/status.json` est un **instantane historique** (`updates_automatically=false`). Il ne saurait prouver la stabilité du SHA qui se trouve aujourd'hui sur `main`. Un pipeline vert issu d'un autre SHA ou un `Reliability V3` vert n'établissent pas la qualification P13, la durabilité A/B ni les tests PHYSICAL.

## Collecte
`python3 tools/health/release_truth.py --output target/release-evidence.json`

La commande lit via l'API GitHub :
- `branches/main.commit.sha` et `protected` ;
- l'historique des runs du **SHA exact** (pas le dernier run vert arbitraire) ;
- la liste des issues critiques encore ouvertes ;
- le statut historique, avec un drapeau explicite si son SHA diffère.

Elle publie `release_verdict=NO_GO` tant qu'une preuve requise manque ; `GO_FOR_REVIEW` ne constitue toujours PAS une autorisation de flash ni un label `STABLE`. Le workflow `Release evidence (report-only)` archive ce JSON et un résumé lisible. Son état vert signifie seulement **rapport correctement produit**.

Aucune règle CI existante ni aucun seuil P13 n'est modifié par ce rapport.

## Éprouver la logique sans GitHub
`python3 -m unittest discover -s tools/ci/reliability -p 'test_release_evidence.py' -v`

L'option `--fixture exemple.json` fournit un jeu de données indépendant des appels réseau ; `--strict` retourne le code 2 si verdict différent de `GO_FOR_REVIEW`. **Ne pas ajouter `--strict` au workflow de rapport tant que le système est expérimental** : il rendrait le job rouge par définition sans corriger les causes. La qualification finale doit employer ses propres gates existantes, sur un SHA immuable.

## Limitations conservées et visibles
- CI Fast/Integration/Reliability/Native IPC verts sur un même SHA : **nécessaire, non suffisant**.
- Ladybird native, Nightly, P13 Final Strict, protection `main`, Trigkey PHYSICAL : exigés séparément.
- Le rapport ne télécharge pas les journaux de QEMU pour faire de l'attribution causale ; cela relève d'un diagnostic distinct.
- Les éventuels runs absents ou pages de résultats tronquées ne sont jamais considérés verts.
- Aucune issue #262 (USB/BOT) ou #264 (SMP) ne doit disparaître de l'acceptation sans preuve documentée.

Le test PHYSICAL Trigkey demeure **NOT_STARTED** jusqu'à collecte réelle, pas tant que l'image démarre seulement dans QEMU.

## Chronologie locale des incidents de boot

```bash
python3 tools/health/incident_timeline.py serial.log \
  --output target/incident-timeline.json
python3 -m unittest discover -s tools/ci/reliability -p 'test_incident_timeline.py'
```

Le JSON ordonne les incidents USB/BOT, l'état A/B, les refus de commit, les paniques SMP et les messages `LOGD` par ligne, session boot et service. Il compte les pertes déclarées par logd et signale toute troncature. Il **ne déduit pas une causalité** d'une simple succession temporelle : il faut une vraie corrélation dans les événements noyau/service pour établir la chaîne complète. Les commandes de terminal et champs non reconnus ne sont pas recopiés.
