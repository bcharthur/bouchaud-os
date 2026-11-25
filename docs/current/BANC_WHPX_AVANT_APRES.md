# Banc WHPX avant / après sur le PC Windows

Procédure **Windows PowerShell 5.1**, du dépôt à jour jusqu'au rapport
automatique. Elle sert à deux comparaisons :

- **services séparés contre BouchaudServices** (#373) : même noyau, deux disques
  construits depuis le même artefact ;
- **noyau avant contre noyau après** un correctif (#375, #378, #379…) : même
  disque, deux bootimages.

Le banc (`tools\windows\Banc-AvantApres.ps1`) s'appuie sur
`tools\windows\Banc-Candidat.ps1`. Celui-ci prend une copie du disque en
`snapshot=on`, lance le navigateur automatiquement, injecte la molette et
vérifie l'accélérateur réel grâce au CPUID publié par le noyau. Les bras
alternent (A B, B A, A B…). Un passage invalide est publié avec sa cause et
n'entre pas dans les verdicts.

> **Ce qui est validé, et où.** Les scripts sont vérifiés dans la CI : syntaxe
> PowerShell, ASCII strict, plan d'exécution en `-DryRun` et rapport calculé
> sur des journaux synthétiques (`tools/ci/test-scripts-windows-trigkey.ps1`,
> job `windows / scripts`). Aucune mesure **WHPX** n'a été faite par la CI ni
> par Claude : seule une exécution sur le PC fournit une preuve WHPX, et le
> rapport l'indique (`accel_reel`).

## 0. Mettre `main` à jour sans perdre les modifications locales

```powershell
cd <dépôt bouchaud-os>
git status
# S'il y a des modifications : on les range (y compris les fichiers non suivis).
git stash push --include-untracked -m "avant-banc"
git fetch origin
git switch main
git merge --ff-only origin/main
# ... banc (étapes 1 à 4) ...
git stash list            # pour retrouver "avant-banc"
git stash pop             # remet les modifications, à la fin
```

`--ff-only` refuse toute fusion qui réécrirait l'historique local. En cas de
refus, rien n'est modifié : le signaler avant d'aller plus loin.

## 1. Préparer les disques (artefact BouchaudServices)

```powershell
$B = "$env:USERPROFILE\bouchaud-banc"
New-Item -ItemType Directory -Force $B | Out-Null

# Artefact Ladybird de l'empreinte de main (il contient BouchaudServices).
.\run.ps1 -RefreshLadybird -PreparerSeulement
Copy-Item ladybird-browser.img "$B\disque-separes.img"
Copy-Item target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin "$B\noyau-main.bin"

# Même artefact, services en liens physiques vers BouchaudServices.
.\run.ps1 -ServicesUniques -PreparerSeulement
Copy-Item ladybird-browser.img "$B\disque-uniques.img"
```

`-PreparerSeulement` construit le noyau et le disque, puis s'arrête **avant
QEMU** (ligne `PREPARATION_TERMINEE bootimage=... disque=...`). Sans
`-RefreshLadybird`, le second appel réutilise l'artefact déjà téléchargé :
les deux disques contiennent exactement les mêmes binaires.

## 2. Construire le noyau d'un correctif sans toucher au dépôt principal

Un worktree séparé laisse intacts la branche courante et les disques.

```powershell
# Exemple : #378 (futex). Autres branches : voir le tableau ci-dessous.
git worktree add ..\bo-apres origin/claude/futex-reveil-compte
Push-Location ..\bo-apres
.\tools\run\cargo-bootimage-safe.cmd
Copy-Item target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin "$B\noyau-apres.bin"
Pop-Location
git worktree remove ..\bo-apres
```

| Correctif | Branche (avant sa fusion) | Ce qu'il change |
|---|---|---|
| #375 | `origin/claude/bouchaud-services-fix-yabxhp` | console série 512 Kio (attente bornée sans progrès), munmap/brk qui sautent les tables absentes, diagnostic HID |
| #378 | `origin/claude/futex-reveil-compte` | réveil futex compté jamais perdu (gel d'ImageDecoder, #376) |
| #379 | `origin/claude/tables-pages-vides` | munmap rend les tables de pages vides (#377) ; contient #375 |

Après la fusion d'un correctif, « après » = `origin/main`, et « avant » = le
commit de `main` qui précède la fusion (`git log --merges origin/main`).

## 3. Lancer les bancs (mêmes paramètres partout)

```powershell
# Services séparés contre BouchaudServices : même noyau, deux disques.
.\tools\windows\Banc-AvantApres.ps1 -Mode disques -Noyau "$B\noyau-main.bin" `
    -DisqueAvant "$B\disque-separes.img" -DisqueApres "$B\disque-uniques.img" `
    -Paires 3 -Accel whpx -Smp 1 -Sortie "$B\services-smp1"

# Correctif noyau : même disque, deux noyaux.
.\tools\windows\Banc-AvantApres.ps1 -Mode noyaux -Avant "$B\noyau-main.bin" `
    -Apres "$B\noyau-apres.bin" -Disque "$B\disque-separes.img" `
    -Paires 3 -Accel whpx -Smp 1 -Sortie "$B\noyau-smp1"

# Même chose en -Smp 2 (sorties distinctes) si le temps le permet.
```

`-DryRun` affiche le plan (ordre des bras, commandes QEMU) sans rien lancer.

## 4. Lire le rapport

`<Sortie>\rapport.md` (et `rapport.json`) donne, par mesure, l'intervalle
[min - max] de chaque bras sur les **passages valides** et un verdict :

| Verdict | Sens |
|---|---|
| `APRES_MEILLEUR` / `AVANT_MEILLEUR` | intervalles disjoints |
| `NON_CONCLUANT` | intervalles qui se recouvrent : aucun gain annoncé |
| `NON_MESURABLE` | aucun passage valide d'un des bras |

| Mesure | Source |
|---|---|
| `page_ms`, `ouverture_ms`, `bureau_ms` | horloge de l'hôte (lancement → document chargé, → 1re trame, QEMU → bureau) |
| `fps_bureau`, `presentation_p99_us`, `cpu_invite_pct` | relevés de l'invité (`[GPU-COMPOSITION]`, `[GPU-TRAMES]`) |
| `ram_qemu_pic_mio` | mémoire de pointe du processus QEMU (hôte) |
| `ram_invite_pic_mio` | `ram_used_bytes` maximal de l'invité |
| `disque_lu_mio` | octets lus sur le disque (`[BACKING-CACHE]`) |
| `hid_pics`, `hid_pire_ms` | `HID_LATENCY_SPIKE` : nombre de pics et pire attente du fil d'entrée entre son réveil et son exécution |

La latence entrée → image n'est **pas** mesurée : aucun marqueur
d'horodatage d'entrée n'existe dans le journal. Les journaux complets de
chaque passage sont dans `<Sortie>\...\<bras>-<paire>\serie.log`.

Pour recalculer un rapport sans relancer :

```powershell
.\tools\windows\Banc-AvantApres.ps1 -AnalyserSortie "$B\noyau-smp1"
```

## 5. Ce qu'il faut renvoyer

Les deux `rapport.md`, la ligne `BANC_AVANT_APRES_FIN` de chaque banc, et les
empreintes des pièces comparées :

```powershell
Get-FileHash "$B\*.bin", "$B\*.img" | Format-Table Hash, Path
```
