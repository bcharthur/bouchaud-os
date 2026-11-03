# CLAUDE — PROCHAINE SESSION

Refaire `git fetch && git rev-parse HEAD origin/main` et lire [status.json](status.json) (source de vérité) puis [STATUS_CURRENT.md](STATUS_CURRENT.md).

## 1. Avant tout nouveau chantier

- Si la PR Atomic Platform n'est pas fusionnée : lire ses checks sur le **merge synthétique**. Tout rouge est lu dans le job puis le journal avant d'agir.
- Si `SIGCHLD_MULTIFIL_ECHEC` réapparaît sur un `p13-primitives-noyau` : le journal du job contient désormais le cas fautif et les sorties des fils (`BOUCHAUD_SIGCHLD_ECHEC_LISIBLE_V1`). Classer **avant** toute autre chose ; banc ciblé : `tools/ci/run_sigchld_repetition.sh BOOT N SMP` (avec `SIGCHLD_STRACE=1`).

## 2. Bloquants humains (ne pas contourner)

1. Runners `bouchaud-kvm4` / `bouchaud-kvm8` ([BOUCHAUD_STABLE_BASELINE.md](BOUCHAUD_STABLE_BASELINE.md)).
2. Protection de `main` : ruleset prêt, application par un administrateur.
3. Clé de version de production : hors ligne / HSM, rotation, sauvegarde, révocation. Ne **jamais** en générer une pour passer un test.
4. Campagne PHYSICAL ([TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md)).

## 3. Candidats de chantier, à choisir après la fusion (pas avant)

- Image Trigkey complète avec les services natifs (`serviced`, `healthd`, `updated`, `logd` dans le ramdisk) : prérequis de toute étape physique A/B.
- Campagne Trigkey physique.
- Runners P13 dédiés.
- Anti-downgrade : version monotone signée, plancher persisté après commit de santé, sans casser le rollback local ([UPDATE_RECOVERY.md](UPDATE_RECOVERY.md) §Authenticité ≠ anti-rollback).
- P10+ / fluidité ; latence SMP1 (`file→élu` attribuée, non corrigée).

## 4. Règles qui ont servi

- Une campagne = un SHA ; ne rien pousser pendant qu'une qualification tourne.
- Un test qui passe après mutation du code qu'il protège ne protège rien : la mutation du refus de clé de petit ordre a révélé un test insuffisant.
- Comparer une implémentation de confiance à **deux** références reconnues ; « comme OpenSSL » n'est pas « correct » (OpenSSL 3 accepte la contrefaçon universelle sur clé de petit ordre).
- Ne pas modifier l'arbre pendant qu'un banc construit ; une charge parallèle sur l'hôte peut faire échouer les sondes **côté hôte** (`compta-stress-probe`) : ce n'est pas un échec Bouchaud.
