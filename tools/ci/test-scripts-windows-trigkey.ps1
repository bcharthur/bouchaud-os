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

# BOUCHAUD_PERF_V1 -- Mesure-Acceleration.ps1 : refus sans bootimage ; en
# simulation, chaque configuration a son accelerateur et `-cpu max`, et rien
# n'est lance.
$mesure = Join-Path $racine 'tools/windows/Mesure-Acceleration.ps1'
try { & $mesure -Bootimage "$t/absent.bin" -DryRun *> $null; Write-Host 'ECHEC mesure sans bootimage : aucun refus'; $echecs++ }
catch { if ($_.Exception.Message -match 'bootimage absent') { Write-Host 'ok    mesure : refus sans bootimage' } else { Write-Host "ECHEC mesure : $($_.Exception.Message)"; $echecs++ } }
$sortie = & $mesure -Bootimage $img -DryRun -Repetitions 2 -WhpxIrqchip on *>&1 | Out-String
$plans = ([regex]::Matches($sortie, 'PLAN ')).Count
# BOUCHAUD_WHPX_CPU_V1 : WHPX ne recoit JAMAIS `-cpu max` (QEMU 11 : MPX + APX,
# `WHPX: Unexpected VP exit code 4`) ; TCG le garde (SSE4.1 emule).
$cpuWhpx = [regex]::Escape('-cpu qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt')
if ($plans -eq 10 -and $sortie -match "PLAN whpx-smp4 #2 : .*-smp 4 .*-accel whpx,kernel-irqchip=on $cpuWhpx" -and $sortie -match 'PLAN tcg-smp1 #1 : .*-accel tcg -cpu max' -and $sortie -notmatch 'whpx,kernel-irqchip=\w+ -cpu max') {
    Write-Host 'ok    mesure : plan TCG (-cpu max) / WHPX (qemu64+SSE4) SMP1..4, irqchip transmis'
} else { Write-Host "ECHEC mesure : plan inattendu ($plans) $sortie"; $echecs++ }

# --- BOUCHAUD_WHPX_CPU_V1 : BouchaudQemu.psm1 -------------------------------
Import-Module (Join-Path $racine 'tools/windows/BouchaudQemu.psm1') -Force
function Verifie([string]$Nom, [bool]$Condition, [string]$Detail = '') {
    if ($Condition) { Write-Host "ok    $Nom" } else { Write-Host "ECHEC $Nom $Detail"; $script:echecs++ }
}
# L'analyseur PowerShell sur les scripts touches : une erreur de syntaxe dans
# run.ps1 ne se verrait sinon que sur Windows.
foreach ($f in @('run.ps1', 'tools/windows/BouchaudQemu.psm1', 'tools/windows/Mesure-Acceleration.ps1', 'tools/windows/Verifier-Ladybird.ps1')) {
    $jetons = $null; $erreursAnalyse = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile((Join-Path $racine $f), [ref]$jetons, [ref]$erreursAnalyse)
    Verifie "analyse $f" ($erreursAnalyse.Count -eq 0) (($erreursAnalyse | ForEach-Object { $_.Message }) -join ' ; ')
}
# Matrice des accelerateurs : aucun repli silencieux, jamais `max` sous WHPX.
$a = Get-BouchaudAccelQemu -Accel 'whpx' -CpuCount 4 -Ladybird $true -WhpxIrqchip 'off'
Verifie 'accel whpx : qemu64+SSE4, sans repli' (($a.arguments -join ' ') -eq '-accel whpx,kernel-irqchip=off -cpu qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt' -and -not $a.repli_tcg_explicite) ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'whpx' -CpuCount 1 -Ladybird $false -WhpxIrqchip 'on'
Verifie 'accel whpx hors Ladybird : meme modele' (($a.arguments -join ' ') -eq '-accel whpx,kernel-irqchip=on -cpu qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt') ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'auto' -CpuCount 1 -Ladybird $true
Verifie 'accel auto SMP1 : WHPX, repli annonce' ($a.accel -eq 'whpx' -and $a.repli_tcg_explicite -and ($a.arguments -notcontains 'tcg') -and ($a.arguments -notcontains 'max')) ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'auto' -CpuCount 4 -Ladybird $true -WhpxQualifie $false
Verifie 'accel auto SMP4 Ladybird non qualifie : TCG max' (($a.arguments -join ' ') -eq '-accel tcg -cpu max' -and $a.accel -eq 'tcg') ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'auto' -CpuCount 4 -Ladybird $true -WhpxQualifie $true
Verifie 'accel auto SMP4 qualifie : WHPX qemu64+SSE4' ($a.accel -eq 'whpx' -and ($a.arguments -contains 'qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt')) ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'tcg' -CpuCount 4 -Ladybird $true
Verifie 'accel tcg Ladybird : max' (($a.arguments -join ' ') -eq '-accel tcg -cpu max') ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'tcg' -CpuCount 1 -Ladybird $false
Verifie 'accel tcg hors Ladybird : inchange' (($a.arguments -join ' ') -eq '-accel tcg') ($a.arguments -join ' ')
$a = Get-BouchaudAccelQemu -Accel 'none' -CpuCount 1 -Ladybird $true
Verifie 'accel none : aucun argument' (@($a.arguments).Count -eq 0)
$rs = Get-Content -LiteralPath (Join-Path $racine 'run.ps1') -Raw
Verifie 'run.ps1 : plus de -cpu max en dur ni de chaine whpx+tcg' ($rs -notmatch '"-cpu",\s*"max"' -and $rs -match 'Get-BouchaudAccelQemu' -and $rs -match 'Invoke-BouchaudQemu' -and $rs -match 'logfile=')

# Verdict : ce que les journaux prouvent.
$serieTcg = Join-Path $t 'serie-tcg.log'
Set-Content -LiteralPath $serieTcg -Value ("$([char]27)[90m[x]$([char]27)[0mBOUCHAUD_CPU_ISA hyperviseur=TCGTCGTCGTCG fournisseur=AuthenticAMD sse3=1 ssse3=1 sse41=1 sse42=1 popcnt=1 xsave=1 osxsave=0 avx=1 avx2=1 avx512f=0 mpx=1 apxf=0 la57=1")
$serieWhpx = Join-Path $t 'serie-whpx.log'
Set-Content -LiteralPath $serieWhpx -Value 'BOUCHAUD_CPU_ISA hyperviseur=VMwareVMware fournisseur=AuthenticAMD sse3=1 ssse3=1 sse41=1 sse42=1 popcnt=1 xsave=0 osxsave=0 avx=0 avx2=0 avx512f=0 mpx=0 apxf=0 la57=0'
$errVide = Join-Path $t 'qemu-vide.log'; Set-Content -LiteralPath $errVide -Value ''
$errVp = Join-Path $t 'qemu-vp.log'
Set-Content -LiteralPath $errVp -Value @('qemu-system-x86_64.exe: warning: this feature conflicts with APX: CPUID[eax=07h,ecx=00h].EBX.mpx [bit 14]', 'WHPX: Unexpected VP exit code 4')
$errInit = Join-Path $t 'qemu-init.log'; Set-Content -LiteralPath $errInit -Value 'qemu-system-x86_64.exe: -accel whpx,kernel-irqchip=off: failed to initialize whpx: Invalid argument'
$v = Get-BouchaudVerdictQemu -JournalSerie $serieTcg -JournalErreurs $errVide
Verifie 'verdict : signature TCG => TCG' ($v.noyau -and $v.accel_reel -eq 'TCG' -and $v.isa.sse41 -eq 1 -and $v.isa.mpx -eq 1)
$code = Write-BouchaudVerdictQemu -Verdict $v -AccelDemande 'whpx' -JournalSerie $serieTcg -JournalErreurs $errVide 6>$null
Verifie 'verdict : TCG jamais presente comme WHPX (code 4)' ($code -eq 4) "code=$code"
$v = Get-BouchaudVerdictQemu -JournalSerie $serieWhpx -JournalErreurs $errVide
$code = Write-BouchaudVerdictQemu -Verdict $v -AccelDemande 'whpx' -JournalSerie $serieWhpx -JournalErreurs $errVide 6>$null
Verifie 'verdict : WHPX reel, aucun defaut (code 0)' ($v.accel_reel -eq 'materiel' -and $code -eq 0) "code=$code"
$v = Get-BouchaudVerdictQemu -JournalSerie (Join-Path $t 'absent.log') -JournalErreurs $errVp
$code = Write-BouchaudVerdictQemu -Verdict $v -AccelDemande 'whpx' -JournalSerie '' -JournalErreurs $errVp 6>$null
Verifie 'verdict : VP exit 4 + conflit MPX/APX remontes (code 3)' (-not $v.noyau -and $v.vp_exit -eq 4 -and $v.conflit_mpx_apx -and $code -eq 3) "code=$code"
$v = Get-BouchaudVerdictQemu -JournalSerie (Join-Path $t 'absent.log') -JournalErreurs $errInit
Verifie 'verdict : WHPX indisponible detecte' ($v.whpx_indisponible -and -not $v.noyau)

# Lancement : stderr recopie dans le journal, arguments intacts, code rendu.
if ($IsLinux -or $IsMacOS) {
    $faux = Join-Path $t 'faux-qemu.sh'
    Set-Content -LiteralPath $faux -Value @('#!/bin/sh', 'for a in "$@"; do printf ''ARG[%s]\n'' "$a" >&2; done', 'echo ''WHPX: Unexpected VP exit code 4'' >&2', 'exit 7')
    & chmod +x $faux
    $errLance = Join-Path $t 'qemu-lance.log'
    $codeLance = Invoke-BouchaudQemu -Exe $faux -Arguments @('-drive', 'format=raw,file=C:\Mes Images\boot.bin', 'x"y', 'c:\p\', '', 'simple') -JournalErreurs $errLance 2>$null
    $journalLance = Get-Content -LiteralPath $errLance -Raw
    $attendus = @('ARG[-drive]', 'ARG[format=raw,file=C:\Mes Images\boot.bin]', 'ARG[x"y]', 'ARG[c:\p\]', 'ARG[]', 'ARG[simple]', 'WHPX: Unexpected VP exit code 4')
    $manquants = @($attendus | Where-Object { -not $journalLance.Contains($_) })
    Verifie 'lancement : stderr journalise, arguments intacts, code de sortie' ($codeLance -eq 7 -and $manquants.Count -eq 0) "code=$codeLance manquants=$($manquants -join ',') journal=$journalLance"
}

# --- Banc : qualifications A (SMP) et B (Ladybird) separees -----------------
# Les 15 essais Windows du 13/11 etaient ok=False parce que le banc exigeait la
# premiere trame d'un Ladybird que rien ne lancait : un faux rouge SMP.
function SerieBanc([int]$n, [string]$sig = 'MicrosoftHv', [switch]$SansBattement, [switch]$Lance, [switch]$Trame, [switch]$Https) {
    $l = @("BOUCHAUD_BOOT_POINT kernel-main t_ms=0 delta_ms=0",
        "BOUCHAUD_CPU_ISA hyperviseur=$sig fournisseur=AuthenticAMD sse3=1 ssse3=1 sse41=1 sse42=1 popcnt=1 xsave=1 osxsave=0 avx=1 avx2=1 avx512f=1 mpx=0 apxf=0 la57=0",
        "BOUCHAUD_SMP_DECOUVERTE count=$n source=madt madt_actifs=$n")
    if ($n -gt 1) { $l += "BOUCHAUD_SMP_PHYSICAL_GREEN online=$n reported=$n" } else { $l += @('[kernel] SMP4_AP_STARTED count=0 reason=single-vcpu', '[kernel] SMP4_SCHEDULER online=1 mode=UP') }
    $l += "[kernel] SMP_NG2_SCHEDULER online=$n mode=thread-load-balance+work-steal quantum=4ms"
    if (-not $SansBattement) { for ($i = 0; $i -lt $n; $i++) { $l += "BOUCHAUD_SMP_BATTEMENT cpu=$i quantums=50 fenetre_ms=200 bat=1" }; $l += "BOUCHAUD_SMP_BATTEMENT_BILAN battants=$n en_ligne=$n mode=lapic-periodique" }
    $l += 'BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=4522 delta_ms=4462'
    if ($Lance) { $l += '[ladybird-bouchaud] BROWSER_HOST_START architecture=UI/Bouchaud' }
    if ($Trame) { $l += 'BOUCHAUD_UI_FIRST_FRAME onglet=1 page=1 zone=1100x538 source=compositor' }
    if ($Https) { $l += '[LB:NAV] onglet=1 document_charge url=https://example.com/' }
    $l += '[LB:PERF] onglet=1 trames=40 present_us_moy=812 present_us_pire=9001'
    $l += '[PROC-STAT] capacite_ms=40000 occupe_ms=10000 depassements=0'
    return ($l -join "`n")
}
$jal = @{ noyau = 13000; ordonnanceur = 15000; bureau = 17500; navigateur = 18000; premiere_trame = 21000; https = 22500 }
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal
Verifie 'banc A : WHPX SMP4 sans Ladybird = A OK, B NOT_RUN (plus de faux rouge)' ($a.smp.statut -eq 'OK' -and $a.ladybird.statut -eq 'NOT_RUN') "$($a.smp.causes) / $($a.ladybird.statut)"
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 1) -Erreurs '' -Accel 'whpx' -N 1 -Jalons $jal
Verifie 'banc A : SMP1 avec battement mesure = OK' ($a.smp.statut -eq 'OK' -and $a.smp.coeurs_qui_battent -eq 1) "$($a.smp.causes)"
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 1 -SansBattement) -Erreurs '' -Accel 'whpx' -N 1 -Jalons $jal
Verifie 'banc A : SMP1 sans preuve d''activite = ECHEC (pas de faux vert)' ($a.smp.statut -eq 'ECHEC' -and ($a.smp.causes -contains 'coeurs_qui_battent=0/1')) "$($a.smp.causes)"
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4 -sig 'TCGTCGTCGTCG') -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal
Verifie 'banc A : signature TCG sous configuration WHPX = ECHEC' ($a.smp.statut -eq 'ECHEC' -and ($a.smp.causes -contains 'accelerateur-reel=TCG'))
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4) -Erreurs 'WHPX: Unexpected VP exit code 4' -Accel 'whpx' -N 4 -Jalons $jal
Verifie 'banc A : sortie de VP = ECHEC' ($a.smp.statut -eq 'ECHEC' -and ($a.smp.causes -contains 'whpx-vp-exit=4'))
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4 -Lance -Trame -Https) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal -LadybirdStatut 'PRET' -LadybirdRaison ''
Verifie 'banc B : lance, 1re trame, HTTPS = OK, metriques separees' ($a.ladybird.statut -eq 'OK' -and $a.navigation.lancement_premiere_trame_ms -eq 3000 -and $a.navigation.lancement_document_https_ms -eq 4500 -and $a.demarrage.hote_jusqu_au_noyau_ms -eq 13000 -and $a.demarrage.noyau_jusqu_au_bureau_ms_invite -eq 4522 -and $a.composition.present_us_moy -eq 812 -and $a.charge_cpu.occupation_pct -eq 25) ($a | ConvertTo-Json -Depth 4 -Compress)
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4 -Lance) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal -LadybirdStatut 'PRET' -LadybirdRaison ''
Verifie 'banc B : lance sans 1re trame = ECHEC avec cause, A reste OK' ($a.ladybird.statut -eq 'ECHEC' -and ($a.ladybird.causes -contains 'premiere-trame-absente') -and $a.smp.statut -eq 'OK') "$($a.ladybird.causes)"
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4 -Lance -Trame) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal -LadybirdStatut 'PRET' -LadybirdRaison ''
Verifie 'banc B : 1re trame sans HTTPS = ECHEC distinct' ($a.ladybird.statut -eq 'ECHEC' -and ($a.ladybird.causes -contains 'document-https-non-charge') -and $a.ladybird.premiere_trame) "$($a.ladybird.causes)"
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal -LadybirdStatut 'PRET' -LadybirdRaison ''
Verifie 'banc B : navigateur jamais lance = ECHEC nomme' ($a.ladybird.causes -contains 'navigateur-non-lance-malgre-BO_AUTOSTART_BROWSER')
$a = Get-BouchaudAnalyseRepetition -Serie (SerieBanc 4) -Erreurs '' -Accel 'whpx' -N 4 -Jalons $jal -LadybirdStatut 'NON_MESURABLE' -LadybirdRaison 'python-introuvable'
Verifie 'banc B : NON_MESURABLE garde sa cause' ($a.ladybird.statut -eq 'NON_MESURABLE' -and ($a.ladybird.causes -contains 'python-introuvable'))

# Disque de banc : copie avec BO_AUTOSTART_BROWSER=1, source intacte.
$py = Get-BouchaudPython
if ($py) {
    $scen = Join-Path $t 'scen'; New-Item -ItemType Directory -Force -Path $scen | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $scen 'autorun'), "uname`nexport BOUCHAUD_M9_URL='https://example.com'`ngui-surface simple`ndesktop`n")
    [System.IO.File]::WriteAllText((Join-Path $scen 'bo-navigateur'), 'ELF')
    $src = Join-Path $t 'ladybird-browser.img'
    & $py[0] @($py | Select-Object -Skip 1) -c "import tarfile,sys; t=tarfile.open(sys.argv[1],'w',format=tarfile.USTAR_FORMAT); t.add(sys.argv[2]+'/autorun',arcname='./autorun'); t.add(sys.argv[2]+'/bo-navigateur',arcname='./bo-navigateur'); t.close()" $src $scen
    $avant = (Get-FileHash -LiteralPath $src -Algorithm SHA256).Hash
    $d = New-BouchaudDisqueBanc -Source $src -Cible (Join-Path $t 'banc.img')
    $apres = (Get-FileHash -LiteralPath $src -Algorithm SHA256).Hash
    $autorunBanc = & $py[0] @($py | Select-Object -Skip 1) -c "import tarfile,sys; print(tarfile.open(sys.argv[1]).extractfile('./autorun').read().decode())" (Join-Path $t 'banc.img') | Out-String
    Verifie 'disque de banc : autostart avant desktop, source intacte' ($d.statut -eq 'PRET' -and $avant -eq $apres -and $autorunBanc -match "BO_AUTOSTART_BROWSER=1\s+desktop") "$($d.statut) $($d.raison) $autorunBanc"
    $d = New-BouchaudDisqueBanc -Source (Join-Path $t 'absent.img') -Cible (Join-Path $t 'b2.img')
    Verifie 'disque de banc : disque absent = NON_MESURABLE avec cause' ($d.statut -eq 'NON_MESURABLE' -and $d.raison -match 'disque-absent')
    [System.IO.File]::WriteAllText((Join-Path $scen 'autorun'), "uname`n")
    & $py[0] @($py | Select-Object -Skip 1) -c "import tarfile,sys; t=tarfile.open(sys.argv[1],'w',format=tarfile.USTAR_FORMAT); t.add(sys.argv[2]+'/autorun',arcname='./autorun'); t.close()" (Join-Path $t 'sans-bureau.img') $scen
    $d = New-BouchaudDisqueBanc -Source (Join-Path $t 'sans-bureau.img') -Cible (Join-Path $t 'b3.img')
    Verifie 'disque de banc : autorun inconnu = NON_MESURABLE avec cause' ($d.statut -eq 'NON_MESURABLE' -and $d.raison -eq 'autorun-sans-ligne-desktop') "$($d.raison)"
}
else { Write-Host 'ECHEC disque de banc : python introuvable sur le runner'; $echecs++ }
$sortie = & $mesure -Bootimage $img -DryRun -Repetitions 1 -AvecLadybird -Configurations @('whpx-smp4') *>&1 | Out-String -Width 4096
Verifie 'mesure -AvecLadybird : la copie de banc est montee, jamais ladybird-browser.img' ($sortie -match 'ladybird-banc\.img,snapshot=on' -and $sortie -notmatch 'ladybird-browser\.img,snapshot') $sortie

Remove-Item -LiteralPath $t -Recurse -Force
if ($echecs) { Write-Host "SCRIPTS_WINDOWS_TRIGKEY_ECHEC echecs=$echecs"; exit 1 }
Write-Host 'SCRIPTS_WINDOWS_TRIGKEY_OK'
# Code explicite : le dernier programme natif lance (le disque de banc refuse,
# code 2, a dessein) laisserait $LASTEXITCODE non nul, et l'etape echouerait
# sur un succes.
exit 0
