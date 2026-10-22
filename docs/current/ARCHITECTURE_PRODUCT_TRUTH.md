# Bouchaud OS — architecture product truth (P1)

> Statut : audit de code source STATIC uniquement. Les preuves QEMU et PHYSICAL
> doivent être collectées séparément et horodatées sur un commit/arbre exact.

## Revue critique externe (16 octobre 2026)

Les deux revues ont correctement identifié des **risques structurants** :
absence de système de fichiers disque général et de cache fichier/mmap unifié,
composants graphiques encore liés au noyau, lints désactivés au niveau crate,
manque de surface de documentation orientée contributeur, largeur matérielle
insuffisante, maintien long terme des ABI et qualité du processus de revue.

**Correction d'un contresens :** `src/kernel/syscall/legacy.rs` n'est PAS la
liste réelle des syscalls Bouchaud OS. L'entrée ring3 existe dans
`src/arch/x86_64/usermode.rs`, le dispatch de compatibilité dans
`src/kernel/abi/` et l'ABI native 1.0 dans `src/kernel/native/abi/`.
Un code présent n'est pas un run : la preuve négative CPL3/SMEP, la gestion
des fautes et l'isolation de WebContent restent à qualifier sur les images.

## Ce que ce patch corrige

1. **IPC natif** : une erreur de copie vers l'espace utilisateur lors de
   `CHANNEL_RECV` ne doit ni perdre le message ni laisser une installation de
   handles : rollback et remise en file. La taille est revalidée après le
   retrait de la file (le `front_sizes` antérieur n'est pas atomique).
2. **Mémoire partagée** : `SHM_WRITE` rejette les offsets et tailles hors
   borne AVANT de copier depuis l’espace utilisateur (prévention d’un coût de
   lecture/allocation sans aucune chance de réussite).
3. **Compatibilité ABI native** : gel explicite des IDs de syscalls, droits,
   catégories d'objets, codes erreurs, structures filaires. Toute modification
   ancienne casse la CI; un nouvel appel exige au moins une version mineure.
4. **Lisibilité** : l'ancien module syscall est identifié comme historique,
   au lieu d'annoncer que l'entrée ring3 serait encore à créer.
5. **Audit continu portable** : scripts Python de contrôle, automatiquement
   découverts par `tools/ci/run_architecture_guards.sh`.

## Ce qui reste un vrai chantier architectural

- **P2 — VFS** : spécifier `inode`, `dentry`, `mount`, cache de pages liées aux
  fichiers, cohérence `read/write/mmap`, eviction et cycle transactionnel disque.
  Préserver l'A/B persistant actuel ; démontrer coupure entre les étapes de
  commit et comparaison SHA après deux boots. Ne pas remplacer `/persist` par un
  filesystem ad hoc non éprouvé.
- **P3 — Services ring3** : isoler progressivement les parties UI et TLS du
  noyau. Débuter par un service peu privilégié, avec contrat IPC, superviseur
  et tests d'injection de crash. `src/gui/` compilé dans le noyau est un sujet
  d'architecture ; `WebContent` déjà en ring3 n'est pas à recoder.
- **P4 — Pilotes** : interface bus/driver, politique DMA/IOMMU, arrêt du device
  sur fault, stratégie de récupération. Prioriser Trigkey réel et NVMe/xHCI.
- **P5 — Qualité** : réduction de `allow(static_mut_refs)` et de `dead_code`
  **module par module** ; traiter les `unsafe` via contrats mémoire et tests
  négatifs avant suppression de suppressions globales.
- **P6 — Distribution** : docs pour contributeurs humains, quickstart,
  licences des dépendances, SBOM/reproductibilité, CI publique, roadmap modeste.

## Commandes reproductibles

```bash
python3 tools/verifie-contrat-abi-v1.py
python3 tools/verifie-reception-ipc-atomique.py
python3 tools/architecture_audit_product.py --repo . --json target/architecture-product-audit.json
# Exécuter ensuite les tests existants ; pas de faux vert runtime :
# tools/test.sh --quick  (nécessite les artefacts de boot et une VM)
```

**Interdictions** : pas de `reset --hard`, `clean -fd`, push sur main, flash ou
écriture NVMe depuis un patch de durcissement architectural.
