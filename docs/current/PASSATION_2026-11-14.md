# Passation du 14/11/2026 — mission « développement, stabilisation et performance »

Point de reprise vivant de la nuit du 13 au 14/11, mis à jour le 14/11 à 19:00 UTC. Chaque verdict cite sa source. Rien ici n'annonce un résultat qui n'a pas été observé.

## 1. État de `main` et des PR

| Objet | SHA | État |
|---|---|---|
| `main` | `eb6f115` | #321, #322, #320, #323 puis #324 (ligne de souffrance du noyau figé) fusionnées le 14/11 ; `main@01dc7fd` vérifiée verte (CI Fast relancé : compile + invariants) |
| #319 `claude/sigchld-boucle` | `2179ddf` | correctif #311, à jour de `main` ; A/B terminé (§2bis) : les trois échecs d'origine ne se reproduisent pas ; **non fusionnée** : un contrôle `robustesse` rouge au SHA exact (gel #283) et décision humaine attendue |
| #324 `claude/souffrance-proprio` | `b259735` | **fusionnée** (`eb6f115`) : 141 contrôles verts |
| #325 `claude/souffrance-site-detenteur` | `adfdf88` | site de prise du détenteur ; seul rouge : le bras `perf avant-simple`, qui démarre le noyau de `main` (gel #283) ; relancé une fois |
| #320 `claude/e2e-ladybird` | `8aed50d` | **fusionnée** (`65a67d6`, 14/11 ~11:00 UTC) : 78 contrôles verts ; `E2E_VERDICT PASS` sous KVM et TCG |
| #321 `claude/python-hors-noyau` | `efcf54f` | **fusionnée** (`c6c3eea`) |

Règle appliquée : une PR n'est fusionnée qu'avec tous ses contrôles obligatoires verts **au SHA exact**. Une relance verte ne règle jamais un intermittent.

## 2. Ce qui a été fait, avec preuves

### #319 — SIGCHLD multi-fil (#311)

Cause mesurée : le signal dirigé vers le processus était confisqué par un fil secondaire. LibCore jette les signaux reçus hors du fil de la boucle.

| Version | Signaux volés |
|---|---|
| avant | 6 / 30 |
| correctif intermédiaire | 9 / 30 |
| correctif complet | 0 / 30 |

Après les retours de revue : 20 passages sur 20.

- Un `fast-gate` rouge à `c1c32ca` venait d'un garde-fou d'architecture (`verifie-cycle-vie.py`) qui exigeait une chaîne littérale. La règle est adaptée en `24a1419` **sans affaiblissement** : elle exige en plus que l'échéance dérive de `deadline`. Mutation vérifiée.
- #311 reste **ouverte** jusqu'à la qualification en CI.

### #320 — parcours E2E de Ladybird

Une seule définition, `tools/e2e/ladybird/scenarios.json`. Premier passage réel (TCG SMP4, job 132240489021) :

- E2E-01 PASS ;
- E2E-02 FAIL, un **faux rouge du banc**. Le marqueur `BOUCHAUD_STAGE2_LADYBIRD_LAUNCHED` n'existait que sous la feature `reference-desktop`.

Reproduit en local avec un faux `/bo-navigateur` PIE : le clic réel dans le menu lance bien l'exécutable. Corrigé en `13a567f` : le marqueur est désormais publié par le noyau livré. Un échec imprime aussi un extrait du journal série (`E2E_EXTRAIT`) dans le journal du job.

Un test qui compte les consommateurs de l'artefact Ladybird passe de 17 à 18, avec `e2e-parcours`.

### #321 — démarrage

L'interpréteur Python (16,4 Mo) quitte l'image de boot (29,3 → 12,9 Mo). Premier octet du noyau sous QEMU TCG, essais entrelacés :

| Image | Premier octet du noyau |
|---|---|
| avant | 10,0 à 11,6 s |
| après | 5,1 à 5,6 s |

- Python est lu sur le disque au premier `python`, en 29 à 38 ms.
- Banc : `tools/ci/run_python_disque.sh`.
- L'image de livraison Trigkey **perd `python`**, ce qui est dit dans la PR et dans le README de python-wasm.
- Anomalie préexistante notée : le code de sortie de `python -c` est perdu, y compris sur l'ancien noyau.
- Attribution du trajet du noyau au bureau sous TCG : 2,4 s sur 5,2 à 5,8 s vont à l'analyse de la police (`fontdue`). **Non optimisé**, parce que ce coût est probablement propre à TCG. La ligne `BOUCHAUD_POLICE_ANALYSEE` le mesurera sous WHPX et sur le matériel.


### #319 : trois échecs nouveaux sur la branche, non classés

| Job | SHA | Vu | Témoin |
|---|---|---|---|
| `ab-rollback (ota-reseau)` : `partie_5_non_reprise` | 24a1419 | 1 sur 2 | vert sur `main` |
| `BRDP client-lent` : délai dépassé | e7e8b96 | 1 sur 2 | vert sur `main@c6c3eea` |
| `ab-rollback (mise-a-jour-morts)` : `updated_battements_manques` | e7e8b96 | 1 fois | jamais en échec sur une trentaine de passages |

Mécanisme : la PR n'agit que sur un processus multi-fil qui reçoit un signal destiné au processus. Les trois cas sont des fils uniques ou des fils noyau. **Rien ne les relie à la PR, rien ne les en écarte.**

À faire : un A/B au même SHA de base, en plusieurs passages, de ces trois bancs avec et sans la PR.

### #320 : parcours E2E, défauts du banc corrigés dans la nuit

1. Le marqueur de lancement n'existait que sous `reference-desktop`.
2. Le disque n'avait pas de zone persistante : le BrowserHost sortait avec le code 1.
3. Entrée sur un lien ne navigue pas : le banc clique désormais dans la page, avec la zone utile publiée par le gestionnaire de fenêtres. E2E-04 passe ; l'activation d'un lien au clavier dans Ladybird reste à vérifier.
4. Course entre capture et extinction dans le banc native IPC.
5. Lignes de cause prises dans la fenêtre du parcours en échec.

6. E2E-03 rendu déterministe. Le site de référence sert un certificat **expiré**, signé par une autorité de test générée au moment du test (aucune clé commitée, `tools/verifie-cle-dev.py`). L'autorité n'est ajoutée qu'au disque E2E. `expired.badssl.com` passe en parcours non bloquant (E2E-03X), où une connexion coupée donne `BLOCKED_EXTERNAL`.

Intermittent sans lien avec la PR : `CONSOLE_PANIQUE_ECHEC 1999/2000`, vert au noyau identique.

## 2bis. Soir du 14/11 : A/B de #319 et le gel #283 qui parle enfin

### A/B de #319 (A = `main` sans #319, B = `main` + #319)

| Banc | A | B |
|---|---|---|
| ab-rollback ota-reseau | 3/3 verts | 3/3 verts |
| ab-rollback mise-a-jour-morts | 3/3 verts | 3/3 verts |
| BRDP + télémétrie | 3/3 verts | 4/4 verts |
| Ladybird `perf` (démarrages) | 1 gel / ~81 | 2 gels / 45 |
| Ladybird `robustesse` (runs) | 0 / 15 | 1 / 5 |
| Piège du Compositor (runs) | 2 / 14 | 1 / 5 |

Les trois échecs qui bloquaient #319 ne se reproduisent pas. Les gels Ladybird relèvent de #283 et existent sur `main` sous les mêmes formes. L'écart vu à `2e61a6b` (2 gels `perf`) ne tient pas sur la série suivante (0/9 de chaque côté). Détail et décision : commentaires de #319.

### #283 : ce que le noyau figé dit désormais

Trois formes de gel, toutes vues sur `main` :

1. **attente IPC** : tous les fils IPC des six processus en `poll(-1)` sur la même file depuis la même seconde, aucun verrou tenu (`jalons manquants=7`) ;
2. **verrou `Mm`** : capturé avec la ligne de #324 sur `main@eb6f115` (job 132498554631) :
   - le CPU 0 (`services-metrics`) attend depuis 820 s le verrou `Mm` d'un processus, tenu par le CPU 1 ;
   - le CPU 1 est le WebWorker qui vient d'être lancé, en faute de page du chargeur (site 212) ; son dernier RIP connu est `vma::protege` ;
   - les CPU 2 et 3 ne reçoivent plus d'IPI ;
3. **silence** : plus aucune ligne, pas même celle de l'IRQ.

#325 ajoute le site où le détenteur a pris le verrou, `#[track_caller]` sur `Mm::lock`, et des lignes non tronquées dans les extraits. Écrire `Location::caller()` dans `SpinLock::try_lock` arrêtait la console asynchrone au démarrage (3/3 sous QEMU) ; cause non établie, laissée ouverte.

## 3. Anomalies ouvertes, non attribuées

Voir `KNOWN_LIMITATIONS.md` :

- **nouvelle** : `ab-rollback (ota-reseau, nvme)` `partie_5_non_reprise` sur #319 à `24a1419`. Après rétablissement du lien, 7 min de `connect` en échec ; noyau identique à `c1c32ca`, où le job était vert. Le diagnostic recopie désormais l'état réseau de l'invité ;
- intermittent `ab-rollback` « recovery-requise » ;
- piège du Compositor `batterie_workers` ;
- `swaps_non_recoltes` ;
- code de sortie de `python -c`.

Issues gardées ouvertes : #283, #293, #303, #311.

## 4. Non fait, et pourquoi

| Point | État |
|---|---|
| WHPX | **non mesuré par moi**. Aucune machine Windows n'est accessible ; les commandes sont plus bas. |
| KVM | uniquement en CI ; pas de `/dev/kvm` en local |
| Trigkey physique | `NOT_RUN`. Aucun flash, aucune écriture sur le NVMe interne. |
| E2E-06 à E2E-10 | `NOT_RUN`, chaque raison écrite dans `scenarios.json` |
| Optimisations Ladybird (CPU, IPC, fautes de page) | le profilage exige l'artefact Ladybird, absent en local ; à faire depuis la CI ou WHPX |
| Lots 4 à 6 (compositeur ring 3 A–F, stockage durable, image Trigkey A/B, P10 GPU) | non avancés pendant cette nuit |

## 5. Commandes (Windows, PowerShell, depuis la racine du dépôt)

Reconstruire et valider (parcours E2E sous WHPX, SMP4, une fois #320 fusionnée) :

```powershell
git pull; .\tools\windows\Verifier-Ladybird.ps1 -Profil complet -Accel whpx -CpuCount 4
```

Lancer WHPX SMP4 avec Ladybird, DOOM et Atelier :

```powershell
.\run.ps1 -Ladybird -Doom -Accel whpx -CpuCount 4 -RamMiB 8192
```

Archiver les journaux, à transmettre :

```powershell
$p = @('target\journaux-qemu', 'target\e2e') | Where-Object { Test-Path $_ }; Compress-Archive -Path $p -DestinationPath "journaux-$(Get-Date -Format yyyyMMdd-HHmmss).zip" -Force
```

Vérifier que le noyau testé est le bon :

```powershell
(Get-FileHash target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin -Algorithm SHA256).Hash
```

Après #321, l'image doit faire environ 12,9 Mo, et non plus 29 Mo.

## 6. Prochaines actions

1. **#319 (décision humaine).**
   - Relire le tableau du §2bis.
   - Relancer une fois le job `robustesse` à `2179ddf`.
   - Fusionner seulement si tous les contrôles sont verts au SHA exact.
   - #311 reste ouverte jusque-là.
2. **#283.** Au prochain gel avec #325, lire `spin=[…@attente<prise …]` pour savoir où le détenteur du verrou `Mm` l'a pris. Croiser avec le RIP `vma::protege` du CPU 1.
3. **`swaps_non_recoltes`.** L'endurance 20 min KVM échoue aussi sur `main@eb6f115` : un parent qui ne récolte pas un enfant, proche de #311. Non attribuée.
4. **Mesures hors CI.** Lire `BOUCHAUD_POLICE_ANALYSEE` et `hote_jusqu_au_noyau_ms` dans les journaux KVM et WHPX avant toute nouvelle optimisation du démarrage.
