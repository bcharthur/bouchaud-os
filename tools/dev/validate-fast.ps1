param(
    [switch]$Bootimage
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Tmp = Join-Path $env:TEMP ("bouchaud-fast-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
$oldLocation = Get-Location
$oldHello = $env:BO_HELLO_EXE

function Add-Result([string]$Name, [bool]$Ok, [double]$Seconds, [string]$Detail = "") {
    $script:results.Add([pscustomobject]@{
        Test = $Name
        Status = if ($Ok) { "PASS" } else { "FAIL" }
        Seconds = [math]::Round($Seconds, 2)
        Detail = $Detail
    }) | Out-Null
}

function Invoke-Step([string]$Name, [scriptblock]$Action) {
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $Action
        if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {
            throw "$Name a retourne le code $LASTEXITCODE"
        }
        $sw.Stop()
        Add-Result $Name $true $sw.Elapsed.TotalSeconds
    }
    catch {
        $sw.Stop()
        Add-Result $Name $false $sw.Elapsed.TotalSeconds $_.Exception.Message
        throw
    }
}

function Invoke-RustTest([string]$Name, [string]$Source, [switch]$SingleThread) {
    $exe = Join-Path $Tmp (([IO.Path]::GetFileNameWithoutExtension($Source)) + ".exe")
    Invoke-Step "$Name / compile" {
        & rustc --edition 2021 --test -o $exe $Source
    }
    Invoke-Step $Name {
        if ($SingleThread) {
            & $exe --test-threads=1
        } else {
            & $exe
        }
    }
}

try {
    Set-Location $Root
    Write-Host "Bouchaud OS - validation rapide" -ForegroundColor Green
    Write-Host "repo   : $Root"
    Write-Host "commit : $(& git rev-parse --short HEAD)"

    Invoke-RustTest "clavier" "tools\gui\test_clavier.rs"
    Invoke-RustTest "damage" "tools\gui\test_degats.rs" -SingleThread
    Invoke-RustTest "protocole GUI" "tools\gui\test_protocole.rs"
    Invoke-RustTest "BKL max/provenance" "tools\smp\test_bkl_max.rs"
    Invoke-RustTest "commutation SMP" "tools\smp\test_commutation.rs"

    $env:BO_HELLO_EXE = Join-Path $Tmp "bo-hello.exe"
    Invoke-Step "fixture PE32+" {
        & python .\tools\exec\fabrique-hello-exe.py $env:BO_HELLO_EXE
    }
    Invoke-RustTest "formats ELF/PE" "tools\exec\test_format.rs"

    Invoke-Step "manifeste polices" {
        & python .\tools\gui\verifie-polices.py
    }

    if (Test-Path ".\tools\verifie-protocole-gui.py") {
        Invoke-Step "coherence protocole GUI" {
            & python .\tools\verifie-protocole-gui.py
        }
    }

    Invoke-Step "git diff --check" {
        & git diff --check
    }

    Invoke-Step "cargo check" {
        & cargo check
    }

    if ($Bootimage) {
        Invoke-Step "cargo bootimage" {
            & cargo bootimage
        }
    }
}
catch {
    Write-Host "`nECHEC: $($_.Exception.Message)" -ForegroundColor Red
}
finally {
    if ($oldHello) { $env:BO_HELLO_EXE = $oldHello } else { Remove-Item Env:BO_HELLO_EXE -ErrorAction SilentlyContinue }
    Set-Location $oldLocation
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue

    Write-Host "`n=== RESUME ===" -ForegroundColor Cyan
    $results | Format-Table -AutoSize
    $failed = @($results | Where-Object Status -eq "FAIL")
    if ($failed.Count -gt 0) {
        Write-Host "VALIDATION: ECHEC ($($failed.Count) etape(s))" -ForegroundColor Red
        exit 1
    }
    Write-Host "VALIDATION: OK" -ForegroundColor Green
    exit 0
}
