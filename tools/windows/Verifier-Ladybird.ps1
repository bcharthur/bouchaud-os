<#
.SYNOPSIS
    BOUCHAUD_E2E_LADYBIRD_V1 -- parcours E2E de Ladybird sur CETTE machine.

.DESCRIPTION
    La meme definition (tools\e2e\ladybird\scenarios.json), le meme
    orchestrateur (tools\e2e\ladybird\e2e.py), les memes assertions et la
    meme structure de rapport que la CI Linux ; seul l'accelerateur change.

      1. verifie Python et QEMU ;
      2. (re)construit le noyau (`cargo bootimage`) sauf -SansConstruction,
         et releve son empreinte SHA-256 : on ne teste jamais un ancien noyau
         sans le savoir ;
      3. prepare un disque E2E a partir de ladybird-browser.img (fabrique par
         run.ps1) SANS le modifier : session normale, navigateur ouvert AU
         MENU par le banc, page d'accueil = site de reference du banc ;
      4. lance QEMU (WHPX ou TCG) et joue les parcours par la souris et le
         clavier de QEMU (chemin reel : pilotes -> fenetres -> Ladybird) ;
      5. ecrit manifest.json, summary.json, results.json, performance.json,
         failures.json, serial.log, qemu.stderr.log, captures, rapport.html ;
      6. archive le tout dans target\e2e\<date>.zip ;
      7. ferme QEMU (aucun processus orphelin : l'orchestrateur le tue dans
         un finally).

    WHPX sur cette machine n'est PAS GitHub : les mesures d'ici ne se
    melangent jamais aux mesures TCG/KVM de la CI.

.EXAMPLE
    .\tools\windows\Verifier-Ladybird.ps1 -Profil complet -Accel whpx -CpuCount 4
    .\tools\windows\Verifier-Ladybird.ps1 -Profil rapide -Accel whpx -CpuCount 1
#>
[CmdletBinding()]
param(
    [ValidateSet('rapide', 'complet')][string]$Profil = 'complet',
    [ValidateSet('whpx', 'tcg')][string]$Accel = 'whpx',
    [int]$CpuCount = 4,
    [int]$RamMiB = 8192,
    [ValidateSet('off', 'on')][string]$WhpxIrqchip = 'off',
    [string]$Qemu = 'C:\Program Files\qemu\qemu-system-x86_64.exe',
    [string]$Disque,
    [switch]$SansConstruction
)
$ErrorActionPreference = 'Stop'
$Racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Racine

function Echec([string]$m) { Write-Host "E2E_REFUS $m" -ForegroundColor Red; exit 2 }

$py = Get-Command python -ErrorAction SilentlyContinue
if (-not $py) { Echec 'python introuvable (run.ps1 l''exige deja)' }
if (-not (Test-Path -LiteralPath $Qemu)) { Echec "QEMU introuvable : $Qemu" }
if (-not $Disque) { $Disque = Join-Path $Racine 'ladybird-browser.img' }
if (-not (Test-Path -LiteralPath $Disque)) { Echec "disque Ladybird absent : $Disque (lancer run.ps1 une fois)" }

if (-not $SansConstruction) {
    Write-Host '==> cargo bootimage (noyau courant)' -ForegroundColor Cyan
    & cargo bootimage
    if ($LASTEXITCODE -ne 0) { Echec "cargo bootimage a echoue (code $LASTEXITCODE)" }
}
$Boot = Join-Path $Racine 'target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin'
if (-not (Test-Path -LiteralPath $Boot)) { Echec "bootimage absent : $Boot" }
$empreinte = (Get-FileHash -LiteralPath $Boot -Algorithm SHA256).Hash.ToLower()
Write-Host "noyau      : $Boot sha256=$empreinte"

$Horodatage = Get-Date -Format 'yyyyMMdd-HHmmss'
$Sortie = Join-Path $Racine "target\e2e\$Horodatage-$Accel-smp$CpuCount"
New-Item -ItemType Directory -Force -Path $Sortie | Out-Null
$port = 18300
$disqueE2E = Join-Path $Sortie 'disque-e2e.img'
# Materiel TLS de banc (certificat expire, autorite de test) genere ICI,
# jamais commite. Sans cryptography ni openssl : E2E-03 NON_MESURABLE.
$tls = Join-Path $Sortie 'tls'
& $py.Source -I (Join-Path $Racine 'tools\e2e\ladybird\tls\genere_tls.py') $tls
$avecTls = ($LASTEXITCODE -eq 0)
$argsDisque = @((Join-Path $Racine 'tools\e2e\ladybird\prepare_disque.py'), '--image', $Disque,
    '--url', "http://10.0.2.2:$port/accueil.html", '--sortie', $disqueE2E)
if ($avecTls) { $argsDisque += @('--ca-test', (Join-Path $tls 'ca.pem')) }
& $py.Source -I @argsDisque
if ($LASTEXITCODE -ne 0) { Echec 'preparation du disque E2E impossible' }

# Profil : la meme definition ; `rapide` joue le demarrage et l'ouverture.
# Windows PowerShell 5.1 supprime un argument natif VIDE : les options
# facultatives ne sont donc ajoutees que si elles ont une valeur.
$arguments = @((Join-Path $Racine 'tools\e2e\ladybird\e2e.py'), '--boot', $Boot, '--disque', $disqueE2E,
    '--accel', $Accel, '--whpx-irqchip', $WhpxIrqchip, '--smp', "$CpuCount", '--memoire', "$RamMiB",
    '--qemu', $Qemu, '--sortie', $Sortie, '--port-fixture', "$port")
if ($avecTls) { $arguments += @('--tls-dir', $tls) }
if ($Profil -eq 'rapide') {
    $arguments += @('--scenarios', 'E2E-01,E2E-02', '--exige', 'E2E-01,E2E-02')
}
else {
    # E2E-03 (refus d'un certificat expire) n'est exige que si le materiel
    # TLS de banc a pu etre genere ; sinon il est NON_MESURABLE, et le dit.
    if ($avecTls) { $arguments += @('--exige', 'E2E-01,E2E-02,E2E-03,E2E-04,E2E-05') }
    else { $arguments += @('--exige', 'E2E-01,E2E-02,E2E-04,E2E-05') }
}
& $py.Source @arguments
$code = $LASTEXITCODE
Remove-Item -LiteralPath $disqueE2E -Force -ErrorAction SilentlyContinue
# Les cles de banc ne partent pas dans l'archive a transmettre.
Remove-Item -LiteralPath $tls -Recurse -Force -ErrorAction SilentlyContinue

$archive = "$Sortie.zip"
Compress-Archive -Path (Join-Path $Sortie '*') -DestinationPath $archive -Force
Write-Host ''
Write-Host "rapport    : $(Join-Path $Sortie 'rapport.html')"
Write-Host "archive    : $archive (a transmettre : manifest, summary, results, performance, failures, journaux, captures)"
exit $code
