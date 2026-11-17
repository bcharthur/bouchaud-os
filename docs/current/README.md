# Bouchaud OS — point d'entrée (2026-11-17)

## En une minute

| Question | Réponse |
|---|---|
| Branche de référence | **`main`**. Le travail se fait sur une seule branche à la fois, avec sa PR. Les verdicts valent pour un SHA. |
| Compiler et démarrer | [README principal → Construire et démarrer](../../README.md#construire-et-démarrer) : `cargo bootimage`, puis QEMU (BIOS, UEFI, VIRGL, Trigkey). |
| Tester en local | `tools/ci/run_host_tests.sh`, `tools/ci/run_architecture_guards.sh` (après `cargo build`), `tools/ci/run_barriere_locale.sh` |
| La CI | [CI_AND_PROOFS.md](CI_AND_PROOFS.md) : trois niveaux (A au push, B avant fusion, C la nuit), checks requis, règle de fusion, inventaire des workflows |
| Ce qui marche, à quel niveau de preuve | [README principal → Ce qui existe réellement](../../README.md#ce-qui-existe-réellement), [status.json](status.json) (instantané daté) |
| Les limites | [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) |
| La dernière passation | [PASSATION_2026-11-17.md](PASSATION_2026-11-17.md), puis sa suite [PASSATION_2026-11-17_SUITE.md](PASSATION_2026-11-17_SUITE.md) |

## Chantiers ouverts, 17/11/2026, par priorité

1. **Réactivité de l'Atelier sous WHPX.**
   - Insights ré-énumérait le bus PCI à chaque trame, soit environ 30 000 E/S, chacune une sortie de VM.
   - Le correctif et le banc A/B sont fusionnés (#333) : `tools/ci/banc_atelier.py`, le job `qemu / reactivite Atelier (KVM, TCG)` et le document [BANC_ATELIER.md](BANC_ATELIER.md).
   - Sous KVM, en CI : 0 accès PCI par trame, pire trame 7,6 ms.
   - Reste à faire : la mesure sur Windows et WHPX (SMP1/2/4).
2. **#283 — Ladybird figé au démarrage à froid.**
   - Les correctifs sont fusionnés (#330, #331, #328).
   - **Le ticket n'est pas clos.** Il faut d'abord une campagne Ladybird sur `main`, puis WHPX ou le matériel.
3. **Performances de Ladybird, puis `CLONE_VFORK`.**
   - Chaque processus du navigateur coûte 11 à 18 ms de copie d'espace, plus 7 à 10 ms de libération ([PERF_LADYBIRD.md](PERF_LADYBIRD.md)).
   - Prérequis : un `Mm` remplaçable par processus (voir la passation).
4. **P13 strict** : SMP8 et verdict final. Bloqué par les runners auto-hébergés non enregistrés (`p13-final-candidate.yml`).
5. **Plateforme Trigkey (PHYSICAL)** : campagne non commencée ([TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md)).
6. **Grands chantiers** (VFS disque, isolation, pilotes, sécurité de production) : [ROADMAP_CURRENT.md](ROADMAP_CURRENT.md).

## Références courantes

- **Architecture et ABI** : [ARCHITECTURE_CURRENT.md](ARCHITECTURE_CURRENT.md), [ARCHITECTURE_PRODUCT_TRUTH.md](ARCHITECTURE_PRODUCT_TRUTH.md), [NATIVE_ABI.md](NATIVE_ABI.md), [KERNEL_RUNTIME.md](KERNEL_RUNTIME.md), [SECURITY_MODEL.md](SECURITY_MODEL.md).
- **Mises à jour** : [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md), [TRIGKEY_INSTALL_UPDATE.md](TRIGKEY_INSTALL_UPDATE.md).
- **Atelier** : [ATELIER_DESIGN_SYSTEM.md](ATELIER_DESIGN_SYSTEM.md), [ATELIER_HANDOFF.md](ATELIER_HANDOFF.md), [ATELIER_AUDIT_ABC.md](ATELIER_AUDIT_ABC.md), [FENETRES_RING3_PLAN.md](FENETRES_RING3_PLAN.md).
- **Navigateur et graphique** :
  - [BOUCHAUD_NAVIGATEUR.md](BOUCHAUD_NAVIGATEUR.md), [E2E_LADYBIRD.md](E2E_LADYBIRD.md), [PERF_LADYBIRD.md](PERF_LADYBIRD.md) ;
  - [P10_GPU_COMPOSITEUR.md](P10_GPU_COMPOSITEUR.md), [P10_PLUS_ROADMAP.md](P10_PLUS_ROADMAP.md).
- **Trigkey et release** : [TRIGKEY_REFERENCE.md](TRIGKEY_REFERENCE.md), [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md), [RELEASE_TRIGKEY_CHECKLIST.md](RELEASE_TRIGKEY_CHECKLIST.md), [TRIGKEY_CLE_USB_TEST.md](TRIGKEY_CLE_USB_TEST.md), [TRIGKEY_PREMIERE_UTILISATION.md](TRIGKEY_PREMIERE_UTILISATION.md), [RELEASE_EVIDENCE_LIVE.md](RELEASE_EVIDENCE_LIVE.md).
- **Fiabilité, stockage, réseau** : [OBSERVABILITY_RELIABILITY.md](OBSERVABILITY_RELIABILITY.md), [NON_REGRESSION.md](NON_REGRESSION.md), [STOCKAGE_S1.md](STOCKAGE_S1.md), [NETWORK_STORAGE.md](NETWORK_STORAGE.md), [BOUCHAUD_STABLE_BASELINE.md](BOUCHAUD_STABLE_BASELINE.md), [P13_QUALIFICATION_TRUTH_GATE.md](P13_QUALIFICATION_TRUTH_GATE.md).

## Historique

Les passations, audits et clôtures datés ont été déplacés le 17/11 dans [`docs/historique/notes/`](../historique/notes/), avec leurs liens réécrits :

- `HANDOFF_*`, `SESSION_2026-11-10`, `CONSOLIDATION_2026-11-08`, `CLAUDE_NEXT_MISSION`, `PASSATION_2026-11-14` ;
- `P13_CLOSEOUT_*`, `P13_MEMORY_CLOSURE_*`, `P1_P13_CURRENT`, `P_LOTS_REGISTER`, `REGISTRE_CONVERGENCE` ;
- `AUDIT_STABILITY_*`, `TECHNICAL_AUDIT_*`, `REMISE_EN_ORDRE_*`, `C5_REVIEW_*`, `PRODUCT_M12_QUALIFICATION`, `P10_GPU_MAINLINE_*`.

Les artefacts de correctifs appliqués à la main, qui encombraient la racine, sont dans [`docs/historique/racine-2026-11/`](../historique/racine-2026-11/README.md). Plus anciens encore : [`docs/history/`](../history/README.md) et [`docs/historique/`](../historique/README.md).

Rien n'a été supprimé de Git.

Niveaux de preuve distincts : STATIC, HOST, TCG, KVM, PHYSICAL. Un workflow vert ne transforme pas un niveau en un autre.
