# CLAUDE — PROCHAINE SESSION (relevé 2026-11-03, fin de mission ATOMIC PLATFORM)

Refaire `git fetch && git rev-parse HEAD` et relire [STATUS_CURRENT.md](STATUS_CURRENT.md) : ce fichier est daté.

## 1. État de départ

Branche `claude/atomic-platform-20261103` (non fusionnée, aucune PR ouverte au relevé). Chaîne A/B complète prouvée sous QEMU/OVMF (TCG) : chargeur, passage, commit de santé, rollback, `updated` ring 3, capacités, recovery, échecs répétés, coupures pendant l'écriture atomique, `logd`, attribution IPC. Détail et preuves : [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md).

## 2. Bloquants qui exigent l'utilisateur (ne pas les contourner)

1. **Runners self-hosted** `bouchaud-kvm4` / `bouchaud-kvm8` : `sudo tools/ci/runner/provision_kvm_runner.sh kvm8 [--install]` sur l'hôte dédié, puis l'enregistrement imprimé (jeton à usage unique). Sans eux `P13_FINAL_STRICT_OK` reste impossible (`BLOCKED_BY_RUNNER_PROVISIONING`) ; SMP8 reste `PENDING_DEDICATED_KVM8`.
2. **Protection de main** : `tools/github/ruleset-main.json`.
3. **PHYSICAL** : [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md) §2 et §2bis (A/B AB1–AB8).

## 3. Travail suivant, dans l'ordre

1. Ouvrir la PR de la branche, qualifier son HEAD (CI Fast, Integration, Reliability V3, Native IPC avec la matrice `ab-rollback` + coupures, Ladybird qualification, Nightly) sur le **même SHA** ; fusion sur décision humaine.
2. **Clé de version de production** : la chaîne (Ed25519 `no_std`, manifeste canonique, vérification noyau au `begin`) est en place et prouvée avec une clé de dev de banc. Créer la clé hors dépôt (HSM ou machine hors ligne), commiter sa seule moitié publique dans `cles_de_confiance`, signer les manifestes dans `release.yml` sans que la clé privée touche un runner hébergé (décision humaine).
3. **Image Trigkey A/B** : `build_trigkey_image.sh` doit produire le disque A/B avec `serviced/healthd/updated/logd` dans le ramdisk (sinon tout candidat est retiré — voulu).
4. **Latence SMP1** : l'attribution (`ipc-attribution`) place l'essentiel dans `file→élu` (p50 ≈ 1,3 ms TCG SMP1), pas dans le chemin de réveil (≈ 12 µs). Proposer, mesurer avant/après, une préemption du réveilleur au profit d'un réveillé interactif — sans toucher aux seuils, et seulement avec cette attribution comme référence.
5. `logd` : faire passer `healthd` et `updated` par lui (canal d'intake par service), persistance optionnelle bornée.
6. Ladybird (§39–40 de la mission) : retrait des patchs OBSOLETE et diagnostics derrière une option — non fait dans cette mission.
7. CAP P10+ seulement ensuite.

## 4. Règles qui ont servi cette session

- Un banc rouge sur sa propre qualification se lit avant d'être « corrigé » : `logd-qualify` comptait mal (`received = accepted + 3` : les événements du superviseur), le correctif était dans le test, et une tautologie (« producteur jamais bloqué ») a été remplacée par un vrai décompte.
- Le banc de coupure a trouvé un vrai défaut produit (`fsync` d'un répertoire sans effet) ; le banc A/B en a trouvé trois (confirmation refusée pendant une installation, session d'installation non liée au processus, relecture monolithique qui affame les battements).
- Ne pas modifier l'arbre pendant qu'un banc construit ; les bancs reconstruisent à chaque scénario.
- Une coupure peut tronquer la dernière ligne série : ne compter que les lignes complètes.
