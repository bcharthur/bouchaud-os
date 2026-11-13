# BOUCHAUD_WHPX_CPU_V1 -- choix de l'accelerateur et du modele CPU, lancement
# de QEMU journalise, verdict lu dans les journaux (jamais suppose).
#
# Utilise par run.ps1 et tools\windows\Mesure-Acceleration.ps1 ; teste sous
# pwsh par tools/ci/test-scripts-windows-trigkey.ps1.
#
# Pourquoi deux modeles CPU :
#
#   TCG  : `max`. QEMU EMULE chaque instruction et leve #UD sur celles que le
#          modele n'annonce pas : sans `max` (modele qemu64), pas de SSE4.1.
#
#   WHPX : `qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt`. Avec `max`, QEMU
#          11.0.50 configure le processeur virtuel avec des fonctions que
#          l'hote n'a pas -- MPX et APX ensemble sur un Ryzen qui n'a aucun
#          des deux ("conflicts with APX/MPX") -- puis
#          "WHPX: Unexpected VP exit code 4" (UnrecoverableException), aucun
#          journal noyau. Le modele retenu demarre (constate sur Windows,
#          Ryzen 7 7800X3D : WHPX SMP2, bureau et Ladybird).
#          CPUID REELLEMENT vu par l'invite sur cette machine (ligne
#          BOUCHAUD_CPU_ISA) : signature `MicrosoftHv`, sse41/sse42/popcnt=1,
#          et AVX, AVX2, AVX-512F annonces par Hyper-V lui-meme (feuille 7 de
#          l'hote, absente de qemu64) ; mpx=0 apxf=0. Le CPUID de l'invite
#          n'est donc PAS seulement celui du modele QEMU : seules certaines
#          feuilles passent par QEMU. Le materiel execute SSE4.1 de toute
#          facon : aucune commande de VMX ou de SVM ne fait lever #UD dessus.
#
# Le CPUID REEL vu par l'invite est publie par le noyau :
#   BOUCHAUD_CPU_ISA hyperviseur=... sse41=... mpx=... apxf=...
# et lu ici. `hyperviseur=TCGTCGTCGTCG` prouve un demarrage TCG : il n'est
# jamais presente comme un demarrage WHPX.

$script:CpuTcg = 'max'
$script:CpuWhpx = 'qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt'

function Get-BouchaudCpuQemu([string]$Accel) {
    if ($Accel -eq 'whpx') { return $script:CpuWhpx }
    return $script:CpuTcg
}

# Les arguments d'acceleration, et ce qu'on annonce. Aucun repli silencieux :
# `-accel whpx -accel tcg` n'est plus jamais passe a QEMU. En `auto`, un
# echec d'initialisation de WHPX est detecte par l'appelant
# (`repli_tcg_explicite`), qui relance en TCG en le DISANT.
function Get-BouchaudAccelQemu {
    param(
        [string]$Accel,
        [int]$CpuCount,
        [bool]$Ladybird,
        [string]$WhpxIrqchip = 'off',
        [bool]$WhpxQualifie = $false
    )
    $whpx = @('-accel', "whpx,kernel-irqchip=$WhpxIrqchip", '-cpu', $script:CpuWhpx)
    $tcg = @('-accel', 'tcg')
    if ($Ladybird) { $tcg += @('-cpu', $script:CpuTcg) }
    $r = [ordered]@{ arguments = @(); accel = $Accel; libelle = ''; repli_tcg_explicite = $false }
    switch ($Accel) {
        'none' {
            $r.libelle = 'aucune acceleration explicite (defaut de QEMU)'
        }
        'auto' {
            if ($Ladybird -and $CpuCount -gt 1 -and -not $WhpxQualifie) {
                # BOUCHAUD_PERF_V1 : WHPX en SMP seulement sur preuve locale.
                $r.arguments = $tcg; $r.accel = 'tcg'
                $r.libelle = "TCG force pour SMP ($CpuCount vCPU) : WHPX non qualifie pour ce noyau -> tools\windows\Mesure-Acceleration.ps1"
            }
            else {
                $r.arguments = $whpx; $r.accel = 'whpx'; $r.repli_tcg_explicite = $true
                $r.libelle = "WHPX SMP$CpuCount, cpu $($script:CpuWhpx) (si WHPX ne s'initialise pas : relance annoncee en TCG)"
            }
        }
        'whpx' {
            $r.arguments = $whpx
            $r.libelle = "WHPX SMP$CpuCount, cpu $($script:CpuWhpx), sans repli"
        }
        'tcg' {
            $r.arguments = $tcg
            $r.libelle = "TCG SMP$CpuCount" + $(if ($Ladybird) { ", cpu $($script:CpuTcg)" } else { '' })
        }
        default {
            $r.arguments = @('-accel', $Accel)
            $r.libelle = "$Accel (non qualifie)"
        }
    }
    return [pscustomobject]$r
}

# Une ligne de commande Windows (regles de CommandLineToArgvW / CRT) : chaque
# argument revient intact, espaces, guillemets et barres obliques compris.
function ConvertTo-BouchaudLigneArguments([string[]]$Arguments) {
    $morceaux = foreach ($a in $Arguments) {
        if ($null -eq $a -or $a -eq '') { '""'; continue }
        if ($a -notmatch '[\s"]') { $a; continue }
        $sb = New-Object System.Text.StringBuilder
        [void]$sb.Append('"')
        $barres = 0
        foreach ($c in $a.ToCharArray()) {
            if ($c -eq '\') { $barres++; continue }
            if ($c -eq '"') { [void]$sb.Append('\' * (2 * $barres + 1)); [void]$sb.Append('"') }
            else { [void]$sb.Append('\' * $barres); [void]$sb.Append($c) }
            $barres = 0
        }
        [void]$sb.Append('\' * (2 * $barres))
        [void]$sb.Append('"')
        $sb.ToString()
    }
    return ($morceaux -join ' ')
}

# Lance QEMU : entree et sortie standard HERITEES (la liaison serie `stdio`
# reste interactive), erreur standard recopiee EN DIRECT sur la console et
# dans $JournalErreurs. Rend le code de sortie de QEMU.
function Invoke-BouchaudQemu {
    param([string]$Exe, [string[]]$Arguments, [string]$JournalErreurs)
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = ConvertTo-BouchaudLigneArguments $Arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    $w = New-Object System.IO.StreamWriter($JournalErreurs, $false)
    $w.AutoFlush = $true
    $demarre = $false
    try {
        [void]$p.Start()
        $demarre = $true
        $lecture = $p.StandardError.ReadLineAsync()
        while ($true) {
            if (-not $lecture.Wait(250)) { continue }
            $ligne = $lecture.Result
            if ($null -eq $ligne) { break }
            $w.WriteLine($ligne)
            [Console]::Error.WriteLine($ligne)
            $lecture = $p.StandardError.ReadLineAsync()
        }
        $p.WaitForExit()
        return $p.ExitCode
    }
    finally {
        $w.Dispose()
        # Interruption de PowerShell : QEMU ne reste pas orphelin.
        if ($demarre -and -not $p.HasExited) { try { $p.Kill() } catch { } }
    }
}

# Lecture PARTAGEE : QEMU garde la liaison serie ouverte en ecriture
# (CreateFile avec FILE_SHARE_READ, chardev/char-file.c) ; ReadAllText refuse
# alors d'ouvrir le fichier ("en cours d'utilisation par un autre processus",
# constate sous Windows). FileShare ReadWrite accepte un ecrivain present.
function Read-BouchaudJournal([string]$Chemin) {
    if (-not $Chemin -or -not (Test-Path -LiteralPath $Chemin)) { return '' }
    $fs = New-Object System.IO.FileStream($Chemin, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
    try {
        $sr = New-Object System.IO.StreamReader($fs)
        $t = $sr.ReadToEnd()
    }
    finally { $fs.Dispose() }
    return [regex]::Replace($t, "$([char]27)\[[0-9;]*m", '')
}

# Ce que les journaux PROUVENT, sans rien supposer.
function Get-BouchaudVerdictQemu {
    param([string]$JournalSerie, [string]$JournalErreurs)
    $serie = Read-BouchaudJournal $JournalSerie
    $err = Read-BouchaudJournal $JournalErreurs
    $v = [ordered]@{
        noyau = $false; signature = $null; accel_reel = 'inconnu'; isa = $null
        vp_exit = $null; whpx_indisponible = $false; conflit_mpx_apx = $false
    }
    $m = [regex]::Match($serie, 'BOUCHAUD_CPU_ISA hyperviseur=(\S+) fournisseur=(\S+)((?: [a-z0-9]+=\d)+)')
    if ($m.Success) {
        $v.noyau = $true
        $v.signature = $m.Groups[1].Value
        $isa = [ordered]@{}
        foreach ($c in [regex]::Matches($m.Groups[3].Value, '([a-z0-9]+)=(\d)')) { $isa[$c.Groups[1].Value] = [int]$c.Groups[2].Value }
        $v.isa = [pscustomobject]$isa
        $v.accel_reel = switch ($v.signature) {
            'TCGTCGTCGTCG' { 'TCG' }
            'aucun' { 'aucun-hyperviseur' }
            default { 'materiel' }
        }
    }
    $e = [regex]::Match($err, 'WHPX: Unexpected VP exit code (\d+)')
    if ($e.Success) { $v.vp_exit = [int]$e.Groups[1].Value }
    $v.whpx_indisponible = $err -match '(?i)failed to initiali[sz]e whpx|WHPX: No accelerator found|invalid accelerator whpx|accelerator whpx (is )?not'
    $v.conflit_mpx_apx = $err -match 'conflicts with (APX|MPX)'
    return [pscustomobject]$v
}

# Le resume imprime apres QEMU, et le code a rendre (0 si rien d'anormal).
function Write-BouchaudVerdictQemu {
    param($Verdict, [string]$AccelDemande, [string]$JournalSerie, [string]$JournalErreurs)
    Write-Host ''
    Write-Host '=== Verdict QEMU (BOUCHAUD_WHPX_CPU_V1) ===' -ForegroundColor DarkGray
    Write-Host ("accel demandee : {0}" -f $AccelDemande)
    if ($Verdict.noyau) {
        $isa = ($Verdict.isa.PSObject.Properties | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' '
        Write-Host ("accel reelle   : {0} (signature CPUID {1})" -f $Verdict.accel_reel, $Verdict.signature)
        Write-Host ("CPUID invite   : {0}" -f $isa)
    }
    else {
        Write-Host 'accel reelle   : inconnue (aucune ligne BOUCHAUD_CPU_ISA : le noyau n''a rien ecrit)' -ForegroundColor Yellow
    }
    if ($JournalSerie) { Write-Host ("journal serie  : {0}" -f $JournalSerie) }
    Write-Host ("journal QEMU   : {0}" -f $JournalErreurs)
    $code = 0
    if ($Verdict.conflit_mpx_apx) {
        Write-Host 'QEMU a active MPX et APX ensemble : modele CPU incompatible avec cet accelerateur.' -ForegroundColor Red
    }
    if ($null -ne $Verdict.vp_exit) {
        Write-Host ("WHPX_ECHEC vp_exit={0} : le processeur virtuel s'est arrete sur une erreur irrecuperable. Joindre les deux journaux ; relancer en TCG : -Accel tcg." -f $Verdict.vp_exit) -ForegroundColor Red
        $code = 3
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.accel_reel -eq 'TCG') {
        Write-Host 'ATTENTION : ce demarrage a tourne sous TCG, PAS sous WHPX.' -ForegroundColor Red
        $code = 4
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.whpx_indisponible) {
        Write-Host 'WHPX_INDISPONIBLE : QEMU n''a pas pu initialiser WHPX (voir journal QEMU).' -ForegroundColor Red
        if ($code -eq 0) { $code = 5 }
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.noyau -and $Verdict.isa.sse41 -ne 1) {
        Write-Host 'CPUID invite sans SSE4.1 : Ladybird choisira ses variantes SSE2.' -ForegroundColor Yellow
    }
    return $code
}

# --- Banc d'acceleration : DEUX qualifications independantes ------------------
#
# A -- WHPX/SMP : accelerateur reel (signature CPUID), CPU decouverts, CPU en
#      ligne, CHAQUE coeur bat (BOUCHAUD_SMP_BATTEMENT, aussi en SMP1 depuis
#      BOUCHAUD_WHPX_CPU_V1), ordonnanceur, ni panique ni sortie de VP. Une
#      application non lancee n'y entre jamais.
# B -- Ladybird : lancement automatique (disque de banc), premiere trame, puis
#      document HTTPS charge, mesures SEPAREMENT. Statuts : OK, ECHEC (avec
#      cause), NOT_RUN (non demande), NON_MESURABLE (cause : disque, URL...).
#
# Les jalons sont les temps MURAUX cote hote (ms depuis le lancement de QEMU),
# releves par l'appelant ; tout le reste est lu dans les journaux.
$script:JalonsBanc = [ordered]@{
    noyau          = 'BOUCHAUD_BOOT_POINT kernel-main'
    ordonnanceur   = 'SMP_NG2_SCHEDULER'
    battement      = 'BOUCHAUD_SMP_BATTEMENT_BILAN'
    bureau         = 'bureau-premiere-trame'
    navigateur     = 'BROWSER_HOST_START'
    premiere_trame = 'BOUCHAUD_UI_FIRST_FRAME onglet=1'
    https          = '[LB:NAV] onglet=1 document_charge url=https://'
}

function Get-BouchaudJalonsBanc { return $script:JalonsBanc }

function Get-BouchaudAnalyseRepetition {
    param(
        [string]$Serie,
        [string]$Erreurs,
        [string]$Accel,
        [int]$N,
        [hashtable]$Jalons,
        [string]$LadybirdStatut = 'NOT_RUN',
        [string]$LadybirdRaison = '-AvecLadybird absent'
    )
    if (-not $Jalons) { $Jalons = @{} }
    $Serie = [regex]::Replace($Serie, "$([char]27)\[[0-9;]*m", '')
    # A -- SMP
    $causesA = New-Object System.Collections.Generic.List[string]
    $sig = [regex]::Match($Serie, 'BOUCHAUD_CPU_ISA hyperviseur=(\S+)')
    $signature = if ($sig.Success) { $sig.Groups[1].Value } else { $null }
    $accelReel = if (-not $signature) { 'inconnu' } elseif ($signature -eq 'TCGTCGTCGTCG') { 'TCG' } else { 'materiel' }
    $attendu = if ($Accel -eq 'tcg') { 'TCG' } else { 'materiel' }
    if ($accelReel -ne $attendu) { $causesA.Add("accelerateur-reel=$accelReel") }
    $vp = [regex]::Match($Erreurs, 'WHPX: Unexpected VP exit code (\d+)')
    if ($vp.Success) { $causesA.Add("whpx-vp-exit=$($vp.Groups[1].Value)") }
    $panique = $Serie -match 'KERNEL PANIC|DOUBLE FAULT|panicked at'
    if ($panique) { $causesA.Add('panique') }
    $d = [regex]::Match($Serie, 'BOUCHAUD_SMP_DECOUVERTE count=(\d+) source=(\w+)')
    $decouverts = if ($d.Success) { [int]$d.Groups[1].Value } else { 0 }
    $source = if ($d.Success) { $d.Groups[2].Value } else { '?' }
    if ($decouverts -ne $N) { $causesA.Add("decouverts=$decouverts") }
    $enLigne = 0
    $e = [regex]::Match($Serie, 'BOUCHAUD_SMP_PHYSICAL_(GREEN|PARTIAL) online=(\d+)')
    if ($e.Success) { $enLigne = [int]$e.Groups[2].Value }
    elseif ($Serie -match 'SMP4_SCHEDULER online=1 mode=UP') { $enLigne = 1 }
    if ($enLigne -ne $N) { $causesA.Add("en_ligne=$enLigne") }
    $battent = 0; $muets = 0
    foreach ($m in [regex]::Matches($Serie, 'BOUCHAUD_SMP_BATTEMENT cpu=(\d+) quantums=(\d+)')) {
        if ([int]$m.Groups[2].Value -gt 0) { $battent++ } else { $muets++ }
    }
    if ($battent -lt $N -or $muets -gt 0) { $causesA.Add("coeurs_qui_battent=$battent/$N") }
    if ($Serie -notmatch 'SMP_NG2_SCHEDULER online=') { $causesA.Add('ordonnanceur-absent') }
    $statutA = if ($causesA.Count -eq 0) { 'OK' } else { 'ECHEC' }

    # B -- Ladybird
    $causesB = New-Object System.Collections.Generic.List[string]
    $statutB = $LadybirdStatut
    $lance = $Serie.Contains($script:JalonsBanc.navigateur)
    $trame = $Serie.Contains($script:JalonsBanc.premiere_trame)
    $https = $Serie.Contains($script:JalonsBanc.https)
    $sigill = $Serie -match 'SIGILL|signal=4\b|invalid opcode'
    if ($statutB -eq 'PRET') {
        if ($panique -or $vp.Success) { $causesB.Add('systeme-arrete') }
        if ($sigill) { $causesB.Add('sigill') }
        if (-not $Serie.Contains($script:JalonsBanc.bureau)) { $causesB.Add('bureau-jamais-atteint') }
        elseif (-not $lance) { $causesB.Add('navigateur-non-lance-malgre-BO_AUTOSTART_BROWSER') }
        elseif (-not $trame) { $causesB.Add('premiere-trame-absente') }
        if ($trame -and -not $https) { $causesB.Add('document-https-non-charge') }
        $statutB = if ($causesB.Count -eq 0) { 'OK' } else { 'ECHEC' }
    }
    elseif ($LadybirdRaison) { $causesB.Add($LadybirdRaison) }

    # Metriques, par categorie, jamais melangees.
    $j = { param($k) if ($Jalons.ContainsKey($k)) { [double]$Jalons[$k] } else { $null } }
    $ecart = { param($a, $b) $x = & $j $a; $y = & $j $b; if ($null -ne $x -and $null -ne $y) { $y - $x } else { $null } }
    $tBureau = [regex]::Match($Serie, 'BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=(\d+)')
    $perf = [regex]::Matches($Serie, '\[LB:PERF\][^\n]*present_us_moy=(\d+)[^\n]*')
    $pire = [regex]::Matches($Serie, '\[LB:PERF\][^\n]*present_us_pire=(\d+)')
    $charge = [regex]::Matches($Serie, '\[PROC-STAT\] capacite_ms=(\d+) occupe_ms=(\d+)')
    $occupation = $null
    if ($charge.Count) {
        $c = $charge[$charge.Count - 1]
        if ([double]$c.Groups[1].Value -gt 0) { $occupation = [math]::Round(100 * [double]$c.Groups[2].Value / [double]$c.Groups[1].Value, 1) }
    }
    return [pscustomobject][ordered]@{
        smp = [pscustomobject][ordered]@{
            statut = $statutA; causes = @($causesA); accel_reel = $accelReel; signature = $signature
            vp_exit = $(if ($vp.Success) { [int]$vp.Groups[1].Value } else { $null }); panique = [bool]$panique
            decouverts = $decouverts; source = $source; en_ligne = $enLigne; coeurs_qui_battent = $battent; coeurs_muets = $muets
        }
        ladybird = [pscustomobject][ordered]@{
            statut = $statutB; causes = @($causesB); lance = $lance; premiere_trame = $trame; https_charge = $https; sigill = [bool]$sigill
        }
        demarrage = [pscustomobject][ordered]@{
            hote_jusqu_au_noyau_ms = (& $j 'noyau'); hote_jusqu_a_l_ordonnanceur_ms = (& $j 'ordonnanceur')
            hote_jusqu_au_bureau_ms = (& $j 'bureau')
            noyau_jusqu_au_bureau_ms_invite = $(if ($tBureau.Success) { [int]$tBureau.Groups[1].Value } else { $null })
        }
        navigation = [pscustomobject][ordered]@{
            lancement_premiere_trame_ms = (& $ecart 'navigateur' 'premiere_trame')
            lancement_document_https_ms = (& $ecart 'navigateur' 'https')
        }
        composition = [pscustomobject][ordered]@{
            present_us_moy = $(if ($perf.Count) { [int]$perf[$perf.Count - 1].Groups[1].Value } else { $null })
            present_us_pire = $(if ($pire.Count) { [int]$pire[$pire.Count - 1].Groups[1].Value } else { $null })
        }
        charge_cpu = [pscustomobject][ordered]@{ occupation_pct = $occupation }
        latence_entree = 'NON_MESURABLE : aucune entree injectee par le banc (-display none)'
        octets_serie = [System.Text.Encoding]::UTF8.GetByteCount($Serie)
    }
}

# Python de l'hote, comme run.ps1 (Ensure-Python), avec les lanceurs usuels.
function Get-BouchaudPython {
    foreach ($c in @(@('python'), @('py', '-3'), @('python3'))) {
        if (Get-Command $c[0] -ErrorAction SilentlyContinue) { return , $c }
    }
    return $null
}

# Copie de banc du disque Ladybird (tools\windows\disque-banc-ladybird.py) :
# rend @{ statut = 'PRET' | 'NON_MESURABLE'; raison; chemin }.
function New-BouchaudDisqueBanc {
    param([string]$Source, [string]$Cible)
    if (-not (Test-Path -LiteralPath $Source)) {
        return @{ statut = 'NON_MESURABLE'; raison = "disque-absent:$Source (lancer run.ps1 une fois)"; chemin = $null }
    }
    $py = Get-BouchaudPython
    if (-not $py) { return @{ statut = 'NON_MESURABLE'; raison = 'python-introuvable'; chemin = $null } }
    $outil = Join-Path $PSScriptRoot 'disque-banc-ladybird.py'
    # Windows PowerShell 5.1 change toute ligne stderr d'un programme natif
    # redirige en erreur TERMINALE sous 'Stop' : la cause doit remonter en
    # NON_MESURABLE, pas en exception.
    $ErrorActionPreference = 'Continue'
    $exe = $py[0]; $pre = @($py | Select-Object -Skip 1)
    $sortie = & $exe @pre '-I' $outil $Source $Cible 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0 -and $sortie -match 'DISQUE_BANC_OK') {
        return @{ statut = 'PRET'; raison = $sortie.Trim(); chemin = $Cible }
    }
    $raison = if ($sortie -match 'DISQUE_BANC_IMPOSSIBLE raison=(\S+)') { $Matches[1] } else { "outil-en-echec:$($sortie.Trim())" }
    return @{ statut = 'NON_MESURABLE'; raison = $raison; chemin = $null }
}

Export-ModuleMember -Function Get-BouchaudJalonsBanc, Get-BouchaudAnalyseRepetition, Get-BouchaudPython, New-BouchaudDisqueBanc, Read-BouchaudJournal, Get-BouchaudCpuQemu, Get-BouchaudAccelQemu, ConvertTo-BouchaudLigneArguments, Invoke-BouchaudQemu, Get-BouchaudVerdictQemu, Write-BouchaudVerdictQemu
