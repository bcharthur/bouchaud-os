# Bouchaud OS — START HERE (2026-11-02)

**Source de vérité active** — vérifier toujours `git rev-parse HEAD` et les CI avant d'utiliser les états ci-dessous.

1. [STATUS_CURRENT.md](STATUS_CURRENT.md) : état vérifié du dépôt et de la baseline.
2. [ARCHITECTURE_CURRENT.md](ARCHITECTURE_CURRENT.md) : architecture et chemins de code.
3. [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) : limites produit et preuves absentes.
4. [CI_AND_PROOFS.md](CI_AND_PROOFS.md) : runs, niveaux de preuve, commandes et risques de faux verts.
5. **[CLAUDE_NEXT_MISSION.md](CLAUDE_NEXT_MISSION.md) : ordre de mission demain.**

[status.json](status.json) est un instantané daté, **pas** un statut qui se réactualise automatiquement. Le rapport de préparation est [REMISE_EN_ORDRE_2026-11-02.md](REMISE_EN_ORDRE_2026-11-02.md).

Les autres pages de `docs/current/`, notamment les dossiers P13 d'octobre, `HANDOFF_CLAUDE_ACTIVE` et `REGISTRE_CONVERGENCE`, sont **archives historiques logiques** : elles ne définissent plus le statut actif. Elles restent en place pour préserver les liens et les preuves, sans opération de déplacement massive. Aucune suppression d'historique Git.

Niveaux de preuve distincts : STATIC, HOST, TCG, KVM, PHYSICAL. Un workflow vert ne transforme pas un niveau en un autre.
