# Bouchaud OS — START HERE (2026-11-03)

**Source de vérité active** — vérifier toujours `git rev-parse HEAD` et les CI avant d'utiliser les états ci-dessous.

1. [STATUS_CURRENT.md](STATUS_CURRENT.md) : état vérifié du dépôt et de la baseline.
2. [ARCHITECTURE_CURRENT.md](ARCHITECTURE_CURRENT.md) : architecture et chemins de code.
3. [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) : limites produit et preuves absentes.
4. [CI_AND_PROOFS.md](CI_AND_PROOFS.md) : runs, niveaux de preuve, commandes et risques de faux verts.
5. **[CLAUDE_NEXT_MISSION.md](CLAUDE_NEXT_MISSION.md) : ordre de mission demain.**
6. [NATIVE_ABI.md](NATIVE_ABI.md) : ABI native 1.1 (référence pour écrire un programme Bouchaud).
7. [BOUCHAUD_STABLE_BASELINE.md](BOUCHAUD_STABLE_BASELINE.md) : verdict de baseline et ce qui la bloque.
8. [UPDATE_RECOVERY.md](UPDATE_RECOVERY.md), [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md), [C5_REVIEW_2026-11-03.md](C5_REVIEW_2026-11-03.md).

[status.json](status.json) est un instantané daté, **pas** un statut qui se réactualise automatiquement. Le rapport de préparation est [REMISE_EN_ORDRE_2026-11-02.md](REMISE_EN_ORDRE_2026-11-02.md).

Les autres pages de `docs/current/`, notamment les dossiers P13 d'octobre, `HANDOFF_CLAUDE_ACTIVE` et `REGISTRE_CONVERGENCE`, sont **archives historiques logiques** : elles ne définissent plus le statut actif. Elles restent en place pour préserver les liens et les preuves, sans opération de déplacement massive. Aucune suppression d'historique Git.

Niveaux de preuve distincts : STATIC, HOST, TCG, KVM, PHYSICAL. Un workflow vert ne transforme pas un niveau en un autre.
