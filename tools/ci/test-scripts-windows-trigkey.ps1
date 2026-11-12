# BOUCHAUD_WINDOWS_V1 -- EXECUTION des chemins de refus des scripts Windows du Trigkey.
#
# Analyser un script ne dit pas qu'il fonctionne (voir test-selection-artefact.ps1).
# Ce test les EXECUTE sous pwsh avec des disques SIMULES (Get-Disk / Set-Disk
# remplaces) : aucune ecriture ne doit jamais etre tentee sans identite
# exacte, cle USB, confirmation tapee. Un refus qui laisserait passer, ou
# une ecriture tentee, fait echouer le test.
$ErrorActionPreference = 'Stop'
$racine = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$script = Join-Path $racine 'tools/windows/Prepare-TrigkeyUSB.ps1'
$echecs = 0

$global:ECRITURES = 0
function global:Set-Disk { param($Number, $IsOffline) $global:ECRITURES++ }
$global:DISQUES = @(
    [pscustomobject]@{ Number = 0; FriendlyName = 'Disque Systeme'; SerialNumber = 'SYS0001'; Size = 512GB; BusType = 'NVMe'; IsBoot = $true; IsSystem = $true },
    [pscustomobject]@{ Number = 3; FriendlyName = 'Cle Test'; SerialNumber = 'CLE42'; Size = 16GB; BusType = 'USB'; IsBoot = $false; IsSystem = $false },
    [pscustomobject]@{ Number = 4; FriendlyName = 'Petite Cle'; SerialNumber = 'MINI1'; Size = 32KB; BusType = 'USB'; IsBoot = $false; IsSystem = $false },
    [pscustomobject]@{ Number = 5; FriendlyName = 'SSD Externe'; SerialNumber = 'SATA9'; Size = 1TB; BusType = 'SATA'; IsBoot = $false; IsSystem = $false },
    [pscustomobject]@{ Number = 6; FriendlyName = 'Cle Sans Serie'; SerialNumber = ''; Size = 16GB; BusType = 'USB'; IsBoot = $false; IsSystem = $false }
)
function global:Get-Disk { param($Number = -1, $ErrorAction)
    if ($Number -lt 0) { return $global:DISQUES }
    return $global:DISQUES | Where-Object Number -EQ $Number
}

$t = Join-Path ([System.IO.Path]::GetTempPath()) "bo-ps-$PID"
New-Item -ItemType Directory -Force -Path $t | Out-Null
$img = Join-Path $t 'cle.img'
[System.IO.File]::WriteAllBytes($img, (New-Object byte[] 65536))
$h = (Get-FileHash -Algorithm SHA256 -LiteralPath $img).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$img.sha256" -Value "$h  cle.img"

function Attendu-Refus([string]$Nom, [string]$Motif, [hashtable]$Params) {
    $avant = $global:ECRITURES
    try {
        & $script @Params *> $null
        Write-Host "ECHEC $Nom : aucun refus"; $script:echecs++
    } catch {
        if ($_.Exception.Message -notmatch [regex]::Escape($Motif)) {
            Write-Host "ECHEC $Nom : refus inattendu ($($_.Exception.Message))"; $script:echecs++
        } elseif ($global:ECRITURES -ne $avant) {
            Write-Host "ECHEC $Nom : une ecriture a ete tentee"; $script:echecs++
        } else { Write-Host "ok    $Nom" }
    }
}

$base = @{ Mode = 'Validation'; Image = $img }
# Les parametres d'un cas : $base, complete ou REMPLACE par $Autres (une
# addition de tables refuse une cle en double, ex. Image).
function P([hashtable]$Autres) {
    $t = $base.Clone()
    foreach ($k in $Autres.Keys) { $t[$k] = $Autres[$k] }
    return $t
}
Attendu-Refus 'sans -Disque' 'obligatoire' (P @{ Identite = 'x' })
Attendu-Refus 'disque systeme' 'non USB' (P @{ Disque = 0; Identite = "Disque Systeme|SYS0001|$(512GB)" })
Attendu-Refus 'identite differente' 'identite differente' (P @{ Disque = 3; Identite = "Cle Test|AUTRE|$(16GB)" })
Attendu-Refus 'taille differente' 'identite differente' (P @{ Disque = 3; Identite = "Cle Test|CLE42|1" })
Attendu-Refus 'disque non USB' 'non USB' (P @{ Disque = 5; Identite = "SSD Externe|SATA9|$(1TB)" })
Attendu-Refus 'cle trop petite' 'trop petite' (P @{ Disque = 4; Identite = "Petite Cle|MINI1|$(32KB)" })
Attendu-Refus 'serie illisible' 'serie illisible' (P @{ Disque = 6; Identite = "Cle Sans Serie||$(16GB)" })
Attendu-Refus 'image alteree' 'empreinte' (P @{ Disque = 3; Identite = "Cle Test|CLE42|$(16GB)"; Image = (& { [System.IO.File]::WriteAllBytes("$t/autre.img", [byte[]](1..64)); Copy-Item "$img.sha256" "$t/autre.img.sha256"; "$t/autre.img" }) })
# Positif : tout est conforme, mais SANS -Execute -> simulation, rien n'est ecrit.
$avant = $global:ECRITURES
$sortie = & $script @base -Disque 3 -Identite "Cle Test|CLE42|$(16GB)" *>&1 | Out-String
if ($sortie -match 'USB_DRY_RUN' -and $global:ECRITURES -eq $avant) { Write-Host 'ok    simulation par defaut' }
else { Write-Host "ECHEC simulation par defaut : $sortie"; $echecs++ }
# Confirmation fausse avec -Execute : refus, rien d'ecrit.
function global:Read-Host { param($Prompt) return 'oui' }
Attendu-Refus 'confirmation fausse' 'confirmation' (P @{ Disque = 3; Identite = "Cle Test|CLE42|$(16GB)"; Execute = $true })

Remove-Item -LiteralPath $t -Recurse -Force
if ($echecs) { Write-Host "SCRIPTS_WINDOWS_TRIGKEY_ECHEC echecs=$echecs"; exit 1 }
Write-Host 'SCRIPTS_WINDOWS_TRIGKEY_OK'
