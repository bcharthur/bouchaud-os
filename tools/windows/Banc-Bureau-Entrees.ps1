<#
.SYNOPSIS
    BOUCHAUD_SHM_RECT_V1 -- A/B du glisser de fenetre du bureau ring 3, sous
    WHPX, sur CETTE machine (PowerShell 5.1).

.DESCRIPTION
    Rejoue `tools\ci\banc_bureau_ring3_entrees.py` (vrais gestes par le
    moniteur QEMU : clic, glisser, frappes, plantage d'une application, case
    de fermeture) pour deux bras, en ALTERNANCE (A B A B ...), et releve pour
    chaque passage :

        BANC_BUREAU_ENTREES_GLISSER pas= moyen_us= max_us= appels_region_par_pas=
                                    copie_us= presentation_us=

    Chaque bras = UN noyau + SON disque : le composited d'apres appelle
    bo_shm_fill_rect (ABI 1.6), qu'un noyau d'avant (1.5) ne connait pas.
    Construire les disques sous Linux ou WSL (musl, mkdisk) :

        python3 tools/ci/banc_bureau_ring3_entrees.py --boot A.bin \
            --sortie disque-A --construit-seulement      # sur l'arbre d'avant
        python3 tools/ci/banc_bureau_ring3_entrees.py --boot B.bin \
            --sortie disque-B --construit-seulement      # sur l'arbre d'apres

    Les disques sont montes en snapshot=on : jamais modifies.

    Aucun seuil : le script publie medianes et pires valeurs, et le statut de
    chaque passage (un passage dont le banc echoue est compte ECHEC, jamais
    retire des chiffres en silence). Ce n'est une mesure WHPX que si QEMU a
    VRAIMENT demarre sous WHPX : le serie.log de chaque passage est garde.

.EXAMPLE
    .\tools\windows\Banc-Bureau-Entrees.ps1 -BootA A.bin -DisqueA disque-A\disque.img `
        -BootB B.bin -DisqueB disque-B\disque.img -Repetitions 3 -Smp 2
#>
param(
    [Parameter(Mandatory = $true)][string]$BootA,
    [Parameter(Mandatory = $true)][string]$DisqueA,
    [Parameter(Mandatory = $true)][string]$BootB,
    [Parameter(Mandatory = $true)][string]$DisqueB,
    [int]$Repetitions = 3,
    [int]$Smp = 2,
    [int]$Memoire = 4096,
    [string]$Qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe",
    [string]$Accel = "whpx",
    [string]$WhpxIrqchip = "off",
    [string]$Python = "python",
    [string]$Sortie = "target\banc-bureau-entrees"
)

$ErrorActionPreference = "Stop"
$racine = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$banc = Join-Path $racine "tools\ci\banc_bureau_ring3_entrees.py"

foreach ($f in @($BootA, $DisqueA, $BootB, $DisqueB, $Qemu, $banc)) {
    if (-not (Test-Path $f)) { throw "Introuvable : $f" }
}
New-Item -ItemType Directory -Force -Path $Sortie | Out-Null

function Get-Mediane([double[]]$v) {
    if ($v.Count -eq 0) { return $null }
    $t = $v | Sort-Object
    $m = [int][math]::Floor($t.Count / 2)
    if ($t.Count % 2 -eq 1) { return $t[$m] }
    return ($t[$m - 1] + $t[$m]) / 2
}

$bras = @(
    @{ Nom = "A"; Boot = (Resolve-Path $BootA).Path; Disque = (Resolve-Path $DisqueA).Path },
    @{ Nom = "B"; Boot = (Resolve-Path $BootB).Path; Disque = (Resolve-Path $DisqueB).Path }
)
$lignes = New-Object System.Collections.ArrayList

for ($r = 1; $r -le $Repetitions; $r++) {
    foreach ($b in $bras) {
        $dossier = Join-Path $Sortie ("{0}-{1}-smp{2}-r{3}" -f $b.Nom, $Accel, $Smp, $r)
        Write-Host ("== bras {0}, passage {1}/{2} ==" -f $b.Nom, $r, $Repetitions)
        $sortieTexte = & $Python $banc --boot $b.Boot --disque $b.Disque --sortie $dossier `
            --accel $Accel --whpx-irqchip $WhpxIrqchip --qemu $Qemu --smp $Smp `
            --memoire $Memoire --moniteur tcp --etiquette ("{0}-r{1}" -f $b.Nom, $r) 2>&1
        $code = $LASTEXITCODE
        $sortieTexte | Out-File -Encoding utf8 (Join-Path $Sortie ("{0}-r{1}.txt" -f $b.Nom, $r))
        $texte = ($sortieTexte | Out-String)
        $statut = "ECHEC"
        if ($code -eq 0 -and $texte -match "BANC_BUREAU_ENTREES_OK") { $statut = "OK" }
        $ligne = [ordered]@{ bras = $b.Nom; passage = $r; statut = $statut; pas = $null; moyen_us = $null;
                             max_us = $null; appels_region_par_pas = $null; copie_us = $null; presentation_us = $null }
        $m = [regex]::Match($texte, "BANC_BUREAU_ENTREES_GLISSER pas=(\d+) moyen_us=(\d+) max_us=(\d+) appels_region_par_pas=(\d+) copie_us=(\d+) presentation_us=(\d+)")
        if ($m.Success) {
            $ligne.pas = [int]$m.Groups[1].Value
            $ligne.moyen_us = [int]$m.Groups[2].Value
            $ligne.max_us = [int]$m.Groups[3].Value
            $ligne.appels_region_par_pas = [int]$m.Groups[4].Value
            $ligne.copie_us = [int]$m.Groups[5].Value
            $ligne.presentation_us = [int]$m.Groups[6].Value
        }
        # L'accelerateur REEL, lu sur la liaison serie, pas deduit de l'option.
        $serie = Join-Path $dossier "serie.log"
        $ligne.isa = ""
        if (Test-Path $serie) {
            $isa = Select-String -Path $serie -Pattern "BOUCHAUD_CPU_ISA[^\r\n]*" | Select-Object -First 1
            if ($isa) { $ligne.isa = $isa.Matches[0].Value }
        }
        Write-Host ("  {0} moyen_us={1} max_us={2} appels={3}" -f $statut, $ligne.moyen_us, $ligne.max_us, $ligne.appels_region_par_pas)
        [void]$lignes.Add((New-Object PSObject -Property $ligne))
    }
}

$csv = Join-Path $Sortie "banc-bureau-entrees.csv"
$lignes | Export-Csv -NoTypeInformation -Encoding UTF8 $csv

Write-Host ""
Write-Host ("BANC_BUREAU_ENTREES_AB accel={0} smp={1} repetitions={2}" -f $Accel, $Smp, $Repetitions)
foreach ($nom in @("A", "B")) {
    $ok = @($lignes | Where-Object { $_.bras -eq $nom -and $_.statut -eq "OK" -and $_.moyen_us -ne $null })
    $echecs = @($lignes | Where-Object { $_.bras -eq $nom -and $_.statut -ne "OK" }).Count
    $moy = Get-Mediane ([double[]]@($ok | ForEach-Object { $_.moyen_us }))
    $pire = $null
    if ($ok.Count -gt 0) { $pire = ($ok | Measure-Object -Property max_us -Maximum).Maximum }
    $pres = Get-Mediane ([double[]]@($ok | ForEach-Object { $_.presentation_us }))
    $app = Get-Mediane ([double[]]@($ok | ForEach-Object { $_.appels_region_par_pas }))
    Write-Host ("  bras {0} : passages_ok={1} echecs={2} moyen_us_mediane={3} max_us_pire={4} presentation_us_mediane={5} appels_region_par_pas={6}" -f `
        $nom, $ok.Count, $echecs, $moy, $pire, $pres, $app)
}
Write-Host "Detail : $csv (et serie.log de chaque passage dans $Sortie)"
