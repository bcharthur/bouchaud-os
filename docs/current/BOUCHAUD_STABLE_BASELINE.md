# Baseline stable — Bouchaud OS

**NOT_ESTABLISHED.** Aucun tag de release ne doit être posé. L'état mesuré (candidat, runs, verdicts) est dans [status.json](status.json) ; ce fichier ne garde que la règle et ce qui la débloque.

## Ce qu'une baseline exigera

Sur **un même SHA** :
- les gates requis verts (`fast-gate`, `integration-gate`, `reliability-gate`) ;
- `native-ipc / verdict` vert, avec la matrice `ab-rollback` et les coupures ;
- Ladybird `palier=qualification`, Nightly et `p13-primitives-noyau` verts ;
- `P13_FINAL_STRICT_OK` sur runners dédiés ;
- la campagne PHYSICAL Trigkey, y compris les étapes A/B AB1–AB8 ;
- une clé de version de production configurée (décision humaine).

## Ce qui débloque P13 (action humaine)

Enregistrer deux runners self-hosted Linux x64 avec `/dev/kvm`. Le contrôle de l'hôte se fait avec `sudo tools/ci/runner/provision_kvm_runner.sh kvm4|kvm8`. Sans `--install`, il ne modifie rien et refuse un hôte inapte sur les points suivants :
- cœurs physiques, sans surallocation SMT ;
- quota ;
- KVM ;
- QEMU ≥ 8 ;
- disque ≥ 60 Gio ;
- RAM ≥ 16 Gio.

Les deux runners :
- label `bouchaud-kvm4` : ≥ 4 cœurs dédiés, pour `dedicated-three-campaigns` ;
- label `bouchaud-kvm8` : ≥ 8 cœurs dédiés, pour `dedicated-smp8` (qualification + 30 min).

Puis : `workflow_dispatch` de `p13-final-candidate.yml` sur le candidat, après un Ladybird `palier=qualification` et un Nightly réussis sur ce même SHA (`tools/ci/p13_final_gate.py`).

## Protection de `main` (action humaine)

`tools/github/ruleset-main.json` contient :
- PR obligatoire ;
- force-push interdit (`non_fast_forward`) ;
- suppression interdite ;
- checks requis `fast-gate`, `integration-gate`, `reliability-gate`.

`native-ipc / verdict` n'y figure pas : son workflow est filtré par chemins, et le requérir bloquerait toute PR qui ne le déclenche pas. Pour une PR qui touche l'update, le loader, le noyau natif ou la persistance, le relecteur vérifie qu'il est vert.

À appliquer par un administrateur du dépôt :

```bash
gh api -X POST repos/bcharthur/bouchaud-os/rulesets --input tools/github/ruleset-main.json
```

Ou par l'interface : Settings → Rules → Rulesets → New → Import a ruleset.

Historique : versions précédentes de ce fichier dans Git ; preuves d'octobre : [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md).
