# CLAUDE — PROCHAINE SESSION (relevé 2026-11-03)

Refaire `git fetch && git rev-parse HEAD` et relire [STATUS_CURRENT.md](STATUS_CURRENT.md) : ce fichier est daté.

## 1. Lire d'abord les verdicts encore ouverts au relevé (candidat `main@c1e55da`)

| Run | Quoi | Si rouge |
|---:|---|---|
| 41592642065 | Ladybird `palier=qualification` (C22 6×6 strict 1024 Kio, primitives P13 strictes, convergence) | SYMPTÔME → journal → cause ; ne pas relancer à l'aveugle |
| 41592677703 | Endurance (tranches 90 min) | idem |
| 41596509318 | C5 KVM (témoin historique rouge attendu, fenêtre verte, image ×24) | un vert du témoin = banc non qualifiant |

Tous verts + Nightly 41592645380 PASS ⇒ seul P13 strict manque à CAP A.

## 2. Bloquants qui exigent l'utilisateur (ne pas les contourner)

1. **Runners self-hosted** `bouchaud-kvm4` et `bouchaud-kvm8` (KVM, cœurs dédiés) → puis `p13-final-candidate.yml` en dispatch. Sans eux, `P13_FINAL_STRICT_OK` est impossible ; ne jamais le simuler avec un runner hébergé.
2. **Protection de main** : appliquer `tools/github/ruleset-main.json` (Settings → Rules → Rulesets → Import, ou `gh api -X POST repos/bcharthur/bouchaud-os/rulesets --input tools/github/ruleset-main.json`).
3. **Campagne PHYSICAL** : [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md), image par `tools/release/build_trigkey_image.sh`.

## 3. Travail suivant, dans l'ordre

1. Fusionner la branche `claude/cap-stable-platform-20261103` (après CI verte) ; requalifier le nouveau HEAD (mêmes gates que §1 + `native-ipc-runtime`, qui exécute désormais `serviced qualify` SMP4+SMP1).
2. **P13 latence interactive SMP1** : IPC natif p50 ≈ 4 ms (aller 1,1 ms / retour 2,5 ms) en mode UP. Instrumenter prêt→élu côté noyau pour les réveils par `WaitSource` (les horodatages `reveil_*` ne couvrent que les sommeils à échéance). Attribuer avant de corriger.
3. **A/B branché** : disposition disque (slot B + bloc), chargeur (`choisit_boot` persisté avant le saut), commit de santé par `serviced` — [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md).
4. **Deuxième service** userland (logging/health → session) sur l'ABI 1.1, en réutilisant `serviced`.
5. Ladybird : retirer les 2 patchs OBSOLETE (décision humaine), puis séparer les DIAGNOSTIC derrière une option avec preuve A/B — [../ladybird/PATCH_MATRIX.md](../ladybird/PATCH_MATRIX.md).
6. CAP C seulement ensuite : P10+ 2D (mesures input→frame), HDA physique.

## 4. Règles qui ont servi cette session

- Une campagne = un SHA ; ne pas pousser sur la branche pendant qu'une qualification y tourne (chaque push annule les runs en cours).
- Un banc peut mentir : `grep -q` sous `pipefail` a failli faire un faux vert (attrapé par `verifie-pipefail-sigpipe.py`). Tester les vérificateurs par mutation.
- Une hypothèse de performance se mesure avant/après : « le quantum de 1 ms cause la latence SMP1 » était fausse.
