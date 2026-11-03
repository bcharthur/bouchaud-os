# Mise à jour, retour arrière et recovery — architecture (CAP B)

**État : logique A/B PROUVÉE SUR HÔTE ; non branchée au chargeur ; PHYSICAL non démontré.**

## Ce qui existe

| Brique | Où | Preuve |
|---|---|---|
| Installation ESP + partition système sur NVMe | `src/platform/pc/installation.rs` (+ `disposition.rs` pur) | `tools/platform/test_installation.rs`, `tools/verifie-installation.py` (hôte, fsck.fat) |
| Image traçable (commit, Ladybird, toolchain, ELF) | `tools/release/build_trigkey_image.sh` | build + boot OVMF/TCG |
| Release reproductible + attestations | `.github/workflows/release.yml` | double build bit-à-bit, Sigstore |
| **Machine d'état A/B + bloc d'amorçage** | `src/kernel/update/ab.rs` (pur, compilé dans le noyau) | `tools/update/test_ab.rs` (CI Fast), tests de mutation |
| Boîte noire extractible | `tools/reference/extract-blackbox.py`, `symbolise-blackbox.py` | bancs `run_blackbox_*` |

## Contrat A/B (`ab.rs`)

```text
download → vérifier la signature/empreinte → commence_installation(inactif) [persisté : slot Vide]
        → écrire l'image → relire + SHA-256 → flush → marque_candidat(essais=N) [persisté]
au boot : choisit_boot() → l'essai est CONSOMMÉ et persisté AVANT de booter le candidat
        → service de santé (serviced : services critiques Running) → sante_ok → commit (actif = candidat)
        → échec, gel ou N essais épuisés → candidat Mauvais, boot de l'ancien actif (inchangé)
retour arrière manuel : revient() tant que la version précédente est Bonne
```

Bloc d'amorçage : **deux copies** de 96 octets (magic, séquence, actif, états, empreintes, CRC32). On écrit toujours la copie la plus ancienne ; on lit la plus récente valide.

Invariants démontrés sur hôte (`test_ab.rs`) :
- coupure à **chaque octet** de chaque écriture : on relit l'ancien ou le nouvel enregistrement, jamais un mélange ; le nouveau n'est jamais visible avant la fin de l'écriture ;
- 20 000 contenus aléatoires dans la copie en cours d'écriture : aucun état fantôme ;
- coupure à chaque octet de chacune des 4 écritures d'une mise à jour : la machine reste amorçable (actif Bon, jamais de boot d'un slot Vide/Mauvais) ;
- un candidat qui plante est booté **exactement** N fois puis rollback ;
- exploration exhaustive (96 états) : invariants tenus, encodage bijectif ;
- refus explicites : écrire sur l'actif, image non vérifiée, 0 essai, candidat déjà en attente, enregistrement forgé à actif non Bon.

## Ce qui reste (ordre proposé)

1. **Disposition disque** : ajouter la seconde partition système (B) et la zone du bloc d'amorçage à `disposition.rs` (pur, testé comme le reste).
2. **Chargeur** : lire le bloc avant de charger le noyau, appeler `choisit_boot`, écrire l'enregistrement retourné *avant* de sauter dans le noyau. Sans cette étape, I3 (essais bornés) n'existe pas physiquement.
3. **Commit de santé** : `serviced` déclare la santé (services critiques `Running` pendant T) → appel noyau `sante_ok`.
4. **Recovery** : un slot recovery minimal (noyau + shell texte + `extract-blackbox` + `revient`), indépendant du compositor et de Ladybird, choisi par une touche au boot ou après deux rollbacks consécutifs.
5. **Signature** : vérifier une signature (pas seulement une empreinte) avant `marque_candidat` ; préparer la chaîne UEFI → loader signé → noyau signé.
6. **Power-loss réel** : banc QEMU qui tue la VM pendant `fsync`/`rename` sur la partition de persistance (`fs::persistance`) et vérifie l'invariant « ancienne ou nouvelle version ».
