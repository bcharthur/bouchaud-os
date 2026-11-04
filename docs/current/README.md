# Bouchaud OS — point d'entrée (2026-11-04)

**Source de vérité active.** Commencez toujours par `git fetch && git rev-parse HEAD origin/main`, puis lisez [status.json](status.json). C'est un instantané daté, pas un statut qui se met à jour seul : un SHA ou un verdict n'est écrit qu'à un seul endroit, et c'est là.

## Comprendre le projet en cinq minutes

| Question | Où lire |
|---|---|
| Qu'est-ce qui fonctionne aujourd'hui, à quel niveau de preuve ? | [README principal](../../README.md#ce-qui-existe-réellement) (tableau), puis [status.json](status.json) (verdicts) |
| Qu'est-ce qui n'a été validé que sous QEMU ? | tout : `physical` = `NOT_STARTED` dans [status.json](status.json) ; KVM est indiqué composant par composant dans le README |
| Qu'est-ce qui n'a jamais été testé physiquement ? | [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md) (protocole non exécuté), [TRIGKEY_REFERENCE.md](TRIGKEY_REFERENCE.md) |
| Quelles sont les limites connues ? | [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) |
| Quelles PR sont ouvertes, et qu'apportent-elles ? | [status.json](status.json) → `open_prs` ; rien de ce qu'elles contiennent n'est sur `main` |
| Quels sont les prochains chantiers ? | [ROADMAP_CURRENT.md](ROADMAP_CURRENT.md), [CLAUDE_NEXT_MISSION.md](CLAUDE_NEXT_MISSION.md) |
| Comment construire et tester ? | [README principal](../../README.md#construire-et-démarrer), [CI_AND_PROOFS.md](CI_AND_PROOFS.md) |

## Documents courants

1. [STATUS_CURRENT.md](STATUS_CURRENT.md) : comment lire `status.json`, et les anomalies ouvertes.
2. [ARCHITECTURE_CURRENT.md](ARCHITECTURE_CURRENT.md) et [ARCHITECTURE_PRODUCT_TRUTH.md](ARCHITECTURE_PRODUCT_TRUTH.md) : architecture, chemins de code, dettes.
3. [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) : limites produit et preuves absentes.
4. [CI_AND_PROOFS.md](CI_AND_PROOFS.md) : workflows, niveaux de preuve, risques de faux verts.
5. [NATIVE_ABI.md](NATIVE_ABI.md) : ABI native (référence pour écrire un programme Bouchaud).
6. [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md) : Atomic Platform A/B, recovery, modèle de sécurité des mises à jour.
7. [BOUCHAUD_STABLE_BASELINE.md](BOUCHAUD_STABLE_BASELINE.md) : ce qu'exige une baseline, et ce qui la bloque.
8. [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md), [TRIGKEY_REFERENCE.md](TRIGKEY_REFERENCE.md), [RELEASE_TRIGKEY_CHECKLIST.md](RELEASE_TRIGKEY_CHECKLIST.md) : machine physique de référence.
9. [BOUCHAUD_NAVIGATEUR.md](BOUCHAUD_NAVIGATEUR.md), [P10_PLUS_ROADMAP.md](P10_PLUS_ROADMAP.md) : navigateur, et présentation graphique ou accélération.
10. [DEPENDENCIES_AND_LICENSES.md](DEPENDENCIES_AND_LICENSES.md) : renvoie aux notices tierces de la racine.

**Documents fusionnés depuis la consolidation** : `ATELIER_HANDOFF.md`, `ATELIER_DESIGN_SYSTEM.md`, `ATELIER_AUDIT_ABC.md` et les captures `atelier/` (PR #242 à #244) ; `TRIGKEY_T1_VIDANGE_USB.md` (PR #246).

**Documents qui n'existent que dans des PR ouvertes**, et ne doivent pas être cherchés sur `main` :
- `STOCKAGE_S1.md` : PR #245.
- `PERF_P1_PREMIERE_TRAME.md` : PR #247.
- `TRIGKEY_PLATFORM_PLAN.md` : branche `claude/trigkey-physical-20261103`, sans PR.

## Archives

Les pages suivantes sont des **archives historiques** et ne définissent plus le statut actif :
- les dossiers P13 d'octobre (`P13_*`), `P1_P13_CURRENT`, `P_LOTS_REGISTER` et `REGISTRE_CONVERGENCE` ;
- `HANDOFF_*` ;
- `AUDIT_STABILITY_*`, `TECHNICAL_AUDIT_2026-11-02`, `REMISE_EN_ORDRE_2026-11-02`, `C5_REVIEW_2026-11-03` ;
- `PRODUCT_M12_QUALIFICATION`.

Elles restent en place pour préserver les liens et les preuves. Plus anciennes encore : [`docs/history/`](../history/README.md) et [`docs/historique/`](../historique/README.md). Aucune suppression d'historique Git.

Niveaux de preuve distincts : STATIC, HOST, TCG, KVM, PHYSICAL. Un workflow vert ne transforme pas un niveau en un autre.
