# État courant — relevé du 2026-11-03 (fin de mission ATOMIC PLATFORM)

```text
MAIN_HEAD=d3b55d17429d49daf4284fedf84e5c904a40a9e3   (fusion PR #239)
BRANCHE_TRAVAIL=claude/atomic-platform-20261103      (non fusionnée ; HEAD : git rev-parse origin/claude/atomic-platform-20261103)
MAIN_PROTECTED=false   (ruleset prêt : tools/github/ruleset-main.json)
P13=SMP1/2/4 endurance PASS ; SMP8 PENDING_DEDICATED_KVM8 — BLOCKED_BY_RUNNER_PROVISIONING
PHYSICAL=NOT_STARTED (protocole : TRIGKEY_CAMPAIGN.md §2 et §2bis A/B)
```

Source machine : [status.json](status.json) — **instantané, pas mis à jour automatiquement.**

## Verdicts de la mission ATOMIC PLATFORM (§49–50)

| Verdict | Résultat | Niveau | Preuve |
|---|---|---|---|
| UPDATE_AB_CORE | PASS | HOST | `tools/update/test_{ab,durabilite,passage}.rs` (CI Fast) |
| LOADER_AB | PASS | TCG | `run_ab_rollback.sh` `panique` `gel` `meta-0` `meta-1` `meta-2` |
| BOOT_ATTEMPT_PERSISTENCE | PASS | HOST + TCG | mutation « boot avant flush » détectée ; SIGKILL à chaque boot |
| HEALTH_COMMIT | PASS | TCG | `sante` `B+ B=` |
| AUTO_ROLLBACK | PASS | TCG | `panique`/`gel` `B B B A A`, `sante-ko` `B- B- B- A= A=` |
| UPDATED_RING3 | PASS | TCG | `mise-a-jour` `A* B+ B=` |
| UPDATE_CAPABILITIES | PASS | TCG | 16 refus/contrôles observés en ring 3 |
| SIGNED_MANIFEST | **PARTIAL** | TCG | HASH VALID vérifié sur la machine ; **SIGNATURE TRUSTED : NOT_STARTED** |
| RECOVERY_BOOT | PASS | TCG | `recovery-requise` `R`, `echecs-repetes` `A- A- A- R` |
| RECOVERY_ROLLBACK | PASS | TCG | `recovery-rollback` `R B=` |
| POWER_LOSS_HARNESS | PASS | TCG | `run_coupure_persistance.sh` (SIGKILL hôte) |
| PERSISTENCE_CRASH_CONSISTENCY | PASS (après correctif) | TCG | 12/12 coupures, 0 incohérence, 0 régression ; avant correctif 3/6 régressions |
| LOGD | PASS | TCG | `serviced logd-qualify` SMP4 + SMP1 |
| SERVICE_SUPERVISION | PASS | TCG | `serviced qualify` + `serviced system` (états système, contrat de santé v3) |
| IPC_LATENCY_ATTRIBUTION | PASS | TCG | `ipc-attribution` : SMP1 p50 signal→file 12 µs, **file→élu 1,29 ms**, élu→retour 71 µs |

Aucun verdict n'est KVM ni PHYSICAL. Tous les TCG ont tourné en local **et** sur les runners hébergés (workflow `native-ipc-runtime`, matrice `ab-rollback` de 11 scénarios : run 41637609167 au SHA `1de19fa3`, vert).

## Défauts réels trouvés et corrigés pendant la mission

1. **`fsync` d'un répertoire sans effet** (`src/compat/linux/mod.rs`) : un `rename` acquitté par `fsync(rep)` n'était pas durable — trouvé par le banc de coupure.
2. **Confirmation de santé refusée pendant une installation** : `begin` réécrit META, l'actif ne pouvait plus se confirmer — trouvé par le banc `mise-a-jour`.
3. **Session d'installation non liée au processus** : l'appel en vol d'un `updated` tué terminait dans la session de son successeur.
4. **Relecture monolithique** de 90 Mio dans un seul appel système : battements manqués, `updated` tué — relecture incrémentale.
5. **Banc IPC** : version ABI codée en dur (`ABI=1.1`).

## Ce qui reste bloqué ou non commencé

- `P13_FINAL_STRICT_OK` : runners `bouchaud-kvm4/kvm8` à enregistrer (`tools/ci/runner/provision_kvm_runner.sh`) — action humaine.
- SIGNATURE TRUSTED (chaîne clé de version → manifeste signé → empreinte) : NOT_STARTED.
- Image Trigkey A/B (ramdisk avec `serviced/healthd/updated/logd`) : non intégrée à `build_trigkey_image.sh`.
- Ladybird §39–40 (scripts OBSOLETE, diagnostics derrière une option) : NOT_STARTED dans cette mission.
- PHYSICAL : NOT_STARTED.

Prochaine étape exacte : [CLAUDE_NEXT_MISSION.md](CLAUDE_NEXT_MISSION.md).
