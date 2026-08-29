param(
    [ValidateRange(30,300)]
    [int]$RuntimeSeconds = 90,

    [ValidateRange(2,20)]
    [int]$DrainAfterPanicSeconds = 8
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

$ExpectedBranch = "perf/native-gui-ng"
$ExpectedHead = "69273d5"

function Get-FreeTcpPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { return ([Net.IPEndPoint]$listener.LocalEndpoint).Port }
    finally { $listener.Stop() }
}

function Get-QemuPids {
    @(Get-Process -Name "qemu-system-x86_64" -ErrorAction SilentlyContinue |
      ForEach-Object Id)
}

function Stop-NewQemu([int[]]$Before) {
    foreach ($qemuProcessId in @(Get-QemuPids | Where-Object { $_ -notin $Before })) {
        Stop-Process -Id $qemuProcessId -Force -ErrorAction SilentlyContinue
    }
}

function Connect-Serial([int]$Port, [Diagnostics.Process]$Child, [datetime]$Deadline) {
    while ((Get-Date) -lt $Deadline) {
        if ($Child.HasExited) {
            throw "run.ps1 s'est termine avant l'ouverture du canal serie."
        }
        $client = New-Object Net.Sockets.TcpClient
        try {
            $client.Connect("127.0.0.1", $Port)
            return $client
        }
        catch {
            $client.Dispose()
            Start-Sleep -Milliseconds 200
        }
    }
    throw "QEMU n'a pas ouvert son canal serie TCP."
}

Write-Host "====================================================" -ForegroundColor Cyan
Write-Host " GATE 1A - CAPTURE BKL FLIGHT RECORDER" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan

$branch = (& git branch --show-current).Trim()
$head = (& git rev-parse --short HEAD).Trim()

if ($branch -ne $ExpectedBranch) {
    throw "Branche '$branch', attendue '$ExpectedBranch'."
}
if ($head -ne $ExpectedHead) {
    throw "HEAD=$head, attendu=$ExpectedHead. Ce collecteur cible exactement le Gate1A fautif."
}
if (@(git status --porcelain --untracked-files=no).Count -gt 0) {
    throw "Des fichiers suivis sont modifies. Aucun reset/clean automatique."
}

$existingQemu = @(Get-QemuPids)
if ($existingQemu.Count -gt 0) {
    throw "QEMU deja actif ($($existingQemu -join ', ')). Ferme-le avant cette capture."
}

$port = Get-FreeTcpPort
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$serialLog = Join-Path $Root "gate1a-bkl-flight-$stamp.serial.log"
$hostOut = Join-Path $Root "gate1a-bkl-flight-$stamp.host.out.log"
$hostErr = Join-Path $Root "gate1a-bkl-flight-$stamp.host.err.log"
$beforeQemu = @(Get-QemuPids)

$args = @(
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

Write-Host "commit : $head"
Write-Host "serie  : 127.0.0.1:$port"
Write-Host "log    : $serialLog"
Write-Host ""
Write-Host "Le collecteur NE coupe PAS QEMU au premier panic." -ForegroundColor Yellow
Write-Host "Il attend [BKL-FR] fin ou $DrainAfterPanicSeconds s apres le panic." -ForegroundColor Yellow

$child = Start-Process `
    -FilePath "powershell.exe" `
    -ArgumentList $args `
    -RedirectStandardOutput $hostOut `
    -RedirectStandardError $hostErr `
    -PassThru

$tcp = $null
$stream = $null
$writer = $null
$panicSeenAt = $null
$flightStarted = $false
$flightFinished = $false
$allTextTail = ""

try {
    $tcp = Connect-Serial $port $child ((Get-Date).AddSeconds(300))
    $stream = $tcp.GetStream()

    Write-Host "SERIE CONNECTEE : boot." -ForegroundColor Green

    $writer = New-Object IO.StreamWriter(
        $serialLog,
        $false,
        [Text.UTF8Encoding]::new($false)
    )
    $writer.AutoFlush = $true

    $buffer = New-Object byte[] 16384
    $encoding = [Text.Encoding]::UTF8
    $deadline = (Get-Date).AddSeconds($RuntimeSeconds)

    while ((Get-Date) -lt $deadline) {
        $readSomething = $false

        while ($stream.DataAvailable) {
            $n = $stream.Read($buffer, 0, $buffer.Length)
            if ($n -le 0) { break }

            $readSomething = $true
            $chunk = $encoding.GetString($buffer, 0, $n)
            $writer.Write($chunk)

            $allTextTail += $chunk
            if ($allTextTail.Length -gt 262144) {
                $allTextTail = $allTextTail.Substring($allTextTail.Length - 262144)
            }

            if (-not $panicSeenAt -and $allTextTail.Contains("*** KERNEL PANIC ***")) {
                $panicSeenAt = Get-Date
                Write-Host ""
                Write-Host "PANIC DETECTE - QEMU reste ouvert pour vider l'enregistreur BKL..." -ForegroundColor Red
            }

            if (-not $flightStarted -and $allTextTail.Contains("[BKL-FR]")) {
                $flightStarted = $true
                Write-Host "BKL-FR commence." -ForegroundColor Yellow
            }

            if ($allTextTail.Contains("[BKL-FR] fin")) {
                $flightFinished = $true
                break
            }
        }

        if ($flightFinished) { break }

        if ($panicSeenAt) {
            $elapsed = ((Get-Date) - $panicSeenAt).TotalSeconds
            if ($elapsed -ge $DrainAfterPanicSeconds) {
                break
            }
        }

        if ($child.HasExited -and -not $stream.DataAvailable) { break }
        if (-not $readSomething) { Start-Sleep -Milliseconds 100 }
    }
}
finally {
    if ($writer) {
        try { $writer.Flush(); $writer.Dispose() } catch {}
    }
    if ($stream) { try { $stream.Dispose() } catch {} }
    if ($tcp) { try { $tcp.Dispose() } catch {} }

    Stop-NewQemu $beforeQemu

    if ($child -and -not $child.HasExited) {
        try { [void]$child.WaitForExit(3000) } catch {}
    }
    if ($child -and -not $child.HasExited) {
        Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
    }
}

Write-Host ""
Write-Host "====================================================" -ForegroundColor Cyan
Write-Host " RESULTAT FORENSIC" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan

if (-not $panicSeenAt) {
    Write-Host "Aucun panic reproduit pendant $RuntimeSeconds s." -ForegroundColor Yellow
    Write-Host "Relance le meme script : la course peut etre intermittente."
    Write-Host "log : $serialLog"
    exit 2
}

if (-not $flightStarted) {
    Write-Host "Panic reproduit, mais aucun [BKL-FR] recu." -ForegroundColor Red
    Write-Host "log : $serialLog"
    Write-Host ""
    Write-Host "Dernieres 100 lignes :"
    Get-Content $serialLog -Tail 100
    exit 3
}

Write-Host "Panic reproduit + enregistreur BKL capture." -ForegroundColor Green
Write-Host "flight complet : $flightFinished"
Write-Host "log : $serialLog"
Write-Host ""

$lines = Get-Content $serialLog
$start = -1
for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    if ($lines[$i] -match '^\[.*\]\s+\[BKL-FR\].*transitions') {
        $start = $i
        break
    }
    if ($lines[$i] -match '\[BKL-FR\].*transitions') {
        $start = $i
        break
    }
}

if ($start -lt 0) {
    # Repli : premiere ligne BKL-FR trouvee.
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '\[BKL-FR\]') {
            $start = $i
            break
        }
    }
}

if ($start -ge 0) {
    Write-Host "=== BKL FLIGHT RECORDER ===" -ForegroundColor Yellow
    $end = [Math]::Min($lines.Count - 1, $start + 90)
    for ($i = $start; $i -le $end; $i++) {
        $lines[$i]
        if ($lines[$i] -match '\[BKL-FR\] fin') { break }
    }
}

Write-Host ""
Write-Host "=== Transition finale utile ===" -ForegroundColor Yellow
$lines |
    Where-Object { $_ -match '\[BKL-FR\].*(GUARD_DROP|RELEASE|SUSPEND|RESUME_BEGIN|RESUME_OK|SWITCH_)' } |
    Select-Object -Last 20

Write-Host ""
Write-Host "Copie-moi la section 'BKL FLIGHT RECORDER' ci-dessus." -ForegroundColor Cyan
