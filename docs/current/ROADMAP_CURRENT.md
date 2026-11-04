# Feuille de route — 2026-11-04

Des **projets**, par ordre de priorité. Ce qui est déjà fait est sur `main` ou dans une PR ouverte ([status.json](status.json) → `open_prs`). Les limites détaillées sont dans [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).

## 1. Clore l'existant (avant tout nouveau chantier)

1. Revue et fusion des PR ouvertes, dans l'ordre de leur empilement : #242 → #243 → #244 → #245 → #247. La PR #246 est indépendante.
2. Classer les deux anomalies ouvertes si elles réapparaissent.
3. Actions humaines : runners KVM4 et KVM8, protection de `main` ([BOUCHAUD_STABLE_BASELINE.md](BOUCHAUD_STABLE_BASELINE.md)).

## 2. Grands chantiers

| Chantier | Première étape mesurable |
|---|---|
| **P13 strict** | `P13_FINAL_STRICT_OK` sur runners dédiés, sur un SHA unique |
| **Stockage généraliste et VFS** | modèle inode, dentry et montage ; cohérence `read`/`write`/`mmap` ; `fsync` ; table `/persist` journalisée au lieu d'être réécrite en entier ; volumes externes (FAT32 sur USB) dans Fichiers |
| **Plateforme Trigkey** | image A/B physique avec les services natifs, puis la campagne [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md) : démarrage, USB, NVMe, réseau, audio, horloges, énergie, redémarrage, endurance |
| **Performance mesurée** | latence d'interaction, coût de l'IPC, copies, allocations, temps de chargement, chacun avec sa ligne `BOUCHAUD_PERF_*` et un A/B sur le même hôte |
| **Moteur graphique** | composition CPU optimisée et copies réduites, présentation GPU 2D, puis étude de la 3D et du GPU AMD ([P10_PLUS_ROADMAP.md](P10_PLUS_ROADMAP.md)) |
| **Runtime applicatif isolé** | une application native hors du noyau, avec contrat IPC, supervision et reprise après crash |
| **Pilotes matériels** | HDA, Wi-Fi, gestion d'énergie, isolation DMA |
| **Sécurité de production** | clé de version hors ligne, anti-retour arrière sans casser le retour arrière local ([UPDATE_RECOVERY.md](UPDATE_RECOVERY.md)) |
| **Installation physique** | installateur et mises à jour sur le disque interne, après la campagne physique |

## 3. Plus tard (projets, non commencés)

IA locale comme service système isolé, conteneurs, modes d'usage, restauration de contexte. Ces sujets ne se développent qu'une fois la plateforme physique qualifiée.
