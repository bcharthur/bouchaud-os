# P13 — correctif de vérité de qualification (base e0a5d8c)

Ce patch ne modifie **pas** le noyau ni les seuils de latence. Il ferme un
faux vert diagnostiqué dans le run 38757205092 : les huit bras KVM A/B de
réveil précis étaient NON_QUALIFIANT mais l'analyseur utilisait leur médiane
pour annoncer CONVAINCANT (SMP4). SMP1 rendait NON_CONVAINCANT sur p95.

## Effet

- Les statistiques A/B et le quorum ne portent désormais que sur les tours
  dont **les deux** bras sont `QUALIFIE_OK`. Aucune donnée supprimée : les
  valeurs des bras rejetés restent imprimées pour diagnostic.
- Si quorum insuffisant : `NON_CONCLUANT`, **jamais** `CONVAINCANT`.
- Le workflow `os-primitives.yml` publie toujours un rapport de qualification
  dans les job logs et le GitHub Step Summary, y compris si les jobs techniques
  restent verts.
- Lancer avec `p13_strict=true` transforme ces rapports en gates bloquants
  sur **l'attribution KVM SMP1/2/4** et l'**A/B KVM SMP1/4**.
- L'IPI expérimental et la cession de classe sont des bancs de recherche ;
  leurs verdicts NON_CONCLUANT ne valent jamais qualification de la baseline.

## Commandes après application, sans lancer deux campagnes concurrentes

```
python -m unittest discover -s tools/ci/reliability -p 'test_ab_*.py' -v
python -m unittest discover -s tools/ci/reliability -p 'test_qualifie_p13.py' -v
python tools/verifie-reveil-precis.py
git diff --check
```

Après revue, commit/push normal sur la branche unique (sans force-push), puis :

```
gh workflow run os-primitives.yml --ref claude/ladybird-observability-performance -f p13_strict=true
```

**Le premier run STRICT est censé être ROUGE** si SMP1/2/4 reste
NON_QUALIFIANT. Ne changer ni critères ni verdicts pour l'obtenir vert.

## Encore ouvert : racine des latences

Il reste à comprendre les réveils tardifs (> ~1 ms), les phases `detection`,
`election` et `reprise`, le vol du temps prouvé par KVM, et à qualifier une
endurance SMP8 sur un hôte disposant réellement de huit coeurs disponibles.
Un runner TCG avec quatre coeurs hôtes ne fournit pas cette preuve.

Autres sujets explicitement **non traités par ce patch** :
`persist-course-probe`, assertion LibJS `Value.cpp:484`, panique lockdep
`courant.rs:276`, sorties Compositor intermittentes, RTL8168 sur Trigkey,
pilote AMD, vrai WebGL/3D et essais physiques. La baseline globale reste
NOT_ESTABLISHED jusqu'aux preuves reproductibles sur un SHA donné.
