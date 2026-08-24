param(
    [string]$Baseline = "",
    [int]$RamMiB = 12288,
    [string]$Accel = "tcg"
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $Root

cargo bootimage
if ($LASTEXITCODE -ne 0) { throw "cargo bootimage failed ($LASTEXITCODE)" }

foreach ($Cpu in @(1, 2, 4)) {
    $Raw = Join-Path $Root "ng6-cpu$Cpu.raw.log"
    $Log = Join-Path $Root "ng6-cpu$Cpu.log"
    if (Test-Path $Raw) { Remove-Item $Raw -Force }

    # Redirection belongs to a child cmd/powershell process. This avoids PS5
    # translating native stderr records into terminating NativeCommandError.
    $Command = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}" -CpuCount {1} -RamMiB {2} -Accel "{3}" > "{4}" 2>&1' -f `
        (Join-Path $Root "run.ps1"), $Cpu, $RamMiB, $Accel, $Raw
    $Process = Start-Process -FilePath "cmd.exe" -ArgumentList @('/d', '/s', '/c', $Command) -Wait -PassThru

    # Windows PowerShell's default file encoding is not relied upon: decode
    # the child output and explicitly publish UTF-8 for the Python parser.
    $Text = Get-Content -LiteralPath $Raw -Raw
    [IO.File]::WriteAllText($Log, $Text, (New-Object Text.UTF8Encoding($false)))
    Remove-Item $Raw -Force
    if ($Process.ExitCode -ne 0) { throw "SMP$Cpu QEMU run failed ($($Process.ExitCode))" }

    python tools/perf/analyze-smp-log.py $Log
    if ($LASTEXITCODE -ne 0) { throw "analysis failed for $Log" }
}

if ($Baseline -and (Test-Path $Baseline)) {
    python tools/perf/compare-smp-logs.py $Baseline ng6-cpu4.log
    if ($LASTEXITCODE -ne 0) { throw "baseline comparison failed" }
} elseif ($Baseline) {
    Write-Warning "Baseline '$Baseline' does not exist; comparison skipped."
}

Write-Host "NG6 validation logs: ng6-cpu1.log ng6-cpu2.log ng6-cpu4.log"
