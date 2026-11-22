# Validation WHPX du rendu Ladybird

Le bras de référence utilise `621a84a8b44061f891e372c70b6fe2531819a6b8`.
Le bras de rendu utilise la tête de #366 à qualifier. Les deux arbres ont
le même traitement des fautes anonymes. La branche de qualification mémoire
`codex/p0-fautes-anonymes-20261122` se mesure dans une autre campagne.

## Entrées

Dans le checkout #366, garder le serveur suivant ouvert pendant toute la campagne.

```powershell
py -m http.server 18082 --bind 127.0.0.1 --directory tools/health
```

Depuis chaque checkout, confirmer le SHA de la tête et le comparer au SHA
prévu pour le bras. Passer le run producteur du **bras courant**.
`45009297744` est le run de référence. Le producteur initial du correctif de
rendu est `45044774669`. Attendre la publication et l’état success avant
l’essai. Recalculer l’empreinte si le code Ladybird change.

```powershell
git rev-parse HEAD
git status --short --untracked-files=no
py tools/ci/ladybird_empreinte.py calcule
$repo = 'bcharthur/bouchaud-os'
# Passer l'identifiant du producteur du bras courant.
$run = 45044774669
gh run view $run -R $repo --json status,conclusion,headSha,url
gh api "repos/$repo/actions/runs/$run/artifacts" --jq '.artifacts[] | [.name,.expired,.digest] | @tsv'
```

Un checkout modifié, un producteur sans artefact disponible, un manifeste
incompatible ou l'absence de journal WHPX rend l’essai non qualifiable.
`run.ps1 -LadybirdRunId` impose la provenance de la recette et contrôle les
empreintes des binaires du manifeste avant de démarrer QEMU. Conserver ce
manifeste ainsi que la sortie du lanceur avec la preuve.

## Campagne

Préparer le même état de disque initial dans chaque checkout et le restaurer
avant chaque passage. Garder QEMU, l’hôte, la résolution et le profil CPU
constants. Faire trois paires A/B, B/A, A/B avec la même URL. Dans une
campagne distincte, ajouter `&scroll-only=1` au bras D et garder l’artefact
et le noyau constants. Le démarrage automatique attend 10 s après *Page prête*.

```powershell
# Créer un dossier d'identifiant différent par passage.
$sortie = New-Item -ItemType Directory -Path "$env:TEMP\bouchaud-whpx-A1" -ErrorAction Stop
$url = 'http://10.0.2.2:18082/ladybird-whpx-p0.html?phases=1&auto-start=1'
$debut = Get-Date
.\run.ps1 -Accel whpx -WhpxIrqchip off -CpuCount 1 -RamMiB 8192 `
  -Surface simple -Audio none -LadybirdRunId $run -Gate0Autostart -LadybirdUrl $url *>&1 |
  Tee-Object -FilePath (Join-Path $sortie 'lanceur.log')
$logs = @(Get-ChildItem target\journaux-qemu\*-serie.log |
  Where-Object LastWriteTime -ge $debut)
if ($logs.Count -ne 1) { throw 'Comparaison non qualifiable. Journal WHPX unique absent.' }
Copy-Item $logs[0].FullName $sortie
Copy-Item native-browser-m9\BOUCHAUD_ARTIFACT_MANIFEST.json $sortie -ErrorAction Stop
Get-FileHash native-browser-m9\BouchaudBrowserHost,native-browser-m9\WebContent -Algorithm SHA256 |
  Out-File (Join-Path $sortie 'binaires.txt')
git rev-parse HEAD | Out-File (Join-Path $sortie 'noyau.txt')
Select-String -Path $logs[0].FullName -Pattern 'BOUCHAUD_BUILD|WHPX_P0_SCROLL|WHPX_P0_CALLBACK|\[LB:FRAME\]|M11_RENDER_STATS' |
  ForEach-Object Line | Set-Content (Join-Path $sortie 'marqueurs.txt')
```

Comparer `degat`, `publie`, `copie_px` et `present_us` pendant la phase
de scroll. Le rectangle publié doit exclure le chrome stable. Examiner aussi
le premier affichage, le changement de taille, le changement d’onglet et le
double tampon dans des essais visuels distincts. Pour la latence visible,
utiliser une capture hôte horodatée. Les callbacks restent une mesure côté page.
