param(
    [ValidateRange(60,900)][int]$BootTimeoutSeconds = 300,
    [ValidateRange(10,120)][int]$MeasureSeconds = 30
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

function Free-Port {
    $l = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
    $l.Start()
    try { return ([Net.IPEndPoint]$l.LocalEndpoint).Port } finally { $l.Stop() }
}

function Qemu-Pids {
    @(Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue | ForEach-Object Id)
}

$branch = (& git branch --show-current).Trim()
if ($branch -ne "perf/native-gui-ng") { throw "Branche attendue : perf/native-gui-ng" }
if (@(git status --porcelain --untracked-files=no).Count -gt 0) { throw "Arbre suivi non propre." }

$port = Free-Port
$before = @(Qemu-Pids)
$out = Join-Path $Root "gate1a-host.out.log"
$err = Join-Path $Root "gate1a-host.err.log"
$serial = Join-Path $Root "gate1a-serial.log"
Remove-Item $out,$err,$serial -Force -ErrorAction SilentlyContinue

$args = @(
    "-NoProfile","-ExecutionPolicy","Bypass","-File","`"$Root\run.ps1`"",
    "-Ladybird","-Gate0Autostart","-Gate0SerialPort","$port",
    "-CpuCount","4","-Audio","none","-Accel","tcg",
    "-LadybirdUrl","`"https://example.com/`""
)
$child = Start-Process powershell.exe -ArgumentList $args `
    -RedirectStandardOutput $out -RedirectStandardError $err -PassThru

$tcp = $null
$writer = $null
try {
    $deadline = (Get-Date).AddSeconds(300)
    while ((Get-Date) -lt $deadline) {
        if ($child.HasExited) { throw "run.ps1 termine avant QEMU." }
        $candidate = New-Object Net.Sockets.TcpClient
        try {
            $candidate.Connect("127.0.0.1",$port)
            $tcp = $candidate
            break
        } catch {
            $candidate.Dispose()
            Start-Sleep -Milliseconds 250
        }
    }
    if (-not $tcp) { throw "Canal serie QEMU absent." }

    $stream = $tcp.GetStream()
    $buf = New-Object byte[] 16384
    $enc = [Text.Encoding]::UTF8
    $writer = New-Object IO.StreamWriter($serial,$false,[Text.UTF8Encoding]::new($false))
    $writer.AutoFlush = $true
    $tail = ""
    $loadedAt = $null
    $fatal = [regex]::new("(?im)\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|smp_lock: release par un CPU non proprietaire")

    $bootDeadline = (Get-Date).AddSeconds($BootTimeoutSeconds)
    while ((Get-Date) -lt $bootDeadline) {
        while ($stream.DataAvailable) {
            $n = $stream.Read($buf,0,$buf.Length)
            if ($n -le 0) { break }
            $chunk = $enc.GetString($buf,0,$n)
            $writer.Write($chunk)
            $tail += $chunk
            if ($tail.Length -gt 65536) { $tail = $tail.Substring($tail.Length-65536) }
            $bad = $fatal.Match($tail)
            if ($bad.Success) { throw "Fatal runtime : $($bad.Value)" }
            if (-not $loadedAt -and $tail.Contains("M11_DOCUMENT_LOADED")) {
                $loadedAt = Get-Date
                Write-Host "M11 charge. Mesure $MeasureSeconds s..." -ForegroundColor Green
            }
        }
        if ($loadedAt -and ((Get-Date)-$loadedAt).TotalSeconds -ge $MeasureSeconds) { break }
        Start-Sleep -Milliseconds 100
    }

    if (-not $loadedAt) { throw "M11_DOCUMENT_LOADED non atteint." }

    Start-Sleep -Milliseconds 200
    while ($stream.DataAvailable) {
        $n = $stream.Read($buf,0,$buf.Length)
        if ($n -le 0) { break }
        $writer.Write($enc.GetString($buf,0,$n))
    }

    Write-Host "`n=== Dernieres metriques GUI-DAMAGE ===" -ForegroundColor Cyan
    $damage = @(Select-String -Path $serial -Pattern "\[GUI-DAMAGE\]" | Select-Object -Last 5)
    if ($damage.Count -eq 0) { throw "Aucune ligne GUI-DAMAGE dans le log." }
    $damage | ForEach-Object { $_.Line }

    Write-Host "`nGate 1A runtime : PASS" -ForegroundColor Green
    Write-Host "log : $serial"
}
finally {
    if ($writer) { try { $writer.Dispose() } catch {} }
    if ($tcp) { try { $tcp.Dispose() } catch {} }

    foreach ($qemuProcessId in @(Qemu-Pids | Where-Object { $_ -notin $before })) {
        Stop-Process -Id $qemuProcessId -Force -ErrorAction SilentlyContinue
    }
    if ($child -and -not $child.HasExited) {
        try { [void]$child.WaitForExit(5000) } catch {}
    }
    if ($child -and -not $child.HasExited) {
        Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
    }
}
