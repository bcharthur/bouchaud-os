# Baseline stable — Bouchaud OS

**NOT_ESTABLISHED (2026-11-03).** Aucun tag de release ne doit être posé.

## Candidat évalué

| Champ | Valeur |
|---|---|
| Bouchaud SHA | `c1e55da99dcc411f45ee68ebd285d91205f82067` (runtime = `5c3cd43a`) |
| Ladybird SHA | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` (`third_party/UPSTREAM.md`) |
| Toolchain | `nightly-2026-06-01` (rustc 1.98.0-nightly 14210df0e) |
| Build | bootimage legacy `dev` (CI) ; image Trigkey : UEFI `uefi-boot,reference-bringup,reference-desktop` |
| Hash kernel / image | produits par `tools/release/build_trigkey_image.sh` au moment de la release (manifest.json) |

## Verdicts

| Exigence 0.1 | Verdict | Preuve |
|---|---|---|
| FAST / Integration / Reliability | PASS | 41539949788 / 41539949905 / 41539949936 |
| IPC / Security | PASS | 41592664152 / 41592667159 |
| Nightly | PASS | 41592645380 |
| Ladybird qualification + C22 strict 6/6 (plafond 1024 Kio) | EN COURS | 41592642065 |
| Endurance | EN COURS | 41592677703 |
| C5 | STATIC_CLOSED ; KVM en cours | revue + 41596509318 |
| **P13_FINAL_STRICT_OK** | **BLOQUÉ** | runners self-hosted `bouchaud-kvm4`, `bouchaud-kvm8` inexistants |
| Aucune panique en qualification | à confirmer à la fin des runs | |
| Image reproductible | outillage prêt (`release.yml` double build ; `build_trigkey_image.sh`) | |
| PHYSICAL | PHYSICAL_PENDING | [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md) |

## Ce qui débloque P13 (action humaine)

Enregistrer deux runners self-hosted Linux x64 avec `/dev/kvm` (contrôle et étapes : `sudo tools/ci/runner/provision_kvm_runner.sh kvm4|kvm8` ; sans `--install` il ne modifie rien et refuse un hôte inapte — cœurs physiques, pas de surallocation SMT, quota, KVM, QEMU ≥ 8, disque ≥ 60 Gio, RAM ≥ 16 Gio) :

- label `bouchaud-kvm4` : ≥ 4 cœurs **dédiés** (pas de voisin bruyant), utilisé par `dedicated-three-campaigns` ;
- label `bouchaud-kvm8` : ≥ 8 cœurs dédiés, utilisé par `dedicated-smp8` (qualification + 30 min).

Puis : `workflow_dispatch` de `p13-final-candidate.yml` sur le candidat, **après** un Ladybird `palier=qualification` et un Nightly réussis sur ce même SHA. Le gate `tools/ci/p13_final_gate.py` exige les deux en `workflow_dispatch` sur la même branche et le même SHA.

Historique : versions précédentes de ce fichier dans l'historique Git ; preuves d'octobre : [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md).
