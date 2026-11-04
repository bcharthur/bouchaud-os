# Feuille de route — 2026-11-04

Des **projets**, par ordre de priorité. Ce qui est déjà fait est sur `main` ou dans une PR ouverte ([status.json](status.json) → `open_prs`). Les limites détaillées sont dans [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).

## 1. Clore l'existant (avant tout nouveau chantier)

1. Atelier (#242 à #244) et la vidange USB (#246) sont fusionnés. Restent #245 puis #247 : chacune doit intégrer `main` (banc scanout corrigé) et refaire son P13 ; #247 a en plus un défaut attribué à corriger (précharge de police, voir [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md)).
2. Décider du traitement du refus de commit A/B sur clé USB en CI (anomalie attribuée, [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md)) ; classer les anomalies non attribuées si elles réapparaissent.
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
