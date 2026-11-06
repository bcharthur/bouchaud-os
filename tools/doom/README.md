# DOOM / Freedoom — application ring 3 pour Bouchaud Atelier

**Etat prouve le 6 novembre 2026 (Windows, QEMU/TCG):** Doomgeneric, compile en ELF x86-64 **ET_DYN statique PIE**, a ete lance sous Bouchaud OS en PID 17. Le protocole GUI V1 a negocie la surface, le WAD `freedoom1.wad` a ete charge et des images ont ete presentees. Ceci est une application externe sous `/doom`, pas du code integre au noyau. Le son est desactive par defaut sous Windows pour eviter le plantage DirectSoundCapture.

## Un nouveau PC Windows, sans WSL

Prerequis : Git, PowerShell 5.1+, Rust + QEMU selon `run.ps1`, Python (packaging de l'image). La compilation native Windows utilise une archive de **Zig 0.13.0 portable**, telechargee automatiquement dans `target/doom-bouchaud/toolchains` : **ne pas installer WSL**.

Depuis la racine du depot (sur la branche contenant ce correctif) :

```powershell
# Compilation explicite (ne lance pas QEMU) :
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\doom\build-windows.ps1 -NoLaunch

# Lancement : reconstruit automatiquement le moteur / WAD s'ils manquent,
# ou si un ancien ELF ET_EXEC incompatible est present.
.\run.ps1 -Doom -RamMiB 8192 -Audio none
```

Le lanceur `run.ps1 -Doom` n'utilise pas un binaire `doom.exe` Windows. Il copie l'ELF Linux/musl sous `/doom`, le WAD sous `/freedoom1.wad` et transmet les entrees au processus ring 3 via `BO_GUI_FD` et la surface via `BO_SURFACE_FD`. Le bureau et le gestionnaire de fenetres restent independants du moteur de jeu.

Si Ladybird n'est pas disponible dans le depot local, `run.ps1` peut necessiter un artefact Ladybird GitHub Actions ou une construction native existante. Ce prerequis est **distinct** du portage DOOM. Exemple de qualification du 6 novembre :

```powershell
.\run.ps1 -Doom -LadybirdRunId 41920504209 -RamMiB 8192 -Audio none
```

## Provenance reproductible

| Composant | Provenance / version | Suivi par Git ? |
| --- | --- | --- |
| Adaptateur Bouchaud `doomgeneric_bouchaud.c` | Ce depot, protocole GUI v1 | **Oui** |
| Constructeur `build-windows.ps1` | Ce depot, verification ET_DYN et PT_INTERP | **Oui** |
| Doomgeneric upstream (GPL) | `ozkl/doomgeneric` commit `dcb7a8dbc7a16ce3dda29382ac9aae9d77d21284` | Non, re-clone |
| Freedoom 0.13.0 (IWAD) | Release `v0.13.0` avec SHA-256 archive epingle | Non, re-telecharge |
| Zig portable Windows | Zig 0.13.0 avec SHA-256 archive epingle | Non, re-telecharge |
| ELF `target/doom-bouchaud/doom-bouchaud` | Sortie de compilation `ET_DYN`, statique | Non, re-genere |
| Logo DOOM | `src/assets/atelier/apps/doom-{18,40,56}.png`, correctif #253 | **Oui** |

Les sources et scripts sont versionnes ; les binaires telecharges ou produits, le WAD et les images QEMU restent ignores par Git. Ne pas pousser `tools/doom/upstream`, `tools/doom/wads` ni `target/` : les licences amont s'appliquent aussi aux redistributions.

## Regressions interdites et preuves

- **Pas de `build-doom.sh` WSL** : la construction de reference est `build-windows.ps1`.
- **Pas d'ELF `ET_EXEC` classique** : le chargeur Bouchaud OS rejette ses segments hors de la plage user. `Check-Elf` exige `ET_DYN`, des `PT_LOAD` relatifs et aucun `PT_INTERP`. Ne pas assouplir le chargeur pour contourner ce controle.
- **Pas d'icone choisie via la position sur le bureau** : l'application DOOM utilise `icones::pour_kind(kind)` (correctif #253) ; les PNG de logo sont suivis.
- **Pas de dependant implicite au PC personnel** : le CI `doom-port-windows.yml` recompile depuis un checkout neuf avec les hashes epingles.
- **Pas d'audio DirectSoundCapture impose** : `run.ps1 -Doom` choisit `-Audio none` par defaut si `-Audio` n'est pas explicitement donne.

Temoins seriels du 6 novembre : `BOUCHAUD_DOOM_ATELIER_LAUNCH pid=17`, `GUI_SURFACE_CONTRAT pid=17`, `[DOOM] BO_GUI_FD=4`, `adding /freedoom1.wad`, `gui: premiere trame du client pid=17 (640x400)`.

### Limites

La preuve locale etablit chargement, affichage et animation. Elle ne qualifie pas encore l'ensemble des touches, le son ou tous les systemes physiques. Le CI controle la compilation et les contrats statiques ; il ne simule pas une session interactive QEMU. Ces points restent a valider avant de declarer #252 completement termine.
