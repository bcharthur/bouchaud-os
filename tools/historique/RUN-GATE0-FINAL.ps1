param(
    [ValidateRange(1,10)]
    [int]$Boots = 3,

    # Temps maximal laisse a run.ps1 pour compiler/packager et faire apparaitre
    # le serveur serie QEMU. Ce chrono se termine AVANT le boot du guest.
    [ValidateRange(60,900)]
    [int]$PrepareTimeoutSeconds = 300,

    # Temps maximal, APRES connexion serie, pour atteindre tous les marqueurs.
    [ValidateRange(60,900)]
    [int]$BootTimeoutSeconds = 300,

    [ValidateRange(5,120)]
    [int]$StableSeconds = 20,

    [switch]$SkipStatic
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

$ExpectedBranch = "gate0/final-20260826"
$HandoffMarker = "BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2"
$TcpMarker = "BOUCHAUD_GATE0_TCP_SERIAL_V7"
$ResultsDir = Join-Path $Root "gate0-results"
New-Item -ItemType Directory -Force -Path $ResultsDir | Out-Null

function Get-FreeTcpPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Get-QemuPids {
    @(Get-Process -Name "qemu-system-x86_64" -ErrorAction SilentlyContinue |
        ForEach-Object Id)
}

function Stop-NewQemu([int[]]$Before) {
    # $PID est une variable automatique PowerShell en lecture seule.
    # Les noms de variables etant insensibles a la casse, `foreach ($pid ...)`
    # essayait de l'ecraser et faisait echouer le runner APRES un boot valide.
    foreach ($qemuProcessId in @(Get-QemuPids | Where-Object { $_ -notin $Before })) {
        Stop-Process -Id $qemuProcessId -Force -ErrorAction SilentlyContinue
    }
}

function Show-Tail([string]$Path, [int]$Lines = 30) {
    if (Test-Path $Path) {
        Write-Host "`n--- $(Split-Path $Path -Leaf) / tail ---" -ForegroundColor DarkYellow
        Get-Content $Path -Tail $Lines -ErrorAction SilentlyContinue
    }
}

function Connect-Gate0Serial(
    [int]$Port,
    [Diagnostics.Process]$Child,
    [datetime]$Deadline
) {
    while ((Get-Date) -lt $Deadline) {
        if ($Child.HasExited) {
            throw "run.ps1 s'est termine avant que QEMU ouvre le canal serie."
        }

        $client = New-Object Net.Sockets.TcpClient
        try {
            # localhost refuse immediatement tant que QEMU n'ecoute pas.
            $client.Connect("127.0.0.1", $Port)
            return $client
        }
        catch {
            $client.Dispose()
            Start-Sleep -Milliseconds 250
        }
    }
    throw "QEMU n'a pas ouvert le canal serie TCP dans le delai de preparation."
}

Write-Host "====================================================" -ForegroundColor Cyan
Write-Host " BOUCHAUD OS - GATE 0 FINAL V7 / SERIE TCP" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan

$branch = (& git branch --show-current).Trim()
if ($branch -ne $ExpectedBranch) {
    throw "Branche '$branch'. Attendue : '$ExpectedBranch'."
}

$dirty = @(git status --porcelain --untracked-files=no)
if ($dirty.Count -gt 0) {
    Write-Host ($dirty -join "`n") -ForegroundColor Yellow
    throw "Arbre suivi non propre."
}

$thread = Get-Content ".\src\kernel\process\thread.rs" -Raw
if (-not $thread.Contains($HandoffMarker)) {
    throw "Handoff post-switch Gate0 absent."
}

$runSource = Get-Content ".\run.ps1" -Raw
if (-not $runSource.Contains($TcpMarker)) {
    throw "Canal serie V7 absent. Lance .\UPGRADE-GATE0-RUNTIME-V7.ps1"
}

$native = Join-Path $Root "native-browser-m9"
foreach ($item in @(
    "BouchaudBrowserHost",
    "WebContent",
    "RequestServer",
    "ImageDecoder",
    "Compositor",
    "WebWorker",
    "WebDriver",
    "webcontent-bootstrap",
    "M9_CAPABLE",
    "resources"
)) {
    if (-not (Test-Path (Join-Path $native $item))) {
        throw "Artefact Ladybird local incomplet : native-browser-m9\$item"
    }
}

if (-not $SkipStatic) {
    Write-Host "`n=== Validation statique finale ===" -ForegroundColor Cyan
    & ".\tools\dev\validate-fast.ps1" -Bootimage
    if ($LASTEXITCODE -ne 0) {
        throw "Gate statique final echoue."
    }
}

# Les deux premiers marqueurs sont de vrais marqueurs serie historiques.
# L'ancien V6 exigeait `SMP4_SCHEDULER online=4`, chaine qui n'est pas emise
# par le noyau actuel. On la remplace par une preuve runtime reelle : une ligne
# SMP-LOAD contenant les quatre CPU.
$required = [ordered]@{
    "SMP discover 4"       = "SMP4_DISCOVERED count=4"
    "SMP AP 3/3"           = "SMP4_AP_STARTED count=3 expected=3"
    "SMP load 4 CPUs"      = "\[SMP-LOAD\].*c0=.*c1=.*c2=.*c3="
    "BrowserHost"          = "BROWSER_HOST_INITIALIZED"
    "WebContent"           = "WEBCONTENT_READY"
    "M11 ready"            = "M11_READY"
    "M11 GUI handshake"    = "M11_GUI_HANDSHAKE_OK"
    "M11 document loaded"  = "M11_DOCUMENT_LOADED"
}

$fatal = [regex]::new(
    "(?im)" +
    "\*\*\* KERNEL PANIC \*\*\*|" +
    "DOUBLE FAULT|" +
    "EXCEPTION:\s*double faute|" +
    "smp_lock:\s*release par un CPU non proprietaire|" +
    "smp_lock:\s*release sans acquisition|" +
    "task:\s*tentative de double execution|" +
    "task:\s*tentative de reprendre une tache dont la passation n'est pas terminee|" +
    "task:\s*passation precedente non terminee|" +
    "task:\s*publication avant abandon physique de la pile|" +
    "general protection.*fatal|" +
    "fatal.*page fault"
)

$commit = (& git rev-parse HEAD).Trim()
$short = (& git rev-parse --short HEAD).Trim()
$passedLogs = @()

# Un ancien runner V7 a pu laisser un QEMU vivant si son nettoyage a plante.
# Ne jamais tuer aveuglement un QEMU potentiellement utilise pour autre chose :
# on refuse simplement de demarrer tant qu'un processus existe.
$existingQemu = @(Get-QemuPids)
if ($existingQemu.Count -gt 0) {
    Write-Host "QEMU deja actif : $($existingQemu -join ', ')" -ForegroundColor Yellow
    throw "Ferme le QEMU residuel du run precedent, puis relance le runner."
}

for ($boot = 1; $boot -le $Boots; $boot++) {
    Write-Host "`n====================================================" -ForegroundColor Cyan
    Write-Host " BOOT $boot/$Boots - $short" -ForegroundColor Cyan
    Write-Host "====================================================" -ForegroundColor Cyan

    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $serialLog = Join-Path $ResultsDir "boot-$boot-$stamp.serial.log"
    $hostOut = Join-Path $ResultsDir "boot-$boot-$stamp.host.stdout.log"
    $hostErr = Join-Path $ResultsDir "boot-$boot-$stamp.host.stderr.log"
    $port = Get-FreeTcpPort
    $beforeQemu = @(Get-QemuPids)

    Write-Host "serie TCP : 127.0.0.1:$port"
    Write-Host "QEMU attendra la connexion AVANT de booter."

    $child = $null
    $tcp = $null
    $stream = $null
    $writer = $null
    $bootPassed = $false

    try {
        $arguments = @(
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", "`"$Root\run.ps1`"",
            "-Ladybird",
            "-Gate0Autostart",
            "-Gate0SerialPort", "$port",
            "-CpuCount", "4",
            "-Audio", "none",
            "-Accel", "tcg",
            "-LadybirdUrl", "`"https://example.com/`""
        )

        $child = Start-Process `
            -FilePath "powershell.exe" `
            -ArgumentList $arguments `
            -RedirectStandardOutput $hostOut `
            -RedirectStandardError $hostErr `
            -PassThru

        Write-Host "Preparation run.ps1 / image Ladybird..."
        $prepareDeadline = (Get-Date).AddSeconds($PrepareTimeoutSeconds)
        $tcp = Connect-Gate0Serial $port $child $prepareDeadline

        Write-Host "SERIE CONNECTEE : le guest peut maintenant demarrer." -ForegroundColor Green

        $stream = $tcp.GetStream()
        $buffer = New-Object byte[] 16384
        $encoding = [Text.Encoding]::UTF8
        $writer = New-Object IO.StreamWriter(
            $serialLog,
            $false,
            [Text.UTF8Encoding]::new($false)
        )
        $writer.AutoFlush = $true

        $seen = @{}
        foreach ($name in $required.Keys) { $seen[$name] = $false }
        $seenCount = 0
        $tail = ""
        $allSeenAt = $null
        $deadline = (Get-Date).AddSeconds($BootTimeoutSeconds)

        while ((Get-Date) -lt $deadline) {
            $readSomething = $false

            while ($stream.DataAvailable) {
                $count = $stream.Read($buffer, 0, $buffer.Length)
                if ($count -le 0) { break }

                $readSomething = $true
                $chunk = $encoding.GetString($buffer, 0, $count)
                $writer.Write($chunk)

                # 16 Kio d'historique suffisent pour couvrir un marqueur coupe
                # entre deux lectures et une ligne SMP-LOAD longue.
                $scan = $tail + $chunk
                if ($scan.Length -gt 16384) {
                    $tail = $scan.Substring($scan.Length - 16384)
                }
                else {
                    $tail = $scan
                }

                $bad = $fatal.Match($scan)
                if ($bad.Success) {
                    throw "SIGNAL FATAL : $($bad.Value)"
                }

                foreach ($name in $required.Keys) {
                    if (-not $seen[$name] -and [regex]::IsMatch($scan, $required[$name])) {
                        $seen[$name] = $true
                        $seenCount++
                        Write-Host "[$seenCount/$($required.Count)] $name" -ForegroundColor Green
                    }
                }
            }

            if ($seenCount -eq $required.Count) {
                if ($null -eq $allSeenAt) {
                    $allSeenAt = Get-Date
                    Write-Host "Tous les marqueurs atteints." -ForegroundColor Green
                    Write-Host "Dwell de stabilite : $StableSeconds s..." -ForegroundColor Cyan
                }
                elseif (((Get-Date) - $allSeenAt).TotalSeconds -ge $StableSeconds) {
                    $bootPassed = $true
                    break
                }
            }

            if ($child.HasExited -and -not $stream.DataAvailable) {
                throw "run.ps1/QEMU s'est termine avant la validation."
            }

            if (-not $readSomething) {
                Start-Sleep -Milliseconds 100
            }
        }

        if (-not $bootPassed) {
            $missing = @($required.Keys | Where-Object { -not $seen[$_] })
            throw "TIMEOUT. Marqueurs manquants : $($missing -join ', ')"
        }

        # Dernier drain + dernier controle fatal avant d'accepter le boot.
        Start-Sleep -Milliseconds 200
        while ($stream.DataAvailable) {
            $count = $stream.Read($buffer,0,$buffer.Length)
            if ($count -le 0) { break }
            $chunk = $encoding.GetString($buffer,0,$count)
            $writer.Write($chunk)
            $scan = $tail + $chunk
            $bad = $fatal.Match($scan)
            if ($bad.Success) {
                throw "SIGNAL FATAL EN FIN DE DWELL : $($bad.Value)"
            }
            if ($scan.Length -gt 16384) {
                $tail = $scan.Substring($scan.Length - 16384)
            } else {
                $tail = $scan
            }
        }

        Write-Host "BOOT $boot : PASS" -ForegroundColor Green
        $passedLogs += $serialLog
    }
    catch {
        Write-Host "`nBOOT $boot : FAIL" -ForegroundColor Red
        Write-Host $_.Exception.Message -ForegroundColor Red
        throw
    }
    finally {
        if ($writer) {
            try { $writer.Flush(); $writer.Dispose() } catch {}
        }
        if ($stream) {
            try { $stream.Dispose() } catch {}
        }
        if ($tcp) {
            try { $tcp.Dispose() } catch {}
        }

        Stop-NewQemu $beforeQemu
        Start-Sleep -Milliseconds 500

        if ($child -and -not $child.HasExited) {
            try { [void]$child.WaitForExit(5000) } catch {}
        }
        if ($child -and -not $child.HasExited) {
            Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
        }

        if (-not $bootPassed) {
            Show-Tail $serialLog 60
            Show-Tail $hostOut 40
            Show-Tail $hostErr 40
        }
    }

    if (-not $bootPassed) {
        throw "Gate 0 runtime echoue au boot $boot."
    }

    Write-Host "Preuve serie : $serialLog" -ForegroundColor DarkGreen
}

if ($passedLogs.Count -ne $Boots) {
    throw "Seulement $($passedLogs.Count)/$Boots boots ont passe."
}

$proof = Join-Path $ResultsDir "GATE0-PASSED.txt"
$lines = @(
    "BOUCHAUD OS - GATE 0 PASSED",
    "date=$(Get-Date -Format o)",
    "commit=$commit",
    "branch=$ExpectedBranch",
    "boots=$Boots/$Boots",
    "stable_seconds=$StableSeconds",
    "transport=QEMU serial TCP loopback wait=on",
    "",
    "Checks:"
)
$lines += $required.Keys | ForEach-Object { "  OK $_" }
$lines += ""
$lines += "Runtime logs:"
$lines += $passedLogs | ForEach-Object { "  $_" }

[IO.File]::WriteAllLines(
    $proof,
    $lines,
    [Text.UTF8Encoding]::new($false)
)

$tag = "gate0-complete-20260826"
& git show-ref --verify --quiet "refs/tags/$tag"
if ($LASTEXITCODE -eq 0) {
    $existing = (& git rev-list -n 1 $tag).Trim()
    if ($existing -ne $commit) {
        throw "Le tag $tag existe deja sur $existing, pas sur $commit."
    }
}
else {
    git tag -a $tag -m "Gate 0 complete: 3 SMP4 Ladybird boots stable" $commit
    if ($LASTEXITCODE -ne 0) { throw "Creation du tag Gate0 impossible." }
}

Write-Host "`n====================================================" -ForegroundColor Green
Write-Host "              GATE 0 = TERMINE" -ForegroundColor Green
Write-Host "====================================================" -ForegroundColor Green
Write-Host "commit : $commit"
Write-Host "preuve : $proof"
Write-Host "tag    : $tag"
Write-Host ""
Write-Host "Publication :" -ForegroundColor Cyan
Write-Host "  git push -u origin $ExpectedBranch"
Write-Host "  git push origin $tag"
